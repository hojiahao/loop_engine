//! Bounded provider-neutral request fingerprints and untrusted AST receipts.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::{DateTime, SecondsFormat, Utc};
use loop_core::factor as domain;
use loop_protocol::wire::{provider::v1::InvokeModelRequest, v1 as wire};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::store::{StoreError, StoreResult};

const REQUEST_DOMAIN: &[u8] = b"loop.provider-invocation/v1\0";
const MAX_DOCUMENT: usize = 262_144;

mod tools;
pub(crate) use tools::{describe_tool, response_call, validate_context, validate_tools};

/// The sole structured-output schema admitted by a discovery plan.
///
/// An object envelope avoids protocol-specific top-level union restrictions.
/// Recursive node validation is independently repeated by the domain registry;
/// schema registration conveys no execution or factor-admission authority.
pub(crate) fn ast_schema() -> StoreResult<wire::JsonSchema> {
    let string = json!({"type":"string","minLength":1,"maxLength":128});
    let branch = |name: &str, fields: &[(&str, Value)]| {
        let mut properties = Map::new();
        properties.insert("node".into(), json!({"type":"string","const":name}));
        let mut required = vec![Value::from("node")];
        for (key, value) in fields {
            properties.insert((*key).into(), value.clone());
            required.push((*key).into());
        }
        json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
    };
    let schema = json!({
        "type":"object",
        "properties":{"ast":{"$ref":"#/$defs/node"}},
        "required":["ast"],
        "additionalProperties":false,
        "$defs":{"node":{"anyOf":[
            branch("field", &[("field", string.clone())]),
            branch("decimal", &[("value", string.clone())]),
            branch("boolean", &[("value", json!({"type":"boolean"}))]),
            branch("enum", &[("enum_type", string.clone()), ("value", string.clone())]),
            branch("call", &[
                ("operator", string.clone()), ("operator_version", string),
                ("arguments", json!({"type":"array","maxItems":1024,"items":{"$ref":"#/$defs/node"}}))
            ])
        ]}}
    });
    let canonical_json = serde_json::to_vec(&schema).map_err(|_| invalid())?;
    Ok(wire::JsonSchema {
        schema_id: "loop.harness-factor/v1".into(),
        schema_version: 1,
        schema_sha256: Some(wire::Sha256Digest {
            value: Sha256::digest(&canonical_json).to_vec(),
        }),
        canonical_json,
    })
}

/// Deny unsupported plan templates before persistence or any model reservation.
///
/// Request IDs belong to dispatch, not configuration. Model, policy and budget
/// authority are checked by the frozen-plan loader, not inferred from this shape.
pub(crate) fn validate_template(invocation: &wire::ModelInvocation) -> StoreResult<()> {
    let output = required(&invocation.structured_output)?;
    if invocation.request_id.is_some()
        || !invocation.tools.is_empty()
        || invocation.tool_choice.is_some()
        || output.strict != Some(true)
        || output.json_schema.as_ref() != Some(&ast_schema()?)
        || invocation.messages.is_empty()
        || invocation.messages.len() > 512
    {
        return Err(invalid());
    }
    for message in &invocation.messages {
        if !matches!(
            wire::ModelRole::try_from(message.role),
            Ok(wire::ModelRole::System | wire::ModelRole::User)
        ) {
            return Err(invalid());
        }
        message_json(message)?;
    }
    model_json(required(&invocation.model)?)?;
    budget_json(required(&invocation.budget)?)?;
    Ok(())
}

/// Hash the exact Provider journal projection, not protobuf wire bytes.
///
/// Accepts the frozen text/AST profile or the fixed research-description
/// conversation. Unknown enums and unsupported content fail closed.
/// This checks representation; deployment authorization remains with the caller.
pub(crate) fn request_digest(request: &InvokeModelRequest) -> StoreResult<[u8; 32]> {
    let value = request_json(request)?;
    let bytes = serde_json::to_vec(&value).map_err(|_| invalid())?;
    if bytes.len() > 524_288 {
        return Err(invalid());
    }
    let mut digest = Sha256::new();
    digest.update(REQUEST_DOMAIN);
    digest.update(bytes);
    Ok(digest.finalize().into())
}

