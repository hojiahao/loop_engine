//! Fixed portfolio subprocess and read-only numerical reconstruction.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use loop_protocol::wire::v1::{JobSpecification, JobSuccess};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::ArtifactBroker;
use crate::manifests::files::ReadBudget;
use crate::manifests::portfolio::{
    PortfolioDocument, PortfolioInputs, PortfolioResolver, PortfolioWork, PreparedPortfolio,
};
use crate::manifests::{LocalArtifacts, ObjectRef, PortfolioPin, model};
use crate::store::{BacktestPolicy, BacktestPreparation, StoreError, StoreResult};

/// Deployment-selected portfolio worker. No caller executable, network/provider
/// environment or protected namespace is exposed to its numerical process.
pub struct PortfolioExecutor {
    resolver: PortfolioResolver,
    outputs: LocalArtifacts,
    python: PathBuf,
    evidence: PathBuf,
    output: PathBuf,
    identity: (u64, u64),
    broker: Arc<ArtifactBroker>,
    permits: tokio::sync::Semaphore,
}

impl PortfolioExecutor {
    /// Pin an installed interpreter, private evidence/output roots and exact job
    /// recipes. Output must be disjoint from evidence/data/view/protected roots
    /// (the deployment loader checks all namespaces). Creates nothing; denies
    /// relative, shared, replaced or missing directories. No database effects.
    pub fn open(
        python: &Path,
        evidence: &Path,
        output: &Path,
        pins: Vec<PortfolioPin>,
        broker: Arc<ArtifactBroker>,
    ) -> StoreResult<Self> {
        if !python.is_absolute()
            || !python.is_file()
            || evidence.starts_with(output)
            || output.starts_with(evidence)
        {
            return Err(StoreError::Invalid("portfolio deployment paths"));
        }
        for path in [evidence, output] {
            let metadata = std::fs::symlink_metadata(path)?;
            if !path.is_absolute()
                || std::fs::canonicalize(path).ok().as_deref() != Some(path)
                || !metadata.is_dir()
                || metadata.mode() & 0o777 != 0o700
                || metadata.uid() != rustix::process::geteuid().as_raw()
            {
                return Err(StoreError::Invalid("portfolio namespace ownership"));
            }
        }
        let metadata = std::fs::symlink_metadata(output)?;
        Ok(Self {
            resolver: PortfolioResolver::open(evidence, pins)?,
            outputs: LocalArtifacts::open(output)?,
            python: python.to_owned(),
            evidence: evidence.to_owned(),
            output: output.to_owned(),
            identity: (metadata.dev(), metadata.ino()),
            broker,
            permits: tokio::sync::Semaphore::new(1),
        })
    }

    pub(crate) async fn inputs(&self, job: &JobSpecification) -> StoreResult<PortfolioInputs> {
        self.check_output()?;
        self.resolver.prepare(job).await
    }

    fn check_output(&self) -> StoreResult<()> {
        let metadata = std::fs::symlink_metadata(&self.output)?;
        if !metadata.is_dir()
            || metadata.mode() & 0o777 != 0o700
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || (metadata.dev(), metadata.ino()) != self.identity
        {
            return Err(StoreError::Corrupt("portfolio output replaced"));
        }
        Ok(())
    }

    pub(crate) async fn execute(
        &self,
        inputs: PortfolioInputs,
        work: &PortfolioWork,
        view: &Path,
        timeout: Duration,
    ) -> StoreResult<PreparedPortfolio> {
        inputs.check()?;
        let artifact = self.run(work, view, timeout).await?;
        inputs.check()?;
        self.check_output()?;
        inputs.seal(&self.outputs, &artifact).await
    }

    async fn run(
        &self,
        work: &PortfolioWork,
        view: &Path,
        timeout: Duration,
    ) -> StoreResult<model::Artifact> {
        let _permit = self
            .permits
            .try_acquire()
            .map_err(|_| StoreError::Unavailable("portfolio execution capacity"))?;
        let bytes =
            serde_json::to_vec(work).map_err(|_| StoreError::Corrupt("portfolio work encoding"))?;
        if bytes.len() > 1_048_576 || timeout.is_zero() || timeout > Duration::from_secs(180) {
            return Err(StoreError::Invalid("portfolio worker budget"));
        }
        self.check_output()?;
        tokio::time::timeout(timeout, async {
            let mut child = tokio::process::Command::new(&self.python)
                .args(["-I", "-m", "loop_research.portfolio_worker"])
                .arg("--evidence")
                .arg(&self.evidence)
                .arg("--view")
                .arg(view)
                .arg("--output")
                .arg(&self.output)
                .env_clear()
                .env("OPENBLAS_NUM_THREADS", "1")
                .env("OMP_NUM_THREADS", "1")
                .current_dir(&self.output)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .map_err(|_| StoreError::Unavailable("portfolio worker spawn"))?;
            let mut input = child
                .stdin
                .take()
                .ok_or(StoreError::Unavailable("portfolio stdin"))?;
            let output = child
                .stdout
                .take()
                .ok_or(StoreError::Unavailable("portfolio stdout"))?;
            let write = async {
                input.write_all(&bytes).await?;
                input.shutdown().await?;
                drop(input);
                Ok::<_, std::io::Error>(())
            };
            let read = async {
                let mut bytes = Vec::new();
                output.take(1_048_577).read_to_end(&mut bytes).await?;
                Ok::<_, std::io::Error>(bytes)
            };
            let (_, result) = tokio::try_join!(write, read)
                .map_err(|_| StoreError::Unavailable("portfolio worker transport"))?;
            if result.is_empty()
                || result.len() > 1_048_576
                || !child
                    .wait()
                    .await
                    .map_err(|_| StoreError::Unavailable("portfolio worker exit"))?
                    .success()
            {
                return Err(StoreError::Unavailable(
                    "portfolio worker refused execution",
                ));
            }
            self.check_output()?;
            serde_json::from_slice(&result)
                .map_err(|_| StoreError::Corrupt("portfolio worker output"))
        })
        .await
        .map_err(|_| StoreError::Unavailable("portfolio worker deadline"))?
    }

