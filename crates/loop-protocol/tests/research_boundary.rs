#![cfg(feature = "research-service")]

use std::collections::{BTreeMap, BTreeSet};

use loop_protocol::wire::research::v1::{
    EnqueueBacktestResponse, ResearchJobHandle, ResearchJobStatus,
};
use loop_protocol::wire::v1::JobId;
use prost::Message;
use prost_types::{DescriptorProto, FileDescriptorProto, FileDescriptorSet};

const BOUNDARY_VECTORS: &str =
    include_str!("../../../tests/contracts/research_boundary_vectors.tsv");
const RESEARCH_FILE: &str = "loop/research/v1/service.proto";

#[test]
fn research_response_round_trip_exposes_only_the_safe_job_projection() {
    let response = EnqueueBacktestResponse {
        job: Some(ResearchJobHandle {
            job_id: Some(JobId {
                value: "job.research.0001".to_owned(),
            }),
            status: ResearchJobStatus::Running.into(),
            revision: 7,
            submitted_at: None,
            updated_at: None,
        }),
    };

    let decoded = EnqueueBacktestResponse::decode(response.encode_to_vec().as_slice())
        .expect("safe research response must round-trip");
    let handle = decoded.job.expect("job handle is required");
    assert_eq!(
        handle.job_id.as_ref().expect("job ID is required").value,
        "job.research.0001"
    );
    assert_eq!(handle.status(), ResearchJobStatus::Running);
    assert_eq!(handle.revision, 7);
}

#[test]
fn research_dependency_closure_excludes_internal_and_locked_contracts() {
    let descriptor = descriptor();
    let files = descriptor
        .file
        .iter()
        .map(|file| (file.name.as_deref().expect("file name"), file))
        .collect::<BTreeMap<_, _>>();
    let mut pending = vec![RESEARCH_FILE];
    let mut visited = BTreeSet::new();
    while let Some(name) = pending.pop() {
        if !visited.insert(name) {
            continue;
        }
        pending.extend(files[name].dependency.iter().map(String::as_str));
    }

    let expected = [
        "google/protobuf/duration.proto",
        "google/protobuf/timestamp.proto",
        "loop/research/v1/service.proto",
        "loop/v1/common.proto",
        "loop/v1/development_data.proto",
        "loop/v1/factor.proto",
        "loop/v1/research_common.proto",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    assert_eq!(visited, expected);
    assert!(!visited.contains("loop/v1/data.proto"));
}

#[test]
fn research_shared_surface_vectors_fail_closed() {
    let descriptor = descriptor();
    for line in BOUNDARY_VECTORS.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let columns = line.split('\t').collect::<Vec<_>>();
        assert_eq!(columns.len(), 4, "invalid shared research vector");
        let present = research_message(&descriptor, columns[1])
            .field
            .iter()
            .any(|field| field.name.as_deref() == Some(columns[2]));
        assert_eq!(
            present,
            columns[3] == "accept",
            "shared research vector {}",
            columns[0]
        );
    }
}

#[test]
fn generated_research_surface_does_not_reference_sensitive_job_types() {
    let generated = include_str!("../src/generated/r#loop.research.v1.rs");
    let code = generated
        .lines()
        .filter(|line| !line.trim_start().starts_with("///"))
        .collect::<Vec<_>>()
        .join("\n");
    for forbidden in [
        "BacktestSpec",
        "HoldoutBacktestJobInput",
        "HoldoutGrantReference",
        "JobLease",
        "JobOutcome",
        "JobRecord",
        "JobSpecification",
        "SampleRole",
        "SampleWindow",
    ] {
        assert!(
            !code.contains(forbidden),
            "found forbidden type {forbidden}"
        );
    }
}

fn descriptor() -> FileDescriptorSet {
    FileDescriptorSet::decode(loop_protocol::FILE_DESCRIPTOR_SET)
        .expect("committed descriptor must decode")
}

fn research_message<'a>(descriptor: &'a FileDescriptorSet, surface: &str) -> &'a DescriptorProto {
    let name = match surface {
        "backtest_input" => "BacktestInput",
        "factor_input" => "FactorEvaluationInput",
        "job_handle" => "ResearchJobHandle",
        "reconciliation_input" => "ReconciliationInput",
        _ => panic!("unknown research surface: {surface}"),
    };
    file(descriptor, RESEARCH_FILE)
        .message_type
        .iter()
        .find(|message| message.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("missing research message {name}"))
}

fn file<'a>(descriptor: &'a FileDescriptorSet, name: &str) -> &'a FileDescriptorProto {
    descriptor
        .file
        .iter()
        .find(|file| file.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("missing descriptor file {name}"))
}
