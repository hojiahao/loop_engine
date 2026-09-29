#![cfg(feature = "discovery-service")]

use std::collections::{BTreeMap, BTreeSet};

use loop_protocol::wire::discovery::v1::{
    DiscoveryJobHandle, DiscoveryJobStatus, DiscoveryStepState, DiscoveryStepView,
    ExecuteDiscoveryRequest, ExecuteDiscoveryResponse, StartDiscoveryResponse,
};
use loop_protocol::wire::v1::JobId;
use prost::Message;
use prost_types::{FileDescriptorSet, Timestamp};
use sha2::{Digest, Sha256};

const DISCOVERY_FILE: &str = "loop/discovery/v1/service.proto";

#[test]
fn execute_request() {
    let request = ExecuteDiscoveryRequest::decode(
        include_bytes!("../../../fixtures/contracts/protocol/v1/discovery_execute_v1.binpb")
            .as_slice(),
    )
    .expect("shared execute request fixture");
    assert_eq!(request.job_id.as_ref().unwrap().value, "discovery.1");
    assert_eq!(request.expected_revision, 3);
    assert_eq!(
        request
            .context
            .as_ref()
            .unwrap()
            .request_id
            .as_ref()
            .unwrap()
            .value,
        "execute.1"
    );
    assert_eq!(
        ExecuteDiscoveryRequest::decode(request.encode_to_vec().as_slice()).unwrap(),
        request
    );
}

#[test]
fn completed_candidate() {
    let response = ExecuteDiscoveryResponse::decode(
        include_bytes!("../../../fixtures/contracts/protocol/v1/discovery_completed_v1.binpb")
            .as_slice(),
    )
    .expect("shared completed step fixture");
    let step = response.step.as_ref().unwrap();
    assert_eq!(step.state(), DiscoveryStepState::Completed);
    assert_eq!(
        step.job.as_ref().unwrap().status(),
        DiscoveryJobStatus::Succeeded
    );
    assert_eq!(step.job.as_ref().unwrap().revision, 5);
    assert_eq!(step.reserved_input_tokens, 4096);
    assert_eq!(step.reserved_output_tokens, 1024);
    let reserve = step.reserved_cost.as_ref().unwrap();
    assert_eq!(reserve.amount.as_ref().unwrap().value, "0.125");
    assert_eq!(reserve.currency_code, "USD");
    let candidate = step.candidate.as_ref().unwrap();
    assert_eq!(candidate.canonicalization_profile, "loop.factor-ast/v1");
    assert_eq!(
        candidate.canonical_json,
        br#"{"node":"field","field":"market.close"}"#
    );
    let mut identity = Sha256::new();
    identity.update(b"loop.factor-ast/v1\0");
    identity.update(&candidate.canonical_json);
    assert_eq!(
        candidate.expression_id.as_ref().unwrap().value,
        format!("sha256:{:x}", identity.finalize())
    );
    assert_eq!(
        ExecuteDiscoveryResponse::decode(response.encode_to_vec().as_slice()).unwrap(),
        response
    );
}

#[test]
fn unknown_step() {
    let step = DiscoveryStepView::decode([16, 127].as_slice()).unwrap();
    assert_eq!(step.state, 127);
    assert!(DiscoveryStepState::try_from(step.state).is_err());
    assert!(step.candidate.is_none());
    assert!(step.reserved_cost.is_none());
}

#[test]
// Scenario: discovery response round trip exposes only the safe job projection.
fn discovery_response_round() {
    let response = StartDiscoveryResponse {
        job: Some(DiscoveryJobHandle {
            job_id: Some(JobId {
                value: "job.discovery.0001".to_owned(),
            }),
            status: DiscoveryJobStatus::Running.into(),
            revision: 7,
            submitted_at: Some(Timestamp {
                seconds: 1_788_192_000,
                nanos: 0,
            }),
            updated_at: Some(Timestamp {
                seconds: 1_788_192_001,
                nanos: 0,
            }),
        }),
    };

    let decoded = StartDiscoveryResponse::decode(response.encode_to_vec().as_slice())
        .expect("safe discovery response must round-trip");
    let handle = decoded.job.expect("job handle is required");
    assert_eq!(
        handle
            .job_id
            .as_ref()
            .expect("job ID is required")
            .value
            .as_str(),
        "job.discovery.0001"
    );
    assert_eq!(handle.status(), DiscoveryJobStatus::Running);
    assert_eq!(handle.revision, 7);
}

#[test]
// Scenario: generated discovery surface does not reference sensitive job types.
fn generated_discovery_surface() {
    let generated = include_str!("../src/generated/r#loop.discovery.v1.rs");
    for forbidden in [
        "JobRecord",
        "JobSpecification",
        "HoldoutBacktestJobInput",
        "HoldoutGrant",
        "BacktestSpec",
    ] {
        assert!(
            !generated.contains(forbidden),
            "found forbidden type {forbidden}"
        );
    }
}

#[test]
// Scenario: discovery dependency closure uses only the development data leaf.
fn discovery_dependency_closure() {
    let descriptor = FileDescriptorSet::decode(loop_protocol::FILE_DESCRIPTOR_SET)
        .expect("committed descriptor must decode");
    let files = descriptor
        .file
        .iter()
        .map(|file| (file.name.as_deref().expect("file name"), file))
        .collect::<BTreeMap<_, _>>();
    let mut pending = vec![DISCOVERY_FILE];
    let mut visited = BTreeSet::new();
    while let Some(name) = pending.pop() {
        if !visited.insert(name) {
            continue;
        }
        pending.extend(files[name].dependency.iter().map(String::as_str));
    }

    assert_eq!(
        visited,
        [
            "google/protobuf/duration.proto",
            "google/protobuf/timestamp.proto",
            "loop/discovery/v1/service.proto",
            "loop/v1/artifact.proto",
            "loop/v1/common.proto",
            "loop/v1/development_data.proto",
            "loop/v1/model.proto",
        ]
        .into_iter()
        .collect()
    );
    assert!(!visited.contains("loop/v1/data.proto"));
}
