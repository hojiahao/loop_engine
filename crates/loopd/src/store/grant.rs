mod closing;
mod resolve;
mod state;

use std::collections::BTreeMap;

use loop_core::audit::{AuditAction, AuditTarget, AuditTargetKind};
use loop_protocol::wire::holdout::v1::{RequestHoldoutGrantRequest, RequestHoldoutGrantResponse};
use loop_protocol::wire::v1::{
    Actor, CommandContext, FreezeManifestReference, HoldoutGrantId, HoldoutGrantRecord,
    HoldoutGrantReference, HoldoutGrantState, HoldoutPeriodRecord, HoldoutPeriodState,
};
use prost::{Enumeration, Message};
use sqlx::{Postgres, Transaction};

use super::lifecycle::validate_context;
use super::postgres::{encode_message, timestamp, timestamp_millis, verified_blob};
use super::{PgJobStore, StoreError, StoreResult, audit, holdout, validate_id};

pub(super) const ISSUE: &str = "loop.holdout.issue-grant";
pub(super) const READ: &str = "loop.holdout.read-grant";

/// Bounded immutable metadata returned only by a server-owned freeze resolver.
/// This ordinary value is not an authority token: the store reparses canonical
/// bytes and bindings, while the resolver must validate the owning freeze and
/// BacktestSpec schemas. Market data must never be placed in this structure.
#[derive(Clone, Debug)]
pub struct ResolvedFreeze {
    /// Exact persisted freeze reference, including its pinned approval policy.
    pub reference: FreezeManifestReference,
    /// Complete canonical plan metadata, limited to the protocol's eight MiB.
    pub canonical_plan: Vec<u8>,
    /// Trusted immutable plan-schema registry digest, not supplied by the caller.
    pub plan_schema_sha256: [u8; 32],
    /// Trusted immutable BacktestSpec-schema registry digest.
    pub backtest_schema_sha256: [u8; 32],
    /// Exact referenced BacktestSpec documents, at most 64 MiB in aggregate.
    /// The resolver's owning parser must independently verify their semantics.
    pub backtest_artifacts: BTreeMap<String, Vec<u8>>,
}

/// Original transactional grant response, not a bearer capability.
#[derive(Clone, Debug, PartialEq)]
pub struct GrantResult {
    /// Grant at this command's original commit, possibly older than current state.
    pub grant: HoldoutGrantRecord,
    /// Period advanced atomically alongside this command's grant state.
    pub period: HoldoutPeriodRecord,
    /// True if the original immutable receipt was returned without a new event.
    pub replayed: bool,
}

/// Explicit terminal disposition; neither case permits issuing a replacement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Enumeration)]
#[repr(i32)]
pub enum GrantClosure {
    /// Close at or after the half-open validity interval has ended.
    Expire = 1,
    /// Explicitly withdraw an issued, still-current grant with a recorded reason.
    Revoke = 2,
}

/// Trusted internal close command. No transport endpoint is registered here.
#[derive(Clone, PartialEq, Message)]
pub struct CloseGrant {
    /// Metadata checked against a separately authenticated principal.
    #[prost(message, optional, tag = "1")]
    pub context: Option<CommandContext>,
    /// Exact reference to the originally issued grant; conveys no authority.
    #[prost(message, optional, tag = "2")]
    pub grant_reference: Option<HoldoutGrantReference>,
    /// Compare-and-swap revision of the issued grant.
    #[prost(uint64, tag = "3")]
    pub expected_grant_revision: u64,
    /// Compare-and-swap revision of its grant-issued period.
    #[prost(uint64, tag = "4")]
    pub expected_period_revision: u64,
    /// Expiry or revocation, separately authorized by server policy.
    #[prost(enumeration = "GrantClosure", tag = "5")]
    pub disposition: i32,
    /// Non-blank explanation, bounded to 4,096 UTF-8 bytes and audited.
    #[prost(string, tag = "6")]
    pub reason: String,
}

pub(super) use closing::close;

