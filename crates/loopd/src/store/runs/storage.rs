use loop_core::audit::{AuditAction, AuditTarget, AuditTargetKind, canonicalize_audit_payload};
use loop_protocol::wire::discovery::v1::{DiscoveryJobHandle, DiscoveryJobStatus};
use loop_protocol::wire::runs::v1::{RunSpecification, RunStatus, RunView};
use loop_protocol::wire::v1::{CommandContext, ExactDecimal, JobRecord, Money, job_specification};
use prost::Message;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction};

use super::super::postgres::{encode_message, record_from_row, timestamp_millis, verified_blob};
use super::super::{StoreError, StoreResult, audit, model_duration, model_money};
use super::RunSnapshot;

pub(super) fn wall_millis(value: Option<&prost_types::Duration>) -> StoreResult<i64> {
    let value = value.ok_or(StoreError::Invalid("run wall time"))?;
    if value.seconds < 0
        || !(0..1_000_000_000).contains(&value.nanos)
        || value.nanos % 1_000_000 != 0
    {
        return Err(StoreError::Invalid("run wall time"));
    }
    value
        .seconds
        .checked_mul(1000)
        .and_then(|seconds| seconds.checked_add((i64::from(value.nanos) + 999_999) / 1_000_000))
        .filter(|value| *value > 0 && *value <= 30 * 86_400_000)
        .ok_or(StoreError::Invalid("run wall time range"))
}

pub(super) fn money(nanos: u64) -> Money {
    let whole = nanos / 1_000_000_000;
    let fraction = nanos % 1_000_000_000;
    let value = if fraction == 0 {
        whole.to_string()
    } else {
        format!("{whole}.{fraction:09}")
            .trim_end_matches('0')
            .to_owned()
    };
    Money {
        amount: Some(ExactDecimal { value }),
        currency_code: "USD".into(),
    }
}

pub(super) fn reserve(
    view: &mut RunView,
    specification: &RunSpecification,
    now: i64,
) -> StoreResult<bool> {
    let child = specification
        .discovery
        .as_ref()
        .and_then(|input| input.budget.as_ref())
        .ok_or(StoreError::Corrupt("run child budget"))?;
    let parent = specification
        .budget
        .as_ref()
        .ok_or(StoreError::Corrupt("run budget"))?;
    let steps = view
        .reserved_steps
        .checked_add(u64::from(child.maximum_steps));
    let input = view
        .reserved_input_tokens
        .checked_add(child.maximum_input_tokens);
    let output = view
        .reserved_output_tokens
        .checked_add(child.maximum_output_tokens);
    let cost = model_money(view.reserved_cost.as_ref())?
        .checked_add(model_money(child.maximum_cost.as_ref())?);
    let cost_limit = model_money(parent.maximum_cost.as_ref())?;
    let deadline = timestamp_millis(
        view.deadline
            .as_ref()
            .ok_or(StoreError::Corrupt("run deadline"))?,
        true,
    )?;
    // Keep the existing Harness's conservative sub-millisecond rounding; only
    // the new parent wall duration requires exact millisecond precision.
    let child_deadline = now.checked_add(model_duration(child.maximum_wall_time.as_ref())?);
    if steps.is_none_or(|value| value > parent.maximum_steps || value > i64::MAX as u64)
        || input.is_none_or(|value| value > parent.maximum_input_tokens || value > i64::MAX as u64)
        || output
            .is_none_or(|value| value > parent.maximum_output_tokens || value > i64::MAX as u64)
        || cost.is_none_or(|value| value > cost_limit)
        || child_deadline.is_none_or(|value| value > deadline)
    {
        return Ok(false);
    }
    view.reserved_steps = steps.expect("checked steps");
    view.reserved_input_tokens = input.expect("checked input");
    view.reserved_output_tokens = output.expect("checked output");
    view.reserved_cost = Some(money(cost.expect("checked cost")));
    Ok(true)
}