fn request_json(request: &InvokeModelRequest) -> StoreResult<Value> {
    let context = required(&request.context)?;
    let invocation = required(&request.invocation)?;
    let model = required(&invocation.model)?;
    if invocation.tools.is_empty() && invocation.tool_choice.is_none() {
        // Preserve the journal representation of older schema registrations;
        // final AST acceptance still requires the installed AST schema.
        if required(&invocation.structured_output)?.strict != Some(true)
            || invocation.messages.is_empty()
            || invocation.messages.len() > 512
            || invocation.messages.iter().any(|message| {
                !matches!(
                    wire::ModelRole::try_from(message.role),
                    Ok(wire::ModelRole::System | wire::ModelRole::User)
                )
            })
        {
            return Err(invalid());
        }
    } else {
        let final_turn = invocation.structured_output.is_some();
        validate_tools(invocation, final_turn)?;
        if final_turn && !tools::has_history(&invocation.messages) {
            return Err(invalid());
        }
    }
    let mut command = Map::new();
    id_field(
        &mut command,
        "requestId",
        context.request_id.as_ref().map(|v| &v.value),
    );
    id_field(
        &mut command,
        "correlationId",
        context.correlation_id.as_ref().map(|v| &v.value),
    );
    id_field(
        &mut command,
        "causationId",
        context.causation_id.as_ref().map(|v| &v.value),
    );
    id_field(
        &mut command,
        "idempotencyKey",
        context.idempotency_key.as_ref().map(|v| &v.value),
    );
    if let Some(actor) = &context.actor {
        let mut value = Map::new();
        id_field(
            &mut value,
            "actorId",
            actor.actor_id.as_ref().map(|v| &v.value),
        );
        let kind = wire::ActorKind::try_from(actor.kind).map_err(|_| invalid())?;
        if actor.kind != 0 {
            value.insert("kind".into(), json!(kind.as_str_name()));
        }
        text_field(&mut value, "displayName", &actor.display_name);
        text_field(
            &mut value,
            "authenticatedSubject",
            &actor.authenticated_subject,
        );
        command.insert("actor".into(), value.into());
    }
    if let Some(value) = &context.requested_at {
        command.insert("requestedAt".into(), timestamp_json(value)?);
    }
    let mut body = Map::new();
    id_field(
        &mut body,
        "requestId",
        invocation.request_id.as_ref().map(|v| &v.value),
    );
    body.insert("model".into(), model_json(model)?);
    let messages = invocation
        .messages
        .iter()
        .map(message_json)
        .collect::<StoreResult<Vec<_>>>()?;
    body.insert("messages".into(), messages.into());
    if let Some(structured) = &invocation.structured_output {
        let mut output = Map::new();
        text_field(&mut output, "name", &structured.name);
        text_field(&mut output, "description", &structured.description);
        output.insert("strict".into(), true.into());
        output.insert(
            "jsonSchema".into(),
            schema_json(required(&structured.json_schema)?),
        );
        body.insert("structuredOutput".into(), output.into());
    }
    if !invocation.tools.is_empty() {
        body.insert(
            "tools".into(),
            invocation
                .tools
                .iter()
                .map(tools::tool_json)
                .collect::<StoreResult<Vec<_>>>()?
                .into(),
        );
    }
    if let Some(choice) = &invocation.tool_choice {
        let mode = wire::ToolChoiceMode::try_from(choice.mode).map_err(|_| invalid())?;
        let mut fields = Map::new();
        if choice.mode != 0 {
            fields.insert("mode".into(), mode.as_str_name().into());
        }
        text_field(&mut fields, "namedTool", &choice.named_tool);
        body.insert("toolChoice".into(), fields.into());
    }
    body.insert("budget".into(), budget_json(required(&invocation.budget)?)?);
    if let Some(policy) = &invocation.request_policy {
        let mut value = Map::new();
        id_field(
            &mut value,
            "policyId",
            policy.policy_id.as_ref().map(|v| &v.value),
        );
        text_field(&mut value, "revision", &policy.revision);
        digest_field(&mut value, "sha256", &policy.sha256);
        body.insert("requestPolicy".into(), value.into());
    }
    Ok(json!({"context": command, "invocation": body}))
}