pub(super) async fn issue(
    store: &PgJobStore,
    principal: &Actor,
    command: RequestHoldoutGrantRequest,
) -> StoreResult<GrantResult> {
    let context = validate_context(command.context.as_ref(), principal)?;
    let (period_id, requested_freeze) = resolve::validate_request(&command)?;
    store
        .holdout_policy
        .authorize_period(ISSUE, principal, period_id)?;
    let mut normalized = command.clone();
    normalize_context(&mut normalized.context);
    let request_blob = encode_message(&normalized)?;
    let mut transaction = store.pool.begin().await?;
    let now = command_time(store, &mut transaction, context).await?;
    store
        .holdout_policy
        .authorize_period(ISSUE, principal, period_id)?;
    let period = state::load_period(&mut transaction, period_id).await?;
    let resolved = resolve::resolve_freeze(store, requested_freeze, &period, now)?;
    if let Some(receipt) = receipt(&mut transaction, context, ISSUE).await? {
        let previous = RequestHoldoutGrantRequest::decode(
            verified_blob(&receipt, "request_blob", "request_sha256")?.as_slice(),
        )
        .map_err(|_| StoreError::Corrupt("grant request receipt"))?;
        if previous != normalized {
            return Err(StoreError::IdempotencyConflict);
        }
        let original = state::decode_response(
            &verified_blob(&receipt, "response_blob", "response_sha256")?,
            &period.canonical_bytes,
        )?;
        let current =
            state::load_grant(&mut transaction, state::grant_id(&original.grant)?).await?;
        state::verify_issued_receipt(&original, &current, &command, &receipt, now)?;
        transaction.commit().await?;
        return Ok(GrantResult {
            replayed: true,
            ..original
        });
    }
    if period.record.revision != command.expected_period_revision {
        return Err(StoreError::RevisionConflict);
    }
    if period.record.state != HoldoutPeriodState::Sealed as i32 {
        return Err(StoreError::InvalidTransition);
    }
    let approvals =
        resolve::load_approvals(&mut transaction, &command, &resolved.reference, now).await?;
    let expires = store
        .holdout_policy
        .grant_expiry(&resolved.reference, &approvals, now)?;
    resolve::validate_expiry(now, expires)?;
    for record in &approvals {
        if expires > state::record_time(record.expires_at.as_ref())? {
            return Err(StoreError::Invalid("grant outlives approval"));
        }
    }
    let reference = HoldoutGrantReference {
        holdout_grant_id: Some(HoldoutGrantId {
            value: format!("grant.{}", uuid::Uuid::new_v4().simple()),
        }),
        holdout_period_id: command.holdout_period_id.clone(),
        freeze_manifest_sha256: resolved
            .reference
            .manifest
            .as_ref()
            .and_then(|value| value.sha256.clone()),
        issued_at: Some(timestamp(now)),
        expires_at: Some(timestamp(expires)),
        holdout_evaluation_plan_id: resolved
            .reference
            .holdout_evaluation_plan
            .as_ref()
            .and_then(|value| value.holdout_evaluation_plan_id.clone()),
        evaluation_plan_sha256: resolved
            .reference
            .holdout_evaluation_plan
            .as_ref()
            .and_then(|value| value.plan_sha256.clone()),
        evaluation_plan_entry_count: resolved
            .reference
            .holdout_evaluation_plan
            .as_ref()
            .expect("validated plan")
            .entry_count,
        canonical_period_sha256: command.canonical_period_sha256.clone(),
    };
    let grant = HoldoutGrantRecord {
        reference: Some(reference.clone()),
        state: HoldoutGrantState::Issued as i32,
        revision: 1,
        approval_records: approvals.iter().map(resolve::approval_reference).collect(),
        consumed_at: None,
        approval_policy: resolved.reference.holdout_approval_policy.clone(),
        holdout_evaluation_plan_id: reference.holdout_evaluation_plan_id.clone(),
        evaluation_plan_sha256: reference.evaluation_plan_sha256.clone(),
        evaluation_plan_entry_count: reference.evaluation_plan_entry_count,
        canonical_period_sha256: reference.canonical_period_sha256.clone(),
    };
    let mut updated = period.record;
    updated.state = HoldoutPeriodState::GrantIssued as i32;
    updated.revision = 2;
    updated.issued_grant_id = reference.holdout_grant_id;
    updated.grant_issued_at = Some(timestamp(now));
    state::insert_grant(&mut transaction, &grant, &resolved.reference, &approvals).await?;
    state::update_period(&mut transaction, &updated, command.expected_period_revision).await?;
    audit::append(
        &mut transaction,
        &store.ledger_id,
        now,
        event(
            context,
            principal,
            AuditAction::HoldoutGrantIssued,
            AuditTarget {
                kind: AuditTargetKind::HoldoutGrantId,
                value: state::grant_id(&grant)?.to_owned(),
            },
            resolve::issued_payload(&grant)?,
        ),
    )
    .await?;
    let result = GrantResult {
        grant,
        period: updated,
        replayed: false,
    };
    save_result(
        &mut transaction,
        context,
        ISSUE,
        &request_blob,
        &result,
        now,
    )
    .await?;
    #[cfg(test)]
    super::crash_tests::fault_point("grant_before_commit").await;
    transaction.commit().await?;
    #[cfg(test)]
    super::crash_tests::fault_point("grant_after_commit").await;
    Ok(result)
}

