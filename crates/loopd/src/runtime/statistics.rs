//! Fixed global-statistics producer; population authority remains in PostgreSQL.

use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use loop_protocol::wire::v1::{JobRecord, JobSpecification, JobSuccess, job_specification};
use serde::{Deserialize, Serialize};

use super::{PortfolioExecutor, process};
use crate::manifests::files::ReadBudget;
use crate::manifests::portfolio::PreparedPortfolio;
use crate::manifests::statistics::{
    GlobalPortfolio, GlobalSnapshot, StatisticsDocument, StatisticsEvidence, StatisticsPolicy,
    StatisticsWork, is_statistics,
};
use crate::manifests::{LocalArtifacts, ObjectRef, model};
use crate::store::{StoreError, StoreResult, TrialSnapshot};

/// Optional private output namespace for global reports. Deployment-owned job
/// pins bind each exact report recipe and frozen input policy.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatisticsConfig {
    /// Existing private CAS, disjoint from every data and worker namespace.
    pub output_store: PathBuf,
}

/// Installed offline numerical supervisor; cannot grant database or holdout access.
pub struct StatisticsExecutor {
    config: StatisticsConfig,
    primary: Arc<PortfolioExecutor>,
    source: LocalArtifacts,
    outputs: LocalArtifacts,
    identity: (u64, u64),
    permits: tokio::sync::Semaphore,
}

pub(crate) struct StatisticsTask<'a> {
    pub job: &'a JobSpecification,
    pub lease: &'a str,
    pub started_ms: i64,
    pub snapshot: TrialSnapshot,
    pub portfolios: Vec<(JobRecord, Arc<PreparedPortfolio>)>,
    pub prior: Option<&'a JobSuccess>,
}

impl StatisticsExecutor {
    /// Open explicitly configured local namespaces without creating files.
    /// Reject shared, relative or replaced roots. The deployment loader also
    /// enforces separation from protected data, views and other worker outputs.
    pub fn open(config: StatisticsConfig, primary: Arc<PortfolioExecutor>) -> StoreResult<Self> {
        let path = &config.output_store;
        let metadata = std::fs::symlink_metadata(path)?;
        if !path.is_absolute()
            || std::fs::canonicalize(path).ok().as_ref() != Some(path)
            || !metadata.is_dir()
            || metadata.mode() & 0o777 != 0o700
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || [&primary.evidence, &primary.output]
                .iter()
                .any(|source| super::reconciliation::overlaps(source, path))
        {
            return Err(StoreError::Invalid("statistics output namespace"));
        }
        Ok(Self {
            source: LocalArtifacts::open(&primary.evidence)?,
            outputs: LocalArtifacts::open(path)?,
            identity: (metadata.dev(), metadata.ino()),
            config,
            primary,
            permits: tokio::sync::Semaphore::new(1),
        })
    }

    fn check_output(&self) -> StoreResult<()> {
        let metadata = std::fs::symlink_metadata(&self.config.output_store)?;
        if !metadata.is_dir()
            || metadata.mode() & 0o777 != 0o700
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || (metadata.dev(), metadata.ino()) != self.identity
        {
            return Err(StoreError::Corrupt("statistics output replaced"));
        }
        Ok(())
    }

    pub(crate) async fn identity(&self, success: &JobSuccess) -> StoreResult<StatisticsDocument> {
        self.check_output()?;
        let reference = report_reference(success)?;
        let file = self
            .outputs
            .load(&reference, true, &mut ReadBudget::new())
            .await?;
        let document: StatisticsDocument = file.json()?;
        crate::store::validate_id(&document.lease_id)?;
        if document.schema != "loop.authorized-global-statistics/v1"
            || document.production_eligible
            || document.started_at_ms < 0
        {
            return Err(StoreError::Corrupt("statistics historical identity"));
        }
        Ok(document)
    }

    pub(crate) async fn execute(
        &self,
        task: StatisticsTask<'_>,
    ) -> StoreResult<StatisticsEvidence> {
        let _permit = self
            .permits
            .try_acquire()
            .map_err(|_| StoreError::Unavailable("statistics capacity"))?;
        tokio::time::timeout(Duration::from_secs(180), self.calculate(task))
            .await
            .map_err(|_| StoreError::Unavailable("statistics deadline"))?
    }