fn model_json(model: &wire::ModelResolutionSnapshot) -> StoreResult<Value> {
    let mut value = Map::new();
    id_field(
        &mut value,
        "resolutionId",
        model.resolution_id.as_ref().map(|v| &v.value),
    );
    id_field(
        &mut value,
        "providerId",
        model.provider_id.as_ref().map(|v| &v.value),
    );
    id_field(
        &mut value,
        "modelId",
        model.model_id.as_ref().map(|v| &v.value),
    );
    text_field(&mut value, "requestedAlias", &model.requested_alias);
    let protocol =
        wire::ModelProtocolFamily::try_from(model.protocol_family).map_err(|_| invalid())?;
    if model.protocol_family != 0 {
        value.insert("protocolFamily".into(), json!(protocol.as_str_name()));
    }
    if let Some(capabilities) = &model.capabilities {
        let mut fields = Map::new();
        integer_field(
            &mut fields,
            "contextWindowTokens",
            capabilities.context_window_tokens,
        );
        integer_field(
            &mut fields,
            "maximumOutputTokens",
            capabilities.maximum_output_tokens,
        );
        for (name, enabled) in [
            ("supportsTools", capabilities.supports_tools),
            (
                "supportsParallelTools",
                capabilities.supports_parallel_tools,
            ),
            (
                "supportsStructuredOutput",
                capabilities.supports_structured_output,
            ),
            ("supportsVision", capabilities.supports_vision),
            ("supportsReasoning", capabilities.supports_reasoning),
            (
                "supportsPromptCaching",
                capabilities.supports_prompt_caching,
            ),
            ("supportsStreaming", capabilities.supports_streaming),
            ("supportsDocuments", capabilities.supports_documents),
        ] {
            if enabled {
                fields.insert(name.into(), true.into());
            }
        }
        value.insert("capabilities".into(), fields.into());
    }
    if let Some(pricing) = &model.pricing {
        let mut fields = Map::new();
        for (name, money) in [
            ("inputPerMillionTokens", &pricing.input_per_million_tokens),
            ("outputPerMillionTokens", &pricing.output_per_million_tokens),
            (
                "cachedInputPerMillionTokens",
                &pricing.cached_input_per_million_tokens,
            ),
            (
                "cacheCreationPerMillionTokens",
                &pricing.cache_creation_per_million_tokens,
            ),
        ] {
            if let Some(money) = money {
                fields.insert(name.into(), money_json(money));
            }
        }
        value.insert("pricing".into(), fields.into());
    }
    for (name, digest) in [
        ("capabilitySha256", &model.capability_sha256),
        ("catalogSha256", &model.catalog_sha256),
        ("providerPluginSha256", &model.provider_plugin_sha256),
        ("snapshotSha256", &model.snapshot_sha256),
    ] {
        digest_field(&mut value, name, digest);
    }
    if let Some(time) = &model.resolved_at {
        value.insert("resolvedAt".into(), timestamp_json(time)?);
    }
    text_field(
        &mut value,
        "providerPluginName",
        &model.provider_plugin_name,
    );
    text_field(
        &mut value,
        "providerPluginVersion",
        &model.provider_plugin_version,
    );
    Ok(value.into())
}

