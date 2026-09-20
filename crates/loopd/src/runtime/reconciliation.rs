//! Two real independent processes, bound to one registered primary result.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use loop_protocol::wire::v1::{
    JobRecord, JobSpecification, JobSuccess, job_outcome, job_specification,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{PortfolioExecutor, process};
use crate::manifests::files::{ReadBudget, VerifiedFile};
use crate::manifests::portfolio::PreparedPortfolio;
use crate::manifests::reconciliation::{
    AlphalensReceipt, ComparisonPolicy, PreparedHandle, PreparedInputs, ValidationDocument,
    ValidationEvidence, ValidationWork, WorkerHandle, ZiplineReceipt, source,
};
use crate::manifests::{LocalArtifacts, ObjectRef, model};
use crate::store::{StoreError, StoreResult};

/// One deployment-owned validation recipe; no caller may select its source or policy.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationPin {
    /// Reconciliation job already authorized by the runtime identity registry.
    pub job_id: String,
    /// Exact primary job; unique in this deployment, so callers cannot cherry-pick reports.
    pub primary_job_id: String,
    /// Immutable reviewed comparison policy in the development evidence store.
    pub policy: ObjectRef,
}

/// Offline installed validator launch configuration. Paths are administrative,
/// never RPC input; the main interpreter is inherited from the portfolio producer.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconciliationConfig {
    /// Pinned uv executable; no shell or caller command is executed.
    pub uv: PathBuf,
    /// Exact secondary Python 3.12.13 interpreter for the Zipline lock.
    pub zipline_python: PathBuf,
    /// Independent Alphalens project with its committed uv lock.
    pub alphalens_project: PathBuf,
    /// Independent Zipline project with its committed uv lock.
    pub zipline_project: PathBuf,
    /// Preinstalled offline uv cache, disjoint from all research namespaces.
    pub cache: PathBuf,
    /// Separate existing private CAS for raw exports and independent results.
    pub output_store: PathBuf,
    /// Exact bounded job/source/policy mappings.
    pub jobs: Vec<ValidationPin>,
}

/// Bounded supervisor. This produces evidence but grants no database authority.
pub struct ReconciliationExecutor {
    config: ReconciliationConfig,
    pins: BTreeMap<String, ValidationPin>,
    primary: Arc<PortfolioExecutor>,
    source: LocalArtifacts,
    outputs: LocalArtifacts,
    identity: (u64, u64),
    permits: tokio::sync::Semaphore,
}

pub(crate) struct ValidationTask<'a> {
    pub job: &'a JobSpecification,
    pub lease: &'a str,
    pub record: JobRecord,
    pub primary: Arc<PreparedPortfolio>,
    pub prior: Option<&'a JobSuccess>,
    pub started_ms: i64,
}

impl ReconciliationExecutor {
    /// Validate an explicitly enabled deployment without creating files or
    /// connecting to a database. Paths must be trusted and output must be private;
    /// the deployment loader also checks protected/data/view namespace separation.
    /// Missing interpreters, locks, references or ambiguous pins deny startup.
    pub fn open(
        config: ReconciliationConfig,
        primary: Arc<PortfolioExecutor>,
    ) -> StoreResult<Self> {
        for path in [&config.uv, &config.zipline_python] {
            if !path.is_absolute() || !path.is_file() {
                return Err(StoreError::Invalid("validator executable"));
            }
        }
        for path in [
            &config.alphalens_project,
            &config.zipline_project,
            &config.cache,
            &config.output_store,
        ] {
            let metadata = std::fs::symlink_metadata(path)?;
            if !path.is_absolute()
                || std::fs::canonicalize(path).ok().as_deref() != Some(path)
                || !metadata.is_dir()
                || metadata.mode() & 0o022 != 0
                || (metadata.uid() != 0 && metadata.uid() != rustix::process::geteuid().as_raw())
            {
                return Err(StoreError::Invalid("validator deployment path"));
            }
        }
        for project in [&config.alphalens_project, &config.zipline_project] {
            if !project.join("uv.lock").is_file() || !project.join("pyproject.toml").is_file() {
                return Err(StoreError::Invalid("validator lock missing"));
            }
        }
        let metadata = std::fs::symlink_metadata(&config.output_store)?;
        if metadata.mode() & 0o777 != 0o700
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || [
                &primary.evidence,
                &primary.output,
                &config.cache,
                &config.alphalens_project,
                &config.zipline_project,
            ]
            .iter()
            .any(|path| overlaps(path, &config.output_store))
        {
            return Err(StoreError::Invalid("validation output namespace"));
        }
        if config.jobs.is_empty() || config.jobs.len() > 4096 {
            return Err(StoreError::Invalid("validation pin bounds"));
        }
        let mut pins = BTreeMap::new();
        let mut sources = BTreeSet::new();
        for pin in &config.jobs {
            crate::store::validate_id(&pin.job_id)?;
            crate::store::validate_id(&pin.primary_job_id)?;
            pin.policy.digest()?;
            if pin.job_id == pin.primary_job_id
                || !(1..=65_536).contains(&pin.policy.byte_size)
                || !sources.insert(&pin.primary_job_id)
                || pins.insert(pin.job_id.clone(), pin.clone()).is_some()
            {
                return Err(StoreError::Invalid("ambiguous validation pin"));
            }
        }
        Ok(Self {
            source: LocalArtifacts::open(&primary.evidence)?,
            outputs: LocalArtifacts::open(&config.output_store)?,
            config,
            pins,
            primary,
            identity: (metadata.dev(), metadata.ino()),
            permits: tokio::sync::Semaphore::new(1),
        })
    }

