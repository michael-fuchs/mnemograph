use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use mnemograph_core::Knowledge;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the HTTP server with the MCP endpoint at /mcp.
    Serve {
        /// Path of the SQLite database; created if missing.
        #[arg(long, env = "MNEMOGRAPH_DB", default_value = "mnemograph.db")]
        db: PathBuf,
        /// Address to listen on.
        #[arg(long, env = "MNEMOGRAPH_LISTEN", default_value = "127.0.0.1:8080")]
        listen: SocketAddr,
        /// Allowed page types, comma-separated.
        #[arg(
            long,
            env = "MNEMOGRAPH_PAGE_TYPES",
            value_delimiter = ',',
            default_values_t = mnemograph_core::DEFAULT_PAGE_TYPES.iter().map(|t| t.to_string())
        )]
        page_types: Vec<String>,
    },
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

    match Cli::parse().command {
        Command::Serve {
            db,
            listen,
            page_types,
        } => serve(db, listen, page_types).await,
        Command::Export { dir, db } => export(db, dir),
        Command::Delete { what, db, yes } => delete(db, what, yes),
        Command::Reindex { db } => reindex(db),
    }
}

/// Opens an existing database; unlike `serve`, the CLI commands never create one.
fn open_existing(db: &Path) -> anyhow::Result<Knowledge> {
    anyhow::ensure!(db.exists(), "database {} does not exist", db.display());
    Knowledge::open(db).with_context(|| format!("opening database {}", db.display()))
}

fn export(db: PathBuf, dir: PathBuf) -> anyhow::Result<()> {
    let summary = open_existing(&db)?.export(&dir)?;
    println!(
        "exported to {}: pages: {}, notes: {}",
        dir.display(),
        summary.pages,
        summary.notes
    );
    Ok(())
}

fn delete(db: PathBuf, what: DeleteTarget, yes: bool) -> anyhow::Result<()> {
    let knowledge = open_existing(&db)?;
    match what {
        DeleteTarget::Page { title } => {
            let page = knowledge
                .page(&title)?
                .with_context(|| format!("no page titled {title:?}"))?;
            let backlinks = knowledge.links(&title)?.backlinks.len();
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

fn reindex(db: PathBuf) -> anyhow::Result<()> {
    let knowledge = open_existing(&db)?;
    knowledge.rebuild_search_index()?;
    tracing::info!(db = %db.display(), "search index rebuilt");
    Ok(())
}

async fn serve(db: PathBuf, listen: SocketAddr, page_types: Vec<String>) -> anyhow::Result<()> {
    // There is no authentication yet, so the server must not be reachable from other machines.
    if !listen.ip().is_loopback() {
        bail!(
            "refusing to listen on {listen}: only loopback addresses are allowed until authentication exists"
        );
    }

    let knowledge = Knowledge::open(&db)
        .with_context(|| format!("opening database {}", db.display()))?
        .with_page_types(page_types);
    let shutdown = CancellationToken::new();
    let app = mnemograph::app(Arc::new(knowledge), shutdown.child_token());

    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .with_context(|| format!("binding {listen}"))?;
    tracing::info!(db = %db.display(), "listening on http://{listen}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            shutdown.cancel();
        })
        .await?;
    Ok(())
}