fn message_json(message: &wire::ModelMessage) -> StoreResult<Value> {
    let role = wire::ModelRole::try_from(message.role).map_err(|_| invalid())?;
    if !matches!(
        role,
        wire::ModelRole::System
            | wire::ModelRole::User
            | wire::ModelRole::Assistant
            | wire::ModelRole::Tool
    ) || message.content.is_empty()
        || message.content.len() > 256
    {
        return Err(invalid());
    }
    let blocks = message
        .content
        .iter()
        .map(|block| match (&block.content, role) {
            (
                Some(wire::content_block::Content::Text(text)),
                wire::ModelRole::System | wire::ModelRole::User,
            ) => {
                let mut fields = Map::new();
                text_field(&mut fields, "text", &text.text);
                Ok(json!({"text": fields}))
            }
            (Some(wire::content_block::Content::ToolCall(call)), wire::ModelRole::Assistant) => {
                tools::call_json(call).map(|value| json!({"toolCall":value}))
            }
            (Some(wire::content_block::Content::ToolResult(result)), wire::ModelRole::Tool) => {
                tools::result_json(result).map(|value| json!({"toolResult":value}))
            }
            _ => Err(invalid()),
        })
        .collect::<StoreResult<Vec<_>>>()?;
    Ok(json!({"role": role.as_str_name(), "content": blocks}))
}

fn schema_json(schema: &wire::JsonSchema) -> Value {
    let mut fields = Map::new();
    text_field(&mut fields, "schemaId", &schema.schema_id);
    if schema.schema_version != 0 {
        fields.insert("schemaVersion".into(), schema.schema_version.into());
    }
    if !schema.canonical_json.is_empty() {
        fields.insert(
            "canonicalJson".into(),
            STANDARD.encode(&schema.canonical_json).into(),
        );
    }
    digest_field(&mut fields, "schemaSha256", &schema.schema_sha256);
    fields.into()
}

fn budget_json(budget: &wire::InvocationBudget) -> StoreResult<Value> {
    let mut fields = Map::new();
    integer_field(
        &mut fields,
        "maximumInputTokens",
        budget.maximum_input_tokens,
    );
    integer_field(
        &mut fields,
        "maximumOutputTokens",
        budget.maximum_output_tokens,
    );
    if let Some(value) = &budget.maximum_cost {
        fields.insert("maximumCost".into(), money_json(value));
    }
    if let Some(value) = &budget.maximum_wall_time {
        if value.seconds < 0
            || !(0..1_000_000_000).contains(&value.nanos)
            || value.seconds > 315_576_000_000
        {
            return Err(invalid());
        }
        fields.insert(
            "maximumWallTime".into(),
            format!("{}{}s", value.seconds, fraction(value.nanos)).into(),
        );
    }
    Ok(fields.into())
}

fn timestamp_json(value: &prost_types::Timestamp) -> StoreResult<Value> {
    if !(-62_135_596_800..=253_402_300_799).contains(&value.seconds)
        || !(0..1_000_000_000).contains(&value.nanos)
    {
        return Err(invalid());
    }
    let value =
        DateTime::<Utc>::from_timestamp(value.seconds, value.nanos as u32).ok_or_else(invalid)?;
    Ok(value.to_rfc3339_opts(SecondsFormat::AutoSi, true).into())
}

fn fraction(nanos: i32) -> String {
    if nanos == 0 {
        String::new()
    } else if nanos % 1_000_000 == 0 {
        format!(".{:03}", nanos / 1_000_000)
    } else if nanos % 1000 == 0 {
        format!(".{:06}", nanos / 1000)
    } else {
        format!(".{nanos:09}")
    }
}

fn money_json(money: &wire::Money) -> Value {
    let mut fields = Map::new();
    if let Some(amount) = &money.amount {
        fields.insert("amount".into(), string_wrapper(&amount.value));
    }
    text_field(&mut fields, "currencyCode", &money.currency_code);
    fields.into()
}

fn string_wrapper(value: &str) -> Value {
    let mut fields = Map::new();
    text_field(&mut fields, "value", value);
    fields.into()
}

fn text_field(fields: &mut Map<String, Value>, name: &str, value: &str) {
    if !value.is_empty() {
        fields.insert(name.into(), value.into());
    }
}

