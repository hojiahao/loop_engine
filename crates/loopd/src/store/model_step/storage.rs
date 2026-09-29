use loop_core::audit::{AuditAction, AuditTarget, AuditTargetKind, canonicalize_audit_payload};
use loop_protocol::job::validate_job_record;
use loop_protocol::wire::provider::v1::InvokeModelRequest;
use loop_protocol::wire::v1::{Actor, JobLease, JobRecord, JobState, LeaseId, ModelResponse};
use prost::Message;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction};

use super::super::{PgJobStore, StoreError, StoreResult, audit, lifecycle, postgres};
use super::{ModelStep, ModelStepCommand, ModelStepState, ReceiptRequest, validation};

pub(super) struct Mutation {
    pub request: ReceiptRequest,
    pub operation: &'static str,
    pub original: JobRecord,
    pub job: JobRecord,
    pub now: i64,
}

pub(super) async fn begin<'a>(
    store: &'a PgJobStore,
    actor: &Actor,
    command: &ModelStepCommand,
    operation: &str,
) -> StoreResult<(Transaction<'a, Postgres>, i64, JobRecord)> {
    let context = lifecycle::validate_context(command.context.as_ref(), actor)?;
    let job_id = &command
        .job_id
        .as_ref()
        .ok_or(StoreError::Invalid("model job ID"))?
        .value;
    super::super::validate_id(job_id)?;
    if command.expected_revision == 0
        || command.expected_revision >= i64::MAX as u64
        || command.ordinal > 1
    {
        return Err(StoreError::Invalid("model expected revision"));
    }
    let mut transaction = store.pool.begin().await?;
    let now = store.observe_clock(&mut transaction).await?;
    let requested = postgres::timestamp_millis(
        context
            .requested_at
            .as_ref()
            .ok_or(StoreError::Invalid("model command time"))?,
        true,
    )?;
    if requested > now || now - requested >= 30_000 {
        return Err(StoreError::Unavailable("model command deadline"));
    }
    let row = sqlx::query("SELECT * FROM jobs WHERE job_id=$1")
        .bind(job_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(StoreError::NotFound)?;
    let job = postgres::record_from_row(&row)?;
    store
        .admission
        .authorize_job_command(operation, actor, &job)?;
    if !matches!(
        job.specification
            .as_ref()
            .and_then(|spec| spec.input.as_ref()),
        Some(loop_protocol::wire::v1::job_specification::Input::Discovery(_))
    ) {
        return Err(StoreError::AdmissionDenied);
    }
    Ok((transaction, now, job))
}

pub(super) fn revision(job: &JobRecord, command: &ModelStepCommand) -> StoreResult<()> {
    if job.revision != command.expected_revision {
        return Err(StoreError::RevisionConflict);
    }
    Ok(())
}

pub(super) fn fence(
    job: &JobRecord,
    actor: &Actor,
    command: &ModelStepCommand,
    now: i64,
) -> StoreResult<()> {
    let lease = &command
        .lease_id
        .as_ref()
        .ok_or(StoreError::LeaseFenced)?
        .value;
    super::super::live_lease(job, actor, lease, now)?;
    Ok(())
}

pub(super) fn deadline(job: &JobRecord) -> StoreResult<i64> {
    postgres::deadline_millis(
        job.specification
            .as_ref()
            .ok_or(StoreError::Corrupt("model specification"))?,
    )
}

pub(super) fn advance(job: &mut JobRecord, now: i64) -> StoreResult<()> {
    job.revision = job
        .revision
        .checked_add(1)
        .filter(|value| *value < i64::MAX as u64)
        .ok_or(StoreError::Invalid("model revision overflow"))?;
    job.updated_at = Some(postgres::timestamp(now));
    validate_job_record(job)?;
    Ok(())
}

pub(super) fn lease(job: &mut JobRecord, actor: &Actor, now: i64, expiry: i64) -> StoreResult<()> {
    job.attempt = job
        .attempt
        .checked_add(1)
        .ok_or(StoreError::Invalid("model attempt overflow"))?;
    job.state = JobState::Running as i32;
    job.active_lease = Some(JobLease {
        lease_id: Some(LeaseId {
            value: format!("lease.{}", uuid::Uuid::new_v4().simple()),
        }),
        job_id: job
            .specification
            .as_ref()
            .and_then(|spec| spec.job_id.clone()),
        owner: Some(actor.clone()),
        acquired_revision: job.revision + 1,
        issued_at: Some(postgres::timestamp(now)),
        heartbeat_at: Some(postgres::timestamp(now)),
        expires_at: Some(postgres::timestamp(expiry)),
    });
    Ok(())
}

pub(super) async fn writer(transaction: &mut Transaction<'_, Postgres>) -> StoreResult<()> {
    sqlx::query("SELECT set_config('loop.model_step_writer', 'v2', true)")
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

fn identity(job: &JobRecord) -> StoreResult<&str> {
    job.specification
        .as_ref()
        .and_then(|spec| spec.job_id.as_ref())
        .map(|id| id.value.as_str())
        .ok_or(StoreError::Corrupt("model job ID"))
}

fn normalize(value: &ReceiptRequest) -> ReceiptRequest {
    let mut value = value.clone();
    if let Some(context) = value
        .command
        .as_mut()
        .and_then(|command| command.context.as_mut())
    {
        context.request_id = None;
        context.requested_at = None;
    }
    value
}

pub(super) async fn replay(
    transaction: &mut Transaction<'_, Postgres>,
    request: &ReceiptRequest,
    operation: &str,
    job: &JobRecord,
) -> StoreResult<bool> {
    let command = request
        .command
        .as_ref()
        .ok_or(StoreError::Invalid("model command"))?;
    let context = command
        .context
        .as_ref()
        .ok_or(StoreError::Invalid("model context"))?;
    let actor = context
        .actor
        .as_ref()
        .and_then(|actor| actor.actor_id.as_ref())
        .ok_or(StoreError::Invalid("model actor ID"))?;
    let key = context
        .idempotency_key
        .as_ref()
        .ok_or(StoreError::Invalid("model key"))?;
    let row = sqlx::query(
        "SELECT * FROM command_receipts WHERE actor_id=$1 AND operation=$2 AND idempotency_key=$3",
    )
    .bind(&actor.value)
    .bind(operation)
    .bind(&key.value)
    .fetch_optional(&mut **transaction)
    .await?;
    let Some(row) = row else {
        return Ok(false);
    };
    let old = ReceiptRequest::decode(
        postgres::verified_blob(&row, "request_blob", "request_sha256")?.as_slice(),
    )
    .map_err(|_| StoreError::Corrupt("model request receipt"))?;
    if old != normalize(request) {
        return Err(StoreError::IdempotencyConflict);
    }
    let record = postgres::decode_record(&postgres::verified_blob(
        &row,
        "response_blob",
        "response_sha256",
    )?)?;
    if record.specification != job.specification
        || record.revision != command.expected_revision + 1
        || record.revision > job.revision
        || row.try_get::<String, _>("job_id")? != identity(job)?
    {
        return Err(StoreError::Corrupt("model receipt binding"));
    }
    Ok(true)
}

pub(super) async fn insert(
    transaction: &mut Transaction<'_, Postgres>,
    step: &ModelStep,
    blob: &[u8],
    now: i64,
) -> StoreResult<()> {
    let ModelStep {
        job,
        request,
        request_sha256: digest,
        reserved_input: input,
        reserved_output: output,
        reserved_nano_usd: cost,
        ..
    } = step;
    let context = request
        .context
        .as_ref()
        .ok_or(StoreError::Invalid("model context"))?;
    let actor = context
        .actor
        .as_ref()
        .and_then(|actor| actor.actor_id.as_ref())
        .ok_or(StoreError::Invalid("model actor"))?;
    let request_id = context
        .request_id
        .as_ref()
        .ok_or(StoreError::Invalid("model request ID"))?;
    let key = context
        .idempotency_key
        .as_ref()
        .ok_or(StoreError::Invalid("model key"))?;
    let result = sqlx::query("INSERT INTO model_steps (job_id,actor_id,request_id,idempotency_key,invocation_sha256,request_blob,request_sha256,reserved_input,reserved_output,reserved_nano_usd,state,created_revision,updated_revision,created_at_ms,updated_at_ms,ordinal) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'reserved',$11,$11,$12,$12,$13) ON CONFLICT DO NOTHING")
        .bind(identity(job)?).bind(&actor.value).bind(&request_id.value).bind(&key.value)
        .bind(digest.as_slice()).bind(blob).bind(Sha256::digest(blob).as_slice())
        .bind(*input as i64).bind(*output as i64).bind(*cost as i64).bind(job.revision as i64).bind(now).bind(step.ordinal as i32)
        .execute(&mut **transaction).await?;
    if result.rows_affected() != 1 {
        return Err(StoreError::IdempotencyConflict);
    }
    Ok(())
}

pub(super) async fn load(
    transaction: &mut Transaction<'_, Postgres>,
    job: &JobRecord,
    ordinal: u32,
) -> StoreResult<Option<ModelStep>> {
    let row = sqlx::query("SELECT * FROM model_steps WHERE job_id=$1 AND ordinal=$2")
        .bind(identity(job)?)
        .bind(ordinal as i32)
        .fetch_optional(&mut **transaction)
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let request = InvokeModelRequest::decode(
        postgres::verified_blob(&row, "request_blob", "request_sha256")?.as_slice(),
    )
    .map_err(|_| StoreError::Corrupt("model request bytes"))?;
    let digest = crate::runtime::model_codec::request_digest(&request)
        .map_err(|_| StoreError::Corrupt("model canonical request"))?;
    let context = request
        .context
        .as_ref()
        .ok_or(StoreError::Corrupt("model context"))?;
    let actor = context
        .actor
        .as_ref()
        .ok_or(StoreError::Corrupt("model actor"))?;
    let (input, output, cost, _) = validation::invocation(job, actor, &request, ordinal)
        .map_err(|_| StoreError::Corrupt("model frozen budget"))?;
    let state = ModelStepState::parse(&row.try_get::<String, _>("state")?)?;
    let created: i64 = row.try_get("created_revision")?;
    let updated: i64 = row.try_get("updated_revision")?;
    if row.try_get::<Vec<u8>, _>("invocation_sha256")? != digest
        || row.try_get::<i64, _>("reserved_input")? != input as i64
        || row.try_get::<i64, _>("reserved_output")? != output as i64
        || row.try_get::<i64, _>("reserved_nano_usd")? != cost as i64
        || row.try_get::<String, _>("actor_id")?
            != actor
                .actor_id
                .as_ref()
                .ok_or(StoreError::Corrupt("model actor ID"))?
                .value
        || row.try_get::<String, _>("request_id")?
            != context
                .request_id
                .as_ref()
                .ok_or(StoreError::Corrupt("model request ID"))?
                .value
        || row.try_get::<String, _>("idempotency_key")?
            != context
                .idempotency_key
                .as_ref()
                .ok_or(StoreError::Corrupt("model key"))?
                .value
        || created < 2
        || updated < created
        || updated as u64 > job.revision
        || row.try_get::<i64, _>("created_at_ms")? > row.try_get::<i64, _>("updated_at_ms")?
        || row.try_get::<i64, _>("updated_at_ms")?
            > postgres::timestamp_millis(
                job.updated_at
                    .as_ref()
                    .ok_or(StoreError::Corrupt("model updated time"))?,
                false,
            )?
        || (state == ModelStepState::Completed
            && !matches!(
                JobState::try_from(job.state),
                Ok(JobState::Succeeded | JobState::InfrastructureFailed | JobState::Running)
            ))
        || (state != ModelStepState::Completed && job.state != JobState::Running as i32)
    {
        return Err(StoreError::Corrupt("model projection binding"));
    }
    let response = if state == ModelStepState::Completed {
        Some(
            ModelResponse::decode(
                postgres::verified_blob(&row, "response_blob", "response_sha256")?.as_slice(),
            )
            .map_err(|_| StoreError::Corrupt("model response bytes"))?,
        )
    } else {
        None
    };
    let mut step = ModelStep {
        ordinal,
        job: job.clone(),
        request,
        request_sha256: digest,
        state,
        response,
        reserved_input: input,
        reserved_output: output,
        reserved_nano_usd: cost,
        tool_result: None,
    };
    if let Some(response) = &step.response {
        validation::response(&step, response)
            .map_err(|_| StoreError::Corrupt("model response binding"))?;
        if job.state == JobState::Running as i32 {
            validation::call(&step, response)
                .map_err(|_| StoreError::Corrupt("intermediate model response"))?;
        }
    }
    step.tool_result = load_tool(transaction, &step).await?;
    Ok(Some(step))
}

pub(super) async fn history(
    transaction: &mut Transaction<'_, Postgres>,
    job: &JobRecord,
) -> StoreResult<Vec<ModelStep>> {
    let rows =
        sqlx::query("SELECT ordinal FROM model_steps WHERE job_id=$1 ORDER BY ordinal LIMIT 3")
            .bind(identity(job)?)
            .fetch_all(&mut **transaction)
            .await?;
    if rows.len() > 2 {
        return Err(StoreError::Corrupt("model history bounds"));
    }
    let mut history = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let ordinal: i32 = row.try_get("ordinal")?;
        if ordinal != index as i32 {
            return Err(StoreError::Corrupt("model history sequence"));
        }
        history.push(
            load(transaction, job, ordinal as u32)
                .await?
                .ok_or(StoreError::Corrupt("model history missing"))?,
        );
    }
    if history.len() == 2 {
        validation::continuation(&history[0], &history[1].request)
            .map_err(|_| StoreError::Corrupt("model history context"))?;
    }
    if job.state == JobState::Succeeded as i32
        && history.last().is_none_or(|step| {
            step.request
                .invocation
                .as_ref()
                .is_none_or(|invocation| invocation.structured_output.is_none())
        })
    {
        return Err(StoreError::Corrupt("model terminal history"));
    }
    validation::totals(job, &history).map_err(|_| StoreError::Corrupt("model history budget"))?;
    Ok(history)
}

pub(super) async fn current(
    transaction: &mut Transaction<'_, Postgres>,
    job: &JobRecord,
    ordinal: u32,
) -> StoreResult<ModelStep> {
    let step = history(transaction, job)
        .await?
        .pop()
        .ok_or(StoreError::NotFound)?;
    if step.ordinal != ordinal {
        return Err(StoreError::Invalid("model current ordinal"));
    }
    Ok(step)
}

async fn load_tool(
    transaction: &mut Transaction<'_, Postgres>,
    step: &ModelStep,
) -> StoreResult<Option<loop_protocol::wire::v1::ToolResultContent>> {
    let row = sqlx::query("SELECT tool_results.*,model_steps.updated_revision AS step_revision,model_steps.updated_at_ms AS step_time FROM tool_results JOIN model_steps USING(job_id,ordinal) WHERE job_id=$1 AND ordinal=$2")
        .bind(identity(&step.job)?).bind(step.ordinal as i32)
        .fetch_optional(&mut **transaction).await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let result = loop_protocol::wire::v1::ToolResultContent::decode(
        postgres::verified_blob(&row, "result_blob", "result_sha256")?.as_slice(),
    )
    .map_err(|_| StoreError::Corrupt("tool result bytes"))?;
    let revision: i64 = row.try_get("created_revision")?;
    let created: i64 = row.try_get("created_at_ms")?;
    if row.try_get::<String, _>("tool_call_id")? != result.tool_call_id
        || revision <= row.try_get::<i64, _>("step_revision")?
        || revision as u64 > step.job.revision
        || created < row.try_get::<i64, _>("step_time")?
        || created
            > postgres::timestamp_millis(
                step.job
                    .updated_at
                    .as_ref()
                    .ok_or(StoreError::Corrupt("tool updated time"))?,
                false,
            )?
        || step.state != super::ModelStepState::Completed
    {
        return Err(StoreError::Corrupt("tool result projection"));
    }
    validation::tool(step, &result).map_err(|_| StoreError::Corrupt("tool result binding"))?;
    Ok(Some(result))
}

pub(super) async fn insert_tool(
    transaction: &mut Transaction<'_, Postgres>,
    step: &ModelStep,
    result: &loop_protocol::wire::v1::ToolResultContent,
    now: i64,
) -> StoreResult<()> {
    let bytes = postgres::encode_message(result)?;
    let inserted = sqlx::query("INSERT INTO tool_results (job_id,ordinal,tool_call_id,result_blob,result_sha256,created_revision,created_at_ms) VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT DO NOTHING")
        .bind(identity(&step.job)?).bind(step.ordinal as i32).bind(&result.tool_call_id)
        .bind(&bytes).bind(Sha256::digest(&bytes).as_slice())
        .bind(step.job.revision as i64).bind(now).execute(&mut **transaction).await?;
    if inserted.rows_affected() != 1 {
        return Err(StoreError::IdempotencyConflict);
    }
    Ok(())
}

pub(super) async fn finish_read(
    mut transaction: Transaction<'_, Postgres>,
    job: JobRecord,
    ordinal: u32,
) -> StoreResult<ModelStep> {
    let step = history(&mut transaction, &job)
        .await?
        .into_iter()
        .find(|step| step.ordinal == ordinal)
        .ok_or(StoreError::Corrupt("model receipt without step"))?;
    transaction.commit().await?;
    Ok(step)
}

pub(super) async fn transition(
    transaction: &mut Transaction<'_, Postgres>,
    job: &JobRecord,
    state: &str,
    response: Option<&ModelResponse>,
    now: i64,
    ordinal: u32,
) -> StoreResult<()> {
    let bytes = response.map(postgres::encode_message).transpose()?;
    let checksum = bytes.as_ref().map(|bytes| Sha256::digest(bytes).to_vec());
    let result = sqlx::query("UPDATE model_steps SET state=$1,updated_revision=$2,updated_at_ms=$3,response_blob=$4,response_sha256=$5 WHERE job_id=$6 AND ordinal=$7")
        .bind(state).bind(job.revision as i64).bind(now).bind(bytes).bind(checksum).bind(identity(job)?).bind(ordinal as i32)
        .execute(&mut **transaction).await?;
    if result.rows_affected() != 1 {
        return Err(StoreError::Corrupt("model step disappeared"));
    }
    Ok(())
}

pub(super) async fn commit(
    store: &PgJobStore,
    mut transaction: Transaction<'_, Postgres>,
    actor: &Actor,
    mutation: Mutation,
) -> StoreResult<()> {
    let Mutation {
        request,
        operation,
        original,
        job,
        now,
    } = mutation;
    let command = request
        .command
        .as_ref()
        .ok_or(StoreError::Invalid("model command"))?;
    let context = command
        .context
        .as_ref()
        .ok_or(StoreError::Invalid("model context"))?;
    lifecycle::write_record(&mut transaction, &job, original.revision).await?;
    audit::append(&mut transaction,&store.ledger_id,now,audit::EventInput {
        actor,
        correlation_id:&context.correlation_id.as_ref().ok_or(StoreError::Invalid("model correlation"))?.value,
        causation_id:&context.causation_id.as_ref().ok_or(StoreError::Invalid("model causation"))?.value,
        action:AuditAction::CommandAccepted,
        target:AuditTarget {kind:AuditTargetKind::JobId,value:identity(&job)?.to_owned()},
        payload:canonicalize_audit_payload("loop.audit.command_accepted",1,
            &serde_json::to_vec(&serde_json::json!({"command":operation,"request_id":context.request_id.as_ref().ok_or(StoreError::Invalid("model request ID"))?.value,"summary":"durable model step transition"}))
                .map_err(|_|StoreError::Invalid("model audit"))?)?,
    }).await?;
    lifecycle::save_receipt(
        &mut transaction,
        context,
        operation,
        identity(&job)?,
        &postgres::encode_message(&normalize(&request))?,
        &job,
        now,
    )
    .await?;
    let last = store.clock.now_millis()?;
    if last < now {
        return Err(StoreError::ClockRegression);
    }
    let requested = postgres::timestamp_millis(
        context
            .requested_at
            .as_ref()
            .ok_or(StoreError::Invalid("model command time"))?,
        true,
    )?;
    if last - requested >= 30_000 {
        return Err(StoreError::Unavailable("model command deadline"));
    }
    if matches!(operation, "loop.model.reserve" | "loop.model.takeover") {
        if operation == "loop.model.reserve" && command.ordinal == 1 {
            fence(&original, actor, command, last)?;
        }
        let lease = job
            .active_lease
            .as_ref()
            .and_then(|lease| lease.lease_id.as_ref())
            .ok_or(StoreError::LeaseFenced)?;
        super::super::live_lease(&job, actor, &lease.value, last)?;
    } else {
        fence(&original, actor, command, last)?;
        if operation == "loop.model.call" {
            let lease = job
                .active_lease
                .as_ref()
                .and_then(|lease| lease.lease_id.as_ref())
                .ok_or(StoreError::LeaseFenced)?;
            super::super::live_lease(&job, actor, &lease.value, last)?;
        }
    }
    store
        .admission
        .authorize_job_command(operation, actor, &job)?;
    sqlx::query("UPDATE store_metadata SET last_observed_at_ms=$1 WHERE singleton=1")
        .bind(last)
        .execute(&mut *transaction)
        .await?;
    #[cfg(test)]
    super::super::crash_tests::fault_point(&format!(
        "model_{}_before",
        operation.rsplit('.').next().unwrap_or("invalid")
    ))
    .await;
    transaction.commit().await?;
    #[cfg(test)]
    super::super::crash_tests::fault_point(&format!(
        "model_{}_after",
        operation.rsplit('.').next().unwrap_or("invalid")
    ))
    .await;
    Ok(())
}
