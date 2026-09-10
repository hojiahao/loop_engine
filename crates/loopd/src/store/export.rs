use loop_core::audit::{AuditAction, AuditTarget, AuditTargetKind, canonicalize_audit_payload};
use loop_protocol::wire::v1::{Actor, BacktestResult, CommandContext, JobId, JobRecord};
use prost::Message;
use prost_types::Timestamp;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction};

use super::lifecycle::validate_context;
use super::postgres::{encode_message, timestamp, timestamp_millis, verified_blob};
use super::{PgJobStore, StoreError, StoreResult, audit, backtest, validate_id};

const OPERATION: &str = "loop.backtests.export_current";

/// Internal metadata-release command, not a public RPC or reusable capability.
#[derive(Clone, PartialEq, Message)]
pub struct ExportBacktest {
    /// Request attribution bound to the separately authenticated principal.
    #[prost(message, optional, tag = "1")]
    pub context: Option<CommandContext>,
    /// Successful backtest whose immutable result is requested.
    #[prost(message, optional, tag = "2")]
    pub job_id: Option<JobId>,
    /// Explicit immutable current context resolved by the service, not `latest`.
    #[prost(string, tag = "3")]
    pub context_id: String,
    /// Required absolute deadline, at most 30 seconds after request time. Each
    /// retry supplies its own deadline; it is not part of semantic retry identity.
    #[prost(message, optional, tag = "4")]
    pub deadline: Option<Timestamp>,
}

/// Metadata released after audit/receipt commit and fresh permission checks.
/// Consumers must not treat this value as authority to fetch protected artifacts
/// or as an assertion that a downstream file was delivered.
#[derive(Clone, Debug, PartialEq)]
pub struct BacktestExport {
    /// Source job; the export does not advance its terminal revision.
    pub job_id: String,
    /// Immutable context for which the metadata was verified current.
    pub context_id: String,
    /// Verified metrics and immutable series references, never inline datasets.
    pub result: BacktestResult,
    /// Time of the original accepted metadata-release decision.
    pub accepted_at: Timestamp,
    /// True when an existing receipt was revalidated without a new audit append.
    pub replayed: bool,
}

#[derive(Clone, PartialEq, Message)]
struct Receipt {
    #[prost(message, optional, tag = "1")]
    job: Option<JobRecord>,
    #[prost(message, optional, tag = "2")]
    result: Option<BacktestResult>,
    #[prost(string, tag = "3")]
    context_id: String,
    #[prost(message, optional, tag = "4")]
    accepted_at: Option<Timestamp>,
    #[prost(string, tag = "5")]
    request_id: String,
}

impl ExportBacktest {
    fn normalized(&self) -> Self {
        let mut command = self.clone();
        if let Some(context) = &mut command.context {
            context.request_id = None;
            context.requested_at = None;
        }
        command.deadline = None;
        command
    }
}