    pub(crate) async fn replay(
        &self,
        inputs: PortfolioInputs,
        job: &JobSpecification,
        success: &JobSuccess,
    ) -> StoreResult<PreparedPortfolio> {
        let artifact = manifest_artifact(success)?;
        let file = self
            .outputs
            .load(&artifact.object, true, &mut ReadBudget::new())
            .await?;
        let document: PortfolioDocument = file.json()?;
        let work = inputs.work(
            &document.lease_id,
            document.trials,
            Some(artifact.object.clone()),
        );
        let view = self.broker.portfolio_view(job, &document.lease_id).await?;
        let prepared = self
            .execute(inputs, &work, &view, Duration::from_secs(180))
            .await?;
        file.check()?;
        if prepared.success.as_ref() != Some(success) {
            return Err(StoreError::Corrupt("portfolio replay outputs"));
        }
        Ok(prepared)
    }
}

impl BacktestPolicy for PortfolioExecutor {
    fn prepare<'a>(
        &'a self,
        job: &'a JobSpecification,
        context: Option<&'a str>,
        success: Option<&'a JobSuccess>,
    ) -> BacktestPreparation<'a> {
        Box::pin(async move {
            let inputs = self.inputs(job).await?;
            let prepared = match success {
                Some(success) => self.replay(inputs, job, success).await?,
                None => inputs.pending(),
            };
            if let Some(context) = context {
                prepared.resolve_current(
                    &loop_protocol::wire::v1::Actor::default(),
                    job,
                    context,
                )?;
            }
            Ok(Some(Arc::new(prepared) as Arc<dyn BacktestPolicy>))
        })
    }

    fn validate_inputs(&self, _: &JobSpecification) -> StoreResult<()> {
        Err(StoreError::Unavailable("unprepared portfolio inputs"))
    }
}

fn manifest_artifact(success: &JobSuccess) -> StoreResult<model::Artifact> {
    let manifests: Vec<_> = success
        .outputs
        .iter()
        .filter(|artifact| {
            artifact
                .schema
                .as_ref()
                .is_some_and(|schema| schema.name == "loop.authorized_portfolio")
        })
        .collect();
    let [wire] = manifests.as_slice() else {
        return Err(StoreError::AdmissionDenied);
    };
    let schema = model::SchemaDocument {
        schema: "loop.artifact-schema/v1".to_owned(),
        name: "loop.authorized_portfolio".to_owned(),
        version: 1,
        media_type: "application/json".to_owned(),
        columns: vec![],
    };
    let bytes = serde_json::to_vec(&schema).map_err(|_| StoreError::Corrupt("portfolio schema"))?;
    let created = wire
        .created_at
        .as_ref()
        .ok_or(StoreError::Corrupt("portfolio output time"))?;
    let artifact = model::Artifact {
        object: ObjectRef {
            sha256: wire
                .artifact_id
                .as_ref()
                .ok_or(StoreError::Corrupt("portfolio output ID"))?
                .value
                .clone(),
            byte_size: wire.byte_size,
        },
        schema: model::SchemaRef {
            name: schema.name,
            version: 1,
            document: ObjectRef {
                sha256: format!("sha256:{:x}", Sha256::digest(&bytes)),
                byte_size: bytes.len() as u64,
            },
        },
        media_type: schema.media_type,
        created_at_ms: created
            .seconds
            .checked_mul(1000)
            .and_then(|value| value.checked_add(i64::from(created.nanos) / 1_000_000))
            .ok_or(StoreError::Corrupt("portfolio time range"))?,
    };
    if artifact.wire()? != **wire {
        return Err(StoreError::Corrupt("portfolio output reference"));
    }
    Ok(artifact)
}
