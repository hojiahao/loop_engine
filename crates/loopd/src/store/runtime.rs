use loop_protocol::wire::holdout::v1::JobBatchHandle;
use loop_protocol::wire::v1::{Actor, HoldoutGrantState, JobRecord, JobState, job_specification};
use prost::Message;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction};

use super::grant::{resolve, state};
use super::postgres::{record_from_row, timestamp_millis, verified_blob};
use super::{PgJobStore, StoreError, StoreResult};
use crate::manifests::data::DataEvidence;
use loop_protocol::wire::jobs::v1::{PrepareJobArtifactsRequest, PrepareJobArtifactsResponse};

#[derive(Clone, PartialEq, Message)]
struct Receipt {
    #[prost(message, optional, tag = "1")]
    response: Option<PrepareJobArtifactsResponse>,
    #[prost(string, tag = "2")]
    request_id: String,
    #[prost(int64, tag = "3")]
    accepted_at_ms: i64,
}

impl PgJobStore {
    pub(crate) async fn runtime_lease(
        &self,
        actor: &Actor,
        job_id: &str,
        lease_id: &str,
    ) -> StoreResult<JobRecord> {
        let job = self
            .runtime_job(actor, job_id, "loop.jobs.artifacts")
            .await?;
        live_lease(&job, actor, lease_id, self.clock.now_millis()?)?;
        Ok(job)
    }
    pub(crate) fn with_runtime_authority(
        mut self,
        authority: std::sync::Arc<crate::runtime::RuntimeAuthority>,
    ) -> Self {
        self.admission = authority;
        self
    }
    pub(crate) async fn runtime_job(
        &self,
        actor: &Actor,
        job_id: &str,
        operation: &str,
    ) -> StoreResult<JobRecord> {
        super::validate_id(job_id)?;
        let mut transaction = self.pool.begin().await?;
        let now = self.observe_clock(&mut transaction).await?;
        let row = sqlx::query("SELECT * FROM jobs WHERE job_id=$1")
            .bind(job_id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or(StoreError::NotFound)?;
        let record = record_from_row(&row)?;
        self.admission
            .authorize_job_command(operation, actor, &record)?;
        protected_job(&mut transaction, &record, now).await?;
        transaction.commit().await?;
        Ok(record)
    }

    pub(crate) async fn accept_data_access(
        &self,
        actor: &Actor,
        command: &PrepareJobArtifactsRequest,
        evidence: &DataEvidence,
    ) -> StoreResult<PrepareJobArtifactsResponse> {
        const OPERATION: &str = "loop.jobs.artifacts";
        let context = super::lifecycle::validate_context(command.context.as_ref(), actor)?;
        let job_id = &command
            .job_id
            .as_ref()
            .ok_or(StoreError::Invalid("data job ID"))?
            .value;
        let lease_id = &command
            .lease_id
            .as_ref()
            .ok_or(StoreError::Invalid("data lease ID"))?
            .value;
        super::validate_id(job_id)?;
        super::validate_id(lease_id)?;
        if command.expected_revision == 0 {
            return Err(StoreError::Invalid("data revision"));
        }
        let mut normalized = command.clone();
        if let Some(context) = &mut normalized.context {
            context.request_id = None;
            context.requested_at = None;
        }
        let mut transaction = self.pool.begin().await?;
        let now = self.observe_clock(&mut transaction).await?;
        let requested = timestamp_millis(
            context
                .requested_at
                .as_ref()
                .ok_or(StoreError::Invalid("data request time"))?,
            true,
        )?;
        if requested > now || now - requested >= 30_000 {
            return Err(StoreError::Unavailable("data request deadline"));
        }
        let row = sqlx::query("SELECT * FROM jobs WHERE job_id=$1")
            .bind(job_id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or(StoreError::NotFound)?;
        let record = record_from_row(&row)?;
        self.admission
            .authorize_job_command(OPERATION, actor, &record)?;
        protected_job(&mut transaction, &record, now).await?;
        let expiry = live_lease(&record, actor, lease_id, now)?;
        evidence.check(
            record
                .specification
                .as_ref()
                .ok_or(StoreError::Corrupt("data job specification"))?,
        )?;
        let key = &context
            .idempotency_key
            .as_ref()
            .ok_or(StoreError::Invalid("data request key"))?
            .value;
        let actor_id = &actor
            .actor_id
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?
            .value;
        let previous = sqlx::query("SELECT * FROM command_receipts WHERE actor_id=$1 AND operation=$2 AND idempotency_key=$3")
            .bind(actor_id).bind(OPERATION).bind(key).fetch_optional(&mut *transaction).await?;
        let response = PrepareJobArtifactsResponse {
            job_id: command.job_id.clone(),
            lease_id: command.lease_id.clone(),
            data_manifest_sha256: Some(loop_protocol::wire::v1::Sha256Digest {
                value: evidence.manifest.digest()?.to_vec(),
            }),
            artifacts: evidence.artifacts(),
            expires_at: Some(super::postgres::timestamp(expiry)),
            view_id: crate::runtime::view_id(job_id, lease_id, &evidence.manifest)?,
        };
        if let Some(row) = previous {
            let old = PrepareJobArtifactsRequest::decode(
                verified_blob(&row, "request_blob", "request_sha256")?.as_slice(),
            )
            .map_err(|_| StoreError::Corrupt("data request receipt"))?;
            if old != normalized {
                return Err(StoreError::IdempotencyConflict);
            }
            let receipt = Receipt::decode(
                verified_blob(&row, "response_blob", "response_sha256")?.as_slice(),
            )
            .map_err(|_| StoreError::Corrupt("data response receipt"))?;
            let original = receipt
                .response
                .ok_or(StoreError::Corrupt("data response absent"))?;
            let old_expiry = timestamp_millis(
                original
                    .expires_at
                    .as_ref()
                    .ok_or(StoreError::Corrupt("data receipt expiry"))?,
                false,
            )?;
            if original.job_id != response.job_id
                || original.lease_id != response.lease_id
                || original.artifacts != response.artifacts
                || original.data_manifest_sha256 != response.data_manifest_sha256
                || original.view_id != response.view_id
                || row.try_get::<String, _>("job_id")? != *job_id
                || row.try_get::<i64, _>("committed_at_ms")? != receipt.accepted_at_ms
                || row.try_get::<String, _>("request_id")? != receipt.request_id
                || receipt.accepted_at_ms
                    < timestamp_millis(
                        record
                            .active_lease
                            .as_ref()
                            .and_then(|lease| lease.issued_at.as_ref())
                            .ok_or(StoreError::LeaseFenced)?,
                        false,
                    )?
                || receipt.accepted_at_ms > now
                || old_expiry > expiry
            {
                return Err(StoreError::Corrupt("data receipt binding"));
            }
            super::validate_id(&receipt.request_id)
                .map_err(|_| StoreError::Corrupt("data receipt request ID"))?;
            if now >= old_expiry {
                return Err(StoreError::LeaseFenced);
            }
            let last = final_time(self, now, requested, &record, actor, lease_id, evidence)?;
            if last >= old_expiry {
                return Err(StoreError::LeaseFenced);
            }
            sqlx::query("UPDATE store_metadata SET last_observed_at_ms=$1 WHERE singleton=1")
                .bind(last)
                .execute(&mut *transaction)
                .await?;
            transaction.commit().await?;
            return Ok(original);
        }
        if record.revision != command.expected_revision {
            return Err(StoreError::RevisionConflict);
        }
        let request_blob = super::postgres::encode_message(&normalized)?;
        let request_id = &context
            .request_id
            .as_ref()
            .ok_or(StoreError::Invalid("data request ID"))?
            .value;
        let response_blob = super::postgres::encode_message(&Receipt {
            response: Some(response.clone()),
            request_id: request_id.clone(),
            accepted_at_ms: now,
        })?;
        sqlx::query("INSERT INTO command_receipts (actor_id, operation, idempotency_key, request_id, job_id, request_blob, request_sha256, response_blob, response_sha256, committed_at_ms) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
            .bind(actor_id).bind(OPERATION).bind(key).bind(request_id).bind(job_id)
            .bind(&request_blob).bind(Sha256::digest(&request_blob).as_slice())
            .bind(&response_blob).bind(Sha256::digest(&response_blob).as_slice()).bind(now)
            .execute(&mut *transaction).await?;
        super::audit::append(&mut transaction, &self.ledger_id, now, super::audit::EventInput {
            actor,
            correlation_id: &context.correlation_id.as_ref().ok_or(StoreError::Invalid("data correlation"))?.value,
            causation_id: &context.causation_id.as_ref().ok_or(StoreError::Invalid("data causation"))?.value,
            action: loop_core::audit::AuditAction::CommandAccepted,
            target: loop_core::audit::AuditTarget { kind: loop_core::audit::AuditTargetKind::JobId, value: job_id.clone() },
            payload: loop_core::audit::canonicalize_audit_payload("loop.audit.command_accepted", 1,
                &serde_json::to_vec(&serde_json::json!({"command":OPERATION,"request_id":request_id,"summary":"lease-bound data view accepted"}))
                    .map_err(|_| StoreError::Invalid("data audit"))?)?,
        }).await?;
        let last = final_time(self, now, requested, &record, actor, lease_id, evidence)?;
        sqlx::query("UPDATE store_metadata SET last_observed_at_ms=$1 WHERE singleton=1")
            .bind(last)
            .execute(&mut *transaction)
            .await?;
        #[cfg(test)]
        super::crash_tests::fault_point("data_before_commit").await;
        transaction.commit().await?;
        #[cfg(test)]
        super::crash_tests::fault_point("data_after_commit").await;
        Ok(response)
    }
}

fn final_time(
    store: &PgJobStore,
    now: i64,
    requested: i64,
    record: &JobRecord,
    actor: &Actor,
    lease_id: &str,
    evidence: &DataEvidence,
) -> StoreResult<i64> {
    let last = store.clock.now_millis()?;
    if last < now {
        return Err(StoreError::ClockRegression);
    }
    if last - requested >= 30_000 {
        return Err(StoreError::Unavailable("data request deadline"));
    }
    live_lease(record, actor, lease_id, last)?;
    evidence.check(
        record
            .specification
            .as_ref()
            .ok_or(StoreError::Corrupt("data job specification"))?,
    )?;
    Ok(last)
}

pub(crate) fn live_lease(
    record: &JobRecord,
    actor: &Actor,
    lease_id: &str,
    now: i64,
) -> StoreResult<i64> {
    let lease = record
        .active_lease
        .as_ref()
        .ok_or(StoreError::LeaseFenced)?;
    let expires = timestamp_millis(
        lease.expires_at.as_ref().ok_or(StoreError::LeaseFenced)?,
        false,
    )?;
    if !matches!(
        JobState::try_from(record.state),
        Ok(JobState::Leased | JobState::Running)
    ) || lease.owner.as_ref() != Some(actor)
        || lease.lease_id.as_ref().map(|id| id.value.as_str()) != Some(lease_id)
        || now
            < timestamp_millis(
                lease.issued_at.as_ref().ok_or(StoreError::LeaseFenced)?,
                false,
            )?
        || now >= expires
    {
        return Err(StoreError::LeaseFenced);
    }
    Ok(expires)
}

pub(super) async fn protected_job(
    transaction: &mut Transaction<'_, Postgres>,
    record: &JobRecord,
    now: i64,
) -> StoreResult<()> {
    let job = record
        .specification
        .as_ref()
        .ok_or(StoreError::Corrupt("runtime job specification"))?;
    let Some(job_specification::Input::HoldoutBacktest(input)) = &job.input else {
        return Ok(());
    };
    let reference = input
        .consumed_grant
        .as_ref()
        .ok_or(StoreError::Corrupt("runtime grant"))?;
    let grant_id = &reference
        .holdout_grant_id
        .as_ref()
        .ok_or(StoreError::Corrupt("runtime grant ID"))?
        .value;
    let current = state::load_grant(transaction, grant_id).await?;
    let period = current
        .period
        .record
        .period
        .as_ref()
        .ok_or(StoreError::Corrupt("runtime period"))?;
    let backtest = input
        .frozen_backtest_spec
        .as_ref()
        .ok_or(StoreError::Corrupt("runtime frozen backtest"))?;
    if current.grant.state != HoldoutGrantState::Consumed as i32
        || current.grant.revision != input.consumed_grant_revision
        || current.grant.reference.as_ref() != Some(reference)
        || state::record_time(current.grant.consumed_at.as_ref())? > now
        || backtest.sample != period.sample
        || backtest.snapshot_ids != period.snapshot_ids
        || backtest
            .provenance
            .as_ref()
            .and_then(|provenance| provenance.data_manifest_sha256.as_ref())
            != period.snapshot_manifest_sha256.as_ref()
    {
        return Err(StoreError::AdmissionDenied);
    }
    let batch_id = &input
        .job_batch_id
        .as_ref()
        .ok_or(StoreError::Corrupt("runtime batch ID"))?
        .value;
    let row = sqlx::query("SELECT * FROM holdout_batches WHERE batch_id=$1")
        .bind(batch_id)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(StoreError::Corrupt("runtime batch absent"))?;
    let handle =
        JobBatchHandle::decode(verified_blob(&row, "handle_blob", "handle_sha256")?.as_slice())
            .map_err(|_| StoreError::Corrupt("runtime batch envelope"))?;
    let index = input
        .evaluation_plan_entry_index
        .checked_sub(1)
        .ok_or(StoreError::Corrupt("runtime batch entry"))? as usize;
    if handle.job_batch_id != input.job_batch_id
        || handle.revision != 1
        || handle.holdout_grant_id != reference.holdout_grant_id
        || handle.holdout_evaluation_plan_id != input.holdout_evaluation_plan_id
        || handle.evaluation_plan_sha256 != input.evaluation_plan_sha256
        || handle.evaluation_plan_entry_count != reference.evaluation_plan_entry_count
        || handle.job_count != reference.evaluation_plan_entry_count
        || handle.job_ids.len() != handle.job_count as usize
        || handle.job_ids.get(index) != job.job_id.as_ref()
        || handle.created_at != job.submitted_at
        || row.try_get::<String, _>("grant_id")? != *grant_id
        || row.try_get::<String, _>("actor_id")?
            != job
                .submitted_by
                .as_ref()
                .and_then(|actor| actor.actor_id.as_ref())
                .ok_or(StoreError::Corrupt("runtime batch actor"))?
                .value
        || row.try_get::<String, _>("period_id")? != state::period_id(&current.grant)?
        || row.try_get::<String, _>("run_id")?
            != job
                .run_id
                .as_ref()
                .ok_or(StoreError::Corrupt("runtime run"))?
                .value
        || row.try_get::<String, _>("plan_id")?
            != input
                .holdout_evaluation_plan_id
                .as_ref()
                .ok_or(StoreError::Corrupt("runtime plan"))?
                .value
        || row.try_get::<Vec<u8>, _>("plan_sha256")?
            != resolve::digest(input.evaluation_plan_sha256.as_ref())?
        || row.try_get::<i32, _>("job_count")? != handle.job_count as i32
        || row.try_get::<i64, _>("created_at_ms")?
            != state::record_time(handle.created_at.as_ref())?
    {
        return Err(StoreError::Corrupt("runtime batch binding"));
    }
    let link = sqlx::query("SELECT entry_index, specification_sha256 FROM holdout_batch_jobs WHERE batch_id=$1 AND job_id=$2")
        .bind(batch_id).bind(&job.job_id.as_ref().ok_or(StoreError::Corrupt("runtime job ID"))?.value)
        .fetch_optional(&mut **transaction).await?.ok_or(StoreError::Corrupt("runtime batch link"))?;
    if link.try_get::<i32, _>("entry_index")? != input.evaluation_plan_entry_index as i32
        || link.try_get::<Vec<u8>, _>("specification_sha256")?
            != Sha256::digest(job.encode_to_vec()).as_slice()
    {
        return Err(StoreError::Corrupt("runtime batch job binding"));
    }
    Ok(())
}
