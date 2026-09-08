use loop_core::audit::{
    ActorKind, AuditAction, AuditActor, AuditEvent, AuditPayload, AuditTarget, AuditTargetKind,
    Sha256Digest, audit_event_sha256, canonicalize_audit_payload, verify_audit_event,
};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, Sqlite, SqlitePool, Transaction};

use super::{StoreError, StoreResult, SubmitJob, sqlite::audit_timestamp};

pub(super) async fn append_submission(
    transaction: &mut Transaction<'_, Sqlite>,
    ledger_id: &str,
    command: &SubmitJob,
    now: i64,
) -> StoreResult<()> {
    let specification = &command.specification;
    append(transaction, ledger_id, now, EventInput {
        actor: specification.submitted_by.as_ref().expect("validated actor"),
        correlation_id: &specification.correlation_id.as_ref().expect("validated correlation").value,
        causation_id: &specification.causation_id.as_ref().expect("validated causation").value,
        action: AuditAction::CommandAccepted,
        target: AuditTarget {
            kind: AuditTargetKind::JobId,
            value: specification.job_id.as_ref().expect("validated job id").value.clone(),
        },
        payload: canonicalize_audit_payload("loop.audit.command_accepted", 1, &serde_json::to_vec(&serde_json::json!({
            "command": "loop.jobs.submit", "request_id": command.request_id, "summary": "job queued"
        })).map_err(|_| StoreError::Invalid("audit payload"))?)?,
    }).await
}

pub(super) struct EventInput<'a> {
    pub actor: &'a loop_protocol::wire::v1::Actor,
    pub correlation_id: &'a str,
    pub causation_id: &'a str,
    pub action: AuditAction,
    pub target: AuditTarget,
    pub payload: AuditPayload,
}

