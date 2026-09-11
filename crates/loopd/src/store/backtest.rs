use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use loop_core::factor::{CanonicalDecimal, Identifier};
use loop_protocol::artifact::validate_artifact_ref;
use loop_protocol::provenance::{ProvenanceSnapshot, assess_provenance};
use loop_protocol::wire::v1::{
    Actor, BacktestEngineKind, BacktestResult, JobKind, JobRecord, JobSpecification, JobState,
    JobSuccess, ResearchProvenanceFingerprint, job_outcome, job_specification,
};
use prost::Message;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction};

use super::postgres::{encode_message, record_from_row, timestamp_millis, verified_blob};
use super::{BacktestExport, ExportBacktest, PgJobStore, StoreError, StoreResult, validate_id};

/// Asynchronous preflight that optionally returns operation-scoped evidence.
/// The future grants no authority; the returned resolver is internal metadata.
pub type BacktestPreparation<'a> =
    Pin<Box<dyn Future<Output = StoreResult<Option<Arc<dyn BacktestPolicy>>>> + Send + 'a>>;

/// Server-owned, bounded result resolution. Implementations must verify immutable
/// manifests, artifact checksums/availability, and the original factor, sample,
/// seed and engine against independently resolved frozen inputs. A matching DTO
/// or a caller-supplied hash is not evidence. No network or unbounded I/O is
/// allowed in these synchronous transaction callbacks; pre-resolve immutable
/// evidence under the owning service's identity. Defaults deny every operation.
pub trait BacktestPolicy: Send + Sync {
    /// Check catalog-level access before materializing files. This supplements
    /// the job/transport policy; it does not authenticate caller metadata.
    fn authorize_materialization(
        &self,
        _principal: &Actor,
        _job: &JobSpecification,
        _context_id: Option<&str>,
    ) -> StoreResult<()> {
        Ok(())
    }

    /// Materialize immutable files before opening a ledger transaction. The
    /// caller first authorizes the operation; this hook grants no permission.
    /// Implementations bound time, bytes and scans and retain file-version
    /// guards for synchronous resolution. Cancellation publishes no decision.
    /// Return a request-scoped resolver, so overlapping preparations cannot
    /// replace each other's evidence. In-memory policies return `None` and keep
    /// their existing resolver; resolution still defaults to denial.
    fn prepare<'a>(
        &'a self,
        _job: &'a JobSpecification,
        _context_id: Option<&'a str>,
        _success: Option<&'a JobSuccess>,
    ) -> BacktestPreparation<'a> {
        Box::pin(async { Ok(None) })
    }

    /// Recheck previously materialized frozen inputs immediately before their
    /// use. The concrete manifest policy refuses unprepared or changed files.
    /// This supplements, never replaces, admission and transport authorization.
    fn validate_inputs(&self, _job: &JobSpecification) -> StoreResult<()> {
        Ok(())
    }

    /// Bind the service-selected numerical worker to the resolved context.
    /// Concrete file-backed policies require an attested source/environment;
    /// default fixture policies make no execution-build claim.
    fn validate_worker(
        &self,
        _provenance: &ResearchProvenanceFingerprint,
        _build: Option<crate::research_worker::ResearchBuild>,
    ) -> StoreResult<()> {
        Ok(())
    }

    /// Resolve an immutable IS-only admission report against the registered
    /// primary result and context. Prove canonical factor/direction, policy,
    /// complete machine gates, finished semantic review and library snapshot.
    /// Authorize access to the entire reviewed library, including replacements.
    /// Missing review or infrastructure failures return an error, never a vote.
    /// Pre-resolve bounded trusted evidence; no network under the ledger lock.
    fn resolve_admission(
        &self,
        _principal: &Actor,
        _job: &JobSpecification,
        _context_id: &str,
    ) -> StoreResult<super::AdmissionEvidence> {
        Err(StoreError::AdmissionDenied)
    }

    /// Independently verify an unexpired, unrevoked human approval binds the
    /// subject, report, exact reason and semantic-only exception. Default deny.
    fn authorize_factor_override(
        &self,
        _principal: &Actor,
        _command: &super::DecideFactor,
        _evidence: &super::AdmissionEvidence,
    ) -> StoreResult<()> {
        Err(StoreError::AdmissionDenied)
    }

    /// Resolve a frozen single-window family and prove IS-only data, canonical
    /// candidate identities, fixed direction/policies and worker build provenance.
    /// Authorize the principal to the entire family and its shared score history,
    /// not only the source job. Candidate IDs alone do not establish membership.
    /// No network is permitted under the transaction lock. Default denial also
    /// excludes development-validation and holdout scores from optimization.
    fn resolve_perturbation_space(
        &self,
        _principal: &Actor,
        _job: &JobSpecification,
        _context_id: &str,
    ) -> StoreResult<loop_protocol::wire::v1::PerturbationSpace> {
        Err(StoreError::AdmissionDenied)
    }
    /// Resolve the exact result manifest referenced by a successful job. This
    /// never grants holdout access, factor admission, or authority to rerun work.
    fn resolve_result(
        &self,
        _job: &JobSpecification,
        _success: &JobSuccess,
    ) -> StoreResult<BacktestResult> {
        Err(StoreError::AdmissionDenied)
    }

    /// Resolve an explicit immutable current-context reference, after transport
    /// authorization. Missing context returns `None`, never the recorded values.
    /// This must revalidate access and reference availability on every read.
    fn resolve_current(
        &self,
        _principal: &Actor,
        _job: &JobSpecification,
        _context_id: &str,
    ) -> StoreResult<Option<ResearchProvenanceFingerprint>> {
        Err(StoreError::AdmissionDenied)
    }
}

