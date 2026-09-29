use super::*;
use prost::Message;

fn fixture() -> InvokeModelRequest {
    let value: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/contracts/protocol/v1/harness_invocation.json"
    ))
    .unwrap();
    InvokeModelRequest::decode(
        STANDARD
            .decode(value["request_base64"].as_str().unwrap())
            .unwrap()
            .as_slice(),
    )
    .unwrap()
}

fn response(request: &InvokeModelRequest, text: &str) -> wire::ModelResponse {
    let text = format!(r#"{{"ast":{text}}}"#);
    let invocation = request.invocation.as_ref().unwrap();
    let schema = invocation
        .structured_output
        .as_ref()
        .unwrap()
        .json_schema
        .as_ref()
        .unwrap();
    let value: Value = serde_json::from_str(&text).unwrap();
    let canonical = serde_json::to_vec(&value).unwrap();
    wire::ModelResponse {
        request_id: invocation.request_id.clone(),
        resolution_id: invocation.model.as_ref().unwrap().resolution_id.clone(),
        finish_reason: wire::ModelFinishReason::Stop as i32,
        content: vec![wire::ContentBlock {
            content: Some(wire::content_block::Content::StructuredOutput(
                wire::StructuredOutputContent {
                    output: Some(wire::JsonDocument {
                        utf8_json: text.as_bytes().to_vec(),
                        canonical_sha256: Some(wire::Sha256Digest {
                            value: Sha256::digest(canonical).to_vec(),
                        }),
                        schema_id: schema.schema_id.clone(),
                        schema_sha256: schema.schema_sha256.clone(),
                    }),
                },
            )),
        }],
        usage: Some(wire::ModelUsage {
            input_tokens: 12,
            output_tokens: 8,
            ..Default::default()
        }),
    }
}

fn ast_request() -> InvokeModelRequest {
    let mut request = fixture();
    request
        .invocation
        .as_mut()
        .unwrap()
        .structured_output
        .as_mut()
        .unwrap()
        .json_schema = Some(ast_schema().unwrap());
    request
}

fn candidate(
    request: &InvokeModelRequest,
    response: &wire::ModelResponse,
) -> StoreResult<wire::CanonicalFactorAst> {
    response_ast(request, response, &domain::us_equities::registry().unwrap())
}

fn document(response: &mut wire::ModelResponse) -> &mut wire::JsonDocument {
    let Some(wire::content_block::Content::StructuredOutput(output)) =
        &mut response.content[0].content
    else {
        panic!("test fixture")
    };
    output.output.as_mut().unwrap()
}

#[test]
fn provider_digest() {
    let fixture_value: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/contracts/protocol/v1/harness_invocation.json"
    ))
    .unwrap();
    let digest = request_digest(&fixture()).unwrap();
    let rendered: String = digest.iter().map(|value| format!("{value:02x}")).collect();
    assert_eq!(rendered, fixture_value["request_sha256"].as_str().unwrap());
}

#[test]
fn protobuf_defaults() {
    let mut request = fixture();
    let context = request.context.as_mut().unwrap();
    context.actor.as_mut().unwrap().display_name.clear();
    context.causation_id = None;
    let model = request.invocation.as_mut().unwrap().model.as_mut().unwrap();
    model.capabilities = Some(wire::ModelCapabilities::default());
    model
        .pricing
        .as_mut()
        .unwrap()
        .cached_input_per_million_tokens = Some(wire::Money::default());
    let value = request_json(&request).unwrap();
    assert!(value["context"]["actor"].get("displayName").is_none());
    assert!(value["context"].get("causationId").is_none());
    assert_eq!(value["invocation"]["model"]["capabilities"], json!({}));
    assert_eq!(
        value["invocation"]["model"]["pricing"]["cachedInputPerMillionTokens"],
        json!({})
    );
}