pub(super) fn child_handle(record: &JobRecord) -> StoreResult<DiscoveryJobHandle> {
    let job = record
        .specification
        .as_ref()
        .ok_or(StoreError::Corrupt("run child specification"))?;
    DiscoveryJobStatus::try_from(record.state)
        .map_err(|_| StoreError::Corrupt("run child status"))?;
    Ok(DiscoveryJobHandle {
        job_id: job.job_id.clone(),
        status: record.state,
        revision: record.revision,
        submitted_at: job.submitted_at,
        updated_at: record.updated_at,
    })
}

pub(super) async fn load(
    transaction: &mut Transaction<'_, Postgres>,
    run_id: &str,
) -> StoreResult<RunSnapshot> {
    let row = sqlx::query("SELECT * FROM research_runs WHERE run_id = $1")
        .bind(run_id)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(StoreError::NotFound)?;
    let spec_bytes = verified_blob(&row, "specification_blob", "specification_sha256")?;
    let view_bytes = verified_blob(&row, "view_blob", "view_sha256")?;
    let specification = RunSpecification::decode(spec_bytes.as_slice())
        .map_err(|_| StoreError::Corrupt("run specification"))?;
    let view = RunView::decode(view_bytes.as_slice())
        .map_err(|_| StoreError::Corrupt("run projection"))?;
    let submitted = view
        .submitted_at
        .as_ref()
        .ok_or(StoreError::Corrupt("run submitted time"))?;
    loop_protocol::runs::validate_specification(&specification, submitted)
        .map_err(|_| StoreError::Corrupt("run specification contract"))?;
    loop_protocol::runs::validate_view(&view)
        .map_err(|_| StoreError::Corrupt("run projection contract"))?;
    if specification.encode_to_vec() != spec_bytes
        || view.encode_to_vec() != view_bytes
        || view.plan_verified
        || specification.run_id != view.run_id
        || specification.budget != view.budget
        || specification.maximum_rounds != view.maximum_rounds
        || specification.run_id.as_ref().expect("validated run").value != run_id
        || row.try_get::<String, _>("owner_id")?
            != specification
                .owner
                .as_ref()
                .expect("validated owner")
                .actor_id
                .as_ref()
                .expect("validated owner ID")
                .value
        || row.try_get::<i32, _>("status")? != view.status
        || row.try_get::<i64, _>("revision")? != view.revision as i64
        || row.try_get::<i32, _>("completed_rounds")? != view.completed_rounds as i32
        || row.try_get::<i64, _>("reserved_steps")? != view.reserved_steps as i64
        || row.try_get::<i64, _>("reserved_input")? != view.reserved_input_tokens as i64
        || row.try_get::<i64, _>("reserved_output")? != view.reserved_output_tokens as i64
        || row.try_get::<i64, _>("reserved_nano_usd")?
            != model_money(view.reserved_cost.as_ref())? as i64
        || row.try_get::<i64, _>("submitted_at_ms")? != timestamp_millis(submitted, true)?
        || row.try_get::<i64, _>("updated_at_ms")?
            != timestamp_millis(view.updated_at.as_ref().expect("validated update"), true)?
        || row.try_get::<i64, _>("deadline_ms")?
            != timestamp_millis(view.deadline.as_ref().expect("validated deadline"), true)?
        || row.try_get::<String, _>("current_job_id")?
            != view
                .current_job
                .as_ref()
                .expect("validated child")
                .job_id
                .as_ref()
                .expect("validated child ID")
                .value
    {
        return Err(StoreError::Corrupt("run projection binding"));
    }
    let deadline = timestamp_millis(submitted, true)?
        .checked_add(wall_millis(
            specification
                .budget
                .as_ref()
                .expect("validated budget")
                .maximum_wall_time
                .as_ref(),
        )?)
        .ok_or(StoreError::Corrupt("run deadline overflow"))?;
    if deadline != row.try_get::<i64, _>("deadline_ms")? {
        return Err(StoreError::Corrupt("run deadline binding"));
    }
    let jobs = sqlx::query(
        "SELECT * FROM jobs WHERE run_id = $1 ORDER BY submitted_at_ms, job_id LIMIT 65",
    )
    .bind(run_id)
    .fetch_all(&mut **transaction)
    .await?;
    let rounds = jobs.len() as u64;
    let child = specification.discovery.as_ref().expect("validated child");
    let budget = child.budget.as_ref().expect("validated child budget");
    if rounds == 0
        || rounds > u64::from(specification.maximum_rounds)
        || view.reserved_steps != rounds * u64::from(budget.maximum_steps)
        || view.reserved_input_tokens
            != rounds
                .checked_mul(budget.maximum_input_tokens)
                .ok_or(StoreError::Corrupt("run input overflow"))?
        || view.reserved_output_tokens
            != rounds
                .checked_mul(budget.maximum_output_tokens)
                .ok_or(StoreError::Corrupt("run output overflow"))?
        || model_money(view.reserved_cost.as_ref())?
            != rounds
                .checked_mul(model_money(budget.maximum_cost.as_ref())?)
                .ok_or(StoreError::Corrupt("run cost overflow"))?
        || u64::from(view.completed_rounds) > rounds
        || (view.status == RunStatus::Active as i32
            && u64::from(view.completed_rounds) + 1 != rounds)
        || (view.status == RunStatus::Completed as i32
            && u64::from(view.completed_rounds) != rounds)
        || view.revision != rounds + u64::from(view.status != RunStatus::Active as i32)
    {
        return Err(StoreError::Corrupt("run reservation history"));
    }
    let current_id = row.try_get::<String, _>("current_job_id")?;
    let first_id = row.try_get::<String, _>("first_job_id")?;
    let mut first_found = false;
    let mut current = None;
    let mut successes = 0;
    let prefix = format!("run.{:x}.", Sha256::digest(run_id.as_bytes()));
    let mut ordinals = std::collections::BTreeSet::new();
    for job_row in jobs {
        let record = record_from_row(&job_row)?;
        if record.state == loop_protocol::wire::v1::JobState::Succeeded as i32 {
            successes += 1;
        }
        let job = record
            .specification
            .as_ref()
            .expect("validated child specification");
        let ordinal = job
            .idempotency_key
            .as_ref()
            .and_then(|key| key.value.strip_prefix(&prefix))
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or(StoreError::Corrupt("run child ordinal"))?;
        if ordinal == 0
            || ordinal > rounds
            || !ordinals.insert(ordinal)
            || job
                .idempotency_key
                .as_ref()
                .expect("validated child key")
                .value
                != format!("{prefix}{ordinal}")
            || timestamp_millis(
                job.submitted_at.as_ref().expect("validated child time"),
                true,
            )? < timestamp_millis(submitted, true)?
            || super::super::postgres::deadline_millis(job)? > deadline
        {
            return Err(StoreError::Corrupt("run child chronology"));
        }
        let Some(job_specification::Input::Discovery(input)) = &job.input else {
            return Err(StoreError::Corrupt("run child input"));
        };
        if job.submitted_by != specification.executor
            || job.protocol_selection != specification.protocol_selection
            || input.dataset != child.dataset
            || input.research_policy != child.research_policy
            || input.maker_model != child.maker_model
            || input.checker_model != child.checker_model
            || input.maximum_candidates != child.maximum_candidates
            || input
                .budget
                .as_ref()
                .is_none_or(|value| value.encode_to_vec() != budget.encode_to_vec())
        {
            return Err(StoreError::Corrupt("run frozen child binding"));
        }
        let id = job
            .job_id
            .as_ref()
            .expect("validated child ID")
            .value
            .as_str();
        if id == first_id {
            if ordinal != 1 || job.submitted_at != view.submitted_at {
                return Err(StoreError::Corrupt("run first child"));
            }
            first_found = true;
        }
        if id == current_id {
            if ordinal != rounds {
                return Err(StoreError::Corrupt("run latest child"));
            }
            current = Some(record);
        } else if record.state != loop_protocol::wire::v1::JobState::Succeeded as i32 {
            return Err(StoreError::Corrupt("run unfinished earlier child"));
        }
    }
    if !first_found {
        return Err(StoreError::Corrupt("run receipt anchor"));
    }
    if view.status != RunStatus::Active as i32 && view.completed_rounds != successes {
        return Err(StoreError::Corrupt("run completion history"));
    }
    let current_job = current.ok_or(StoreError::Corrupt("run current child"))?;
    let handle = view.current_job.as_ref().expect("validated child");
    if (view.status == RunStatus::Active as i32
        && (handle.status != DiscoveryJobStatus::Queued as i32
            || handle.revision != 1
            || handle.updated_at != handle.submitted_at))
        || handle.revision > current_job.revision
        || handle.submitted_at
            != current_job
                .specification
                .as_ref()
                .expect("validated child")
                .submitted_at
    {
        return Err(StoreError::Corrupt("run child observation"));
    }
    Ok(RunSnapshot {
        specification,
        view,
        current_job,
    })
}

