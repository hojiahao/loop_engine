#![cfg(feature = "discovery-service")]

use std::collections::{BTreeMap, BTreeSet};

use loop_protocol::wire::discovery::v1::{
    DiscoveryJobHandle, DiscoveryJobStatus, StartDiscoveryResponse,
};
use loop_protocol::wire::v1::JobId;
use prost::Message;
use prost_types::{FileDescriptorSet, Timestamp};

const DISCOVERY_FILE: &str = "loop/discovery/v1/service.proto";

#[test]
fn discovery_response_round_trip_exposes_only_the_safe_job_projection() {
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
fn generated_discovery_surface_does_not_reference_sensitive_job_types() {
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
fn discovery_dependency_closure_uses_only_the_development_data_leaf() {
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
