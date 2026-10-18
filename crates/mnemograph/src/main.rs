use std::net::SocketAddr;
use std::path::PathBuf;
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
        Command::Serve { db, listen } => serve(db, listen).await,
    }
}

async fn serve(db: PathBuf, listen: SocketAddr) -> anyhow::Result<()> {
    // There is no authentication yet, so the server must not be reachable from other machines.
    if !listen.ip().is_loopback() {
        bail!(
            "refusing to listen on {listen}: only loopback addresses are allowed until authentication exists"
        );
    }

    let knowledge =
        Knowledge::open(&db).with_context(|| format!("opening database {}", db.display()))?;
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
