use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};

use serde_json::{Value, json};
use tempfile::TempDir;

use super::*;
use crate::runtime::model_codec::ast_schema;
use crate::test_support;

struct Fixture {
    directory: TempDir,
    document: Value,
    input: DiscoveryJobInput,
    invocation: wire::ModelInvocation,
    tool_invocation: Option<wire::ModelInvocation>,
    protocol: wire::ProtocolSelectionSnapshot,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let (_, wire::job_specification::Input::Discovery(common)) =
            test_support::research::inputs().remove(0)
        else {
            unreachable!()
        };
        let mut input = DiscoveryJobInput::decode(common.encode_to_vec().as_slice()).unwrap();
        let request_policy = input.research_policy.clone();
        input.research_policy = None;
        input.maximum_candidates = 1;
        let budget = input.budget.as_mut().unwrap();
        budget.maximum_steps = 1;
        budget.maximum_wall_time = Some(prost_types::Duration {
            seconds: 120,
            nanos: 0,
        });
        input
            .maker_model
            .as_mut()
            .unwrap()
            .capabilities
            .as_mut()
            .unwrap()
            .supports_structured_output = true;
        let data = ObjectRef {
            sha256: format!("sha256:{}", "01".repeat(32)),
            byte_size: 100,
        };
        input.dataset.as_mut().unwrap().manifest_sha256 = Some(wire::Sha256Digest {
            value: data.digest().unwrap().to_vec(),
        });
        let invocation = wire::ModelInvocation {
            request_policy,
            model: input.maker_model.clone(),
            messages: vec![wire::ModelMessage {
                role: wire::ModelRole::User as i32,
                content: vec![wire::ContentBlock {
                    content: Some(wire::content_block::Content::Text(wire::TextContent {
                        text: "Return one AST candidate.".to_owned(),
                    })),
                }],
            }],
            structured_output: Some(wire::StructuredOutputDefinition {
                name: "factor_candidate".to_owned(),
                json_schema: Some(ast_schema().unwrap()),
                strict: Some(true),
                ..Default::default()
            }),
            budget: Some(wire::InvocationBudget {
                maximum_input_tokens: budget.maximum_input_tokens,
                maximum_output_tokens: budget.maximum_output_tokens,
                maximum_cost: budget.maximum_cost.clone(),
                maximum_wall_time: Some(prost_types::Duration {
                    seconds: 30,
                    nanos: 0,
                }),
            }),
            ..Default::default()
        };
        let mut protocol = test_support::command(1)
            .specification
            .protocol_selection
            .unwrap();
        protocol.enabled_features = FEATURES
            .iter()
            .map(|feature| (*feature).to_owned())
            .collect();
        protocol.schema_descriptor_sha256 = Some(wire::Sha256Digest {
            value: Sha256::digest(loop_protocol::FILE_DESCRIPTOR_SET).to_vec(),
        });
        protocol.selection_sha256 = Some(wire::Sha256Digest {
            value: protocol_selection_sha256(&protocol).unwrap().to_vec(),
        });
        let registry = put(
            directory.path(),
            &us_equities::registry().unwrap().canonical_bytes(),
        );
        let document = json!({
            "schema":"loop.discovery-plan/v1", "id":"plan.fixture", "revision":"1",
            "actor_id":"agent.fixture", "run_id":"run.fixture",
            "provider_sha256":format!("sha256:{}", "02".repeat(32)),
            "data":data, "registry":registry,
        });
        Self {
            directory,
            document,
            input,
            invocation,
            tool_invocation: None,
            protocol,
        }
    }

    fn reference(&mut self) -> ObjectRef {
        self.document["input"] = json!(put(self.directory.path(), &self.input.encode_to_vec()));
        self.document["invocation"] =
            json!(put(self.directory.path(), &self.invocation.encode_to_vec()));
        if let Some(invocation) = &self.tool_invocation {
            self.document["tool_invocation"] =
                json!(put(self.directory.path(), &invocation.encode_to_vec()));
        }
        self.document["protocol"] =
            json!(put(self.directory.path(), &self.protocol.encode_to_vec()));
        put(
            self.directory.path(),
            &serde_json::to_vec(&self.document).unwrap(),
        )
    }

    fn load(&mut self) -> StoreResult<FrozenPlan> {
        let reference = self.reference();
        FrozenPlan::load(self.directory.path(), &reference)
    }

    fn controlled() -> Self {
        let mut fixture = Self::new();
        fixture.document["schema"] = "loop.discovery-plan/v2".into();
        let model = fixture.input.maker_model.as_mut().unwrap();
        model.capabilities.as_mut().unwrap().supports_tools = true;
        fixture.invocation.model = Some(model.clone());
        fixture.invocation.tools = vec![model_codec::describe_tool().unwrap()];
        fixture.invocation.tool_choice = Some(wire::ToolChoice {
            mode: wire::ToolChoiceMode::None as i32,
            named_tool: String::new(),
        });
        let money = |amount: &str| wire::Money {
            currency_code: "USD".into(),
            amount: Some(wire::ExactDecimal {
                value: amount.into(),
            }),
        };
        fixture.invocation.budget.as_mut().unwrap().maximum_cost = Some(money("1"));
        let budget = fixture.input.budget.as_mut().unwrap();
        budget.maximum_steps = 3;
        budget.maximum_input_tokens *= 2;
        budget.maximum_output_tokens *= 2;
        budget.maximum_cost = Some(money("2"));
        let mut first = fixture.invocation.clone();
        first.structured_output = None;
        first.tool_choice.as_mut().unwrap().mode = wire::ToolChoiceMode::Required as i32;
        fixture.tool_invocation = Some(first);
        fixture
            .protocol
            .enabled_features
            .push("discovery.tool-context.v1".into());
        fixture.protocol.enabled_features.sort();
        fixture.protocol.selection_sha256 = Some(wire::Sha256Digest {
            value: protocol_selection_sha256(&fixture.protocol)
                .unwrap()
                .to_vec(),
        });
        fixture
    }
}

