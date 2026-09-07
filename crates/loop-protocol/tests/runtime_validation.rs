use std::{fs, path::PathBuf};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use loop_protocol::{
    runtime_validation::{
        RichStatusDetail, RuntimeValidationCode, SERVICE_ERROR_TYPE_URL,
        validate_job_wire_dispatch_candidate, validate_operational_failure,
    },
    wire::v1::{
        ArtifactJobInput, DiscoveryJobInput, ErrorCategory, ErrorDetail, JobKind, JobSpecification,
        ServiceError, job_specification,
    },
};
use prost::Message;
use serde_json::Value;

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn protocol_fixture(name: &str) -> Vec<u8> {
    fs::read(
        repository_root()
            .join("fixtures/contracts/protocol/v1")
            .join(name),
    )
    .expect("committed semantic fixture must be readable")
}

struct OperationalFixture {
    status: i32,
    body: Vec<u8>,
    type_url: String,
    detail: Vec<u8>,
    expected_code: String,
    expected_message: String,
    expected_retryable: bool,
}

fn operational_fixture() -> OperationalFixture {
    let document: Value = serde_json::from_slice(
        &fs::read(repository_root().join("tests/contracts/operational_failure.json"))
            .expect("operational fixture must be readable"),
    )
    .expect("operational fixture must be valid JSON");
    let detail = &document["details"][0];
    OperationalFixture {
        status: document["grpc_status_code"]
            .as_i64()
            .expect("fixture status must be an integer") as i32,
        body: STANDARD
            .decode(
                document["response_body_base64"]
                    .as_str()
                    .expect("fixture body must be base64"),
            )
            .expect("fixture body base64 must decode"),
        type_url: detail["type_url"]
            .as_str()
            .expect("fixture detail URL must be a string")
            .to_owned(),
        detail: STANDARD
            .decode(
                detail["value_base64"]
                    .as_str()
                    .expect("fixture detail must be base64"),
            )
            .expect("fixture detail base64 must decode"),
        expected_code: document["expected"]["code"]
            .as_str()
            .expect("expected code must be a string")
            .to_owned(),
        expected_message: document["expected"]["message"]
            .as_str()
            .expect("expected message must be a string")
            .to_owned(),
        expected_retryable: document["expected"]["retryable"]
            .as_bool()
            .expect("expected retryable must be a boolean"),
    }
}

#[test]
fn validates_shared_operational_failure_fixture() {
    let fixture = operational_fixture();
    let detail = RichStatusDetail {
        type_url: &fixture.type_url,
        value: &fixture.detail,
    };
    let validated = validate_operational_failure(fixture.status, &fixture.body, &[detail])
        .expect("valid ServiceError rich status must be accepted");

    assert_eq!(validated.category(), ErrorCategory::Dependency);
    assert_eq!(validated.code, fixture.expected_code);
    assert_eq!(validated.message, fixture.expected_message);
    assert_eq!(validated.retryable, fixture.expected_retryable);
}

#[test]
fn operational_failure_type_and_transport_checks_fail_closed() {
    let fixture = operational_fixture();
    for forged_url in [
        "type.googleapis.com/loop.v1.FactorRejection",
        "type.googleapis.com/vendor.FutureError",
    ] {
        let error = validate_operational_failure(
            fixture.status,
            &fixture.body,
            &[RichStatusDetail {
                type_url: forged_url,
                value: &fixture.detail,
            }],
        )
        .expect_err("non-ServiceError type URL must fail closed");
        assert_eq!(error.code, RuntimeValidationCode::UnexpectedDetail);
    }

    let valid_detail = RichStatusDetail {
        type_url: SERVICE_ERROR_TYPE_URL,
        value: &fixture.detail,
    };
    assert_eq!(
        validate_operational_failure(0, &fixture.body, &[valid_detail])
            .expect_err("OK status must not carry an operational failure")
            .code,
        RuntimeValidationCode::InvalidStatus
    );
    assert_eq!(
        validate_operational_failure(fixture.status, b"response", &[valid_detail])
            .expect_err("non-OK response body must be empty")
            .code,
        RuntimeValidationCode::UnexpectedResponseBody
    );
}