pub(super) async fn get(
    store: &PgJobStore,
    principal: &Actor,
    grant_id: &str,
) -> StoreResult<Option<HoldoutGrantRecord>> {
    validate_id(grant_id)?;
    store
        .holdout_policy
        .authorize_grant_read(principal, grant_id)?;
    let mut transaction = store.pool.begin().await?;
    // One repeatable-read view avoids mixing a grant with a concurrently closed period.
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *transaction)
        .await?;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT FROM holdout_grants WHERE grant_id = $1)")
            .bind(grant_id)
            .fetch_one(&mut *transaction)
            .await?;
    if !exists {
        return Ok(None);
    }
    let state = state::load_grant(&mut transaction, grant_id).await?;
    store
        .holdout_policy
        .authorize_period(READ, principal, state::period_id(&state.grant)?)?;
    transaction.commit().await?;
    Ok(Some(state.grant))
}

pub(super) fn normalize_context(context: &mut Option<CommandContext>) {
    if let Some(context) = context {
        context.request_id = None;
        context.requested_at = None;
    }
}

pub(super) async fn command_time(
    store: &PgJobStore,
    transaction: &mut Transaction<'_, Postgres>,
    context: &CommandContext,
) -> StoreResult<i64> {
    let now = store.observe_clock(transaction).await?;
    if timestamp_millis(
        context
            .requested_at
            .as_ref()
            .ok_or(StoreError::Invalid("command time"))?,
        true,
    )? > now
    {
        return Err(StoreError::Invalid("future command time"));
    }
    Ok(now)
}

pub(super) async fn receipt(
    transaction: &mut Transaction<'_, Postgres>,
    context: &CommandContext,
    operation: &str,
) -> StoreResult<Option<sqlx::postgres::PgRow>> {
    Ok(sqlx::query("SELECT * FROM holdout_command_receipts WHERE actor_id = $1 AND operation = $2 AND idempotency_key = $3")
        .bind(&context.actor.as_ref().expect("validated actor").actor_id.as_ref().expect("validated actor id").value)
        .bind(operation).bind(&context.idempotency_key.as_ref().expect("validated key").value)
        .fetch_optional(&mut **transaction).await?)
}

pub(super) async fn save_result(
    transaction: &mut Transaction<'_, Postgres>,
    context: &CommandContext,
    operation: &str,
    request: &[u8],
    result: &GrantResult,
    now: i64,
) -> StoreResult<()> {
    let response = RequestHoldoutGrantResponse {
        grant: Some(result.grant.clone()),
        period_record: Some(result.period.clone()),
    };
    holdout::save_receipt(
        transaction,
        context,
        operation,
        state::period_id(&result.grant)?,
        request,
        &encode_message(&response)?,
        now,
    )
    .await
}

pub(super) fn event<'a>(
    context: &'a CommandContext,
    principal: &'a Actor,
    action: AuditAction,
    target: AuditTarget,
    payload: loop_core::audit::AuditPayload,
) -> audit::EventInput<'a> {
    audit::EventInput {
        actor: principal,
        correlation_id: &context
            .correlation_id
            .as_ref()
            .expect("validated correlation")
            .value,
        causation_id: &context
            .causation_id
            .as_ref()
            .expect("validated causation")
            .value,
        action,
        target,
        payload,
    }
}