/// Safe default until production manifest and data registries are available.
pub struct DenyBacktest;

impl BacktestPolicy for DenyBacktest {}

/// Backend-independent current-metric consumption boundary. It exposes no SQL,
/// holdout capability, raw data, or mutable historical metrics.
pub trait BacktestRepository: Send + Sync {
    /// Read a successful result only when its evidence is valid and all six
    /// fingerprints match a server-resolved current context. `principal` must
    /// originate from transport authentication, not request metadata. Protected
    /// jobs additionally require the independent holdout policy. Missing jobs,
    /// unfinished jobs, denied/unresolved references, corruption, stale metrics,
    /// and storage outages are errors, never empty/current results. This read
    /// has no state, audit or external export side effects; cancellation rolls
    /// back the read transaction. Export handlers must use this same gate.
    fn current_backtest(
        &self,
        principal: &Actor,
        job_id: &str,
        context_id: &str,
    ) -> impl Future<Output = StoreResult<BacktestResult>> + Send;

    /// Release current result metadata only after an immutable receipt and audit
    /// commit. Both read and export permission are required; protected results
    /// also retain the holdout read gate. Every retry revalidates current access,
    /// evidence and provenance, so an old receipt cannot bypass revocation or
    /// stale inputs. The receipt is not a capability or proof of file delivery.
    /// No dataset, file or remote destination is written by this internal API.
    /// Invalid input, deadlines, clock regression, conflicts, denied/stale data
    /// and storage faults fail closed. Cancellation before commit rolls back;
    /// retry resolves an uncertain commit without a duplicate export audit.
    fn export_current(
        &self,
        principal: &Actor,
        command: ExportBacktest,
    ) -> impl Future<Output = StoreResult<BacktestExport>> + Send;
}

impl BacktestRepository for PgJobStore {
    async fn current_backtest(
        &self,
        principal: &Actor,
        job_id: &str,
        context_id: &str,
    ) -> StoreResult<BacktestResult> {
        validate_current_request(principal, job_id, context_id)?;
        let prepared = self
            .prepare_research(
                principal,
                job_id,
                Some(context_id),
                "loop.backtests.read_current",
                None,
            )
            .await?;
        let mut transaction = self.pool.begin().await?;
        let (_, result) = current_in_transaction(
            &prepared,
            &mut transaction,
            principal,
            job_id,
            context_id,
            "loop.backtests.read_current",
        )
        .await?;
        transaction.commit().await?;
        Ok(result)
    }

    async fn export_current(
        &self,
        principal: &Actor,
        command: ExportBacktest,
    ) -> StoreResult<BacktestExport> {
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            super::export::execute(self, principal, command),
        )
        .await
        .map_err(|_| StoreError::Unavailable("export timeout"))?
    }
}

