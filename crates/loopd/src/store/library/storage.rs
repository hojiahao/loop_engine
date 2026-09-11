use loop_protocol::wire::v1::CommandContext;
use prost::Message;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction, postgres::PgRow};

use super::types::Receipt;
use super::{FactorState, OPERATION};
use crate::store::postgres::{encode_message, timestamp_millis, verified_blob};
use crate::store::{StoreError, StoreResult};

pub(super) async fn state(
    transaction: &mut Transaction<'_, Postgres>,
    context: &str,
    factor: &str,
) -> StoreResult<Option<FactorState>> {
    let row = sqlx::query("SELECT s.*, r.response_blob, r.response_sha256, r.committed_at_ms FROM factor_states s LEFT JOIN command_receipts r USING (actor_id, operation, idempotency_key) WHERE s.context_id = $1 AND s.factor_spec_id = $2")
        .bind(context).bind(factor).fetch_optional(&mut **transaction).await?;
    row.as_ref().map(verified_state).transpose()
}

pub(super) async fn active(
    transaction: &mut Transaction<'_, Postgres>,
    context: &str,
) -> StoreResult<Vec<FactorState>> {
    let rows = sqlx::query("SELECT s.*, r.response_blob, r.response_sha256, r.committed_at_ms FROM factor_states s LEFT JOIN command_receipts r USING (actor_id, operation, idempotency_key) WHERE s.context_id = $1 AND s.status = 'admitted' ORDER BY s.factor_spec_id LIMIT 4097")
        .bind(context).fetch_all(&mut **transaction).await?;
    if rows.len() > 4096 {
        return Err(StoreError::Unavailable("active library capacity"));
    }
    rows.iter().map(verified_state).collect()
}

fn verified_state(row: &PgRow) -> StoreResult<FactorState> {
    let state = FactorState {
        factor_spec_id: row.try_get("factor_spec_id")?,
        revision: row
            .try_get::<i64, _>("revision")?
            .try_into()
            .map_err(|_| StoreError::Corrupt("factor revision"))?,
        status: row.try_get("status")?,
        admissions: row
            .try_get::<i64, _>("admissions")?
            .try_into()
            .map_err(|_| StoreError::Corrupt("factor admissions"))?,
        retirements: row
            .try_get::<i64, _>("retirements")?
            .try_into()
            .map_err(|_| StoreError::Corrupt("factor retirements"))?,
        source_job_id: row.try_get("source_job_id")?,
    };
    let receipt =
        Receipt::decode(verified_blob(row, "response_blob", "response_sha256")?.as_slice())
            .map_err(|_| StoreError::Corrupt("factor receipt encoding"))?;
    super::validate_receipt(&receipt)?;
    let command = receipt
        .command
        .as_ref()
        .ok_or(StoreError::Corrupt("factor receipt command"))?;
    let context = command
        .context
        .as_ref()
        .ok_or(StoreError::Corrupt("factor receipt context"))?;
    if !receipt.states.contains(&state)
        || receipt
            .states
            .iter()
            .filter(|entry| entry.factor_spec_id == state.factor_spec_id)
            .count()
            != 1
        || row.try_get::<String, _>("context_id")? != command.context_id
        || row.try_get::<String, _>("actor_id")?
            != context
                .actor
                .as_ref()
                .and_then(|actor| actor.actor_id.as_ref())
                .ok_or(StoreError::Corrupt("factor receipt actor"))?
                .value
        || row.try_get::<String, _>("idempotency_key")?
            != context
                .idempotency_key
                .as_ref()
                .ok_or(StoreError::Corrupt("factor receipt key"))?
                .value
        || receipt.principal != context.actor
        || row.try_get::<i64, _>("updated_at_ms")? != row.try_get::<i64, _>("committed_at_ms")?
        || row.try_get::<i64, _>("updated_at_ms")?
            != timestamp_millis(
                receipt
                    .accepted_at
                    .as_ref()
                    .ok_or(StoreError::Corrupt("factor receipt time"))?,
                false,
            )?
    {
        return Err(StoreError::Corrupt("factor state receipt binding"));
    }
    Ok(state)
}