pub(super) async fn persist(
    transaction: &mut Transaction<'_, Postgres>,
    specification: &RunSpecification,
    view: &RunView,
    first_job: Option<&str>,
) -> StoreResult<()> {
    loop_protocol::runs::validate_view(view)?;
    let spec_bytes = encode_message(specification)?;
    let view_bytes = encode_message(view)?;
    sqlx::query("SELECT set_config('loop.run_writer', 'v1', true)")
        .execute(&mut **transaction)
        .await?;
    let run_id = &view.run_id.as_ref().expect("validated run").value;
    let current = &view
        .current_job
        .as_ref()
        .expect("validated child")
        .job_id
        .as_ref()
        .expect("validated ID")
        .value;
    let now = timestamp_millis(view.updated_at.as_ref().expect("validated update"), true)?;
    if let Some(first) = first_job {
        sqlx::query("INSERT INTO research_runs (run_id, owner_id, first_job_id, current_job_id, status, revision, completed_rounds, reserved_steps, reserved_input, reserved_output, reserved_nano_usd, submitted_at_ms, updated_at_ms, deadline_ms, specification_blob, specification_sha256, view_blob, view_sha256) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$12,$13,$14,$15,$16,$17)")
            .bind(run_id).bind(&specification.owner.as_ref().expect("validated owner").actor_id.as_ref().expect("validated owner ID").value)
            .bind(first).bind(current).bind(view.status).bind(view.revision as i64).bind(view.completed_rounds as i32)
            .bind(view.reserved_steps as i64).bind(view.reserved_input_tokens as i64).bind(view.reserved_output_tokens as i64)
            .bind(model_money(view.reserved_cost.as_ref())? as i64).bind(now)
            .bind(timestamp_millis(view.deadline.as_ref().expect("validated deadline"), true)?)
            .bind(&spec_bytes).bind(Sha256::digest(&spec_bytes).to_vec()).bind(&view_bytes).bind(Sha256::digest(&view_bytes).to_vec())
            .execute(&mut **transaction).await?;
    } else {
        let affected = sqlx::query("UPDATE research_runs SET current_job_id=$2,status=$3,revision=$4,completed_rounds=$5,reserved_steps=$6,reserved_input=$7,reserved_output=$8,reserved_nano_usd=$9,updated_at_ms=$10,view_blob=$11,view_sha256=$12 WHERE run_id=$1 AND revision=$4-1")
            .bind(run_id).bind(current).bind(view.status).bind(view.revision as i64).bind(view.completed_rounds as i32)
            .bind(view.reserved_steps as i64).bind(view.reserved_input_tokens as i64).bind(view.reserved_output_tokens as i64)
            .bind(model_money(view.reserved_cost.as_ref())? as i64).bind(now).bind(&view_bytes).bind(Sha256::digest(&view_bytes).to_vec())
            .execute(&mut **transaction).await?.rows_affected();
        if affected != 1 {
            return Err(StoreError::RevisionConflict);
        }
    }
    Ok(())
}

