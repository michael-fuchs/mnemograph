use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use mnemograph::web::login::LoginSettings;
use mnemograph::{AppConfig, AuthSettings, Authenticator, SphereConfig, WriteLimits};
use mnemograph_core::{Knowledge, Visibility};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
    /// The sphere that pages and notes from before spheres get when the database is upgraded.
    /// Needed only for that upgrade.
    #[arg(long, env = "MNEMOGRAPH_DEFAULT_SPHERE", global = true)]
    default_sphere: Option<String>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the HTTP server with the MCP endpoint at /mcp.
    Serve(Box<ServeArgs>),
    /// Export all pages (as Markdown files) and notes (as JSONL) into a new directory.
    Export {
        /// Directory to write into; must not exist yet or be empty.
        dir: PathBuf,
        /// Path of the SQLite database.
        #[arg(long, env = "MNEMOGRAPH_DB", default_value = "mnemograph.db")]
        db: PathBuf,
    },
    /// Permanently delete a page or a note. Only the owner can delete; agents cannot.
    Delete {
        #[command(subcommand)]
        what: DeleteTarget,
        /// Path of the SQLite database.
        #[arg(
            long,
            env = "MNEMOGRAPH_DB",
            default_value = "mnemograph.db",
            global = true
        )]
        db: PathBuf,
        /// Delete without asking for confirmation.
        #[arg(long, short, global = true)]
        yes: bool,
    },
    /// Rebuild the search index from the database.
    Reindex {
        /// Path of the SQLite database.
        #[arg(long, env = "MNEMOGRAPH_DB", default_value = "mnemograph.db")]
        db: PathBuf,
    },
}

#[derive(clap::Args)]
struct ServeArgs {
    /// Path of the SQLite database; created if missing.
    #[arg(long, env = "MNEMOGRAPH_DB", default_value = "mnemograph.db")]
    db: PathBuf,
    /// Address to listen on.
    #[arg(long, env = "MNEMOGRAPH_LISTEN", default_value = "127.0.0.1:8080")]
    listen: SocketAddr,
    /// The spheres file: mnemograph's part of the central configuration (see dev/spheres.toml).
    #[arg(long, env = "MNEMOGRAPH_SPHERES")]
    spheres: PathBuf,
    /// Allowed page types, comma-separated.
    #[arg(
        long,
        env = "MNEMOGRAPH_PAGE_TYPES",
        value_delimiter = ',',
        default_values_t = mnemograph_core::DEFAULT_PAGE_TYPES.iter().map(|t| t.to_string())
    )]
    page_types: Vec<String>,
    /// Issuer of agents' access tokens (the OIDC provider), exactly as in their `iss` claim.
    /// Without it, authentication is disabled and only loopback addresses are allowed.
    #[arg(long, env = "MNEMOGRAPH_OIDC_ISSUER")]
    oidc_issuer: Option<String>,
    /// The URL under which clients reach mnemograph; `<public-url>/mcp` is the audience
    /// access tokens must have.
    #[arg(
        long,
        env = "MNEMOGRAPH_PUBLIC_URL",
        default_value = "http://localhost:8080"
    )]
    public_url: String,
    /// Scopes an access token must have, comma-separated.
    #[arg(
        long,
        env = "MNEMOGRAPH_REQUIRED_SCOPES",
        value_delimiter = ',',
        default_value = "mnemograph"
    )]
    required_scopes: Vec<String>,
    /// Client ID of the web UI at the OIDC provider (for the owner's browser login).
    #[arg(
        long,
        env = "MNEMOGRAPH_WEB_CLIENT_ID",
        default_value = "mnemograph-web"
    )]
    web_client_id: String,
    /// Secret of the web UI's client, if it is confidential; a public client uses PKCE alone.
    #[arg(long, env = "MNEMOGRAPH_WEB_CLIENT_SECRET", hide_env_values = true)]
    web_client_secret: Option<String>,
    /// Only members of this group at the OIDC provider may log in to the web UI.
    #[arg(
        long,
        env = "MNEMOGRAPH_OWNER_GROUP",
        default_value = "mnemograph_owner"
    )]
    owner_group: String,
    /// Largest page body an agent may write, in bytes.
    #[arg(long, env = "MNEMOGRAPH_MAX_PAGE_BYTES", default_value_t = WriteLimits::default().max_page_bytes)]
    max_page_bytes: usize,
    /// Largest note an agent may write, in bytes.
    #[arg(long, env = "MNEMOGRAPH_MAX_NOTE_BYTES", default_value_t = WriteLimits::default().max_note_bytes)]
    max_note_bytes: usize,
    /// Writes (write_page, add_note) each agent may make per minute.
    #[arg(long, env = "MNEMOGRAPH_WRITES_PER_MINUTE", default_value_t = WriteLimits::default().writes_per_minute)]
    writes_per_minute: u32,
    /// The most pages and stubs the whole-wiki graph shows; the most-linked are kept.
    #[arg(long, env = "MNEMOGRAPH_GRAPH_MAX_NODES", default_value_t = mnemograph::web::DEFAULT_GRAPH_NODES)]
    graph_max_nodes: usize,
    /// Days the access log keeps what agents read and searched (shown in the statistics).
    #[arg(long, env = "MNEMOGRAPH_ACCESS_LOG_DAYS", default_value_t = mnemograph::DEFAULT_ACCESS_LOG_DAYS, value_parser = clap::value_parser!(u32).range(1..))]
    access_log_days: u32,
}

