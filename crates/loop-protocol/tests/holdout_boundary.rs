#![cfg(feature = "holdout-service")]

use loop_protocol::wire::holdout::v1::{
    ConsumeGrantAndEnqueueBacktestRequest, ConsumeGrantAndEnqueueBacktestResponse, JobBatchHandle,
};
use loop_protocol::wire::v1::{
    ArtifactId, ArtifactRef, ArtifactSchemaReference, FreezeManifestReference,
    HoldoutBacktestJobInput, HoldoutEvaluationPlanId, HoldoutEvaluationPlanReference,
    HoldoutGrantId, HoldoutGrantReference, HoldoutPeriodId, JobBatchId, JobId, Sha256Digest,
};
use prost::Message;
use prost_types::{DescriptorProto, FileDescriptorProto, FileDescriptorSet, Timestamp};

const BOUNDARY_VECTORS: &str =
    include_str!("../../../tests/contracts/holdout_boundary_vectors.tsv");

fn digest(byte: u8) -> Sha256Digest {
    Sha256Digest {
        value: vec![byte; 32],
    }
}

fn plan_reference() -> HoldoutEvaluationPlanReference {
    let plan_digest = digest(7);
    HoldoutEvaluationPlanReference {
        holdout_evaluation_plan_id: Some(HoldoutEvaluationPlanId {
            value: format!("sha256:{}", "0a".repeat(32)),
        }),
        canonical_plan: Some(ArtifactRef {
            artifact_id: Some(ArtifactId {
                value: format!("sha256:{}", "07".repeat(32)),
            }),
            uri: format!("artifact://sha256/{}", "07".repeat(32)),
            sha256: Some(plan_digest.clone()),
            schema: Some(ArtifactSchemaReference {
                name: "loop.holdout_evaluation_plan".to_owned(),
                version: 1,
                schema_sha256: Some(digest(8)),
            }),
            media_type: "application/json".to_owned(),
            byte_size: 512,
            row_count: None,
            created_at: Some(Timestamp {
                seconds: 1,
                nanos: 0,
            }),
            manifest_sha256: None,
        }),
        plan_sha256: Some(plan_digest),
        entry_count: 2,
        holdout_period_id: Some(HoldoutPeriodId {
            value: format!("sha256:{}", "09".repeat(32)),
        }),
        canonical_period_sha256: Some(digest(9)),
    }
}

fn grant_reference() -> HoldoutGrantReference {
    HoldoutGrantReference {
        holdout_grant_id: Some(HoldoutGrantId {
            value: "grant.holdout.0001".to_owned(),
        }),
        holdout_period_id: Some(HoldoutPeriodId {
            value: format!("sha256:{}", "09".repeat(32)),
        }),
        freeze_manifest_sha256: Some(digest(6)),
        issued_at: None,
        expires_at: None,
        holdout_evaluation_plan_id: Some(HoldoutEvaluationPlanId {
            value: format!("sha256:{}", "0a".repeat(32)),
        }),
        evaluation_plan_sha256: Some(digest(7)),
        evaluation_plan_entry_count: 2,
        canonical_period_sha256: Some(digest(9)),
    }
}

#[test]
fn frozen_plan_and_narrow_batch_round_trip() {
    let freeze = FreezeManifestReference {
        holdout_evaluation_plan: Some(plan_reference()),
        ..Default::default()
    };
    let decoded_freeze = FreezeManifestReference::decode(freeze.encode_to_vec().as_slice())
        .expect("freeze manifest with a frozen plan must round-trip");
    assert_eq!(
        decoded_freeze
            .holdout_evaluation_plan
            .expect("frozen plan is required")
            .entry_count,
        2
    );

    let request = ConsumeGrantAndEnqueueBacktestRequest {
        context: None,
        grant_reference: Some(grant_reference()),
        expected_grant_revision: 3,
        expected_period_revision: 4,
    };
    let decoded_request =
        ConsumeGrantAndEnqueueBacktestRequest::decode(request.encode_to_vec().as_slice())
            .expect("reference-only consume request must round-trip");
    assert_eq!(decoded_request.expected_grant_revision, 3);
    assert_eq!(decoded_request.expected_period_revision, 4);

    let response = ConsumeGrantAndEnqueueBacktestResponse {
        consumed_grant: Some(grant_reference()),
        job_batch: Some(JobBatchHandle {
            job_batch_id: Some(JobBatchId {
                value: "batch.holdout.0001".to_owned(),
            }),
            holdout_grant_id: Some(HoldoutGrantId {
                value: "grant.holdout.0001".to_owned(),
            }),
            holdout_evaluation_plan_id: Some(HoldoutEvaluationPlanId {
                value: format!("sha256:{}", "0a".repeat(32)),
            }),
            evaluation_plan_sha256: Some(digest(7)),
            evaluation_plan_entry_count: 2,
            job_count: 2,
            job_ids: vec![
                JobId {
                    value: "job.holdout.0001".to_owned(),
                },
                JobId {
                    value: "job.holdout.0002".to_owned(),
                },
            ],
            revision: 1,
            created_at: None,
        }),
        period_record: None,
    };
    let decoded_response =
        ConsumeGrantAndEnqueueBacktestResponse::decode(response.encode_to_vec().as_slice())
            .expect("narrow batch response must round-trip");
    let batch = decoded_response
        .job_batch
        .expect("batch handle is required");
    assert_eq!(batch.job_count, 2);
    assert_eq!(batch.job_ids.len(), 2);
}

