//! Authorized, bounded projections that remain available without an executor.

use loop_core::audit::{AuditAction, AuditEvent, AuditTargetKind, Sha256Digest};
use loop_protocol::wire::{discovery::v1 as wire, v1};
use sqlx::{Postgres, Row, Transaction, postgres::PgRow};

use super::{ModelStep, PgJobStore, StoreError, StoreResult, storage};
use crate::store::{audit, postgres, validate_id};

impl PgJobStore {
    /// Read job and verified history in one serialized owner/run-authorized
    /// observation. No plan, lease or model executor is required. Corrupt evidence
    /// and clock regression deny; cancellation drops the read transaction.
    pub(crate) async fn discovery_status(
        &self,
        actor: &v1::Actor,
        job_id: &str,
    ) -> StoreResult<(v1::JobRecord, Vec<ModelStep>)> {
        validate_id(job_id)?;
        let mut transaction = self.pool.begin().await?;
        self.observe_clock(&mut transaction).await?;
        let job = self.observed_job(&mut transaction, actor, job_id).await?;
        let history = storage::history(&mut transaction, &job).await?;
        transaction.commit().await?;
        Ok((job, history))
    }

    /// Return only allowlisted job events after authorizing the original owner
    /// and run. Limits are 1..100, and nonzero cursors must identify this job.
    /// Each event and its immediate ledger predecessor are checked, without
    /// claiming a complete global-chain proof. Indexed scans read at most 101
    /// rows. Unsupported/corrupt evidence denies the page; cancellation drops
    /// the bounded transaction. Repeated reads never append audit or receipts.
    pub(crate) async fn discovery_events(
        &self,
        actor: &v1::Actor,
        job_id: &str,
        after: u64,
        limit: u32,
    ) -> StoreResult<wire::ListDiscoveryEventsResponse> {
        validate_id(job_id)?;
        if !(1..=100).contains(&limit) {
            return Err(StoreError::Invalid("discovery event limit"));
        }
        let cursor =
            i64::try_from(after).map_err(|_| StoreError::Invalid("discovery event cursor"))?;
        let mut transaction = self.pool.begin().await?;
        self.observe_clock(&mut transaction).await?;
        self.observed_job(&mut transaction, actor, job_id).await?;
        if after != 0 {
            let row = sqlx::query("SELECT * FROM audit_events WHERE job_id=$1 AND sequence=$2")
                .bind(job_id)
                .bind(cursor)
                .fetch_optional(&mut *transaction)
                .await?
                .ok_or(StoreError::Invalid("discovery event cursor"))?;
            checked_event(&mut transaction, &row, &self.ledger_id, job_id).await?;
        }
        let rows = sqlx::query(
            "SELECT * FROM audit_events WHERE job_id=$1 AND sequence>$2 ORDER BY sequence LIMIT $3",
        )
        .bind(job_id)
        .bind(cursor)
        .bind(i64::from(limit) + 1)
        .fetch_all(&mut *transaction)
        .await?;
        let has_more = rows.len() > limit as usize;
        let mut events = Vec::with_capacity(rows.len().min(limit as usize));
        for row in &rows {
            let event = checked_event(&mut transaction, row, &self.ledger_id, job_id).await?;
            if events.len() < limit as usize {
                events.push(event);
            }
        }
        transaction.commit().await?;
        Ok(wire::ListDiscoveryEventsResponse {
            next_after_sequence: events.last().map_or(after, |event| event.sequence),
            events,
            has_more,
        })
    }

    async fn observed_job(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        actor: &v1::Actor,
        job_id: &str,
    ) -> StoreResult<v1::JobRecord> {
        let row = sqlx::query("SELECT * FROM jobs WHERE job_id=$1")
            .bind(job_id)
            .fetch_optional(&mut **transaction)
            .await?
            .ok_or(StoreError::NotFound)?;
        let job = postgres::record_from_row(&row)?;
        self.admission
            .authorize_job_command("loop.discovery.read_control", actor, &job)?;
        if job.specification.as_ref().is_none_or(|specification| {
            specification.kind != v1::JobKind::Discovery as i32
                || specification.submitted_by.as_ref() != Some(actor)
        }) {
            return Err(StoreError::AdmissionDenied);
        }
        Ok(job)
    }
}

async fn checked_event(
    transaction: &mut Transaction<'_, Postgres>,
    row: &PgRow,
    ledger: &str,
    job_id: &str,
) -> StoreResult<wire::DiscoveryEvent> {
    let event = audit::event_from_row(row, ledger)?;
    if event.target.kind != AuditTargetKind::JobId
        || event.target.value != job_id
        || row.try_get::<Option<String>, _>("job_id")?.as_deref() != Some(job_id)
    {
        return Err(StoreError::Corrupt("discovery event target"));
    }
    let previous = if event.sequence == 1 {
        Sha256Digest::ZERO
    } else {
        let row = sqlx::query("SELECT * FROM audit_events WHERE sequence=$1")
            .bind((event.sequence - 1) as i64)
            .fetch_optional(&mut **transaction)
            .await?
            .ok_or(StoreError::Corrupt("discovery event predecessor"))?;
        audit::event_from_row(&row, ledger)?.event_sha256
    };
    if event.previous_event_sha256 != previous {
        return Err(StoreError::Corrupt("discovery event predecessor"));
    }
    let operation = operation(&event)?;
    Ok(wire::DiscoveryEvent {
        sequence: event.sequence,
        occurred_at: Some(postgres::timestamp(row.try_get("occurred_at_ms")?)),
        operation: operation as i32,
    })
}

fn operation(event: &AuditEvent) -> StoreResult<wire::DiscoveryOperation> {
    if event.action != AuditAction::CommandAccepted
        || event.payload.schema_name != "loop.audit.command_accepted"
        || event.payload.schema_version != 1
    {
        return Err(StoreError::Corrupt("unsupported discovery event"));
    }
    let payload: serde_json::Value = serde_json::from_slice(&event.payload.canonical_bytes)
        .map_err(|_| StoreError::Corrupt("discovery event payload"))?;
    use wire::DiscoveryOperation as Operation;
    match payload.get("command").and_then(serde_json::Value::as_str) {
        Some("loop.discovery.start" | "loop.jobs.submit") => Ok(Operation::Start),
        Some("loop.model.reserve") => Ok(Operation::Reserve),
        Some("loop.model.dispatch") => Ok(Operation::Dispatch),
        Some("loop.model.uncertain") => Ok(Operation::Uncertain),
        Some("loop.model.finish") => Ok(Operation::Finish),
        Some("loop.model.call") => Ok(Operation::Call),
        Some("loop.tool.record") => Ok(Operation::ToolRecord),
        Some("loop.model.takeover") => Ok(Operation::Takeover),
        Some("loop.model.resume") => Ok(Operation::Resume),
        Some("loop.model.retry") => Ok(Operation::Retry),
        Some("loop.model.fail") => Ok(Operation::Fail),
        Some("loop.model.reconcile") => Ok(Operation::Reconcile),
        Some("loop.discovery.pause") => Ok(Operation::Pause),
        Some("loop.discovery.cancel") => Ok(Operation::Cancel),
        Some("loop.discovery.expire") => Ok(Operation::Expire),
        _ => Err(StoreError::Corrupt("unsupported discovery operation")),
    }
}