pub(super) async fn write_states(
    transaction: &mut Transaction<'_, Postgres>,
    receipt: &Receipt,
) -> StoreResult<()> {
    let command = receipt
        .command
        .as_ref()
        .ok_or(StoreError::Invalid("factor command"))?;
    let context = command
        .context
        .as_ref()
        .ok_or(StoreError::Invalid("factor context"))?;
    let (actor, key) = receipt_key(context)?;
    let now = timestamp_millis(
        receipt
            .accepted_at
            .as_ref()
            .ok_or(StoreError::Invalid("factor time"))?,
        true,
    )?;
    for state in &receipt.states {
        let sql = if state.revision == 1 {
            "INSERT INTO factor_states (context_id, factor_spec_id, revision, status, admissions, retirements, source_job_id, updated_at_ms, actor_id, operation, idempotency_key) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"
        } else {
            "UPDATE factor_states SET revision = $3, status = $4, admissions = $5, retirements = $6, source_job_id = $7, updated_at_ms = $8, actor_id = $9, operation = $10, idempotency_key = $11 WHERE context_id = $1 AND factor_spec_id = $2 AND revision = $3 - 1"
        };
        let changed = sqlx::query(sql)
            .bind(&command.context_id)
            .bind(&state.factor_spec_id)
            .bind(state.revision as i64)
            .bind(&state.status)
            .bind(state.admissions as i64)
            .bind(state.retirements as i64)
            .bind(&state.source_job_id)
            .bind(now)
            .bind(actor)
            .bind(OPERATION)
            .bind(key)
            .execute(&mut **transaction)
            .await?;
        if changed.rows_affected() != 1 {
            return Err(StoreError::RevisionConflict);
        }
    }
    Ok(())
}

pub(super) async fn load_receipt(
    transaction: &mut Transaction<'_, Postgres>,
    command: &super::DecideFactor,
) -> StoreResult<Option<Receipt>> {
    let context = command
        .context
        .as_ref()
        .ok_or(StoreError::Invalid("factor context"))?;
    let (actor, key) = receipt_key(context)?;
    let row = sqlx::query("SELECT * FROM command_receipts WHERE actor_id = $1 AND operation = $2 AND idempotency_key = $3")
        .bind(actor).bind(OPERATION).bind(key).fetch_optional(&mut **transaction).await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let request = super::DecideFactor::decode(
        verified_blob(&row, "request_blob", "request_sha256")?.as_slice(),
    )
    .map_err(|_| StoreError::Corrupt("factor request encoding"))?;
    if request != command.normalized() {
        return Err(StoreError::IdempotencyConflict);
    }
    let receipt =
        Receipt::decode(verified_blob(&row, "response_blob", "response_sha256")?.as_slice())
            .map_err(|_| StoreError::Corrupt("factor response encoding"))?;
    let original = receipt
        .command
        .as_ref()
        .ok_or(StoreError::Corrupt("factor original request"))?;
    if original.normalized() != request
        || original
            .context
            .as_ref()
            .and_then(|c| c.request_id.as_ref())
            .ok_or(StoreError::Corrupt("factor request id"))?
            .value
            != row.try_get::<String, _>("request_id")?
        || original
            .source_job_id
            .as_ref()
            .ok_or(StoreError::Corrupt("factor source job"))?
            .value
            != row.try_get::<String, _>("job_id")?
        || timestamp_millis(
            receipt
                .accepted_at
                .as_ref()
                .ok_or(StoreError::Corrupt("factor time"))?,
            false,
        )? != row.try_get::<i64, _>("committed_at_ms")?
    {
        return Err(StoreError::Corrupt("factor receipt binding"));
    }
    Ok(Some(receipt))
}

pub(super) async fn save_receipt(
    transaction: &mut Transaction<'_, Postgres>,
    receipt: &Receipt,
) -> StoreResult<()> {
    let command = receipt
        .command
        .as_ref()
        .ok_or(StoreError::Invalid("factor command"))?;
    let context = command
        .context
        .as_ref()
        .ok_or(StoreError::Invalid("factor context"))?;
    let (actor, key) = receipt_key(context)?;
    let request = encode_message(&command.normalized())?;
    let response = encode_message(receipt)?;
    let now = timestamp_millis(
        receipt
            .accepted_at
            .as_ref()
            .ok_or(StoreError::Invalid("factor time"))?,
        true,
    )?;
    sqlx::query("INSERT INTO command_receipts (actor_id, operation, idempotency_key, request_id, job_id, request_blob, request_sha256, response_blob, response_sha256, committed_at_ms) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
        .bind(actor).bind(OPERATION).bind(key)
        .bind(&context.request_id.as_ref().ok_or(StoreError::Invalid("request id"))?.value)
        .bind(&command.source_job_id.as_ref().ok_or(StoreError::Invalid("source job"))?.value)
        .bind(&request).bind(Sha256::digest(&request).to_vec())
        .bind(&response).bind(Sha256::digest(&response).to_vec()).bind(now)
        .execute(&mut **transaction).await?;
    sqlx::query("UPDATE store_metadata SET last_observed_at_ms = $1 WHERE singleton = 1")
        .bind(now)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

fn receipt_key(context: &CommandContext) -> StoreResult<(&str, &str)> {
    let actor = context
        .actor
        .as_ref()
        .and_then(|a| a.actor_id.as_ref())
        .ok_or(StoreError::AdmissionDenied)?;
    let key = context
        .idempotency_key
        .as_ref()
        .ok_or(StoreError::Invalid("factor key"))?;
    Ok((&actor.value, &key.value))
}