pub(super) async fn receipt(
    transaction: &mut Transaction<'_, Postgres>,
    context: &CommandContext,
    operation: &str,
    request: &[u8],
    run_id: &str,
) -> StoreResult<Option<RunView>> {
    let row = sqlx::query(
        "SELECT * FROM command_receipts WHERE actor_id=$1 AND operation=$2 AND idempotency_key=$3",
    )
    .bind(
        &context
            .actor
            .as_ref()
            .expect("validated actor")
            .actor_id
            .as_ref()
            .expect("validated ID")
            .value,
    )
    .bind(operation)
    .bind(
        &context
            .idempotency_key
            .as_ref()
            .expect("validated key")
            .value,
    )
    .fetch_optional(&mut **transaction)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    if verified_blob(&row, "request_blob", "request_sha256")? != request {
        return Err(StoreError::IdempotencyConflict);
    }
    let bytes = verified_blob(&row, "response_blob", "response_sha256")?;
    let view = RunView::decode(bytes.as_slice()).map_err(|_| StoreError::Corrupt("run receipt"))?;
    loop_protocol::runs::validate_view(&view)
        .map_err(|_| StoreError::Corrupt("run receipt contract"))?;
    let parent = sqlx::query("SELECT first_job_id, revision FROM research_runs WHERE run_id=$1")
        .bind(run_id)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(StoreError::Corrupt("run receipt parent"))?;
    let snapshot = load(transaction, run_id).await?;
    let child_budget = snapshot
        .specification
        .discovery
        .as_ref()
        .expect("validated child")
        .budget
        .as_ref()
        .expect("validated child budget");
    let rounds = view.reserved_steps / u64::from(child_budget.maximum_steps);
    let recorded_job = sqlx::query("SELECT * FROM jobs WHERE job_id=$1 AND run_id=$2")
        .bind(
            &view
                .current_job
                .as_ref()
                .expect("validated child")
                .job_id
                .as_ref()
                .expect("validated ID")
                .value,
        )
        .bind(run_id)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(StoreError::Corrupt("run receipt child"))?;
    let recorded_job = record_from_row(&recorded_job)?;
    let child_key = format!("run.{:x}.{rounds}", Sha256::digest(run_id.as_bytes()));
    let handle = view.current_job.as_ref().expect("validated child");
    if view.encode_to_vec() != bytes
        || view.plan_verified
        || view.run_id.as_ref().expect("validated run").value != run_id
        || parent.try_get::<String, _>("first_job_id")? != row.try_get::<String, _>("job_id")?
        || view.revision as i64 > parent.try_get::<i64, _>("revision")?
        || timestamp_millis(view.updated_at.as_ref().expect("validated time"), true)?
            != row.try_get::<i64, _>("committed_at_ms")?
        || view.budget != snapshot.specification.budget
        || view.maximum_rounds != snapshot.specification.maximum_rounds
        || view.submitted_at != snapshot.view.submitted_at
        || view.deadline != snapshot.view.deadline
        || rounds == 0
        || rounds > u64::from(view.maximum_rounds)
        || view.reserved_steps != rounds * u64::from(child_budget.maximum_steps)
        || view.reserved_input_tokens
            != rounds
                .checked_mul(child_budget.maximum_input_tokens)
                .ok_or(StoreError::Corrupt("run receipt tokens"))?
        || view.reserved_output_tokens
            != rounds
                .checked_mul(child_budget.maximum_output_tokens)
                .ok_or(StoreError::Corrupt("run receipt tokens"))?
        || model_money(view.reserved_cost.as_ref())?
            != rounds
                .checked_mul(model_money(child_budget.maximum_cost.as_ref())?)
                .ok_or(StoreError::Corrupt("run receipt cost"))?
        || view.reserved_steps > snapshot.view.reserved_steps
        || (view.status == RunStatus::Active as i32
            && (u64::from(view.completed_rounds) + 1 != rounds
                || view.revision != rounds
                || handle.status != DiscoveryJobStatus::Queued as i32
                || handle.revision != 1
                || handle.submitted_at != view.updated_at
                || handle.updated_at != handle.submitted_at))
        || (view.status != RunStatus::Active as i32
            && (view != snapshot.view
                || u64::from(view.completed_rounds)
                    + u64::from(
                        view.current_job.as_ref().expect("validated child").status
                            != DiscoveryJobStatus::Succeeded as i32,
                    )
                    != rounds))
        || view.current_job.as_ref().expect("validated child").revision > recorded_job.revision
        || view
            .current_job
            .as_ref()
            .expect("validated child")
            .submitted_at
            != recorded_job
                .specification
                .as_ref()
                .expect("validated child")
                .submitted_at
        || recorded_job
            .specification
            .as_ref()
            .expect("validated child")
            .idempotency_key
            .as_ref()
            .map(|key| key.value.as_str())
            != Some(child_key.as_str())
        || timestamp_millis(
            handle.submitted_at.as_ref().expect("validated child time"),
            true,
        )? > timestamp_millis(
            view.updated_at.as_ref().expect("validated receipt time"),
            true,
        )?
    {
        return Err(StoreError::Corrupt("run receipt binding"));
    }
    Ok(Some(view))
}

