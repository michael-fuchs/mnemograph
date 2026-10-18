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
    /// Rebuild the search index from the database.
    Reindex {
        /// Path of the SQLite database.
        #[arg(long, env = "MNEMOGRAPH_DB", default_value = "mnemograph.db")]
        db: PathBuf,
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