fn integer_field(fields: &mut Map<String, Value>, name: &str, value: u64) {
    if value != 0 {
        fields.insert(name.into(), value.to_string().into());
    }
}

fn id_field(fields: &mut Map<String, Value>, name: &str, value: Option<&String>) {
    if let Some(value) = value {
        fields.insert(name.into(), string_wrapper(value));
    }
}

fn digest_field(fields: &mut Map<String, Value>, name: &str, value: &Option<wire::Sha256Digest>) {
    if let Some(value) = value {
        fields.insert(name.into(), string_wrapper(&STANDARD.encode(&value.value)));
    }
}

/// Validate a completed structured response and normalize its AST candidate.
///
/// The caller supplies the frozen request and registry; no supplier route is
/// consulted. Missing/mismatched identities, usage, schema or document hashes
/// are operational failures. This never chooses direction or admits a factor.
pub(crate) fn response_ast(
    request: &InvokeModelRequest,
    response: &wire::ModelResponse,
    registry: &domain::OperatorPolicyRegistry,
) -> StoreResult<wire::CanonicalFactorAst> {
    response_valid(request, response, wire::ModelFinishReason::Stop)?;
    let invocation = required(&request.invocation)?;
    let Some(wire::content_block::Content::StructuredOutput(output)) = &response.content[0].content
    else {
        return Err(invalid());
    };
    let document = required(&output.output)?;
    let schema = required(&required(&invocation.structured_output)?.json_schema)?;
    let digest = required(&schema.schema_sha256)?;
    if schema != &ast_schema()?
        || document.utf8_json.is_empty()
        || document.utf8_json.len() > MAX_DOCUMENT
        || document.schema_id != schema.schema_id
        || document.schema_sha256.as_ref() != Some(digest)
        || digest.value.as_slice() != Sha256::digest(&schema.canonical_json).as_slice()
    {
        return Err(invalid());
    }
    // The closed node parser rejects unknown/duplicate fields before a generic
    // JSON projection could discard them. AST values contain no JSON numbers.
    let envelope: AstEnvelope =
        serde_json::from_slice(&document.utf8_json).map_err(|_| invalid())?;
    let projected: Value = serde_json::from_slice(&document.utf8_json).map_err(|_| invalid())?;
    let canonical = serde_json::to_vec(&projected).map_err(|_| invalid())?;
    if required(&document.canonical_sha256)?.value.as_slice()
        != Sha256::digest(&canonical).as_slice()
    {
        return Err(invalid());
    }
    let expression = envelope.ast.expression()?;
    let limits = domain::ValidationLimits::default();
    let normalized =
        domain::canonicalize_expression(&expression, registry, limits).map_err(|_| invalid())?;
    let root_type = match &normalized {
        domain::FactorExpr::Field(field) => registry.field_type(field.field()),
        domain::FactorExpr::Call(call) => registry
            .definition_for(call.operator())
            .map(|value| value.output_type()),
        domain::FactorExpr::Literal(_) => None,
    };
    if root_type != Some(&domain::ValueType::Series) {
        return Err(invalid());
    }
    let canonical_json =
        domain::canonical_expression_bytes(&normalized, registry, limits).map_err(|_| invalid())?;
    let identity = domain::expression_id(&normalized, registry, limits).map_err(|_| invalid())?;
    Ok(wire::CanonicalFactorAst {
        expression_id: Some(wire::FactorExpressionId {
            value: identity.to_string(),
        }),
        ast: Some(wire::FactorAst {
            schema_version: 1,
            root: Some(wire_node(&normalized)),
        }),
        canonicalization_profile: "loop.factor-ast/v1".into(),
        canonical_json,
    })
}

