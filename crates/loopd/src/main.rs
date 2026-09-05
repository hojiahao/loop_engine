use std::net::SocketAddr;

use anyhow::Context;
use clap::Parser;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "loopd", version, about = "Loop Engine control plane")]
struct Args {
    #[arg(long, env = "LOOPD_BIND", default_value = "127.0.0.1:8080")]
    bind: SocketAddr,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .json()
        .init();

    let listener = tokio::net::TcpListener::bind(args.bind)
        .await
        .with_context(|| format!("failed to bind loopd to {}", args.bind))?;
    info!(bind = %args.bind, "loopd listening");
    axum::serve(listener, loopd::app())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("loopd server failed")
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "failed to install shutdown signal");
    }
}
