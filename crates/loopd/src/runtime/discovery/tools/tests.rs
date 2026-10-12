use std::fs;

use loop_core::factor::us_equities;
use loop_protocol::wire::v1::DiscoveryJobInput;
use tempfile::TempDir;

use super::*;

struct Fixture {
    directory: TempDir,
    source: LocalArtifacts,
    reference: ObjectRef,
    artifact: ObjectRef,
    job: wire::JobSpecification,
}

impl Fixture {
    fn new(protected: bool) -> Self {
        let directory = tempfile::Builder::new()
            .prefix("loop-description-test-")
            .tempdir()
            .unwrap();
        let put = |bytes: &[u8]| {
            let digest = format!("{:x}", Sha256::digest(bytes));
            fs::write(directory.path().join(&digest), bytes).unwrap();
            ObjectRef {
                sha256: format!("sha256:{digest}"),
                byte_size: bytes.len() as u64,
            }
        };
        let artifact = put(b"session,security,close\n2016-01-04,security.1,100\n");
        let document = put(concat!(
            r#"{"schema":"loop.artifact-schema/v1","name":"loop.synthetic_prices","version":1,"#,
            r#""media_type":"text/csv","columns":["session","security","close"]}"#,
        )
        .as_bytes());
        let role = if protected {
            "first_locked_confirmation"
        } else {
            "in_sample"
        };
        let bytes = format!(
            concat!(
                r#"{{"schema":"loop.development-dataset/v1","sample":{{"role":"{role}","#,
                r#""start":"2016-01-04","end":"2016-01-08"}},"quality":"synthetic","#,
                r#""snapshots":[{{"snapshot_id":"snapshot.description","source":"private-vendor","#,
                r#""dataset":"private-dataset-name","entitlement":"private-license","known_through_ms":1452286800000,"#,
                r#""artifacts":[{{"object":{artifact},"schema":{{"name":"loop.synthetic_prices","#,
                r#""version":1,"document":{document}}},"media_type":"text/csv","created_at_ms":1452286800000}}]}}]}}"#,
            ),
            role = role,
            artifact = serde_json::to_string(&artifact).unwrap(),
            document = serde_json::to_string(&document).unwrap(),
        );
        let reference = put(bytes.as_bytes());
        let job = wire::JobSpecification {
            input: Some(wire::job_specification::Input::Discovery(
                DiscoveryJobInput {
                    dataset: Some(wire::DevelopmentDatasetReference {
                        snapshot_ids: vec![wire::SnapshotId {
                            value: "snapshot.description".into(),
                        }],
                        manifest_sha256: Some(wire::Sha256Digest {
                            value: reference.digest().unwrap().to_vec(),
                        }),
                    }),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        let source = LocalArtifacts::open(directory.path()).unwrap();
        Self {
            directory,
            source,
            reference,
            artifact,
            job,
        }
    }

    async fn describe(&self, call: &wire::ToolCallContent) -> StoreResult<wire::ToolResultContent> {
        describe(
            &self.source,
            &self.reference,
            &self.job,
            &us_equities::registry().unwrap(),
            call,
        )
        .await
    }
}

fn call() -> wire::ToolCallContent {
    let schema = crate::runtime::model_codec::describe_tool()
        .unwrap()
        .input_schema
        .unwrap();
    wire::ToolCallContent {
        tool_call_id: "call-description".into(),
        tool_name: "research_describe".into(),
        arguments: Some(wire::JsonDocument {
            utf8_json: b"{}".to_vec(),
            canonical_sha256: Some(wire::Sha256Digest {
                value: Sha256::digest(b"{}").to_vec(),
            }),
            schema_id: schema.schema_id,
            schema_sha256: schema.schema_sha256,
        }),
    }
}

#[test]
fn schema_document() {
    let document: Value = serde_json::from_str(include_str!(
        "../../../../../../config/schemas/research-description.v1.json"
    ))
    .unwrap();
    assert_eq!(
        serde_json::to_vec(&document).unwrap(),
        result_schema().unwrap().canonical_json
    );
}

#[tokio::test]
async fn verified_description() {
    let fixture = Fixture::new(false);
    let result = fixture.describe(&call()).await.unwrap();
    let Some(wire::tool_result_content::Result::Json(document)) = &result.result else {
        panic!("expected description")
    };
    let value: Value = serde_json::from_slice(&document.utf8_json).unwrap();
    assert_eq!(value["schema"], RESULT_SCHEMA);
    assert_eq!(value["dataset_id"], fixture.reference.sha256);
    assert_eq!(value["sample"]["role"], "in_sample");
    assert_eq!(
        value["artifacts"][0]["schema"]["name"],
        "loop.synthetic_prices"
    );
    assert!(
        value["fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field["name"] == "market.close")
    );
    let text = String::from_utf8(document.utf8_json.clone()).unwrap();
    for denied in [
        "private-vendor",
        "private-license",
        "private-dataset-name",
        "artifact://",
        "security.1",
        fixture.directory.path().to_str().unwrap(),
    ] {
        assert!(!text.contains(denied));
    }
    assert_eq!(result, fixture.describe(&call()).await.unwrap());
}

#[tokio::test]
async fn protected_denied() {
    assert!(matches!(
        Fixture::new(true).describe(&call()).await,
        Err(StoreError::AdmissionDenied)
    ));
}

#[tokio::test]
async fn changed_artifact() {
    let fixture = Fixture::new(false);
    fixture.describe(&call()).await.unwrap();
    let path = fixture.directory.path().join(&fixture.artifact.sha256[7..]);
    let mut bytes = fs::read(&path).unwrap();
    bytes[0] ^= 1;
    fs::write(path, bytes).unwrap();
    assert!(fixture.describe(&call()).await.is_err());
}

#[tokio::test]
async fn mismatched_dataset() {
    let mut fixture = Fixture::new(false);
    fixture.job.input = Some(wire::job_specification::Input::Discovery(
        DiscoveryJobInput::default(),
    ));
    assert!(fixture.describe(&call()).await.is_err());
}

#[tokio::test]
async fn unknown_tool() {
    let mut call = call();
    call.tool_name = "shell".into();
    assert!(Fixture::new(false).describe(&call).await.is_err());
}

#[tokio::test]
async fn arguments_denied() {
    let mut call = call();
    let arguments = call.arguments.as_mut().unwrap();
    arguments.utf8_json = br#"{"path":"/tmp"}"#.to_vec();
    arguments.canonical_sha256 = Some(wire::Sha256Digest {
        value: Sha256::digest(&arguments.utf8_json).to_vec(),
    });
    assert!(Fixture::new(false).describe(&call).await.is_err());
}

#[tokio::test]
async fn result_tampering() {
    let mut result = Fixture::new(false).describe(&call()).await.unwrap();
    let Some(wire::tool_result_content::Result::Json(document)) = &mut result.result else {
        panic!("expected description")
    };
    let mut value: Value = serde_json::from_slice(&document.utf8_json).unwrap();
    value["path"] = "/tmp/private".into();
    document.utf8_json = serde_json::to_vec(&value).unwrap();
    document.canonical_sha256 = Some(wire::Sha256Digest {
        value: Sha256::digest(&document.utf8_json).to_vec(),
    });
    assert!(validate_result(&result).is_err());
}

#[tokio::test]
async fn result_oversize() {
    let mut result = Fixture::new(false).describe(&call()).await.unwrap();
    let Some(wire::tool_result_content::Result::Json(document)) = &mut result.result else {
        panic!("expected description")
    };
    document.utf8_json = vec![b' '; RESULT_LIMIT + 1];
    document.canonical_sha256 = Some(wire::Sha256Digest {
        value: Sha256::digest(&document.utf8_json).to_vec(),
    });
    assert!(validate_result(&result).is_err());
}
