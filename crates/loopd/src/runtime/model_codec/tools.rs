//! Closed research-description conversation; no executable model tool surface.

use prost::Message;

use super::*;

const TOOL_NAME: &str = "research_describe";
const RESULT_SCHEMA: &str = "loop.research-description/v1";

/// Return the single registered metadata tool and its closed empty input.
/// This schema permits no paths, dataset selection, arguments or capabilities.
pub(crate) fn describe_tool() -> StoreResult<wire::ToolDefinition> {
    let canonical_json = serde_json::to_vec(&json!({
        "type":"object", "properties":{}, "required":[], "additionalProperties":false
    }))
    .map_err(|_| invalid())?;
    Ok(wire::ToolDefinition {
        name: TOOL_NAME.into(),
        description: "Describe the frozen development research inputs and operator registry."
            .into(),
        input_schema: Some(wire::JsonSchema {
            schema_id: "loop.research-describe/v1".into(),
            schema_version: 1,
            schema_sha256: Some(wire::Sha256Digest {
                value: Sha256::digest(&canonical_json).to_vec(),
            }),
            canonical_json,
        }),
        strict: Some(true),
    })
}

/// Validate the fixed two-turn profile without granting execution authority.
/// Plan loading separately denies request IDs. Templates may have no history;
/// actual final requests additionally require the persisted call/result pair.
pub(crate) fn validate_tools(
    invocation: &wire::ModelInvocation,
    final_turn: bool,
) -> StoreResult<()> {
    if invocation.tools != [describe_tool()?] {
        return Err(invalid());
    }
    let choice = required(&invocation.tool_choice)?;
    let selected = wire::ToolChoiceMode::try_from(choice.mode).map_err(|_| invalid())?;
    if final_turn {
        let output = required(&invocation.structured_output)?;
        if selected != wire::ToolChoiceMode::None
            || !choice.named_tool.is_empty()
            || output.strict != Some(true)
            || output.json_schema.as_ref() != Some(&ast_schema()?)
        {
            return Err(invalid());
        }
    } else if invocation.structured_output.is_some()
        || !matches!(
            selected,
            wire::ToolChoiceMode::Required | wire::ToolChoiceMode::Named
        )
        || (selected == wire::ToolChoiceMode::Named && choice.named_tool != TOOL_NAME)
        || (selected == wire::ToolChoiceMode::Required && !choice.named_tool.is_empty())
        || has_history(&invocation.messages)
    {
        return Err(invalid());
    }
    validate_context(&invocation.messages)?;
    let model = required(&invocation.model)?;
    let capabilities = required(&model.capabilities)?;
    if !capabilities.supports_tools || (final_turn && !capabilities.supports_structured_output) {
        return Err(invalid());
    }
    model_json(model)?;
    budget_json(required(&invocation.budget)?)?;
    Ok(())
}

/// Validate a bounded typed conversation and its exact tool-call pairing.
/// Rejects unknown content, system messages after user content, duplicate calls,
/// orphan results, incomplete history and overlarge context without mutation.
pub(crate) fn validate_context(messages: &[wire::ModelMessage]) -> StoreResult<()> {
    if messages.is_empty()
        || messages.len() > 32
        || messages
            .iter()
            .try_fold(0usize, |size, message| {
                size.checked_add(message.encoded_len())
            })
            .is_none_or(|size| size > 131_072)
    {
        return Err(invalid());
    }
    let mut seen_user = false;
    let mut call_id: Option<&str> = None;
    let mut finished = false;
    for message in messages {
        message_json(message)?;
        let role = wire::ModelRole::try_from(message.role).map_err(|_| invalid())?;
        match role {
            wire::ModelRole::System if !seen_user && call_id.is_none() => {}
            wire::ModelRole::User if call_id.is_none() => seen_user = true,
            wire::ModelRole::Assistant
                if seen_user && call_id.is_none() && message.content.len() == 1 =>
            {
                let Some(wire::content_block::Content::ToolCall(call)) =
                    &message.content[0].content
                else {
                    return Err(invalid());
                };
                validate_call(call)?;
                call_id = Some(&call.tool_call_id);
            }
            wire::ModelRole::Tool
                if !finished && call_id.is_some() && message.content.len() == 1 =>
            {
                let Some(wire::content_block::Content::ToolResult(result)) =
                    &message.content[0].content
                else {
                    return Err(invalid());
                };
                if Some(result.tool_call_id.as_str()) != call_id {
                    return Err(invalid());
                }
                finished = true;
            }
            _ => return Err(invalid()),
        }
    }
    if !seen_user || (call_id.is_some() && !finished) {
        return Err(invalid());
    }
    Ok(())
}