impl PgJobStore {
    pub(super) async fn prepare_research(
        &self,
        principal: &Actor,
        job_id: &str,
        context_id: Option<&str>,
        operation: &'static str,
        completion: Option<&JobSuccess>,
    ) -> StoreResult<Self> {
        use super::JobRepository;

        let record = self.get(job_id).await?.ok_or(StoreError::NotFound)?;
        self.admission
            .authorize_job_command(operation, principal, &record)?;
        let specification = specification(&record)?;
        // Protected data never reaches a development materializer. Its own
        // resolver and capability boundary remain independently default-deny.
        if !matches!(
            specification.input,
            Some(job_specification::Input::Backtest(_))
        ) {
            return Ok(self.clone());
        }
        self.backtest_policy
            .authorize_materialization(principal, specification, context_id)?;
        let policy = self
            .backtest_policy
            .prepare(
                specification,
                context_id,
                completion.or_else(|| success(&record)),
            )
            .await?;
        let mut prepared = self.clone();
        if let Some(policy) = policy {
            prepared.backtest_policy = policy;
        }
        prepared.backtest_policy.validate_inputs(specification)?;
        Ok(prepared)
    }
}

pub(super) async fn current_in_transaction(
    store: &PgJobStore,
    transaction: &mut Transaction<'_, Postgres>,
    principal: &Actor,
    job_id: &str,
    context_id: &str,
    operation: &'static str,
) -> StoreResult<(JobRecord, BacktestResult)> {
    validate_current_request(principal, job_id, context_id)?;
    let row = sqlx::query("SELECT * FROM jobs WHERE job_id = $1")
        .bind(job_id)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(StoreError::NotFound)?;
    let record = record_from_row(&row)?;
    store
        .admission
        .authorize_job_command(operation, principal, &record)?;
    if operation != "loop.backtests.read_current" {
        store
            .admission
            .authorize_job_command("loop.backtests.read_current", principal, &record)?;
    }
    let specification = specification(&record)?;
    if let Some(job_specification::Input::HoldoutBacktest(input)) = &specification.input {
        let period = input
            .consumed_grant
            .as_ref()
            .and_then(|grant| grant.holdout_period_id.as_ref())
            .ok_or(StoreError::Corrupt("holdout result period"))?;
        store.holdout_policy.authorize_period(
            "loop.holdout.read_result",
            principal,
            &period.value,
        )?;
    }
    let result = verify_stored(transaction, &record)
        .await?
        .ok_or(StoreError::InvalidTransition)?;
    let success = success(&record).ok_or(StoreError::Corrupt("successful backtest outcome"))?;
    if store
        .backtest_policy
        .resolve_result(specification, success)?
        != result
    {
        return Err(StoreError::Corrupt(
            "resolved result differs from registered evidence",
        ));
    }
    let current = store
        .backtest_policy
        .resolve_current(principal, specification, context_id)?;
    let current = current
        .as_ref()
        .map(ProvenanceSnapshot::try_from)
        .transpose()?;
    let recorded = snapshot(result.provenance.as_ref())?;
    let frozen = frozen_provenance(specification)?;
    assess_provenance(&recorded, &frozen, current.as_ref())?.require_current()?;
    Ok((record, result))
}

fn validate_current_request(principal: &Actor, job_id: &str, context_id: &str) -> StoreResult<()> {
    validate_id(job_id)?;
    validate_id(context_id)?;
    validate_id(
        &principal
            .actor_id
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?
            .value,
    )?;
    if principal.authenticated_subject.is_empty() {
        return Err(StoreError::AdmissionDenied);
    }
    Ok(())
}

