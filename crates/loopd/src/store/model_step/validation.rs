use loop_protocol::wire::provider::v1::InvokeModelRequest;
use loop_protocol::wire::v1::{self as wire, content_block, tool_result_content};
use loop_protocol::wire::v1::{Actor, JobRecord, ModelResponse, Money, job_specification};
use sha2::{Digest, Sha256};

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
    ordinal: u32,
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
        || specification.submitted_by.as_ref() != Some(actor)
        || discovery.maximum_candidates != 1
        || !matches!(job_budget.maximum_steps, 1 | 3)
        || ordinal > u32::from(job_budget.maximum_steps == 3)
        || budget.maximum_input_tokens == 0
        || budget.maximum_input_tokens > job_budget.maximum_input_tokens
        || budget.maximum_input_tokens > i64::MAX as u64
        || budget.maximum_output_tokens == 0
        || budget.maximum_output_tokens > job_budget.maximum_output_tokens
        || budget.maximum_output_tokens > i64::MAX as u64
        || cost == 0
        || cost > ceiling
        || wall > job_wall
        || invocation.messages.is_empty()
        || invocation.messages.len() > 512
    {
        return Err(StoreError::Invalid("frozen model invocation"));
    }
    if job_budget.maximum_steps == 3 {
        crate::runtime::model_codec::validate_tools(invocation, ordinal == 1)?;
    } else if !invocation.tools.is_empty()
        || invocation.structured_output.is_none()
        || invocation.tool_choice.as_ref().is_some_and(|choice| {
            choice.mode != wire::ToolChoiceMode::None as i32 || !choice.named_tool.is_empty()
        })
    {
        return Err(StoreError::Invalid("frozen final invocation"));
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

pub(super) fn call(step: &super::ModelStep, response: &ModelResponse) -> StoreResult<()> {
    self::response(step, response)?;
    let invocation = step
        .request
        .invocation
        .as_ref()
        .ok_or(StoreError::Corrupt("model invocation"))?;
    if step.ordinal != 0
        || invocation.tools.len() != 1
        || response.finish_reason != wire::ModelFinishReason::ToolCall as i32
        || response.content.len() != 1
    {
        return Err(StoreError::Invalid("intermediate tool response"));
    }
    let Some(content_block::Content::ToolCall(call)) = &response.content[0].content else {
        return Err(StoreError::Invalid("intermediate tool content"));
    };
    validate_id(&call.tool_call_id)?;
    let schema = invocation.tools[0]
        .input_schema
        .as_ref()
        .ok_or(StoreError::Invalid("tool input schema"))?;
    let arguments = call
        .arguments
        .as_ref()
        .ok_or(StoreError::Invalid("tool arguments"))?;
    let value = document(arguments)?;
    if call.tool_name != "research_describe"
        || call.tool_name != invocation.tools[0].name
        || arguments.schema_id != schema.schema_id
        || arguments.schema_sha256 != schema.schema_sha256
        || value != serde_json::json!({})
    {
        return Err(StoreError::Invalid("registered tool binding"));
    }
    Ok(())
}

pub(super) fn tool(step: &super::ModelStep, result: &wire::ToolResultContent) -> StoreResult<()> {
    let response = step
        .response
        .as_ref()
        .ok_or(StoreError::Invalid("missing tool call"))?;
    call(step, response)?;
    let Some(content_block::Content::ToolCall(call)) = &response.content[0].content else {
        return Err(StoreError::Invalid("tool call"));
    };
    let Some(tool_result_content::Result::Json(json)) = &result.result else {
        return Err(StoreError::Invalid("tool result format"));
    };
    if result.tool_call_id != call.tool_call_id
        || result.status != wire::ToolResultStatus::Success as i32
        || json.schema_id != "loop.research-description/v1"
        || postgres::encode_message(result)?.len() > 262_144
    {
        return Err(StoreError::Invalid("tool result binding"));
    }
    document(json)?;
    Ok(())
}

fn document(document: &wire::JsonDocument) -> StoreResult<serde_json::Value> {
    if document.utf8_json.is_empty()
        || document.utf8_json.len() > 262_144
        || document.schema_id.is_empty()
        || document.schema_id.len() > 128
        || document
            .schema_sha256
            .as_ref()
            .is_none_or(|digest| digest.value.len() != 32)
    {
        return Err(StoreError::Invalid("tool document bounds"));
    }
    let value: serde_json::Value = serde_json::from_slice(&document.utf8_json)
        .map_err(|_| StoreError::Invalid("tool document JSON"))?;
    let bytes =
        serde_json::to_vec(&value).map_err(|_| StoreError::Invalid("tool document JSON"))?;
    if document
        .canonical_sha256
        .as_ref()
        .is_none_or(|digest| digest.value.as_slice() != Sha256::digest(&bytes).as_slice())
    {
        return Err(StoreError::Invalid("tool document digest"));
    }
    Ok(value)
}

pub(super) fn continuation(
    prior: &super::ModelStep,
    request: &InvokeModelRequest,
) -> StoreResult<()> {
    let response = prior
        .response
        .as_ref()
        .ok_or(StoreError::Invalid("previous model response"))?;
    call(prior, response)?;
    let result = prior
        .tool_result
        .as_ref()
        .ok_or(StoreError::Invalid("previous tool result"))?;
    tool(prior, result)?;
    let original = prior
        .request
        .invocation
        .as_ref()
        .ok_or(StoreError::Corrupt("previous invocation"))?;
    let invocation = request
        .invocation
        .as_ref()
        .ok_or(StoreError::Invalid("next invocation"))?;
    let mut messages = original.messages.clone();
    messages.push(wire::ModelMessage {
        role: wire::ModelRole::Assistant as i32,
        content: response.content.clone(),
    });
    messages.push(wire::ModelMessage {
        role: wire::ModelRole::Tool as i32,
        content: vec![wire::ContentBlock {
            content: Some(content_block::Content::ToolResult(result.clone())),
        }],
    });
    if invocation.messages != messages
        || invocation.request_policy != original.request_policy
        || invocation.model != original.model
    {
        return Err(StoreError::Invalid("persisted model context"));
    }
    Ok(())
}

pub(super) fn totals(job: &JobRecord, steps: &[super::ModelStep]) -> StoreResult<()> {
    let Some(job_specification::Input::Discovery(input)) = job
        .specification
        .as_ref()
        .and_then(|specification| specification.input.as_ref())
    else {
        return Err(StoreError::Corrupt("model job input"));
    };
    let budget = input
        .budget
        .as_ref()
        .ok_or(StoreError::Corrupt("model job budget"))?;
    let total = steps
        .iter()
        .try_fold((0_u64, 0_u64, 0_u64), |sum, step| {
            Some((
                sum.0.checked_add(step.reserved_input)?,
                sum.1.checked_add(step.reserved_output)?,
                sum.2.checked_add(step.reserved_nano_usd)?,
            ))
        })
        .ok_or(StoreError::Invalid("model cumulative overflow"))?;
    let maximum = if budget.maximum_steps == 3 { 2 } else { 1 };
    if steps.len() > maximum
        || total.0 > budget.maximum_input_tokens
        || total.1 > budget.maximum_output_tokens
        || total.2 > money(budget.maximum_cost.as_ref())?
    {
        return Err(StoreError::Invalid("model cumulative budget"));
    }
    Ok(())
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