#[test]
fn timestamp_precision() {
    for (nanos, suffix) in [
        (0, "00Z"),
        (123_000_000, "00.123Z"),
        (123_456_000, "00.123456Z"),
        (123_456_789, "00.123456789Z"),
    ] {
        let encoded = timestamp_json(&prost_types::Timestamp {
            seconds: 1_790_553_600,
            nanos,
        })
        .unwrap();
        assert!(encoded.as_str().unwrap().ends_with(suffix));
    }
    assert!(
        timestamp_json(&prost_types::Timestamp {
            seconds: 0,
            nanos: -1
        })
        .is_err()
    );
    assert!(
        timestamp_json(&prost_types::Timestamp {
            seconds: 253_402_300_800,
            nanos: 0
        })
        .is_err()
    );
}

#[test]
fn duration_precision() {
    for (nanos, expected) in [
        (0, "4s"),
        (123_000_000, "4.123s"),
        (123_456_000, "4.123456s"),
        (123_456_789, "4.123456789s"),
    ] {
        let budget = wire::InvocationBudget {
            maximum_wall_time: Some(prost_types::Duration { seconds: 4, nanos }),
            ..Default::default()
        };
        assert_eq!(budget_json(&budget).unwrap()["maximumWallTime"], expected);
    }
}

#[test]
fn profile_denied() {
    let baseline = fixture();
    let mut request = baseline.clone();
    request.invocation.as_mut().unwrap().messages[0].role = wire::ModelRole::Assistant as i32;
    assert!(request_digest(&request).is_err());
    request = baseline.clone();
    request.invocation.as_mut().unwrap().tool_choice = Some(wire::ToolChoice::default());
    assert!(request_digest(&request).is_err());
    request = baseline.clone();
    request
        .invocation
        .as_mut()
        .unwrap()
        .structured_output
        .as_mut()
        .unwrap()
        .strict = None;
    assert!(request_digest(&request).is_err());
    request = baseline;
    request
        .invocation
        .as_mut()
        .unwrap()
        .model
        .as_mut()
        .unwrap()
        .protocol_family = 127;
    assert!(request_digest(&request).is_err());
}