pub(super) async fn record_completion(
    store: &PgJobStore,
    transaction: &mut Transaction<'_, Postgres>,
    record: &JobRecord,
    now: i64,
) -> StoreResult<()> {
    if !is_completed_backtest(record)? {
        return Ok(());
    }
    let job = specification(record)?;
    let success = success(record).ok_or(StoreError::Corrupt("successful backtest outcome"))?;
    let result = store.backtest_policy.resolve_result(job, success)?;
    validate_result(&result, record)?;
    let blob = encode_message(&result)?;
    sqlx::query(
        "INSERT INTO backtest_results (job_id, job_revision, backtest_id, engine,
         manifest_sha256, result_blob, result_sha256, committed_at_ms)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(
        &job.job_id
            .as_ref()
            .ok_or(StoreError::Corrupt("job identity"))?
            .value,
    )
    .bind(record.revision as i64)
    .bind(
        &result
            .backtest_id
            .as_ref()
            .ok_or(StoreError::Invalid("backtest identity"))?
            .value,
    )
    .bind(result.engine)
    .bind(
        &result
            .result_manifest_sha256
            .as_ref()
            .ok_or(StoreError::Invalid("result manifest"))?
            .value,
    )
    .bind(&blob)
    .bind(Sha256::digest(&blob).to_vec())
    .bind(now)
    .execute(&mut **transaction)
    .await?;
    #[cfg(test)]
    super::crash_tests::fault_point("result_after_insert").await;
    Ok(())
}

pub(super) async fn verify_stored(
    transaction: &mut Transaction<'_, Postgres>,
    record: &JobRecord,
) -> StoreResult<Option<BacktestResult>> {
    if !is_completed_backtest(record)? {
        return Ok(None);
    }
    let job = specification(record)?;
    let job_id = &job
        .job_id
        .as_ref()
        .ok_or(StoreError::Corrupt("job identity"))?
        .value;
    let row = sqlx::query("SELECT * FROM backtest_results WHERE job_id = $1")
        .bind(job_id)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(StoreError::Corrupt("backtest result missing"))?;
    let result =
        BacktestResult::decode(verified_blob(&row, "result_blob", "result_sha256")?.as_slice())
            .map_err(|_| StoreError::Corrupt("backtest result encoding"))?;
    validate_result(&result, record).map_err(|_| StoreError::Corrupt("backtest result binding"))?;
    if row.try_get::<i64, _>("job_revision")? != record.revision as i64
        || row.try_get::<String, _>("backtest_id")?
            != result
                .backtest_id
                .as_ref()
                .ok_or(StoreError::Corrupt("backtest identity"))?
                .value
        || row.try_get::<i32, _>("engine")? != result.engine
        || row.try_get::<Vec<u8>, _>("manifest_sha256")?
            != result
                .result_manifest_sha256
                .as_ref()
                .ok_or(StoreError::Corrupt("result manifest"))?
                .value
        || row.try_get::<i64, _>("committed_at_ms")?
            != timestamp_millis(
                record
                    .updated_at
                    .as_ref()
                    .ok_or(StoreError::Corrupt("job update time"))?,
                false,
            )?
    {
        return Err(StoreError::Corrupt("backtest result projection"));
    }
    Ok(Some(result))
}

fn specification(record: &JobRecord) -> StoreResult<&JobSpecification> {
    record
        .specification
        .as_ref()
        .ok_or(StoreError::Corrupt("job specification"))
}

pub(super) fn is_completed_backtest(record: &JobRecord) -> StoreResult<bool> {
    Ok(matches!(
        JobKind::try_from(specification(record)?.kind),
        Ok(JobKind::Backtest | JobKind::HoldoutBacktest)
    ) && record.state == JobState::Succeeded as i32)
}

fn success(record: &JobRecord) -> Option<&JobSuccess> {
    match record.outcome.as_ref()?.outcome.as_ref()? {
        job_outcome::Outcome::Success(value) => Some(value),
        _ => None,
    }
}

fn snapshot(value: Option<&ResearchProvenanceFingerprint>) -> StoreResult<ProvenanceSnapshot> {
    Ok(ProvenanceSnapshot::try_from(
        value.ok_or(StoreError::Invalid("research provenance"))?,
    )?)
}

fn frozen_provenance(job: &JobSpecification) -> StoreResult<ProvenanceSnapshot> {
    match job.input.as_ref() {
        Some(job_specification::Input::Backtest(input)) => snapshot(input.provenance.as_ref()),
        Some(job_specification::Input::HoldoutBacktest(input)) => snapshot(
            input
                .frozen_backtest_spec
                .as_ref()
                .ok_or(StoreError::Corrupt("frozen backtest spec"))?
                .provenance
                .as_ref(),
        ),
        _ => Err(StoreError::Invalid("backtest job required")),
    }
}

