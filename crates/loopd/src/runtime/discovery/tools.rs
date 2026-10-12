//! One bounded, read-only tool over server-pinned development evidence.

use loop_core::factor::{Identifier, OperatorPolicyRegistry, ValueType};
use loop_protocol::wire::v1 as wire;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::manifests::{LocalArtifacts, ObjectRef, data};
use crate::store::{StoreError, StoreResult, validate_id};

const RESULT_SCHEMA: &str = "loop.research-description/v1";
const RESULT_LIMIT: usize = 16_384;

/// Describe only the frozen job's verified development data and installed fields.
/// Unknown tool names, nonempty arguments, changed files and protected samples
/// fail closed before a result is returned. Repeating a read grants no mutation
/// authority; the caller owns durable call/result commits and cancellation.
pub(super) async fn describe(
    source: &LocalArtifacts,
    reference: &ObjectRef,
    job: &wire::JobSpecification,
    registry: &OperatorPolicyRegistry,
    call: &wire::ToolCallContent,
) -> StoreResult<wire::ToolResultContent> {
    validate_call(call)?;
    if !matches!(
        job.input,
        Some(wire::job_specification::Input::Discovery(_))
    ) {
        return Err(StoreError::AdmissionDenied);
    }
    let evidence = data::resolve(source, reference, job, false).await?;
    let mut description = evidence.description(job)?;
    let object = description.as_object_mut().ok_or_else(invalid)?;
    object.insert("schema".into(), RESULT_SCHEMA.into());
    object.insert("registry_id".into(), registry.identity().to_string().into());
    object.insert("fields".into(), fields(registry)?);
    let canonical = serde_json::to_vec(&description).map_err(|_| invalid())?;
    if canonical.len() > RESULT_LIMIT {
        return Err(StoreError::Invalid("research description size"));
    }
    let schema = result_schema()?;
    let result = wire::ToolResultContent {
        tool_call_id: call.tool_call_id.clone(),
        status: wire::ToolResultStatus::Success as i32,
        result: Some(wire::tool_result_content::Result::Json(
            wire::JsonDocument {
                canonical_sha256: Some(wire::Sha256Digest {
                    value: Sha256::digest(&canonical).to_vec(),
                }),
                utf8_json: canonical,
                schema_id: schema.schema_id,
                schema_sha256: schema.schema_sha256,
            },
        )),
    };
    validate_result(&result)?;
    evidence.check(job)?;
    Ok(result)
}

/// Closed result contract for Provider registration and persisted tool context.
/// This describes evidence only, never numerical evaluation or factor admission.
pub(super) fn result_schema() -> StoreResult<wire::JsonSchema> {
    let text = json!({"type":"string","minLength":1,"maxLength":128});
    let digest = json!({"type":"string","minLength":71,"maxLength":71});
    let date = json!({"type":"string","minLength":10,"maxLength":10});
    let sample = closed(json!({
        "role":{"type":"string","enum":["operator_warmup","in_sample","development_validation"]},
        "start":date,"end":date,
    }));
    let schema = closed(json!({
        "name":text,"version":{"type":"integer","minimum":1,"maximum":u32::MAX},
        "sha256":digest,
    }));
    let artifact = closed(json!({
        "snapshot_id":text,"schema":schema,"media_type":text,
        "byte_size":{"type":"integer","minimum":1,"maximum":268_435_456},
    }));
    let field = closed(json!({"name":text,"value_type":text}));
    let document = closed(json!({
        "schema":{"type":"string","const":RESULT_SCHEMA},
        "dataset_id":digest,"registry_id":digest,
        "snapshot_ids":{"type":"array","minItems":1,"maxItems":128,"uniqueItems":true,"items":text},
        "sample":sample,
        "artifacts":{"type":"array","minItems":1,"maxItems":128,"items":artifact},
        "fields":{"type":"array","minItems":1,"maxItems":128,"items":field},
    }));
    let canonical_json = serde_json::to_vec(&document).map_err(|_| invalid())?;
    Ok(wire::JsonSchema {
        schema_id: RESULT_SCHEMA.into(),
        schema_version: 1,
        schema_sha256: Some(wire::Sha256Digest {
            value: Sha256::digest(&canonical_json).to_vec(),
        }),
        canonical_json,
    })
}