/// Validate one tool-call response against the exact frozen request and usage.
/// Unregistered tools, multiple calls and malformed arguments are operational
/// failures. Returning a call does not authorize its execution or a data read.
pub(crate) fn response_call(
    request: &InvokeModelRequest,
    response: &wire::ModelResponse,
) -> StoreResult<wire::ToolCallContent> {
    validate_tools(required(&request.invocation)?, false)?;
    response_valid(request, response, wire::ModelFinishReason::ToolCall)?;
    let Some(wire::content_block::Content::ToolCall(call)) = &response.content[0].content else {
        return Err(invalid());
    };
    validate_call(call)?;
    Ok(call.clone())
}

pub(super) fn has_history(messages: &[wire::ModelMessage]) -> bool {
    messages
        .iter()
        .any(|message| message.role == wire::ModelRole::Assistant as i32)
}

fn validate_call(call: &wire::ToolCallContent) -> StoreResult<()> {
    if call.tool_name != TOOL_NAME || !valid_name(&call.tool_call_id) {
        return Err(invalid());
    }
    let argument = required(&call.arguments)?;
    let schema = required(&describe_tool()?.input_schema)?.clone();
    if argument.utf8_json != b"{}"
        || argument.schema_id != schema.schema_id
        || argument.schema_sha256 != schema.schema_sha256
        || required(&argument.canonical_sha256)?.value != Sha256::digest(b"{}").to_vec()
    {
        return Err(invalid());
    }
    Ok(())
}

fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

pub(super) fn tool_json(tool: &wire::ToolDefinition) -> StoreResult<Value> {
    let mut value = Map::new();
    text_field(&mut value, "name", &tool.name);
    text_field(&mut value, "description", &tool.description);
    value.insert(
        "inputSchema".into(),
        schema_json(required(&tool.input_schema)?),
    );
    value.insert("strict".into(), required(&tool.strict)?.to_owned().into());
    Ok(value.into())
}

pub(super) fn call_json(call: &wire::ToolCallContent) -> StoreResult<Value> {
    validate_call(call)?;
    let mut value = Map::new();
    text_field(&mut value, "toolCallId", &call.tool_call_id);
    text_field(&mut value, "toolName", &call.tool_name);
    value.insert(
        "arguments".into(),
        document_json(required(&call.arguments)?)?,
    );
    Ok(value.into())
}

pub(super) fn result_json(result: &wire::ToolResultContent) -> StoreResult<Value> {
    let Some(wire::tool_result_content::Result::Json(document)) = &result.result else {
        return Err(invalid());
    };
    if !valid_name(&result.tool_call_id)
        || result.status != wire::ToolResultStatus::Success as i32
        || document.schema_id != RESULT_SCHEMA
    {
        return Err(invalid());
    }
    // Runtime-produced metadata must already be canonical. This catches
    // duplicate keys and altered bytes before they enter persisted context.
    // Integer bounds keep this metadata profile identical to RFC 8785/JS;
    // arbitrary floating-point research arrays are not tool context.
    let parsed: Value = serde_json::from_slice(&document.utf8_json).map_err(|_| invalid())?;
    if !safe_value(&parsed)
        || serde_json::to_vec(&parsed).map_err(|_| invalid())? != document.utf8_json
        || required(&document.canonical_sha256)?.value.as_slice()
            != Sha256::digest(&document.utf8_json).as_slice()
    {
        return Err(invalid());
    }
    Ok(json!({
        "toolCallId": result.tool_call_id,
        "status": wire::ToolResultStatus::Success.as_str_name(),
        "json": document_json(document)?,
    }))
}

fn safe_value(value: &Value) -> bool {
    match value {
        Value::Number(number) => number.as_i64().is_some_and(|number| {
            (-9_007_199_254_740_991..=9_007_199_254_740_991).contains(&number)
        }),
        Value::Array(values) => values.iter().all(safe_value),
        Value::Object(values) => {
            values.keys().all(|key| key.is_ascii()) && values.values().all(safe_value)
        }
        _ => true,
    }
}

fn document_json(document: &wire::JsonDocument) -> StoreResult<Value> {
    if document.utf8_json.is_empty()
        || document.utf8_json.len() > 65_536
        || required(&document.canonical_sha256)?.value.len() != 32
        || required(&document.schema_sha256)?.value.len() != 32
    {
        return Err(invalid());
    }
    let mut value = Map::new();
    value.insert(
        "utf8Json".into(),
        STANDARD.encode(&document.utf8_json).into(),
    );
    digest_field(&mut value, "canonicalSha256", &document.canonical_sha256);
    text_field(&mut value, "schemaId", &document.schema_id);
    digest_field(&mut value, "schemaSha256", &document.schema_sha256);
    Ok(value.into())
}