fn validate_result(result: &BacktestResult, record: &JobRecord) -> StoreResult<()> {
    let job = specification(record)?;
    validate_id(
        &result
            .backtest_id
            .as_ref()
            .ok_or(StoreError::Invalid("backtest identity"))?
            .value,
    )?;
    if !matches!(
        BacktestEngineKind::try_from(result.engine),
        Ok(BacktestEngineKind::PrimaryCrossSectional
            | BacktestEngineKind::AlphalensValidation
            | BacktestEngineKind::ZiplineValidation)
    ) || result.engine_version.is_empty()
        || result.engine_version.len() > 128
        || !result
            .engine_version
            .bytes()
            .all(|byte| byte.is_ascii_graphic())
        || result.metrics.is_empty()
        || result.metrics.len() > 256
    {
        return Err(StoreError::Invalid("backtest engine or metrics"));
    }
    if let Some(job_specification::Input::HoldoutBacktest(input)) = &job.input
        && input
            .frozen_backtest_spec
            .as_ref()
            .and_then(|spec| spec.backtest_id.as_ref())
            != result.backtest_id.as_ref()
    {
        return Err(StoreError::Invalid("frozen backtest identity"));
    }
    let frozen = frozen_provenance(job)?;
    assess_provenance(&snapshot(result.provenance.as_ref())?, &frozen, None)?;
    let completed = result
        .completed_at
        .as_ref()
        .ok_or(StoreError::Invalid("result completion time"))?;
    timestamp_millis(completed, true)?;
    let submitted = job
        .submitted_at
        .as_ref()
        .ok_or(StoreError::Corrupt("job submission time"))?;
    let committed = record
        .updated_at
        .as_ref()
        .ok_or(StoreError::Corrupt("job update time"))?;
    if (completed.seconds, completed.nanos) < (submitted.seconds, submitted.nanos)
        || (completed.seconds, completed.nanos) > (committed.seconds, committed.nanos)
    {
        return Err(StoreError::Invalid("result completion time binding"));
    }
    let mut previous: Option<&str> = None;
    for metric in &result.metrics {
        Identifier::new(&metric.name).map_err(|_| StoreError::Invalid("metric name"))?;
        for label in [&metric.unit, &metric.estimator] {
            if label.trim().is_empty() || label.len() > 256 || label.chars().any(char::is_control) {
                return Err(StoreError::Invalid("metric unit or estimator"));
            }
        }
        let value = &metric
            .value
            .as_ref()
            .ok_or(StoreError::Invalid("metric value"))?
            .value;
        if value.len() > 1_024 || previous.is_some_and(|name| name >= metric.name.as_str()) {
            return Err(StoreError::Invalid("metric size or order"));
        }
        CanonicalDecimal::new(value).map_err(|_| StoreError::Invalid("metric decimal"))?;
        previous = Some(&metric.name);
    }
    let success = success(record).ok_or(StoreError::Corrupt("backtest outcome"))?;
    let manifest = &result
        .result_manifest_sha256
        .as_ref()
        .ok_or(StoreError::Invalid("result manifest"))?
        .value;
    if manifest.len() != 32
        || !success.outputs.iter().any(|artifact| {
            artifact
                .sha256
                .as_ref()
                .is_some_and(|digest| digest.value == *manifest)
        })
    {
        return Err(StoreError::Invalid("result manifest output binding"));
    }
    let artifacts = result
        .artifacts
        .as_ref()
        .ok_or(StoreError::Invalid("backtest artifacts"))?;
    for artifact in [
        &artifacts.factor_values,
        &artifacts.target_positions,
        &artifacts.orders,
        &artifacts.fills,
        &artifacts.nav,
        &artifacts.simple_returns,
        &artifacts.risk_exposures,
        &artifacts.cost_ledger,
    ] {
        let artifact = artifact
            .as_ref()
            .ok_or(StoreError::Invalid("backtest artifact"))?;
        validate_artifact_ref(artifact)
            .map_err(|_| StoreError::Invalid("backtest artifact reference"))?;
        if !success.outputs.contains(artifact) {
            return Err(StoreError::Invalid("backtest artifact output binding"));
        }
    }
    Ok(())
}