    async fn calculate(&self, task: StatisticsTask<'_>) -> StoreResult<StatisticsEvidence> {
        self.check_output()?;
        let StatisticsTask {
            job,
            lease,
            started_ms,
            snapshot,
            portfolios,
            prior,
        } = task;
        if !is_statistics(job) || portfolios.len() > 64 {
            return Err(StoreError::AdmissionDenied);
        }
        let Some(job_specification::Input::Artifact(input)) = &job.input else {
            return Err(StoreError::AdmissionDenied);
        };
        let artifact = input.input.as_ref().ok_or(StoreError::AdmissionDenied)?;
        let policy = ObjectRef {
            sha256: artifact
                .artifact_id
                .as_ref()
                .ok_or(StoreError::AdmissionDenied)?
                .value
                .clone(),
            byte_size: artifact.byte_size,
        };
        if !(1..=65_536).contains(&policy.byte_size) {
            return Err(StoreError::Invalid("statistics policy bounds"));
        }
        let mut budget = ReadBudget::new();
        let policy_file = self.source.load(&policy, true, &mut budget).await?;
        let settings: StatisticsPolicy = policy_file.json()?;
        settings.validate()?;
        let reference = input.policy.as_ref().ok_or(StoreError::AdmissionDenied)?;
        let digest = policy.digest()?;
        if reference
            .policy_id
            .as_ref()
            .is_none_or(|id| id.value != settings.policy_id)
            || reference.revision != settings.revision
            || reference
                .sha256
                .as_ref()
                .is_none_or(|value| value.value != digest)
            || artifact
                .sha256
                .as_ref()
                .is_none_or(|value| value.value != digest)
        {
            return Err(StoreError::Corrupt("statistics policy binding"));
        }
        let old = match prior {
            Some(value) => Some(self.identity(value).await?),
            None => None,
        };
        if let Some(old) = &old {
            let original = self
                .outputs
                .load(&old.snapshot, true, &mut ReadBudget::new())
                .await?;
            let current = serde_json::to_vec(&GlobalSnapshot::project(&snapshot)?)
                .map_err(|_| StoreError::Corrupt("statistics snapshot encoding"))?;
            if original.bytes()? != current {
                return Err(StoreError::StaleTrials);
            }
        }
        let mut work = StatisticsWork {
            schema: "loop.global-statistics-work/v1",
            job_id: job
                .job_id
                .as_ref()
                .ok_or(StoreError::AdmissionDenied)?
                .value
                .clone(),
            lease_id: lease.to_owned(),
            started_at_ms: old.as_ref().map_or(started_ms, |old| old.started_at_ms),
            policy: policy.clone(),
            snapshot: GlobalSnapshot::project(&snapshot)?,
            portfolios: Vec::with_capacity(portfolios.len()),
            manifest: prior.map(report_reference).transpose()?,
        };
        let mut command = process::command(&self.primary.python, &self.config.output_store);
        command
            .args(["-I", "-m", "loop_research.global_worker", "--evidence"])
            .arg(&self.primary.evidence)
            .arg("--primary-store")
            .arg(&self.primary.output)
            .arg("--output")
            .arg(&self.config.output_store);
        for (record, portfolio) in &portfolios {
            let source = record
                .specification
                .as_ref()
                .ok_or(StoreError::Corrupt("statistics source"))?;
            portfolio.check(source)?;
            let source_lease = portfolio
                .lease_id
                .as_ref()
                .ok_or(StoreError::Corrupt("portfolio lease"))?;
            let manifest = super::portfolio::manifest_artifact(
                portfolio
                    .success
                    .as_ref()
                    .ok_or(StoreError::Corrupt("portfolio success"))?,
            )?;
            let pin = &portfolio.inputs.pin;
            work.portfolios.push(GlobalPortfolio {
                job_id: pin.job_id.clone(),
                evaluation_job_id: pin.evaluation_job_id.clone(),
                lease_id: source_lease.clone(),
                specification: pin.specification.clone(),
                request: pin.request.clone(),
                manifest: manifest.object,
            });
            command.arg("--view").arg(
                self.primary
                    .broker
                    .portfolio_view(source, source_lease)
                    .await?,
            );
        }
        let request = serde_json::to_vec(&work)
            .map_err(|_| StoreError::Corrupt("statistics work encoding"))?;
        let (_, bytes) =
            process::run(&mut command, &request, Duration::from_secs(180), &[0]).await?;
        let report: model::Artifact = serde_json::from_slice(&bytes)
            .map_err(|_| StoreError::Corrupt("statistics worker response"))?;
        if report.schema.name != "loop.authorized_global_statistics"
            || report.schema.version != 1
            || report.media_type != "application/json"
            || report.created_at_ms != work.started_at_ms
        {
            return Err(StoreError::Corrupt("statistics report schema"));
        }
        // Input and output reads each have their own bounded I/O budget. The
        // supervised calculation is bounded separately by the operation/lease.
        let mut budget = ReadBudget::new();
        let mut files = vec![policy_file];
        let schema_file = self
            .outputs
            .load(&report.schema.document, true, &mut budget)
            .await?;
        let schema: model::SchemaDocument = schema_file.json()?;
        if schema.schema != "loop.artifact-schema/v1"
            || schema.name != report.schema.name
            || schema.version != 1
            || schema.media_type != report.media_type
            || !schema.columns.is_empty()
        {
            return Err(StoreError::Corrupt("statistics schema document"));
        }
        files.push(schema_file);
        let file = self.outputs.load(&report.object, true, &mut budget).await?;
        let document: StatisticsDocument = file.json()?;
        if document.schema != "loop.authorized-global-statistics/v1"
            || document.production_eligible
            || document.job_id != work.job_id
            || document.lease_id != work.lease_id
            || document.started_at_ms != work.started_at_ms
            || document.policy != policy
        {
            return Err(StoreError::Corrupt("statistics manifest binding"));
        }
        files.push(file);
        let snapshot_file = self
            .outputs
            .load(&document.snapshot, true, &mut budget)
            .await?;
        if snapshot_file.bytes()?
            != serde_json::to_vec(&work.snapshot)
                .map_err(|_| StoreError::Corrupt("statistics snapshot encoding"))?
        {
            return Err(StoreError::Corrupt("statistics snapshot commitment"));
        }
        files.push(snapshot_file);
        let request_file = self
            .outputs
            .load(&document.request, true, &mut budget)
            .await?;
        let mut expected = serde_json::to_value(&work)
            .map_err(|_| StoreError::Corrupt("statistics request encoding"))?;
        expected
            .as_object_mut()
            .ok_or(StoreError::Corrupt("statistics request shape"))?
            .remove("manifest");
        let actual: serde_json::Value = serde_json::from_slice(request_file.bytes()?)
            .map_err(|_| StoreError::Corrupt("statistics request JSON"))?;
        if actual != expected {
            return Err(StoreError::Corrupt("statistics request commitment"));
        }
        files.push(request_file);
        let summary_file = self
            .outputs
            .load(&document.summary, true, &mut budget)
            .await?;
        let summary = serde_json::from_slice(summary_file.bytes()?)
            .map_err(|_| StoreError::Corrupt("statistics summary JSON"))?;
        files.push(summary_file);
        // The return matrix is data, not metadata. Retain its byte/version guard
        // without materializing it in Rust or returning its rows over RPC.
        files.push(
            self.outputs
                .load(&document.matrix, false, &mut budget)
                .await?,
        );
        self.check_output()?;
        let proof = StatisticsEvidence {
            job: job.clone(),
            snapshot,
            portfolios,
            document,
            summary,
            report,
            files,
        };
        proof.check()?;
        if let Some(original) = prior
            && proof.success()? != *original
        {
            return Err(StoreError::Corrupt("registered statistics outcome"));
        }
        Ok(proof)
    }
}

fn report_reference(success: &JobSuccess) -> StoreResult<ObjectRef> {
    let [artifact] = success.outputs.as_slice() else {
        return Err(StoreError::Corrupt("statistics output set"));
    };
    Ok(ObjectRef {
        sha256: artifact
            .artifact_id
            .as_ref()
            .ok_or(StoreError::Corrupt("statistics artifact ID"))?
            .value
            .clone(),
        byte_size: artifact.byte_size,
    })
}