#[test]
fn operational_failure_rejects_unbounded_or_malformed_service_error_fields() {
    let fixture = operational_fixture();
    let baseline = ServiceError::decode(fixture.detail.as_slice())
        .expect("shared fixture must contain a ServiceError");

    let mut valid_detail = baseline.clone();
    valid_detail.details.push(ErrorDetail {
        field_path: "$.artifact.sha256".to_owned(),
        code: "digest_mismatch".to_owned(),
        message: "declared and computed digests differ".to_owned(),
    });
    assert_service_error_result(valid_detail).expect("one bounded detail must be accepted");

    let mut unknown_category = baseline.clone();
    unknown_category.category = 999;
    assert_eq!(
        assert_service_error_result(unknown_category)
            .expect_err("unknown error categories must fail closed"),
        RuntimeValidationCode::UnsupportedEnum
    );

    let mut invalid = Vec::new();
    let mut value = baseline.clone();
    value.code = "Invalid Code".to_owned();
    invalid.push(value);
    let mut value = baseline.clone();
    value.code = "a".repeat(129);
    invalid.push(value);
    let mut value = baseline.clone();
    value.message = "é".repeat(1_025);
    invalid.push(value);
    let mut value = baseline.clone();
    value.message = "forged\nrecord".to_owned();
    invalid.push(value);
    for detail in [
        ErrorDetail::default(),
        ErrorDetail {
            field_path: "$.field\tname".to_owned(),
            code: "invalid_field".to_owned(),
            message: "invalid field".to_owned(),
        },
        ErrorDetail {
            field_path: "x".repeat(513),
            code: "invalid_field".to_owned(),
            message: "invalid field".to_owned(),
        },
        ErrorDetail {
            field_path: "$.field".to_owned(),
            code: "INVALID".to_owned(),
            message: "invalid code".to_owned(),
        },
        ErrorDetail {
            field_path: "$.field".to_owned(),
            code: "invalid_field".to_owned(),
            message: " ".to_owned(),
        },
    ] {
        let mut value = baseline.clone();
        value.details.push(detail);
        invalid.push(value);
    }

    for value in invalid {
        assert_eq!(
            assert_service_error_result(value)
                .expect_err("malformed ServiceError fields must fail closed"),
            RuntimeValidationCode::InvalidServiceError
        );
    }
}

fn assert_service_error_result(service_error: ServiceError) -> Result<(), RuntimeValidationCode> {
    let encoded = service_error.encode_to_vec();
    validate_operational_failure(
        14,
        b"",
        &[RichStatusDetail {
            type_url: SERVICE_ERROR_TYPE_URL,
            value: &encoded,
        }],
    )
    .map(|_| ())
    .map_err(|error| error.code)
}

#[test]
fn unknown_job_enum_and_oneof_fixtures_fail_closed() {
    let unknown_enum = JobSpecification::decode(
        protocol_fixture("job_specification_v1_unknown_enum.binpb").as_slice(),
    )
    .expect("unknown enum fixture must decode at the DTO layer");
    assert_eq!(unknown_enum.kind, 127);
    assert!(matches!(
        unknown_enum.input,
        Some(job_specification::Input::Discovery(_))
    ));
    assert_eq!(
        validate_job_wire_dispatch_candidate(&unknown_enum, &[JobKind::Discovery])
            .expect_err("unknown job kind must not select a default action")
            .code,
        RuntimeValidationCode::UnsupportedEnum
    );

    let unknown_oneof = JobSpecification::decode(
        protocol_fixture("job_specification_v1_unknown_oneof.binpb").as_slice(),
    )
    .expect("unknown oneof fixture must decode at the DTO layer");
    assert_eq!(unknown_oneof.kind(), JobKind::Report);
    assert!(unknown_oneof.input.is_none());
    assert_eq!(
        validate_job_wire_dispatch_candidate(&unknown_oneof, &[JobKind::Report])
            .expect_err("unknown oneof must not select a default action")
            .code,
        RuntimeValidationCode::UnsupportedOneof
    );
}

#[test]
fn known_job_dispatch_requires_a_complete_envelope_and_matching_input() {
    let valid = JobSpecification {
        kind: JobKind::Discovery.into(),
        input: Some(job_specification::Input::Discovery(
            DiscoveryJobInput::default(),
        )),
        ..Default::default()
    };
    assert_eq!(
        validate_job_wire_dispatch_candidate(&valid, &[JobKind::Discovery])
            .expect_err("a matching but incomplete envelope must fail closed")
            .code,
        RuntimeValidationCode::InvalidSpecification
    );

    let mismatched = JobSpecification {
        kind: JobKind::Discovery.into(),
        input: Some(job_specification::Input::Artifact(
            ArtifactJobInput::default(),
        )),
        ..Default::default()
    };
    assert_eq!(
        validate_job_wire_dispatch_candidate(&mismatched, &[JobKind::Discovery])
            .expect_err("known but incompatible discriminants must be rejected")
            .code,
        RuntimeValidationCode::IncompatibleVariant
    );

    let disabled = JobSpecification {
        kind: JobKind::ProspectiveObservation.into(),
        input: Some(job_specification::Input::Artifact(
            ArtifactJobInput::default(),
        )),
        ..Default::default()
    };
    assert_eq!(
        validate_job_wire_dispatch_candidate(&disabled, &[JobKind::Discovery])
            .expect_err("a known shape without an enabled handler must be rejected")
            .code,
        RuntimeValidationCode::InvalidSpecification
    );
}
