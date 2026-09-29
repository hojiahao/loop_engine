use loop_protocol::wire::provider::v1::InvokeModelRequest;
use loop_protocol::wire::v1::{Actor, JobRecord, ModelResponse, Money, job_specification};

use super::super::{StoreError, StoreResult, postgres, validate_id};

pub(crate) fn money(value: Option<&Money>) -> StoreResult<u64> {
    let value = value.ok_or(StoreError::Invalid("model budget currency"))?;
    let text = &value
        .amount
        .as_ref()
        .ok_or(StoreError::Invalid("model budget amount"))?
        .value;
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if value.currency_code != "USD"
        || whole.is_empty()
        || whole.len() > 6
        || fraction.len() > 9
        || (whole.len() > 1 && whole.starts_with('0'))
        || !whole.bytes().all(|value| value.is_ascii_digit())
        || !fraction.bytes().all(|value| value.is_ascii_digit())
        || (text.contains('.') && (fraction.is_empty() || fraction.ends_with('0')))
    {
        return Err(StoreError::Invalid("model USD precision"));
    }
    let whole: u64 = whole
        .parse()
        .map_err(|_| StoreError::Invalid("model USD amount"))?;
    let fraction: u64 = if fraction.is_empty() {
        0
    } else {
        fraction
            .parse::<u64>()
            .map_err(|_| StoreError::Invalid("model USD fraction"))?
            * 10u64.pow(9 - fraction.len() as u32)
    };
    Ok(whole * 1_000_000_000 + fraction)
}

pub(crate) fn duration(value: Option<&prost_types::Duration>) -> StoreResult<i64> {
    let value = value.ok_or(StoreError::Invalid("model wall time"))?;
    if value.seconds < 0 || !(0..1_000_000_000).contains(&value.nanos) {
        return Err(StoreError::Invalid("model wall time"));
    }
    let millis = value
        .seconds
        .checked_mul(1_000)
        .and_then(|seconds| seconds.checked_add((i64::from(value.nanos) + 999_999) / 1_000_000))
        .ok_or(StoreError::Invalid("model wall time"))?;
    if !(1..=120_000).contains(&millis) {
        return Err(StoreError::Invalid("model wall time range"));
    }
    Ok(millis)
}

pub(super) fn invocation(
    job: &JobRecord,
    actor: &Actor,
    request: &InvokeModelRequest,
) -> StoreResult<(u64, u64, u64, i64)> {
    let specification = job
        .specification
        .as_ref()
        .ok_or(StoreError::Corrupt("model job specification"))?;
    let Some(job_specification::Input::Discovery(discovery)) = &specification.input else {
        return Err(StoreError::AdmissionDenied);
    };
    let job_budget = discovery
        .budget
        .as_ref()
        .ok_or(StoreError::Corrupt("model job budget"))?;
    let context = super::super::lifecycle::validate_context(request.context.as_ref(), actor)?;
    let invocation = request
        .invocation
        .as_ref()
        .ok_or(StoreError::Invalid("model invocation"))?;
    let budget = invocation
        .budget
        .as_ref()
        .ok_or(StoreError::Invalid("model invocation budget"))?;
    let cost = money(budget.maximum_cost.as_ref())?;
    let ceiling = money(job_budget.maximum_cost.as_ref())?;
    let wall = duration(budget.maximum_wall_time.as_ref())?;
    let job_wall = duration(job_budget.maximum_wall_time.as_ref())?;
    if invocation.request_id != context.request_id
        || invocation.model != discovery.maker_model
        || invocation.request_policy.is_none()
        || context.actor.as_ref() != Some(actor)
        || discovery.maximum_candidates != 1
        || job_budget.maximum_steps != 1
        || budget.maximum_input_tokens == 0
        || budget.maximum_input_tokens > job_budget.maximum_input_tokens
        || budget.maximum_input_tokens > i64::MAX as u64
        || budget.maximum_output_tokens == 0
        || budget.maximum_output_tokens > job_budget.maximum_output_tokens
        || budget.maximum_output_tokens > i64::MAX as u64
        || cost == 0
        || cost > ceiling
        || wall > job_wall
        || !invocation.tools.is_empty()
        || invocation.messages.is_empty()
        || invocation.messages.len() > 512
    {
        return Err(StoreError::Invalid("frozen model invocation"));
    }
    validate_id(
        &invocation
            .request_id
            .as_ref()
            .ok_or(StoreError::Invalid("model request ID"))?
            .value,
    )?;
    Ok((
        budget.maximum_input_tokens,
        budget.maximum_output_tokens,
        cost,
        wall,
    ))
}

pub(super) fn response(step: &super::ModelStep, response: &ModelResponse) -> StoreResult<()> {
    let invocation = step
        .request
        .invocation
        .as_ref()
        .ok_or(StoreError::Corrupt("model invocation"))?;
    let usage = response
        .usage
        .as_ref()
        .ok_or(StoreError::Invalid("model response usage"))?;
    if response.request_id != invocation.request_id
        || response.resolution_id
            != invocation
                .model
                .as_ref()
                .and_then(|model| model.resolution_id.clone())
        || response.content.is_empty()
        || response.content.len() > 256
        || !(1..=4).contains(&response.finish_reason)
        || usage.input_tokens > step.reserved_input
        || usage.output_tokens > step.reserved_output
        || usage.cached_input_tokens > usage.input_tokens
        || usage.cache_creation_input_tokens > usage.input_tokens - usage.cached_input_tokens
        || usage.reasoning_tokens > usage.output_tokens
        || usage
            .charged_cost
            .as_ref()
            .map(|cost| money(Some(cost)))
            .transpose()?
            .is_some_and(|cost| cost > step.reserved_nano_usd)
    {
        return Err(StoreError::Invalid("model response binding"));
    }
    if postgres::encode_message(response)?.len() > 1_048_576 {
        return Err(StoreError::Invalid("model response size"));
    }
    Ok(())
}
