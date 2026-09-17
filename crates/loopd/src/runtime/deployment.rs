use std::fs::File;
use std::io::Read;
use std::net::SocketAddr;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rustix::fs::{Mode, OFlags, open};
use serde::Deserialize;
use tonic::transport::{Certificate, Identity as TlsIdentity, ServerTlsConfig};

use super::{
    ArtifactBroker, DataPin, FactorExecutor, Identity, JobPin, PortfolioExecutor, RuntimeAuthority,
};
use crate::manifests::{EvaluationPin, EvaluationResolver, PortfolioPin};
use crate::store::{StoreError, StoreResult, SystemClock};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Configuration {
    schema: String,
    bind: SocketAddr,
    server_certificate_file: PathBuf,
    server_key_file: PathBuf,
    client_ca_file: PathBuf,
    identities: Vec<Identity>,
    jobs: Vec<JobPin>,
    development_store: PathBuf,
    protected_store: PathBuf,
    view_store: PathBuf,
    data: Vec<DataPin>,
    #[serde(default)]
    evaluation: Option<EvaluationConfiguration>,
    #[serde(default)]
    portfolio: Option<PortfolioConfiguration>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EvaluationConfiguration {
    python: PathBuf,
    output_store: PathBuf,
    contexts: Vec<EvaluationPin>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PortfolioConfiguration {
    python: PathBuf,
    output_store: PathBuf,
    jobs: Vec<PortfolioPin>,
}

/// Explicit startup-only runtime configuration. It deliberately has no Debug
/// implementation; TLS private-key bytes must not reach telemetry.
pub struct RuntimeDeployment {
    /// Listener selected by trusted deployment configuration, never RPC input.
    pub bind: SocketAddr,
    /// Shared actor/job policy for transport and transactions.
    pub authority: Arc<RuntimeAuthority>,
    /// Broker owning distinct source and job-view namespaces.
    pub artifacts: Arc<ArtifactBroker>,
    /// Optional fixed numerical implementation; absent means fail closed.
    pub evaluator: Option<Arc<FactorExecutor>>,
    /// Optional portfolio execution and current-result reconstruction policy.
    pub portfolio: Option<Arc<PortfolioExecutor>>,
    tls: ServerTlsConfig,
}

impl RuntimeDeployment {
    /// Load bounded JSON and mTLS material through no-follow regular-file opens.
    /// Configuration and certificates must not be group/world writable; keys
    /// must additionally be private. Every file must be owned by this runtime
    /// UID or root. Missing client CA/key/namespace mappings deny startup. This
    /// never provisions credentials, creates directories or mutates a database.
    pub fn load(path: &Path) -> StoreResult<Self> {
        let bytes = read_file(path, false, 1_048_576)?;
        let config: Configuration = serde_json::from_slice(&bytes)
            .map_err(|_| StoreError::Invalid("runtime configuration JSON"))?;
        if config.schema != "loop.runtime/v1" || config.bind.port() == 0 {
            return Err(StoreError::Invalid("runtime configuration version or bind"));
        }
        let certificate = read_file(&config.server_certificate_file, false, 131_072)?;
        let key = read_file(&config.server_key_file, true, 131_072)?;
        let ca = read_file(&config.client_ca_file, false, 131_072)?;
        let authority = Arc::new(RuntimeAuthority::new(
            config.identities,
            config.jobs,
            Arc::new(SystemClock),
        )?);
        let artifacts = Arc::new(ArtifactBroker::open(
            &config.development_store,
            &config.protected_store,
            &config.view_store,
            config.data,
        )?);
        let portfolio = config
            .portfolio
            .map(|portfolio| {
                let mut paths = vec![
                    &config.development_store,
                    &config.protected_store,
                    &config.view_store,
                ];
                if let Some(evaluation) = &config.evaluation {
                    paths.push(&evaluation.output_store);
                }
                if paths.iter().any(|path| {
                    portfolio.output_store.starts_with(path)
                        || path.starts_with(&portfolio.output_store)
                }) {
                    return Err(StoreError::Invalid(
                        "portfolio output overlaps input namespace",
                    ));
                }
                PortfolioExecutor::open(
                    &portfolio.python,
                    &config.development_store,
                    &portfolio.output_store,
                    portfolio.jobs,
                    artifacts.clone(),
                )
                .map(Arc::new)
            })
            .transpose()?;
        let evaluator = config
            .evaluation
            .map(|evaluation| {
                if [
                    &config.development_store,
                    &config.protected_store,
                    &config.view_store,
                ]
                .iter()
                .any(|path| {
                    evaluation.output_store.starts_with(path)
                        || path.starts_with(&evaluation.output_store)
                }) {
                    return Err(StoreError::Invalid(
                        "evaluation output overlaps input namespace",
                    ));
                }
                let resolver = Arc::new(EvaluationResolver::open(
                    &config.development_store,
                    evaluation.contexts,
                )?);
                FactorExecutor::open(&evaluation.python, &evaluation.output_store, resolver)
                    .map(Arc::new)
            })
            .transpose()?;
        Ok(Self {
            bind: config.bind,
            authority,
            artifacts,
            evaluator,
            portfolio,
            tls: ServerTlsConfig::new()
                .identity(TlsIdentity::from_pem(certificate, key))
                .client_ca_root(Certificate::from_pem(ca)),
        })
    }

    /// Return an mTLS configuration that always requires a verified client chain.
    /// Cloning retains sensitive key bytes only inside tonic's TLS configuration.
    pub fn tls(&self) -> ServerTlsConfig {
        self.tls.clone()
    }
}

fn read_file(path: &Path, private: bool, maximum: u64) -> StoreResult<Vec<u8>> {
    if !path.is_absolute() || std::fs::canonicalize(path).ok().as_deref() != Some(path) {
        return Err(StoreError::Invalid("runtime configuration path"));
    }
    let descriptor = open(
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| StoreError::Unavailable("runtime configuration file"))?;
    let mut file = File::from(descriptor);
    let metadata = file
        .metadata()
        .map_err(|_| StoreError::Unavailable("runtime configuration metadata"))?;
    let uid = rustix::process::geteuid().as_raw();
    if !metadata.is_file()
        || metadata.size() == 0
        || metadata.size() > maximum
        || metadata.mode() & (if private { 0o077 } else { 0o022 }) != 0
        || (metadata.uid() != 0 && metadata.uid() != uid)
    {
        return Err(StoreError::Invalid(
            "runtime configuration permissions or bounds",
        ));
    }
    let before = (
        metadata.dev(),
        metadata.ino(),
        metadata.size(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec(),
    );
    let mut bytes = Vec::new();
    (&mut file)
        .take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| StoreError::Unavailable("runtime configuration read"))?;
    for metadata in [file.metadata()?, std::fs::symlink_metadata(path)?] {
        if !metadata.is_file()
            || bytes.len() as u64 != metadata.size()
            || before
                != (
                    metadata.dev(),
                    metadata.ino(),
                    metadata.size(),
                    metadata.mtime(),
                    metadata.mtime_nsec(),
                    metadata.ctime(),
                    metadata.ctime_nsec(),
                )
        {
            return Err(StoreError::Corrupt("runtime configuration changed"));
        }
    }
    Ok(bytes)
}