    pub(crate) fn for_primary(&self, id: &str) -> Option<&str> {
        self.pins
            .values()
            .find(|pin| pin.primary_job_id == id)
            .map(|pin| pin.job_id.as_str())
    }

    pub(crate) async fn lease(&self, success: &JobSuccess) -> StoreResult<String> {
        self.check_output()?;
        let [artifact] = success.outputs.as_slice() else {
            return Err(StoreError::Corrupt("validation output set"));
        };
        let reference = ObjectRef {
            sha256: artifact
                .artifact_id
                .as_ref()
                .ok_or(StoreError::Corrupt("validation artifact ID"))?
                .value
                .clone(),
            byte_size: artifact.byte_size,
        };
        let file = self
            .outputs
            .load(&reference, true, &mut ReadBudget::new())
            .await?;
        let document: ValidationDocument = file.json()?;
        crate::store::validate_id(&document.lease_id)?;
        Ok(document.lease_id)
    }

    fn check_output(&self) -> StoreResult<()> {
        let metadata = std::fs::symlink_metadata(&self.config.output_store)?;
        if !metadata.is_dir()
            || metadata.mode() & 0o777 != 0o700
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || (metadata.dev(), metadata.ino()) != self.identity
        {
            return Err(StoreError::Corrupt("validation output replaced"));
        }
        Ok(())
    }

    pub(crate) async fn execute(
        &self,
        task: ValidationTask<'_>,
        timeout: Duration,
    ) -> StoreResult<ValidationEvidence> {
        let _permit = self
            .permits
            .try_acquire()
            .map_err(|_| StoreError::Unavailable("validation capacity"))?;
        if timeout.is_zero() || timeout > Duration::from_secs(180) {
            return Err(StoreError::Invalid("validation execution budget"));
        }
        tokio::time::timeout(timeout, self.calculate(task))
            .await
            .map_err(|_| StoreError::Unavailable("reconciliation deadline"))?
    }