#[test]
fn controlled_plan() {
    let plan = Fixture::controlled().load().unwrap();
    assert!(plan.tool_invocation.is_some());
    assert_eq!(plan.input.budget.unwrap().maximum_steps, 3);
}

#[test]
fn template_messages() {
    let mut fixture = Fixture::controlled();
    fixture.tool_invocation.as_mut().unwrap().messages[0].content = vec![];
    assert!(fixture.load().is_err());
}

#[test]
fn history_capacity() {
    let mut fixture = Fixture::controlled();
    let message = fixture.invocation.messages[0].clone();
    fixture.invocation.messages = vec![message; 31];
    fixture.tool_invocation.as_mut().unwrap().messages = fixture.invocation.messages.clone();
    assert!(fixture.load().is_err());
}

#[test]
fn history_bytes() {
    let mut fixture = Fixture::controlled();
    let Some(wire::content_block::Content::Text(text)) =
        &mut fixture.invocation.messages[0].content[0].content
    else {
        panic!("text fixture")
    };
    text.text = "x".repeat(112_641);
    fixture.tool_invocation.as_mut().unwrap().messages = fixture.invocation.messages.clone();
    assert!(fixture.load().is_err());
}

#[test]
fn cumulative_input() {
    let mut fixture = Fixture::controlled();
    fixture.input.budget.as_mut().unwrap().maximum_input_tokens -= 1;
    assert!(fixture.load().is_err());
}

#[test]
fn cumulative_output() {
    let mut fixture = Fixture::controlled();
    fixture.input.budget.as_mut().unwrap().maximum_output_tokens -= 1;
    assert!(fixture.load().is_err());
}

#[test]
fn cumulative_cost() {
    let mut fixture = Fixture::controlled();
    fixture
        .input
        .budget
        .as_mut()
        .unwrap()
        .maximum_cost
        .as_mut()
        .unwrap()
        .amount
        .as_mut()
        .unwrap()
        .value = "1.9".into();
    assert!(fixture.load().is_err());
}

#[test]
fn cumulative_wall() {
    let mut fixture = Fixture::controlled();
    fixture
        .input
        .budget
        .as_mut()
        .unwrap()
        .maximum_wall_time
        .as_mut()
        .unwrap()
        .seconds = 90;
    assert!(fixture.load().is_err());
}