pub(super) async fn execute(
    store: &PgJobStore,
    principal: &Actor,
    command: ExportBacktest,
) -> StoreResult<BacktestExport> {
    let context = validate_context(command.context.as_ref(), principal)?;
    let job_id = &command
        .job_id
        .as_ref()
        .ok_or(StoreError::Invalid("export job id"))?
        .value;
    validate_id(job_id)?;
    validate_id(&command.context_id)?;
    let requested_at = timestamp_millis(
        context
            .requested_at
            .as_ref()
            .ok_or(StoreError::Invalid("command time"))?,
        true,
    )?;
    let deadline = timestamp_millis(
        command
            .deadline
            .as_ref()
            .ok_or(StoreError::Invalid("export deadline"))?,
        true,
    )?;
    if deadline <= requested_at || deadline - requested_at > 30_000 {
        return Err(StoreError::Invalid("export deadline"));
    }
    let actor_id = &principal
        .actor_id
        .as_ref()
        .ok_or(StoreError::AdmissionDenied)?
        .value;
    let key = &context
        .idempotency_key
        .as_ref()
        .ok_or(StoreError::Invalid("command key"))?
        .value;
    let request_id = &context
        .request_id
        .as_ref()
        .ok_or(StoreError::Invalid("request id"))?
        .value;
    let normalized = command.normalized();
    let request_blob = encode_message(&normalized)?;
    let mut transaction = store.pool.begin().await?;
    let now = store.observe_clock(&mut transaction).await?;
    check_time(now, requested_at, deadline)?;
    // Reads and retries share the same permission, evidence and freshness gate.
    let (record, result) = backtest::current_in_transaction(
        store,
        &mut transaction,
        principal,
        job_id,
        &command.context_id,
        OPERATION,
    )
    .await?;
    let row = sqlx::query(
        "SELECT * FROM command_receipts WHERE actor_id = $1 AND operation = $2 AND idempotency_key = $3",
    ).bind(actor_id).bind(OPERATION).bind(key).fetch_optional(&mut *transaction).await?;
    if let Some(row) = row {
        let original = ExportBacktest::decode(
            verified_blob(&row, "request_blob", "request_sha256")?.as_slice(),
        )
        .map_err(|_| StoreError::Corrupt("export receipt request"))?;
        if original != normalized {
            return Err(StoreError::IdempotencyConflict);
        }
        let receipt =
            Receipt::decode(verified_blob(&row, "response_blob", "response_sha256")?.as_slice())
                .map_err(|_| StoreError::Corrupt("export receipt response"))?;
        let accepted_at = receipt
            .accepted_at
            .as_ref()
            .ok_or(StoreError::Corrupt("export receipt time"))?;
        let accepted_ms = timestamp_millis(accepted_at, false)?;
        let completed_ms = timestamp_millis(
            record
                .updated_at
                .as_ref()
                .ok_or(StoreError::Corrupt("job update time"))?,
            false,
        )?;
        if receipt.job.as_ref() != Some(&record)
            || receipt.result.as_ref() != Some(&result)
            || receipt.context_id != command.context_id
            || row.try_get::<String, _>("job_id")? != *job_id
            || row.try_get::<String, _>("request_id")? != receipt.request_id
            || row.try_get::<i64, _>("committed_at_ms")? != accepted_ms
            || accepted_ms < completed_ms
            || accepted_ms > now
        {
            return Err(StoreError::Corrupt("export receipt binding"));
        }
        validate_id(&receipt.request_id)
            .map_err(|_| StoreError::Corrupt("export receipt request id"))?;
        final_time(store, now, deadline)?;
        transaction.commit().await?;
        return Ok(BacktestExport {
            job_id: job_id.clone(),
            context_id: command.context_id,
            result,
            accepted_at: *accepted_at,
            replayed: true,
        });
    }
    let accepted_ms = final_time(store, now, deadline)?;
    let accepted_at = timestamp(accepted_ms);
    let receipt = Receipt {
        job: Some(record),
        result: Some(result.clone()),
        context_id: command.context_id.clone(),
        accepted_at: Some(accepted_at),
        request_id: request_id.clone(),
    };
    insert_receipt(
        &mut transaction,
        context,
        job_id,
        &request_blob,
        &receipt,
        accepted_ms,
    )
    .await?;
    #[cfg(test)]
    super::crash_tests::fault_point("export_after_receipt").await;
    audit::append(
        &mut transaction,
        &store.ledger_id,
        accepted_ms,
        audit::EventInput {
            actor: principal,
            correlation_id: &context
                .correlation_id
                .as_ref()
                .ok_or(StoreError::Invalid("correlation id"))?
                .value,
            causation_id: &context
                .causation_id
                .as_ref()
                .ok_or(StoreError::Invalid("causation id"))?
                .value,
            action: AuditAction::CommandAccepted,
            target: AuditTarget {
                kind: AuditTargetKind::JobId,
                value: job_id.clone(),
            },
            payload: canonicalize_audit_payload(
                "loop.audit.command_accepted",
                1,
                &serde_json::to_vec(&serde_json::json!({
                    "command": OPERATION, "request_id": request_id,
                    "summary": format!("current metadata release; context={}", command.context_id),
                }))
                .map_err(|_| StoreError::Invalid("export audit payload"))?,
            )?,
        },
    )
    .await?;
    #[cfg(test)]
    super::crash_tests::fault_point("export_before_commit").await;
    final_time(store, accepted_ms, deadline)?;
    transaction.commit().await?;
    #[cfg(test)]
    super::crash_tests::fault_point("export_after_commit").await;
    Ok(BacktestExport {
        job_id: job_id.clone(),
        context_id: command.context_id,
        result,
        accepted_at,
        replayed: false,
    })
}

fn check_time(now: i64, requested_at: i64, deadline: i64) -> StoreResult<()> {
    if requested_at > now {
        return Err(StoreError::Invalid("future command time"));
    }
    if now >= deadline {
        return Err(StoreError::Unavailable("export deadline exceeded"));
    }
    Ok(())
}

fn final_time(store: &PgJobStore, previous: i64, deadline: i64) -> StoreResult<i64> {
    let now = store.clock.now_millis()?;
    if now < previous {
        return Err(StoreError::ClockRegression);
    }
    check_time(now, previous, deadline)?;
    Ok(now)
}

async fn insert_receipt(
    transaction: &mut Transaction<'_, Postgres>,
    context: &CommandContext,
    job_id: &str,
    request: &[u8],
    receipt: &Receipt,
    now: i64,
) -> StoreResult<()> {
    let response = encode_message(receipt)?;
    sqlx::query(
        "INSERT INTO command_receipts (actor_id, operation, idempotency_key, request_id, job_id,
        request_blob, request_sha256, response_blob, response_sha256, committed_at_ms)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(
        &context
            .actor
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?
            .actor_id
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?
            .value,
    )
    .bind(OPERATION)
    .bind(
        &context
            .idempotency_key
            .as_ref()
            .ok_or(StoreError::Invalid("command key"))?
            .value,
    )
    .bind(&receipt.request_id)
    .bind(job_id)
    .bind(request)
    .bind(Sha256::digest(request).to_vec())
    .bind(&response)
    .bind(Sha256::digest(&response).to_vec())
    .bind(now)
    .execute(&mut **transaction)
    .await?;
    sqlx::query("UPDATE store_metadata SET last_observed_at_ms = $1 WHERE singleton = 1")
        .bind(now)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}