    async fn calculate(&self, task: ValidationTask<'_>) -> StoreResult<ValidationEvidence> {
        let ValidationTask {
            job,
            lease,
            record,
            primary,
            prior,
            started_ms,
        } = task;
        self.check_output()?;
        let id = &job
            .job_id
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?
            .value;
        let pin = self.pins.get(id).ok_or(StoreError::AdmissionDenied)?;
        let (source_id, context_id) = source(job)?;
        let specification = record
            .specification
            .as_ref()
            .ok_or(StoreError::Corrupt("primary specification"))?;
        primary.check(specification)?;
        if pin.primary_job_id != source_id
            || specification
                .job_id
                .as_ref()
                .is_none_or(|id| id.value != source_id)
            || primary.inputs.context_id() != context_id
        {
            return Err(StoreError::Corrupt("validation registered source"));
        }
        let Some(job_outcome::Outcome::Success(success)) = record
            .outcome
            .as_ref()
            .and_then(|value| value.outcome.as_ref())
        else {
            return Err(StoreError::InvalidTransition);
        };
        let manifest = super::portfolio::manifest_artifact(success)?;
        let mut budget = ReadBudget::new();
        let policy_file = self.source.load(&pin.policy, true, &mut budget).await?;
        let policy: ComparisonPolicy = policy_file.json()?;
        policy.validate()?;
        let Some(job_specification::Input::Reconciliation(input)) = &job.input else {
            return Err(StoreError::AdmissionDenied);
        };
        let reference = input
            .reconciliation_policy
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?;
        let policy_digest = pin.policy.digest()?;
        if reference
            .policy_id
            .as_ref()
            .is_none_or(|id| id.value != policy.policy_id)
            || reference.revision != policy.revision
            || reference
                .sha256
                .as_ref()
                .is_none_or(|value| value.value != policy_digest)
        {
            return Err(StoreError::Corrupt("comparison policy binding"));
        }
        let mut files = vec![policy_file];
        let old = if let Some(success) = prior {
            let [artifact] = success.outputs.as_slice() else {
                return Err(StoreError::Corrupt("validation output set"));
            };
            let reference = ObjectRef {
                sha256: artifact
                    .artifact_id
                    .as_ref()
                    .ok_or(StoreError::Corrupt("validation artifact ID"))?
                    .value
                    .clone(),
                byte_size: artifact.byte_size,
            };
            let file = self
                .outputs
                .load(&reference, true, &mut ReadBudget::new())
                .await?;
            let document: ValidationDocument = file.json()?;
            files.push(file);
            Some((reference, document))
        } else {
            None
        };
        let original_lease = primary
            .lease_id
            .as_deref()
            .ok_or(StoreError::Corrupt("primary lease binding"))?;
        let trials = primary
            .lineage
            .as_ref()
            .ok_or(StoreError::Corrupt("primary trial lineage"))?
            .trials
            .clone();
        let work = ValidationWork {
            schema: "loop.validation-work/v1",
            job_id: id.clone(),
            lease_id: lease.to_owned(),
            primary_revision: record.revision,
            primary: primary
                .inputs
                .work(original_lease, trials, Some(manifest.object.clone())),
            policy: pin.policy.clone(),
            prepared: old.as_ref().map(|(_, document)| document.prepared.clone()),
        };
        let handle = self
            .prepare_inputs(&work, specification, original_lease)
            .await?;
        let input_file = self
            .outputs
            .load(&handle.reference, true, &mut ReadBudget::new())
            .await?;
        let inputs = input_file.json::<PreparedInputs>()?;
        if inputs != handle.inputs
            || inputs.schema != "loop.validation-inputs/v1"
            || inputs.job_id != *id
            || inputs.lease_id != lease
            || inputs.primary_job_id != source_id
            || inputs.primary_revision != record.revision
            || inputs.primary_manifest != manifest.object
            || inputs.policy != pin.policy
            || inputs.production_eligible
        {
            return Err(StoreError::Corrupt("validation input binding"));
        }
        files.push(input_file);
        for reference in [&inputs.alphalens, &inputs.zipline, &inputs.statistics] {
            let file = self
                .outputs
                .load(reference, true, &mut ReadBudget::new())
                .await?;
            let value: serde_json::Value = serde_json::from_slice(file.bytes()?)
                .map_err(|_| StoreError::Corrupt("validation input JSON"))?;
            self.guard_references(&value, &mut files).await?;
            files.push(file);
        }
        let (alpha, zipline) = self
            .run_validators(&inputs, old.as_ref().map(|(_, value)| value), &mut files)
            .await?;
        let document = ValidationDocument {
            schema: "loop.authorized-reconciliation/v1".to_owned(),
            job_id: id.clone(),
            lease_id: lease.to_owned(),
            primary_job_id: source_id.to_owned(),
            primary_revision: record.revision,
            primary_manifest: manifest.object,
            context_id,
            policy: pin.policy.clone(),
            prepared: handle.reference,
            alphalens: alpha.receipt,
            zipline: zipline.receipt,
            disposition: alpha
                .artifacts
                .disposition
                .combined(zipline.artifacts.disposition),
            production_eligible: false,
            admission_prerequisites: vec![
                "licensed_historical_data".to_owned(),
                "global_synchronous_trial_returns".to_owned(),
                "completed_semantic_review".to_owned(),
            ],
            started_at_ms: old
                .as_ref()
                .map_or(started_ms, |(_, document)| document.started_at_ms),
        };
        if old
            .as_ref()
            .is_some_and(|(_, original)| *original != document)
        {
            return Err(StoreError::Corrupt("validation differs from replay"));
        }
        primary.check(specification)?;
        for file in &files {
            file.check()?;
        }
        self.check_output()?;
        let schema = model::SchemaDocument {
            schema: "loop.artifact-schema/v1".to_owned(),
            name: "loop.authorized_reconciliation".to_owned(),
            version: 1,
            media_type: "application/json".to_owned(),
            columns: vec![],
        };
        let schema_ref = self.publish(&schema, prior.is_some()).await?;
        let reference = self.publish(&document, prior.is_some()).await?;
        if old
            .as_ref()
            .is_some_and(|(original, _)| *original != reference)
        {
            return Err(StoreError::Corrupt("validation replay identity"));
        }
        let report = model::Artifact {
            object: reference.clone(),
            schema: model::SchemaRef {
                name: schema.name,
                version: 1,
                document: schema_ref.clone(),
            },
            media_type: schema.media_type,
            created_at_ms: document.started_at_ms,
        };
        for reference in [&schema_ref, &reference] {
            files.push(
                self.outputs
                    .load(reference, true, &mut ReadBudget::new())
                    .await?,
            );
        }
        let proof = ValidationEvidence {
            job: job.clone(),
            primary_record: record,
            primary,
            document,
            report,
            files,
        };
        proof.check()?;
        if let Some(success) = prior {
            let actual = proof.success()?;
            if actual != *success {
                return Err(StoreError::Corrupt("registered validation outcome"));
            }
        }
        Ok(proof)
    }

