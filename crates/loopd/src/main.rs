use std::io::Read;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use loopd::store::{PgJobStore, StoreOptions};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "loopd", version, about = "Loop Engine control plane")]
struct Args {
    #[arg(long, env = "LOOPD_BIND", default_value = "127.0.0.1:8080")]
    bind: SocketAddr,
    #[arg(
        long,
        env = "LOOPD_DATABASE_URL_FILE",
        default_value = "var/secrets/loopd-database-url"
    )]
    database_url_file: PathBuf,
    /// Apply schema changes using an explicit deployment credential, then exit.
    #[arg(long)]
    migrate: bool,
    /// Verify the configured database and exit without starting an HTTP listener.
    #[arg(long)]
    check_database: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .json()
        .init();

    let mut file = std::fs::File::open(&args.database_url_file)
        .context("private database connection file is unavailable")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        anyhow::ensure!(
            file.metadata()?.permissions().mode() & 0o077 == 0,
            "database connection file must not be group/world accessible"
        );
    }
    let mut url = String::new();
    (&mut file).take(16_385).read_to_string(&mut url)?;
    anyhow::ensure!(url.len() <= 16_384, "database connection file is too large");
    let mut options = StoreOptions::new(url.trim())?;
    options.apply_migrations = args.migrate;
    let store = PgJobStore::open(options)
        .await
        .context("failed to open durable state")?;
    store
        .verify_configuration()
        .await
        .context("durable state is not ready")?;
    if args.migrate || args.check_database {
        info!("PostgreSQL TLS and schema verification passed");
        store.close().await;
        return Ok(());
    }
    let listener = tokio::net::TcpListener::bind(args.bind)
        .await
        .with_context(|| format!("failed to bind loopd to {}", args.bind))?;
    info!(bind = %args.bind, "loopd listening");
    let result = axum::serve(listener, loopd::app_with_store(store.clone()))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("loopd server failed");
    store.close().await;
    result
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "failed to install shutdown signal");
    }
}