#[derive(Subcommand)]
enum DeleteTarget {
    /// Delete a page with all its revisions and aliases; links to it become stubs.
    Page {
        /// Title or alias of the page.
        title: String,
    },
    /// Delete a single note.
    Note {
        /// ID of the note.
        id: i64,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let Cli {
        command,
        default_sphere,
    } = Cli::parse();
    let default_sphere = default_sphere.as_deref();
    match command {
        Command::Serve(args) => serve(*args, default_sphere).await,
        Command::Export { dir, db } => export(db, dir, default_sphere),
        Command::Delete { what, db, yes } => delete(db, what, yes, default_sphere),
        Command::Reindex { db } => reindex(db, default_sphere),
    }
}

/// Opens a database, upgrading it if needed; unlike `serve`, the CLI commands never create one.
fn open_existing(db: &Path, default_sphere: Option<&str>) -> anyhow::Result<Knowledge> {
    anyhow::ensure!(db.exists(), "database {} does not exist", db.display());
    open(db, default_sphere)
}

fn open(db: &Path, default_sphere: Option<&str>) -> anyhow::Result<Knowledge> {
    Knowledge::open_with_default_sphere(db, default_sphere)
        .with_context(|| format!("opening database {}", db.display()))
}

fn export(db: PathBuf, dir: PathBuf, default_sphere: Option<&str>) -> anyhow::Result<()> {
    let summary = open_existing(&db, default_sphere)?.export(&dir)?;
    println!(
        "exported to {}: pages: {}, notes: {}",
        dir.display(),
        summary.pages,
        summary.notes
    );
    Ok(())
}

fn delete(
    db: PathBuf,
    what: DeleteTarget,
    yes: bool,
    default_sphere: Option<&str>,
) -> anyhow::Result<()> {
    let knowledge = open_existing(&db, default_sphere)?;
    match what {
        DeleteTarget::Page { title } => {
            let page = knowledge
                .page(&title, Visibility::All)?
                .with_context(|| format!("no page titled {title:?}"))?;
            let backlinks = knowledge.links(&title, Visibility::All)?.backlinks.len();
            let mut question = format!(
                "The page {:?} will be deleted with {} revision(s)",
                page.title, page.revision
            );
            if !page.aliases.is_empty() {
                question += &format!(" and the aliases {}", page.aliases.join(", "));
            }
            question.push('.');
            if backlinks > 0 {
                question += &format!(" {backlinks} page(s) link to it; their links become stubs.");
            }
            if yes || confirm(&question)? {
                let deleted = knowledge.delete_page(&page.title)?;
                println!("deleted the page {:?}", deleted.title);
            } else {
                println!("nothing deleted");
            }
        }
        DeleteTarget::Note { id } => {
            if yes || confirm(&format!("The note {id} will be deleted."))? {
                knowledge.delete_note(id)?;
                println!("deleted the note {id}");
            } else {
                println!("nothing deleted");
            }
        }
    }
    Ok(())
}

/// Asks a yes/no question on the terminal; anything but "y" or "yes" means no.
fn confirm(question: &str) -> anyhow::Result<bool> {
    print!("{question} This cannot be undone. Delete? [y/N] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(answer.trim().to_lowercase().as_str(), "y" | "yes"))
}

fn reindex(db: PathBuf, default_sphere: Option<&str>) -> anyhow::Result<()> {
    let knowledge = open_existing(&db, default_sphere)?;
    knowledge.rebuild_search_index()?;
    tracing::info!(db = %db.display(), "search index rebuilt");
    Ok(())
}

async fn serve(args: ServeArgs, default_sphere: Option<&str>) -> anyhow::Result<()> {
    let ServeArgs {
        db,
        listen,
        spheres,
        page_types,
        oidc_issuer,
        public_url,
        required_scopes,
        max_page_bytes,
        max_note_bytes,
        writes_per_minute,
        graph_max_nodes,
        access_log_days,
        web_client_id,
        web_client_secret,
        owner_group,
    } = args;
    let spheres = SphereConfig::load(&spheres)?.names();
    if let Some(default_sphere) = default_sphere {
        anyhow::ensure!(
            spheres.contains(default_sphere),
            "the default sphere {default_sphere:?} is not configured; configured spheres: {}",
            spheres.names().join(", ")
        );
    }
    tracing::info!(spheres = ?spheres.names(), "spheres configured");
    let public_url = reqwest::Url::parse(&public_url)
        .with_context(|| format!("invalid public URL {public_url:?}"))?;
    let public_base = public_url.as_str().trim_end_matches('/').to_owned();

    let auth = match oidc_issuer {
        Some(issuer) => {
            let settings = AuthSettings {
                issuer,
                resource: format!("{public_base}/mcp"),
                required_scopes: required_scopes
                    .into_iter()
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty())
                    .collect(),
                spheres: spheres.clone(),
            };
            let authenticator = Authenticator::discover(settings)
                .await
                .context("setting up authentication")?;
            tracing::info!(
                issuer = %authenticator.settings().issuer,
                resource = %authenticator.settings().resource,
                "agents must authenticate"
            );
            Some(Arc::new(authenticator))
        }
        None => {
            // Without authentication the server must not be reachable from other machines.
            if !listen.ip().is_loopback() {
                bail!(
                    "refusing to listen on {listen} without authentication: set --oidc-issuer or use a loopback address"
                );
            }
            tracing::warn!(
                "authentication is disabled (no --oidc-issuer); every agent is anonymous"
            );
            None
        }
    };

    let web_login = auth.as_ref().map(|authenticator| LoginSettings {
        authenticator: authenticator.clone(),
        client_id: web_client_id,
        client_secret: web_client_secret,
        redirect_uri: format!("{public_base}/auth/callback"),
        owner_group,
        secure_cookies: public_url.scheme() == "https",
    });

    let mut allowed_hosts: Vec<String> =
        ["localhost", "127.0.0.1", "::1"].map(String::from).to_vec();
    if let Some(host) = public_url.host_str() {
        allowed_hosts.push(host.to_owned());
    }

    let knowledge = open(&db, default_sphere)?.with_page_types(page_types);
    let shutdown = CancellationToken::new();
    let app = mnemograph::app(
        Arc::new(knowledge),
        AppConfig {
            auth,
            spheres,
            allowed_hosts,
            web_login,
            graph_max_nodes,
            access_log_days,
            limits: WriteLimits {
                max_page_bytes,
                max_note_bytes,
                writes_per_minute,
            },
            shutdown: shutdown.child_token(),
        },
    );

    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .with_context(|| format!("binding {listen}"))?;
    tracing::info!(db = %db.display(), "listening on http://{listen}, reachable as {public_base}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            shutdown.cancel();
        })
        .await?;
    Ok(())
}