    async fn prepare_inputs(
        &self,
        work: &ValidationWork,
        job: &JobSpecification,
        lease: &str,
    ) -> StoreResult<PreparedHandle> {
        let view = self.primary.broker.portfolio_view(job, lease).await?;
        let mut command = process::command(&self.primary.python, &self.config.output_store);
        command
            .args([
                "-I",
                "-m",
                "loop_research.reconciliation_worker",
                "--evidence",
            ])
            .arg(&self.primary.evidence)
            .arg("--view")
            .arg(view)
            .arg("--primary-store")
            .arg(&self.primary.output)
            .arg("--output")
            .arg(&self.config.output_store);
        let (_, bytes) = process::run(
            &mut command,
            &serde_json::to_vec(work)
                .map_err(|_| StoreError::Corrupt("validation work encoding"))?,
            Duration::from_secs(180),
            &[0],
        )
        .await?;
        serde_json::from_slice(&bytes)
            .map_err(|_| StoreError::Corrupt("validation export response"))
    }

    async fn run_validators(
        &self,
        inputs: &PreparedInputs,
        old: Option<&ValidationDocument>,
        files: &mut Vec<Arc<VerifiedFile>>,
    ) -> StoreResult<(WorkerHandle<AlphalensReceipt>, WorkerHandle<ZiplineReceipt>)> {
        let (code, bytes) = self
            .worker(
                false,
                old.map(|value| &value.alphalens)
                    .unwrap_or(&inputs.alphalens),
                old.is_some(),
            )
            .await?;
        let alpha: WorkerHandle<AlphalensReceipt> = serde_json::from_slice(&bytes)
            .map_err(|_| StoreError::Corrupt("Alphalens response"))?;
        if alpha.artifacts.schema != "loop.alphalens-receipt/v1"
            || alpha.artifacts.inputs != inputs.alphalens
            || alpha.artifacts.production_eligible
            || alpha.artifacts.disposition.exit_code() != code
        {
            return Err(StoreError::Corrupt("Alphalens receipt binding"));
        }
        self.guard_report(&alpha.receipt, &alpha.artifacts, files)
            .await?;
        let (code, bytes) = self
            .worker(
                true,
                old.map(|value| &value.zipline).unwrap_or(&inputs.zipline),
                old.is_some(),
            )
            .await?;
        let zipline: WorkerHandle<ZiplineReceipt> =
            serde_json::from_slice(&bytes).map_err(|_| StoreError::Corrupt("Zipline response"))?;
        if zipline.artifacts.schema != "loop.zipline-receipt/v1"
            || zipline.artifacts.inputs != inputs.zipline
            || zipline.artifacts.production_eligible
            || zipline.artifacts.disposition.exit_code() != code
        {
            return Err(StoreError::Corrupt("Zipline receipt binding"));
        }
        self.guard_report(&zipline.receipt, &zipline.artifacts, files)
            .await?;
        Ok((alpha, zipline))
    }