pub(super) async fn append(
    transaction: &mut Transaction<'_, Sqlite>,
    ledger_id: &str,
    now: i64,
    input: EventInput<'_>,
) -> StoreResult<()> {
    let head = sqlx::query("SELECT * FROM audit_events ORDER BY sequence DESC LIMIT 1")
        .fetch_optional(&mut **transaction)
        .await?;
    let (sequence, previous_event_sha256) = if let Some(row) = head {
        let head = event_from_row(&row, ledger_id)?;
        (
            head.sequence
                .checked_add(1)
                .ok_or(StoreError::Corrupt("audit sequence overflow"))?,
            head.event_sha256,
        )
    } else {
        (1, Sha256Digest::ZERO)
    };
    let actor = input.actor;
    let actor_kind = match loop_protocol::wire::v1::ActorKind::try_from(actor.kind) {
        Ok(loop_protocol::wire::v1::ActorKind::Human) => ActorKind::Human,
        Ok(loop_protocol::wire::v1::ActorKind::Service) => ActorKind::Service,
        Ok(loop_protocol::wire::v1::ActorKind::Agent) => ActorKind::Agent,
        Ok(loop_protocol::wire::v1::ActorKind::Scheduler) => ActorKind::Scheduler,
        _ => return Err(StoreError::Invalid("audit actor kind")),
    };
    let mut event = AuditEvent {
        audit_ledger_id: ledger_id.to_owned(),
        sequence,
        previous_event_sha256,
        audit_event_id: format!("audit.{}", uuid::Uuid::new_v4().simple()),
        occurred_at: audit_timestamp(now)?,
        correlation_id: input.correlation_id.to_owned(),
        causation_id: input.causation_id.to_owned(),
        actor: AuditActor {
            actor_id: actor
                .actor_id
                .as_ref()
                .expect("validated actor id")
                .value
                .clone(),
            kind: actor_kind,
            display_name: actor.display_name.clone(),
            authenticated_subject: actor.authenticated_subject.clone(),
        },
        action: input.action,
        target: input.target,
        payload: input.payload,
        event_sha256: Sha256Digest::ZERO,
    };
    event.event_sha256 = audit_event_sha256(&event)?;
    verify_audit_event(&event)?;
    let sequence =
        i64::try_from(event.sequence).map_err(|_| StoreError::Corrupt("audit sequence range"))?;
    sqlx::query(
        "INSERT INTO audit_events (
            sequence, event_id, previous_sha256, occurred_at_ms, correlation_id,
            causation_id, actor_id, actor_kind, actor_display_name, actor_subject,
            action, job_id, target_kind, target_id, payload_schema, payload_blob,
            payload_sha256, event_sha256
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(sequence)
    .bind(&event.audit_event_id)
    .bind(event.previous_event_sha256.as_bytes().as_slice())
    .bind(now)
    .bind(&event.correlation_id)
    .bind(&event.causation_id)
    .bind(&event.actor.actor_id)
    .bind(event.actor.kind.as_str())
    .bind(&event.actor.display_name)
    .bind(&event.actor.authenticated_subject)
    .bind(event.action.as_str())
    .bind((event.target.kind == AuditTargetKind::JobId).then_some(&event.target.value))
    .bind(event.target.kind.as_str())
    .bind(&event.target.value)
    .bind(&event.payload.schema_name)
    .bind(&event.payload.canonical_bytes)
    .bind(event.payload.payload_sha256.as_bytes().as_slice())
    .bind(event.event_sha256.as_bytes().as_slice())
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

pub(super) async fn read_page(
    pool: &SqlitePool,
    ledger_id: &str,
    after: u64,
    limit: u32,
) -> StoreResult<Vec<AuditEvent>> {
    if limit == 0 || limit > 500 {
        return Err(StoreError::Invalid("audit page size"));
    }
    let after = i64::try_from(after).map_err(|_| StoreError::Invalid("audit cursor"))?;
    let mut transaction = pool.begin().await?;
    let mut previous = if after == 0 {
        Sha256Digest::ZERO
    } else {
        let row = sqlx::query("SELECT * FROM audit_events WHERE sequence = ?")
            .bind(after)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or(StoreError::Invalid("audit cursor absent"))?;
        event_from_row(&row, ledger_id)?.event_sha256
    };
    let rows =
        sqlx::query("SELECT * FROM audit_events WHERE sequence > ? ORDER BY sequence LIMIT ?")
            .bind(after)
            .bind(i64::from(limit))
            .fetch_all(&mut *transaction)
            .await?;
    let mut events = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let event = event_from_row(row, ledger_id)?;
        if event.previous_event_sha256 != previous
            || event.sequence != after as u64 + index as u64 + 1
        {
            return Err(StoreError::Corrupt("audit chain discontinuity"));
        }
        previous = event.event_sha256;
        events.push(event);
    }
    transaction.commit().await?;
    Ok(events)
}

fn event_from_row(row: &SqliteRow, ledger_id: &str) -> StoreResult<AuditEvent> {
    let event = AuditEvent {
        audit_ledger_id: ledger_id.to_owned(),
        sequence: u64::try_from(row.try_get::<i64, _>("sequence")?)
            .map_err(|_| StoreError::Corrupt("audit sequence"))?,
        previous_event_sha256: row_digest(row, "previous_sha256")?,
        audit_event_id: row.try_get("event_id")?,
        occurred_at: audit_timestamp(row.try_get("occurred_at_ms")?)?,
        correlation_id: row.try_get("correlation_id")?,
        causation_id: row.try_get("causation_id")?,
        actor: AuditActor {
            actor_id: row.try_get("actor_id")?,
            kind: ActorKind::try_from(row.try_get::<&str, _>("actor_kind")?)?,
            display_name: row.try_get("actor_display_name")?,
            authenticated_subject: row.try_get("actor_subject")?,
        },
        action: AuditAction::try_from(row.try_get::<&str, _>("action")?)?,
        target: AuditTarget {
            kind: AuditTargetKind::try_from(row.try_get::<&str, _>("target_kind")?)?,
            value: row.try_get("target_id")?,
        },
        payload: AuditPayload {
            schema_name: row.try_get("payload_schema")?,
            schema_version: 1,
            canonical_bytes: row.try_get("payload_blob")?,
            payload_sha256: row_digest(row, "payload_sha256")?,
        },
        event_sha256: row_digest(row, "event_sha256")?,
    };
    verify_audit_event(&event)?;
    Ok(event)
}

fn row_digest(row: &SqliteRow, name: &str) -> StoreResult<Sha256Digest> {
    let bytes: Vec<u8> = row.try_get(name)?;
    Ok(Sha256Digest::from_bytes(
        bytes
            .try_into()
            .map_err(|_| StoreError::Corrupt("audit digest length"))?,
    ))
}