#[test]
fn internal_job_repeats_frozen_plan_identity_and_entry_index() {
    let job = HoldoutBacktestJobInput {
        consumed_grant: Some(grant_reference()),
        consumed_grant_revision: 4,
        frozen_backtest_spec: None,
        budget: None,
        job_batch_id: Some(JobBatchId {
            value: "batch.holdout.0001".to_owned(),
        }),
        holdout_evaluation_plan_id: Some(HoldoutEvaluationPlanId {
            value: format!("sha256:{}", "0a".repeat(32)),
        }),
        evaluation_plan_sha256: Some(digest(7)),
        evaluation_plan_entry_index: 2,
    };
    let decoded = HoldoutBacktestJobInput::decode(job.encode_to_vec().as_slice())
        .expect("internal plan-derived job must round-trip");
    assert_eq!(decoded.evaluation_plan_entry_index, 2);
    assert_eq!(
        decoded
            .holdout_evaluation_plan_id
            .expect("plan ID is required")
            .value,
        format!("sha256:{}", "0a".repeat(32))
    );
}

#[test]
fn shared_holdout_surface_vectors_fail_closed() {
    let descriptor = FileDescriptorSet::decode(loop_protocol::FILE_DESCRIPTOR_SET)
        .expect("committed descriptor must decode");
    for line in BOUNDARY_VECTORS.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let columns = line.split('\t').collect::<Vec<_>>();
        assert_eq!(columns.len(), 4, "invalid shared holdout vector");
        let present = surface_contains(&descriptor, columns[1], columns[2]);
        assert_eq!(
            present,
            columns[3] == "accept",
            "shared holdout vector {}",
            columns[0]
        );
    }
}

fn surface_contains(descriptor: &FileDescriptorSet, surface: &str, member: &str) -> bool {
    match surface {
        "consume_request" => message(
            descriptor,
            "loop/holdout/v1/service.proto",
            surface_message(surface),
        )
        .field
        .iter()
        .any(|field| field.name.as_deref() == Some(member)),
        "consume_response" | "get_period_request" => message(
            descriptor,
            "loop/holdout/v1/service.proto",
            surface_message(surface),
        )
        .field
        .iter()
        .any(|field| field.name.as_deref() == Some(member)),
        "freeze_manifest" => message(
            descriptor,
            "loop/v1/holdout.proto",
            surface_message(surface),
        )
        .field
        .iter()
        .any(|field| field.name.as_deref() == Some(member)),
        "internal_holdout_job" => {
            message(descriptor, "loop/v1/job.proto", surface_message(surface))
                .field
                .iter()
                .any(|field| field.name.as_deref() == Some(member))
        }
        "holdout_service" => file(descriptor, "loop/holdout/v1/service.proto")
            .service
            .iter()
            .find(|service| service.name.as_deref() == Some("HoldoutService"))
            .expect("HoldoutService must exist")
            .method
            .iter()
            .any(|method| method.name.as_deref() == Some(member)),
        _ => panic!("unknown shared holdout surface: {surface}"),
    }
}

fn surface_message(surface: &str) -> &str {
    match surface {
        "consume_request" => "ConsumeGrantAndEnqueueBacktestRequest",
        "consume_response" => "ConsumeGrantAndEnqueueBacktestResponse",
        "get_period_request" => "GetHoldoutPeriodRequest",
        "freeze_manifest" => "FreezeManifestReference",
        "internal_holdout_job" => "HoldoutBacktestJobInput",
        _ => panic!("surface has no message: {surface}"),
    }
}

fn message<'a>(
    descriptor: &'a FileDescriptorSet,
    file_name: &str,
    message_name: &str,
) -> &'a DescriptorProto {
    file(descriptor, file_name)
        .message_type
        .iter()
        .find(|message| message.name.as_deref() == Some(message_name))
        .unwrap_or_else(|| panic!("missing message {message_name}"))
}

fn file<'a>(descriptor: &'a FileDescriptorSet, name: &str) -> &'a FileDescriptorProto {
    descriptor
        .file
        .iter()
        .find(|file| file.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("missing file {name}"))
}
