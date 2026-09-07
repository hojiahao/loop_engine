use std::collections::BTreeMap;

use loop_core::holdout::{
    CanonicalHoldoutEvaluationPlan, CanonicalHoldoutPeriod,
    parse_canonical_holdout_evaluation_plan, parse_canonical_holdout_period,
};
use loop_protocol::holdout::{validate_holdout_evaluation_plan_reference, validate_holdout_period};
use loop_protocol::wire::v1::{
    ArtifactId, ArtifactRef, ArtifactSchemaReference, CivilDate, HoldoutEvaluationPlanId,
    HoldoutEvaluationPlanReference, HoldoutPeriod, HoldoutPeriodId, SampleRole, SampleWindow,
    Sha256Digest, SnapshotId,
};
use prost_types::Timestamp;
use serde_json::Value;

const GOLDEN: &str = include_str!("../../../tests/contracts/holdout_identity_golden.json");

#[test]
fn wire_period_and_plan_are_validated_as_exact_canonical_projections() {
    let fixture: Value = serde_json::from_str(GOLDEN).unwrap();
    let period_fixture = &fixture["periods"][0];
    let plan_fixture = &fixture["plans"][0];
    let trusted_plan = decode_digest(fixture["trusted_plan_schema_sha256"].as_str().unwrap());
    let trusted_backtest =
        decode_digest(fixture["trusted_backtest_schema_sha256"].as_str().unwrap());
    let resolved = resolved_artifacts(&fixture);
    let period = parse_canonical_holdout_period(
        period_fixture["canonical_json"]
            .as_str()
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    let plan = parse_canonical_holdout_evaluation_plan(
        plan_fixture["canonical_json"].as_str().unwrap().as_bytes(),
        &period,
        &trusted_backtest,
        &resolved,
    )
    .unwrap();

    let wire_period = period_wire(&period, period_fixture["canonical_json"].as_str().unwrap());
    assert_eq!(
        validate_holdout_period(&wire_period, &period.canonical_bytes)
            .unwrap()
            .holdout_period_id,
        period.holdout_period_id
    );

    let mut wire_plan = plan_wire(
        &plan,
        &period,
        plan_fixture["entry_count"].as_u64().unwrap() as u32,
        &trusted_plan,
    );
    assert_eq!(
        validate_holdout_evaluation_plan_reference(
            &wire_plan,
            &plan.canonical_bytes,
            &period,
            &trusted_plan,
            &trusted_backtest,
            &resolved,
        )
        .unwrap()
        .holdout_evaluation_plan_id,
        plan.holdout_evaluation_plan_id
    );

    wire_plan.canonical_plan.as_mut().unwrap().created_at = None;
    assert!(
        validate_holdout_evaluation_plan_reference(
            &wire_plan,
            &plan.canonical_bytes,
            &period,
            &trusted_plan,
            &trusted_backtest,
            &resolved,
        )
        .is_err()
    );
    wire_plan.canonical_plan.as_mut().unwrap().created_at = Some(Timestamp {
        seconds: 1,
        nanos: 0,
    });
    wire_plan.canonical_plan.as_mut().unwrap().row_count = Some(1);
    assert!(
        validate_holdout_evaluation_plan_reference(
            &wire_plan,
            &plan.canonical_bytes,
            &period,
            &trusted_plan,
            &trusted_backtest,
            &resolved,
        )
        .is_err()
    );
}

#[test]
fn wire_adapters_reject_missing_and_malformed_repeated_identities() {
    let fixture: Value = serde_json::from_str(GOLDEN).unwrap();
    let period_fixture = &fixture["periods"][0];
    let plan_fixture = &fixture["plans"][0];
    let trusted_plan = decode_digest(fixture["trusted_plan_schema_sha256"].as_str().unwrap());
    let trusted_backtest =
        decode_digest(fixture["trusted_backtest_schema_sha256"].as_str().unwrap());
    let resolved = resolved_artifacts(&fixture);
    let period = parse_canonical_holdout_period(
        period_fixture["canonical_json"]
            .as_str()
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    let plan = parse_canonical_holdout_evaluation_plan(
        plan_fixture["canonical_json"].as_str().unwrap().as_bytes(),
        &period,
        &trusted_backtest,
        &resolved,
    )
    .unwrap();

    let mut wire_period = period_wire(&period, period_fixture["canonical_json"].as_str().unwrap());
    wire_period.canonical_period_sha256 = Some(Sha256Digest { value: vec![0; 31] });
    assert!(validate_holdout_period(&wire_period, &period.canonical_bytes).is_err());

    let mut wire_plan = plan_wire(
        &plan,
        &period,
        plan_fixture["entry_count"].as_u64().unwrap() as u32,
        &trusted_plan,
    );
    wire_plan.plan_sha256 = None;
    assert!(
        validate_holdout_evaluation_plan_reference(
            &wire_plan,
            &plan.canonical_bytes,
            &period,
            &trusted_plan,
            &trusted_backtest,
            &resolved,
        )
        .is_err()
    );
}

fn period_wire(period: &CanonicalHoldoutPeriod, source: &str) -> HoldoutPeriod {
    let json: Value = serde_json::from_str(source).unwrap();
    let date = |value: &str| CivilDate {
        year: value[0..4].parse().unwrap(),
        month: value[5..7].parse().unwrap(),
        day: value[8..10].parse().unwrap(),
    };
    HoldoutPeriod {
        holdout_period_id: Some(HoldoutPeriodId {
            value: period.holdout_period_id.clone(),
        }),
        sample: Some(SampleWindow {
            role: SampleRole::FirstLockedConfirmation as i32,
            start_inclusive: Some(date(json["sample"]["start_inclusive"].as_str().unwrap())),
            end_inclusive: Some(date(json["sample"]["end_inclusive"].as_str().unwrap())),
        }),
        snapshot_ids: json["snapshot_ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| SnapshotId {
                value: value.as_str().unwrap().to_owned(),
            })
            .collect(),
        snapshot_manifest_sha256: Some(Sha256Digest {
            value: decode_digest(json["snapshot_manifest_sha256"].as_str().unwrap()).to_vec(),
        }),
        canonical_period_sha256: Some(Sha256Digest {
            value: period.canonical_period_sha256.to_vec(),
        }),
    }
}

fn plan_wire(
    plan: &CanonicalHoldoutEvaluationPlan,
    period: &CanonicalHoldoutPeriod,
    entry_count: u32,
    trusted_plan: &[u8; 32],
) -> HoldoutEvaluationPlanReference {
    let raw_id = encode_digest(&plan.plan_sha256);
    HoldoutEvaluationPlanReference {
        holdout_evaluation_plan_id: Some(HoldoutEvaluationPlanId {
            value: plan.holdout_evaluation_plan_id.clone(),
        }),
        canonical_plan: Some(ArtifactRef {
            artifact_id: Some(ArtifactId {
                value: raw_id.clone(),
            }),
            uri: format!("artifact://sha256/{}", &raw_id[7..]),
            sha256: Some(Sha256Digest {
                value: plan.plan_sha256.to_vec(),
            }),
            schema: Some(ArtifactSchemaReference {
                name: "loop.holdout_evaluation_plan".to_owned(),
                version: 1,
                schema_sha256: Some(Sha256Digest {
                    value: trusted_plan.to_vec(),
                }),
            }),
            media_type: "application/json".to_owned(),
            byte_size: plan.canonical_bytes.len() as u64,
            row_count: None,
            created_at: Some(Timestamp {
                seconds: 1,
                nanos: 0,
            }),
            manifest_sha256: None,
        }),
        plan_sha256: Some(Sha256Digest {
            value: plan.plan_sha256.to_vec(),
        }),
        entry_count,
        holdout_period_id: Some(HoldoutPeriodId {
            value: period.holdout_period_id.clone(),
        }),
        canonical_period_sha256: Some(Sha256Digest {
            value: period.canonical_period_sha256.to_vec(),
        }),
    }
}

fn resolved_artifacts(fixture: &Value) -> BTreeMap<String, Vec<u8>> {
    fixture["backtest_artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|artifact| {
            (
                artifact["sha256"].as_str().unwrap().to_owned(),
                artifact["content"].as_str().unwrap().as_bytes().to_vec(),
            )
        })
        .collect()
}

fn decode_digest(value: &str) -> [u8; 32] {
    let mut output = [0_u8; 32];
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[7 + index * 2..9 + index * 2], 16).unwrap();
    }
    output
}

fn encode_digest(value: &[u8; 32]) -> String {
    format!(
        "sha256:{}",
        value
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}