fn response_valid(
    request: &InvokeModelRequest,
    response: &wire::ModelResponse,
    finish: wire::ModelFinishReason,
) -> StoreResult<()> {
    request_digest(request)?;
    let invocation = required(&request.invocation)?;
    let model = required(&invocation.model)?;
    let budget = required(&invocation.budget)?;
    let usage = required(&response.usage)?;
    if response.request_id != invocation.request_id
        || response.request_id.is_none()
        || response.resolution_id != model.resolution_id
        || response.resolution_id.is_none()
        || response.finish_reason != finish as i32
        || response.content.len() != 1
        || usage.input_tokens > budget.maximum_input_tokens
        || usage.output_tokens > budget.maximum_output_tokens
        || usage
            .cached_input_tokens
            .checked_add(usage.cache_creation_input_tokens)
            .is_none_or(|value| value > usage.input_tokens)
        || usage.reasoning_tokens > usage.output_tokens
        || usage.charged_cost.is_some()
    {
        return Err(invalid());
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AstEnvelope {
    ast: AstNode,
}

#[derive(Deserialize)]
#[serde(tag = "node", deny_unknown_fields)]
enum AstNode {
    #[serde(rename = "field")]
    Field { field: domain::Identifier },
    #[serde(rename = "decimal")]
    Decimal { value: domain::CanonicalDecimal },
    #[serde(rename = "boolean")]
    Boolean { value: bool },
    #[serde(rename = "enum")]
    Enumeration {
        enum_type: domain::Identifier,
        value: domain::Identifier,
    },
    #[serde(rename = "call")]
    Call {
        operator: domain::Identifier,
        operator_version: domain::PositiveInteger,
        arguments: Vec<AstNode>,
    },
}

impl AstNode {
    fn expression(self) -> StoreResult<domain::FactorExpr> {
        Ok(match self {
            Self::Field { field } => domain::FactorExpr::Field(domain::FieldRef::new(field)),
            Self::Decimal { value } => domain::FactorExpr::Literal(domain::Literal::Decimal(value)),
            Self::Boolean { value } => domain::FactorExpr::Literal(domain::Literal::Boolean(value)),
            Self::Enumeration { enum_type, value } => domain::FactorExpr::Literal(
                domain::Literal::Enumeration(domain::EnumLiteral::new(enum_type, value)),
            ),
            Self::Call {
                operator,
                operator_version,
                arguments,
            } => domain::FactorExpr::Call(domain::OperatorCall::new(
                domain::OperatorRef::new(operator, operator_version),
                arguments
                    .into_iter()
                    .map(Self::expression)
                    .collect::<StoreResult<Vec<_>>>()?,
            )),
        })
    }
}

fn wire_node(expression: &domain::FactorExpr) -> wire::FactorAstNode {
    use wire::{factor_ast_node::Node, factor_literal::Value};
    let node = match expression {
        domain::FactorExpr::Field(field) => Node::Field(wire::FieldReference {
            field: field.field().as_str().into(),
        }),
        domain::FactorExpr::Literal(literal) => Node::Literal(wire::FactorLiteral {
            value: Some(match literal {
                domain::Literal::Decimal(value) => Value::Decimal(wire::ExactDecimal {
                    value: value.as_str().into(),
                }),
                domain::Literal::Boolean(value) => Value::Boolean(*value),
                domain::Literal::Enumeration(value) => {
                    Value::Enumeration(wire::VersionedEnumLiteral {
                        enum_type: value.enum_type().as_str().into(),
                        value: value.value().as_str().into(),
                    })
                }
            }),
        }),
        domain::FactorExpr::Call(call) => Node::Call(wire::OperatorCall {
            operator: Some(wire::OperatorReference {
                operator: call.operator().name().as_str().into(),
                operator_version: call.operator().semantic_version().as_str().into(),
            }),
            arguments: call.arguments().iter().map(wire_node).collect(),
        }),
    };
    wire::FactorAstNode { node: Some(node) }
}

fn required<T>(value: &Option<T>) -> StoreResult<&T> {
    value.as_ref().ok_or_else(invalid)
}

fn invalid() -> StoreError {
    StoreError::Invalid("harness model envelope")
}

#[cfg(test)]
#[path = "model_codec_tests.rs"]
mod tests;
