use std::io::Read;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use loopd::runtime::{RuntimeDeployment, RuntimeService};
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
    /// Deployment-owned PostgreSQL namespace, shared by migration and runtime.
    #[arg(long, env = "LOOPD_DATABASE_SCHEMA", default_value = "public")]
    database_schema: String,
    /// Apply schema changes using an explicit deployment credential, then exit.
    #[arg(long)]
    migrate: bool,
    /// Verify the configured database and exit without starting an HTTP listener.
    #[arg(long)]
    check_database: bool,
    /// Enable the independent mTLS job endpoint using explicit private deployment configuration.
    #[arg(long, env = "LOOPD_RUNTIME_CONFIG")]
    runtime_config: Option<PathBuf>,
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
    options.schema = args.database_schema;
    options.apply_migrations = args.migrate;
    let runtime = args
        .runtime_config
        .as_deref()
        .map(RuntimeDeployment::load)
        .transpose()
        .context("runtime deployment configuration is unavailable or invalid")?;
    if let Some(runtime) = &runtime {
        options.admission = runtime.authority.clone();
        if let Some(portfolio) = &runtime.portfolio {
            options.backtest_policy = portfolio.clone();
        }
    }
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
    let (listener, runtime_listener) =
        bind_servers(args.bind, runtime.as_ref().map(|runtime| runtime.bind)).await?;
    info!(bind = %listener.local_addr()?, "loopd listening");
    let http = async {
        axum::serve(listener, loopd::app_with_store(store.clone()))
            .with_graceful_shutdown(shutdown_signal())
            .await
            .context("loopd server failed")
    };
    let result = if let Some((runtime, listener)) = runtime.zip(runtime_listener) {
        let tls = runtime.tls();
        let address = listener.local_addr()?;
        let mut service = RuntimeService::new(store.clone(), runtime.authority, runtime.artifacts);
        if let Some(executor) = runtime.evaluator {
            service = service.with_factor_executor(executor);
        }
        if let Some(executor) = runtime.portfolio {
            service = service.with_portfolio_executor(executor);
        }
        if let Some(executor) = runtime.reconciliation {
            service = service.with_reconciler(executor);
        }
        if let Some(executor) = runtime.statistics {
            service = service.with_statistician(executor);
        }
        if let Some(executor) = runtime.discovery {
            service = service.with_discoverer(executor);
        }
        if let Some(catalog) = runtime.runs {
            service = service.with_runs(catalog);
        }
        let rpc = async {
            info!(bind = %address, "loopd authenticated runtime listening");
            loopd::runtime::serve(service, listener, tls, shutdown_signal())
                .await
                .context("loopd runtime failed")
        };
        tokio::try_join!(http, rpc).map(|_| ())
    } else {
        http.await
    };
    store.close().await;
    result
}

async fn bind_servers(
    http: SocketAddr,
    rpc: Option<SocketAddr>,
) -> anyhow::Result<(tokio::net::TcpListener, Option<tokio::net::TcpListener>)> {
    // Reserve the configured RPC address first. An ephemeral HTTP listener must
    // not claim that port between a previous runtime's shutdown and this start.
    let runtime = match rpc {
        Some(address) => Some(
            tokio::net::TcpListener::bind(address)
                .await
                .context("failed to bind authenticated runtime")?,
        ),
        None => None,
    };
    let listener = tokio::net::TcpListener::bind(http)
        .await
        .with_context(|| format!("failed to bind loopd to {http}"))?;
    Ok((listener, runtime))
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate());
        let Ok(mut terminate) = terminate else {
            tracing::error!("failed to install termination signal");
            return;
        };
        tokio::select! {
            signal = tokio::signal::ctrl_c() => {
                if signal.is_err() { tracing::error!("failed to install interrupt signal"); }
            }
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    if tokio::signal::ctrl_c().await.is_err() {
        tracing::error!("failed to install interrupt signal");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ephemeral_separate() {
        let reserved = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = reserved.local_addr().unwrap();
        drop(reserved);
        let (http, rpc) = bind_servers("127.0.0.1:0".parse().unwrap(), Some(address))
            .await
            .unwrap();
        assert_eq!(rpc.unwrap().local_addr().unwrap(), address);
        assert_ne!(http.local_addr().unwrap(), address);
    }

    #[tokio::test]
    async fn collision_reports_http() {
        let reserved = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = reserved.local_addr().unwrap();
        drop(reserved);
        let error = bind_servers(address, Some(address)).await.unwrap_err();
        // RPC owns the port before HTTP is attempted; a collision never changes
        // or retries either configured endpoint and leaves no listener behind.
        assert_eq!(
            error.to_string(),
            format!("failed to bind loopd to {address}")
        );
        tokio::net::TcpListener::bind(address).await.unwrap();
    }
}