    async fn worker(
        &self,
        zipline: bool,
        reference: &ObjectRef,
        replay: bool,
    ) -> StoreResult<(i32, Vec<u8>)> {
        self.check_output()?;
        let project = if zipline {
            &self.config.zipline_project
        } else {
            &self.config.alphalens_project
        };
        let python = if zipline {
            &self.config.zipline_python
        } else {
            &self.primary.python
        };
        let mut command = process::command(&self.config.uv, &self.config.output_store);
        command
            .env("UV_CACHE_DIR", &self.config.cache)
            .env("UV_PYTHON_DOWNLOADS", "never")
            .env("UV_LINK_MODE", "copy")
            .args(["--no-config", "run", "--project"])
            .arg(project)
            .arg("--python")
            .arg(python)
            .args(["--isolated", "--locked", "--offline", "python", "-I", "-m"])
            .arg(if zipline {
                "loop_zipline.cli"
            } else {
                "loop_alphalens.cli"
            })
            .arg(if replay { "validate" } else { "run" })
            .arg(if replay { "--receipt" } else { "--input" })
            .arg(&reference.sha256)
            .arg("--store")
            .arg(&self.config.output_store);
        process::run(&mut command, &[], Duration::from_secs(180), &[0, 3, 4]).await
    }

    async fn guard_report<T: Serialize>(
        &self,
        reference: &ObjectRef,
        report: &T,
        files: &mut Vec<Arc<VerifiedFile>>,
    ) -> StoreResult<()> {
        let file = self
            .outputs
            .load(reference, true, &mut ReadBudget::new())
            .await?;
        let bytes = serde_json::to_vec(report)
            .map_err(|_| StoreError::Corrupt("validator receipt encoding"))?;
        if file.bytes()? != bytes {
            return Err(StoreError::Corrupt("validator output differs from receipt"));
        }
        self.guard_references(
            &serde_json::to_value(report)
                .map_err(|_| StoreError::Corrupt("validator references"))?,
            files,
        )
        .await?;
        files.push(file);
        Ok(())
    }

    async fn guard_references(
        &self,
        value: &serde_json::Value,
        files: &mut Vec<Arc<VerifiedFile>>,
    ) -> StoreResult<()> {
        let mut pending = vec![value];
        let mut count = 0;
        let mut budget = ReadBudget::new();
        while let Some(value) = pending.pop() {
            count += 1;
            if count > 4096 || files.len() > 256 {
                return Err(StoreError::Invalid("validation reference bound"));
            }
            match value {
                serde_json::Value::Object(values)
                    if values.contains_key("sha256") && values.contains_key("byte_size") =>
                {
                    let reference: ObjectRef = serde_json::from_value(value.clone())
                        .map_err(|_| StoreError::Corrupt("validation object reference"))?;
                    if reference.byte_size > 64 * 1024 * 1024 {
                        return Err(StoreError::Invalid("validation object size"));
                    }
                    files.push(self.outputs.load(&reference, false, &mut budget).await?);
                }
                serde_json::Value::Object(values) => pending.extend(values.values()),
                serde_json::Value::Array(values) => pending.extend(values),
                _ => {}
            }
        }
        Ok(())
    }

    async fn publish<T: Serialize>(&self, value: &T, replay: bool) -> StoreResult<ObjectRef> {
        self.check_output()?;
        let bytes = serde_json::to_vec(value)
            .map_err(|_| StoreError::Corrupt("validation manifest encoding"))?;
        if bytes.is_empty() || bytes.len() > 1_048_576 {
            return Err(StoreError::Invalid("validation manifest bound"));
        }
        let reference = ObjectRef {
            sha256: format!("sha256:{:x}", Sha256::digest(&bytes)),
            byte_size: bytes.len() as u64,
        };
        if !replay {
            let root = self.config.output_store.clone();
            let name = reference.sha256[7..].to_owned();
            tokio::task::spawn_blocking(move || -> StoreResult<()> {
                let mut temporary = tempfile::Builder::new()
                    .prefix(".loop-reconciliation-")
                    .tempfile_in(&root)?;
                temporary.write_all(&bytes)?;
                temporary.as_file().sync_all()?;
                match temporary.persist_noclobber(root.join(name)) {
                    Ok(_) => {}
                    Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(StoreError::Io(error.error)),
                }
                std::fs::File::open(root)?.sync_all()?;
                Ok(())
            })
            .await
            .map_err(|_| StoreError::Unavailable("validation publication"))??;
        }
        self.check_output()?;
        self.outputs
            .load(&reference, true, &mut ReadBudget::new())
            .await?;
        Ok(reference)
    }
}

pub(super) fn overlaps(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}
