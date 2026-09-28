use loop_core::audit::{AuditAction, AuditTarget, AuditTargetKind, state_transition_payload};
use loop_protocol::wire::v1::{Actor, HoldoutGrantState, HoldoutPeriodState};
use prost::Message;
use sqlx::Row;

use super::super::lifecycle::validate_context;
use super::super::postgres::{encode_message, timestamp, verified_blob};
use super::super::validate_id;
use super::{CloseGrant, GrantClosure, GrantResult, PgJobStore, StoreError, StoreResult, state};

pub(in crate::store) async fn close(
    store: &PgJobStore,
    principal: &Actor,
    command: CloseGrant,
) -> StoreResult<GrantResult> {
    let context = validate_context(command.context.as_ref(), principal)?;
    let reference = command
        .grant_reference
        .as_ref()
        .ok_or(StoreError::Invalid("close grant reference"))?;
    let id = &reference
        .holdout_grant_id
        .as_ref()
        .ok_or(StoreError::Invalid("close grant identity"))?
        .value;
    let period_id = &reference
        .holdout_period_id
        .as_ref()
        .ok_or(StoreError::Invalid("close period identity"))?
        .value;
    validate_id(id)?;
    validate_id(period_id)?;
    for revision in [
        command.expected_grant_revision,
        command.expected_period_revision,
    ] {
        if revision == 0 || revision > i64::MAX as u64 {
            return Err(StoreError::Invalid("close revision"));
        }
    }
    if command.reason.trim().is_empty()
        || command.reason.len() > 4096
        || command
            .reason
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(StoreError::Invalid("close reason"));
    }
    let disposition = GrantClosure::try_from(command.disposition)
        .map_err(|_| StoreError::Invalid("grant disposition"))?;
    let (operation, terminal_state) = match disposition {
        GrantClosure::Expire => ("loop.holdout.expire-grant", HoldoutGrantState::Expired),
        GrantClosure::Revoke => ("loop.holdout.revoke-grant", HoldoutGrantState::Revoked),
    };
    store
        .holdout_policy
        .authorize_period(operation, principal, period_id)?;
    let mut normalized = command.clone();
    super::normalize_context(&mut normalized.context);
    let request_blob = encode_message(&normalized)?;
    let mut transaction = store.pool.begin().await?;
    let now = super::command_time(store, &mut transaction, context).await?;
    store
        .holdout_policy
        .authorize_period(operation, principal, period_id)?;
    let current = state::load_grant(&mut transaction, id).await?;
    if current.grant.reference.as_ref() != Some(reference) {
        return Err(StoreError::Invalid("close grant binding"));
    }
    if let Some(receipt) = super::receipt(&mut transaction, context, operation).await? {
        let previous = CloseGrant::decode(
            verified_blob(&receipt, "request_blob", "request_sha256")?.as_slice(),
        )
        .map_err(|_| StoreError::Corrupt("close receipt request"))?;
        if previous != normalized {
            return Err(StoreError::IdempotencyConflict);
        }
        let original = state::decode_response(
            &verified_blob(&receipt, "response_blob", "response_sha256")?,
            &current.period.canonical_bytes,
        )?;
        let terminal = state::record_time(original.period.terminal_at.as_ref())?;
        if original.grant != current.grant
            || original.period != current.period.record
            || original.grant.state != terminal_state as i32
            || command.expected_grant_revision != 1
            || command.expected_period_revision != 2
            || receipt.try_get::<String, _>("period_id")? != *period_id
            || receipt.try_get::<i64, _>("committed_at_ms")? != terminal
            || terminal > now
        {
            return Err(StoreError::Corrupt("close receipt binding"));
        }
        transaction.commit().await?;
        return Ok(GrantResult {
            replayed: true,
            ..original
        });
    }
    if current.grant.revision != command.expected_grant_revision
        || current.period.record.revision != command.expected_period_revision
    {
        return Err(StoreError::RevisionConflict);
    }
    if current.grant.state != HoldoutGrantState::Issued as i32 {
        return Err(StoreError::InvalidTransition);
    }
    let expires = state::record_time(reference.expires_at.as_ref())?;
    if (disposition == GrantClosure::Expire && now < expires)
        || (disposition == GrantClosure::Revoke && now >= expires)
    {
        return Err(StoreError::Invalid("grant closing time"));
    }
    let mut grant = current.grant;
    grant.state = terminal_state as i32;
    grant.revision = 2;
    let mut period = current.period.record;
    period.state = HoldoutPeriodState::Closed as i32;
    period.revision = 3;
    period.terminal_at = Some(timestamp(now));
    state::update_grant(
        &mut transaction,
        &grant,
        command.expected_grant_revision,
        now,
    )
    .await?;
    state::update_period(&mut transaction, &period, command.expected_period_revision).await?;
    super::audit::append(
        &mut transaction,
        &store.ledger_id,
        now,
        super::event(
            context,
            principal,
            AuditAction::StateTransitioned,
            AuditTarget {
                kind: AuditTargetKind::HoldoutPeriodId,
                value: period_id.clone(),
            },
            state_transition_payload(
                "holdout.grant_issued",
                match disposition {
                    GrantClosure::Expire => "holdout.closed.expired",
                    GrantClosure::Revoke => "holdout.closed.revoked",
                },
                &command.reason,
            )?,
        ),
    )
    .await?;
    let result = GrantResult {
        grant,
        period,
        replayed: false,
    };
    super::save_result(
        &mut transaction,
        context,
        operation,
        &request_blob,
        &result,
        now,
    )
    .await?;
    #[cfg(test)]
    super::super::crash_tests::fault_point("close_before_commit").await;
    transaction.commit().await?;
    #[cfg(test)]
    super::super::crash_tests::fault_point("close_after_commit").await;
    Ok(result)
}