#[test]
fn tools_capability() {
    let mut fixture = Fixture::controlled();
    fixture
        .input
        .maker_model
        .as_mut()
        .unwrap()
        .capabilities
        .as_mut()
        .unwrap()
        .supports_tools = false;
    fixture.invocation.model = fixture.input.maker_model.clone();
    fixture.tool_invocation.as_mut().unwrap().model = fixture.input.maker_model.clone();
    assert!(fixture.load().is_err());
}

#[test]
fn legacy_tools() {
    let mut fixture = Fixture::controlled();
    fixture.document["schema"] = "loop.discovery-plan/v1".into();
    assert!(fixture.load().is_err());
}

fn put(root: &Path, bytes: &[u8]) -> ObjectRef {
    let digest = format!("{:x}", Sha256::digest(bytes));
    let path = root.join(&digest);
    fs::write(&path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    ObjectRef {
        sha256: format!("sha256:{digest}"),
        byte_size: bytes.len() as u64,
    }
}

fn document_ref(fixture: &Fixture) -> ObjectRef {
    put(
        fixture.directory.path(),
        &serde_json::to_vec(&fixture.document).unwrap(),
    )
}

#[test]
fn policy_binding() {
    let mut fixture = Fixture::new();
    let reference = fixture.reference();
    let plan = FrozenPlan::load(fixture.directory.path(), &reference).unwrap();
    let policy = plan.input.research_policy.as_ref().unwrap();
    assert_eq!(
        policy.sha256.as_ref().unwrap().value,
        reference.digest().unwrap()
    );
    assert_eq!(
        plan.invocation.request_policy,
        fixture.invocation.request_policy
    );
    assert_ne!(plan.invocation.request_policy.as_ref(), Some(policy));
    assert_eq!(plan.input.maximum_candidates, 1);
    assert!(plan.invocation.request_id.is_none());
    plan.check().unwrap();
}

#[test]
fn input_matching() {
    let mut fixture = Fixture::new();
    let plan = fixture.load().unwrap();
    let mut job = test_support::command(1).specification;
    job.kind = wire::JobKind::Discovery as i32;
    job.input = Some(wire::job_specification::Input::Discovery(plan.job_input()));
    job.protocol_selection = Some(plan.protocol.clone());
    job.submitted_by
        .as_mut()
        .unwrap()
        .actor_id
        .as_mut()
        .unwrap()
        .value = plan.actor_id.clone();
    plan.matches(&job).unwrap();
    let Some(wire::job_specification::Input::Discovery(input)) = job.input.as_mut() else {
        unreachable!()
    };
    input.maximum_candidates = 2;
    assert!(matches!(
        plan.matches(&job),
        Err(StoreError::AdmissionDenied)
    ));
}

#[test]
fn immutable_objects() {
    let mut fixture = Fixture::new();
    let plan = fixture.load().unwrap();
    let path = &plan.files[2].0;
    let mut bytes = fs::read(path).unwrap();
    bytes[0] ^= 1;
    fs::write(path, bytes).unwrap();
    assert!(matches!(plan.check(), Err(StoreError::Corrupt(_))));
}

#[test]
fn missing_object() {
    let mut fixture = Fixture::new();
    let plan = fixture.load().unwrap();
    fs::remove_file(&plan.files[2].0).unwrap();
    assert!(plan.check().is_err());
}

#[test]
fn private_permissions() {
    let mut fixture = Fixture::new();
    let reference = fixture.reference();
    fs::set_permissions(
        fixture.directory.path().join(&reference.sha256[7..]),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert!(FrozenPlan::load(fixture.directory.path(), &reference).is_err());
}

#[test]
fn symlink_object() {
    let mut fixture = Fixture::new();
    let reference = fixture.reference();
    let original = fixture.directory.path().join(&reference.sha256[7..]);
    let moved = fixture.directory.path().join("original");
    fs::rename(&original, &moved).unwrap();
    symlink(&moved, &original).unwrap();
    assert!(FrozenPlan::load(fixture.directory.path(), &reference).is_err());
}

#[test]
fn unknown_fields() {
    let mut fixture = Fixture::new();
    fixture.document["extra"] = true.into();
    assert!(fixture.load().is_err());
}

#[test]
fn duplicate_fields() {
    let mut fixture = Fixture::new();
    fixture.reference();
    let mut bytes = serde_json::to_vec(&fixture.document).unwrap();
    bytes.pop();
    bytes.extend_from_slice(b",\"revision\":\"1\"}");
    let reference = put(fixture.directory.path(), &bytes);
    assert!(FrozenPlan::load(fixture.directory.path(), &reference).is_err());
}

#[test]
fn unknown_protobuf() {
    let mut fixture = Fixture::new();
    fixture.reference();
    let mut bytes = fixture.input.encode_to_vec();
    bytes.extend_from_slice(&[0xf8, 0x07, 1]);
    fixture.document["input"] = json!(put(fixture.directory.path(), &bytes));
    assert!(FrozenPlan::load(fixture.directory.path(), &document_ref(&fixture)).is_err());
}

#[test]
fn step_limit() {
    let mut fixture = Fixture::new();
    fixture.input.budget.as_mut().unwrap().maximum_steps = 2;
    assert!(fixture.load().is_err());
}

#[test]
fn candidate_limit() {
    let mut fixture = Fixture::new();
    fixture.input.maximum_candidates = 2;
    assert!(fixture.load().is_err());
}

#[test]
fn token_ceiling() {
    let mut fixture = Fixture::new();
    fixture
        .invocation
        .budget
        .as_mut()
        .unwrap()
        .maximum_input_tokens += 1;
    assert!(fixture.load().is_err());
}

#[test]
fn cost_ceiling() {
    let mut fixture = Fixture::new();
    fixture
        .invocation
        .budget
        .as_mut()
        .unwrap()
        .maximum_cost
        .as_mut()
        .unwrap()
        .amount
        .as_mut()
        .unwrap()
        .value = "99999".to_owned();
    assert!(fixture.load().is_err());
}

#[test]
fn wall_ceiling() {
    let mut fixture = Fixture::new();
    fixture
        .invocation
        .budget
        .as_mut()
        .unwrap()
        .maximum_wall_time
        .as_mut()
        .unwrap()
        .seconds = 115;
    assert!(fixture.load().is_err());
}

#[test]
fn schema_document() {
    let document: Value = serde_json::from_slice(include_bytes!(
        "../../../../../../config/schemas/discovery-ast.v1.json"
    ))
    .unwrap();
    assert_eq!(
        serde_json::to_vec(&document).unwrap(),
        ast_schema().unwrap().canonical_json
    );
}

#[test]
fn model_binding() {
    let mut fixture = Fixture::new();
    fixture
        .invocation
        .model
        .as_mut()
        .unwrap()
        .model_id
        .as_mut()
        .unwrap()
        .value = "other".to_owned();
    assert!(fixture.load().is_err());
}

#[test]
fn data_binding() {
    let mut fixture = Fixture::new();
    fixture
        .input
        .dataset
        .as_mut()
        .unwrap()
        .manifest_sha256
        .as_mut()
        .unwrap()
        .value[0] ^= 1;
    assert!(fixture.load().is_err());
}

#[test]
fn template_policy() {
    let mut fixture = Fixture::new();
    fixture.input.research_policy = Some(Default::default());
    assert!(fixture.load().is_err());
}

#[test]
fn provider_policy() {
    let mut fixture = Fixture::new();
    fixture.invocation.request_policy = None;
    assert!(fixture.load().is_err());
}

#[test]
fn registry_binding() {
    let mut fixture = Fixture::new();
    fixture.document["registry"] = json!(put(fixture.directory.path(), b"{}"));
    assert!(fixture.load().is_err());
}

#[test]
fn protocol_digest() {
    let mut fixture = Fixture::new();
    fixture.protocol.selection_sha256.as_mut().unwrap().value[0] ^= 1;
    assert!(fixture.load().is_err());
}

#[test]
fn descriptor_pin() {
    let mut fixture = Fixture::new();
    fixture
        .protocol
        .schema_descriptor_sha256
        .as_mut()
        .unwrap()
        .value[0] ^= 1;
    fixture.protocol.selection_sha256 = Some(wire::Sha256Digest {
        value: protocol_selection_sha256(&fixture.protocol)
            .unwrap()
            .to_vec(),
    });
    assert!(fixture.load().is_err());
}