#[test]
fn field_candidate() {
    let request = ast_request();
    let result = candidate(
        &request,
        &response(&request, r#"{ "field":"market.close", "node":"field" }"#),
    )
    .unwrap();
    assert_eq!(
        result.canonical_json,
        br#"{"node":"field","field":"market.close"}"#
    );
    assert!(result.expression_id.unwrap().value.starts_with("sha256:"));
}

#[test]
fn call_candidate() {
    let request = ast_request();
    let result = candidate(
        &request,
        &response(&request, r#"{"node":"call","operator":"add","operator_version":"2","arguments":[{"node":"field","field":"market.open"},{"node":"field","field":"market.close"}]}"#),
    ).unwrap();
    assert_eq!(result.canonical_json, br#"{"node":"call","operator":"add","operator_version":"2","arguments":[{"node":"field","field":"market.close"},{"node":"field","field":"market.open"}]}"#);
}

#[test]
fn template_pinned() {
    let mut invocation = ast_request().invocation.unwrap();
    invocation.request_id = None;
    validate_template(&invocation).unwrap();
    invocation
        .structured_output
        .as_mut()
        .unwrap()
        .json_schema
        .as_mut()
        .unwrap()
        .schema_id
        .push_str("-other");
    assert!(validate_template(&invocation).is_err());
    invocation.structured_output.as_mut().unwrap().json_schema = Some(ast_schema().unwrap());
    invocation.request_id = Some(wire::RequestId {
        value: "caller-supplied".into(),
    });
    assert!(validate_template(&invocation).is_err());
}

#[test]
fn envelope_closed() {
    let request = ast_request();
    let mut result = response(&request, r#"{"node":"field","field":"market.close"}"#);
    for text in [
        r#"{"ast":{"node":"field","field":"market.close"},"execute":"code"}"#,
        r#"{"ast":{"node":"field","field":"market.close"},"ast":{"node":"field","field":"market.open"}}"#,
    ] {
        document(&mut result).utf8_json = text.as_bytes().to_vec();
        document(&mut result).canonical_sha256 = Some(wire::Sha256Digest {
            value: Sha256::digest(
                serde_json::to_vec(&serde_json::from_str::<Value>(text).unwrap()).unwrap(),
            )
            .to_vec(),
        });
        assert!(candidate(&request, &result).is_err());
    }
}

#[test]
fn identity_denied() {
    let request = ast_request();
    let mut result = response(&request, r#"{"node":"field","field":"market.close"}"#);
    result.request_id.as_mut().unwrap().value.push_str("-other");
    assert!(candidate(&request, &result).is_err());
    result.request_id = request.invocation.as_ref().unwrap().request_id.clone();
    result.resolution_id = None;
    assert!(candidate(&request, &result).is_err());
}

#[test]
fn usage_denied() {
    let request = ast_request();
    let baseline = response(&request, r#"{"node":"field","field":"market.close"}"#);
    for usage in [
        wire::ModelUsage {
            input_tokens: 1025,
            output_tokens: 8,
            ..Default::default()
        },
        wire::ModelUsage {
            input_tokens: 12,
            output_tokens: 65,
            ..Default::default()
        },
        wire::ModelUsage {
            input_tokens: 12,
            output_tokens: 8,
            cached_input_tokens: 10,
            cache_creation_input_tokens: 3,
            ..Default::default()
        },
        wire::ModelUsage {
            input_tokens: 12,
            output_tokens: 8,
            reasoning_tokens: 9,
            ..Default::default()
        },
        wire::ModelUsage {
            input_tokens: 12,
            output_tokens: 8,
            charged_cost: Some(wire::Money::default()),
            ..Default::default()
        },
    ] {
        let mut result = baseline.clone();
        result.usage = Some(usage);
        assert!(candidate(&request, &result).is_err());
    }
}

#[test]
fn finish_denied() {
    let request = ast_request();
    let mut result = response(&request, r#"{"node":"field","field":"market.close"}"#);
    for reason in [0, 2, 3, 4, 127] {
        result.finish_reason = reason;
        assert!(candidate(&request, &result).is_err());
    }
}

#[test]
fn document_denied() {
    let request = ast_request();
    let baseline = response(&request, r#"{"node":"field","field":"market.close"}"#);
    let mut result = baseline.clone();
    document(&mut result).schema_id.push_str("-other");
    assert!(candidate(&request, &result).is_err());
    result = baseline.clone();
    document(&mut result).schema_sha256.as_mut().unwrap().value[0] ^= 1;
    assert!(candidate(&request, &result).is_err());
    result = baseline;
    document(&mut result)
        .canonical_sha256
        .as_mut()
        .unwrap()
        .value[0] ^= 1;
    assert!(candidate(&request, &result).is_err());
}

#[test]
fn ast_denied() {
    let request = ast_request();
    for text in [
        r#"{"node":"field","field":"market.close","field":"market.open"}"#,
        r#"{"node":"field","field":"market.close","code":"execute()"}"#,
        r#"{"node":"field","field":"holdout.future"}"#,
        r#"{"node":"decimal","value":"2"}"#,
        r#"{"node":"call","operator":"eval","operator_version":"2","arguments":[]}"#,
    ] {
        assert!(candidate(&request, &response(&request, text)).is_err());
    }
}

#[test]
fn schema_denied() {
    let mut request = ast_request();
    let result = response(&request, r#"{"node":"field","field":"market.close"}"#);
    request
        .invocation
        .as_mut()
        .unwrap()
        .structured_output
        .as_mut()
        .unwrap()
        .json_schema
        .as_mut()
        .unwrap()
        .canonical_json
        .push(b' ');
    assert!(candidate(&request, &result).is_err());
}