/// Reject altered, oversized or non-allowlisted tool evidence before replay.
/// Digest validation is integrity checking, not authorization of caller input;
/// only immutable receipts may supply continuation context.
pub(super) fn validate_result(result: &wire::ToolResultContent) -> StoreResult<()> {
    validate_id(&result.tool_call_id)?;
    let Some(wire::tool_result_content::Result::Json(document)) = &result.result else {
        return Err(invalid());
    };
    let schema = result_schema()?;
    if result.status != wire::ToolResultStatus::Success as i32
        || document.schema_id != schema.schema_id
        || document.schema_sha256 != schema.schema_sha256
        || document.utf8_json.len() > RESULT_LIMIT
        || document
            .canonical_sha256
            .as_ref()
            .map(|digest| digest.value.as_slice())
            != Some(Sha256::digest(&document.utf8_json).as_slice())
    {
        return Err(invalid());
    }
    let value: Value = serde_json::from_slice(&document.utf8_json).map_err(|_| invalid())?;
    if serde_json::to_vec(&value).map_err(|_| invalid())? != document.utf8_json {
        return Err(invalid());
    }
    let schema: Value = serde_json::from_slice(&schema.canonical_json).map_err(|_| invalid())?;
    validate_shape(&value, &schema)?;
    for name in ["dataset_id", "registry_id"] {
        loop_core::factor::ExpressionId::parse(value[name].as_str().ok_or_else(invalid)?)
            .map_err(|_| invalid())?;
    }
    for artifact in value["artifacts"].as_array().ok_or_else(invalid)? {
        loop_core::factor::ExpressionId::parse(
            artifact["schema"]["sha256"].as_str().ok_or_else(invalid)?,
        )
        .map_err(|_| invalid())?;
    }
    let sample = &value["sample"];
    let start = chrono::NaiveDate::parse_from_str(
        sample["start"].as_str().ok_or_else(invalid)?,
        "%Y-%m-%d",
    )
    .map_err(|_| invalid())?;
    let end =
        chrono::NaiveDate::parse_from_str(sample["end"].as_str().ok_or_else(invalid)?, "%Y-%m-%d")
            .map_err(|_| invalid())?;
    let (lower, upper) = match sample["role"].as_str() {
        Some("operator_warmup") => ("2005-01-01", "2006-12-31"),
        Some("in_sample") => ("2007-01-01", "2016-12-31"),
        Some("development_validation") => ("2017-01-01", "2020-12-31"),
        _ => return Err(invalid()),
    };
    if start > end
        || sample["start"].as_str().ok_or_else(invalid)? < lower
        || sample["end"].as_str().ok_or_else(invalid)? > upper
    {
        return Err(invalid());
    }
    Ok(())
}

fn validate_call(call: &wire::ToolCallContent) -> StoreResult<()> {
    validate_id(&call.tool_call_id)?;
    let schema = crate::runtime::model_codec::describe_tool()?
        .input_schema
        .ok_or_else(invalid)?;
    let arguments = call.arguments.as_ref().ok_or_else(invalid)?;
    if call.tool_name != "research_describe"
        || arguments.schema_id != schema.schema_id
        || arguments.schema_sha256 != schema.schema_sha256
        || arguments.utf8_json != b"{}"
        || arguments
            .canonical_sha256
            .as_ref()
            .map(|digest| digest.value.as_slice())
            != Some(Sha256::digest(b"{}").as_slice())
    {
        return Err(invalid());
    }
    Ok(())
}

fn fields(registry: &OperatorPolicyRegistry) -> StoreResult<Value> {
    let value: Value =
        serde_json::from_slice(&registry.canonical_bytes()).map_err(|_| invalid())?;
    let entries = value
        .get("fields")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    let mut fields = Vec::with_capacity(entries.len());
    for entry in entries {
        let name = entry
            .get("field")
            .and_then(Value::as_str)
            .ok_or_else(invalid)?;
        let identifier = Identifier::new(name).map_err(|_| invalid())?;
        let value_type = match registry.field_type(&identifier).ok_or_else(invalid)? {
            ValueType::Series => "series".into(),
            ValueType::Decimal => "decimal".into(),
            ValueType::Boolean => "boolean".into(),
            ValueType::Enumeration(name) => format!("enum:{name}"),
        };
        fields.push(json!({"name":name,"value_type":value_type}));
    }
    Ok(fields.into())
}

fn closed(properties: Value) -> Value {
    let required: Vec<_> = properties
        .as_object()
        .into_iter()
        .flat_map(|map| map.keys())
        .cloned()
        .collect();
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}

// Validate only the finite schema vocabulary above, never arbitrary JSON Schema.
// Unknown shape rules fail closed rather than silently becoming permissive.
fn validate_shape(value: &Value, schema: &Value) -> StoreResult<()> {
    match schema["type"].as_str() {
        Some("object") => {
            let object = value.as_object().ok_or_else(invalid)?;
            let properties = schema["properties"].as_object().ok_or_else(invalid)?;
            if object.len() != properties.len() {
                return Err(invalid());
            }
            for (name, property) in properties {
                validate_shape(object.get(name).ok_or_else(invalid)?, property)?;
            }
        }
        Some("array") => {
            let array = value.as_array().ok_or_else(invalid)?;
            if !(1..=128).contains(&array.len()) {
                return Err(invalid());
            }
            for (index, item) in array.iter().enumerate() {
                validate_shape(item, &schema["items"])?;
                if schema["uniqueItems"] == true && array[..index].contains(item) {
                    return Err(invalid());
                }
            }
        }
        Some("integer") => {
            let number = value.as_u64().ok_or_else(invalid)?;
            if !(schema["minimum"].as_u64().ok_or_else(invalid)?
                ..=schema["maximum"].as_u64().ok_or_else(invalid)?)
                .contains(&number)
            {
                return Err(invalid());
            }
        }
        Some("string") => {
            let text = value.as_str().ok_or_else(invalid)?;
            if let Some(expected) = schema.get("const") {
                if value != expected {
                    return Err(invalid());
                }
            } else if let Some(allowed) = schema["enum"].as_array() {
                if !allowed.contains(value) {
                    return Err(invalid());
                }
            } else {
                let length = text.len() as u64;
                if !(schema["minLength"].as_u64().ok_or_else(invalid)?
                    ..=schema["maxLength"].as_u64().ok_or_else(invalid)?)
                    .contains(&length)
                    || text.chars().any(char::is_control)
                {
                    return Err(invalid());
                }
            }
        }
        _ => return Err(invalid()),
    }
    Ok(())
}

fn invalid() -> StoreError {
    StoreError::Invalid("research description")
}

#[cfg(test)]
mod tests;