pub(super) struct ReceiptInput<'a> {
    pub operation: &'a str,
    pub request: &'a [u8],
    pub view: &'a RunView,
    pub first_job: &'a str,
    pub now: i64,
}

pub(super) async fn record_receipt(
    transaction: &mut Transaction<'_, Postgres>,
    ledger: &str,
    context: &CommandContext,
    input: ReceiptInput<'_>,
) -> StoreResult<()> {
    let ReceiptInput {
        operation,
        request,
        view,
        first_job,
        now,
    } = input;
    let response = encode_message(view)?;
    sqlx::query("INSERT INTO command_receipts(actor_id,operation,idempotency_key,request_id,job_id,request_blob,request_sha256,response_blob,response_sha256,committed_at_ms) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
        .bind(&context.actor.as_ref().expect("validated actor").actor_id.as_ref().expect("validated ID").value)
        .bind(operation).bind(&context.idempotency_key.as_ref().expect("validated key").value)
        .bind(&context.request_id.as_ref().expect("validated request").value).bind(first_job)
        .bind(request).bind(Sha256::digest(request).to_vec()).bind(&response).bind(Sha256::digest(&response).to_vec()).bind(now)
        .execute(&mut **transaction).await?;
    audit::append(transaction, ledger, now, audit::EventInput {
        actor: context.actor.as_ref().expect("validated actor"), correlation_id: &context.correlation_id.as_ref().expect("validated correlation").value,
        causation_id: &context.causation_id.as_ref().expect("validated causation").value,
        action: AuditAction::CommandAccepted,
        target: AuditTarget { kind: AuditTargetKind::RunId, value: view.run_id.as_ref().expect("validated run").value.clone() },
        payload: canonicalize_audit_payload("loop.audit.command_accepted", 1,
            &serde_json::to_vec(&serde_json::json!({"command":operation,"request_id":context.request_id.as_ref().expect("validated request").value,"summary":"run reservation committed"}))
                .map_err(|_| StoreError::Invalid("run audit"))?)?,
    }).await?;
    sqlx::query("UPDATE store_metadata SET last_observed_at_ms=$1 WHERE singleton=1")
        .bind(now)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}
