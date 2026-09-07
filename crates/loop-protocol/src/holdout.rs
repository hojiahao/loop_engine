//! Wire-to-domain validation for holdout canonical identities.

use std::collections::BTreeMap;

use loop_core::holdout::{
    CanonicalHoldoutEvaluationPlan, CanonicalHoldoutPeriod, HoldoutEvaluationPlanReference,
    HoldoutValidationCode, HoldoutValidationError, PlanArtifactReference,
    validate_holdout_evaluation_plan_reference as validate_domain_plan_reference,
    verify_holdout_period_identity,
};

use crate::artifact::validate_artifact_ref;
use crate::wire::v1::{
    HoldoutEvaluationPlanReference as WirePlanReference, HoldoutPeriod as WirePeriod, SampleRole,
};

pub fn validate_holdout_period(
    wire: &WirePeriod,
    canonical_period_bytes: &[u8],
) -> Result<CanonicalHoldoutPeriod, HoldoutValidationError> {
    let period_id = wire
        .holdout_period_id
        .as_ref()
        .ok_or_else(|| invalid_reference("holdout_period_id"))?;
    let digest = require_digest(
        wire.canonical_period_sha256.as_ref(),
        "canonical_period_sha256",
    )?;
    let parsed = verify_holdout_period_identity(canonical_period_bytes, &period_id.value, &digest)?;
    let sample = wire
        .sample
        .as_ref()
        .ok_or_else(|| invalid_reference("sample"))?;
    let expected_role = match SampleRole::try_from(sample.role).ok() {
        Some(SampleRole::FirstLockedConfirmation) => "first_locked_confirmation",
        Some(SampleRole::SecondLockedHistoricalHoldout) => "second_locked_historical_holdout",
        _ => return Err(invalid_reference("sample.role")),
    };
    let start = sample
        .start_inclusive
        .as_ref()
        .map(format_date)
        .ok_or_else(|| invalid_reference("sample.start_inclusive"))?;
    let end = sample
        .end_inclusive
        .as_ref()
        .map(format_date)
        .ok_or_else(|| invalid_reference("sample.end_inclusive"))?;
    let snapshots = wire
        .snapshot_ids
        .iter()
        .map(|value| value.value.as_str())
        .collect::<Vec<_>>();
    let manifest = require_digest(
        wire.snapshot_manifest_sha256.as_ref(),
        "snapshot_manifest_sha256",
    )?;
    if parsed.value.sample.role.as_str() != expected_role
        || parsed.value.sample.start_inclusive != start
        || parsed.value.sample.end_inclusive != end
        || parsed
            .value
            .snapshot_ids
            .iter()
            .map(String::as_str)
            .ne(snapshots)
        || parsed.value.snapshot_manifest_sha256 != encode_digest(&manifest)
    {
        return Err(invalid_reference("holdout_period"));
    }
    Ok(parsed)
}

pub fn validate_holdout_evaluation_plan_reference(
    wire: &WirePlanReference,
    canonical_plan_bytes: &[u8],
    expected_period: &CanonicalHoldoutPeriod,
    trusted_plan_schema_sha256: &[u8; 32],
    trusted_backtest_schema_sha256: &[u8; 32],
    resolved_backtest_artifacts: &BTreeMap<String, Vec<u8>>,
) -> Result<CanonicalHoldoutEvaluationPlan, HoldoutValidationError> {
    let artifact_wire = wire
        .canonical_plan
        .as_ref()
        .ok_or_else(|| invalid_reference("canonical_plan"))?;
    let artifact =
        validate_artifact_ref(artifact_wire).map_err(|_| invalid_reference("canonical_plan"))?;
    let reference = HoldoutEvaluationPlanReference {
        holdout_evaluation_plan_id: wire
            .holdout_evaluation_plan_id
            .as_ref()
            .ok_or_else(|| invalid_reference("holdout_evaluation_plan_id"))?
            .value
            .clone(),
        canonical_plan: PlanArtifactReference {
            artifact_id: artifact.artifact_id,
            uri: artifact.uri,
            sha256: artifact.sha256,
            schema_name: artifact.schema_name,
            schema_version: artifact.schema_version,
            schema_sha256: artifact.schema_sha256,
            media_type: artifact.media_type,
            byte_size: artifact.byte_size,
            has_row_count: artifact.row_count.is_some(),
            has_manifest_sha256: artifact.manifest_sha256.is_some(),
        },
        plan_sha256: require_digest(wire.plan_sha256.as_ref(), "plan_sha256")?,
        entry_count: wire.entry_count,
        holdout_period_id: wire
            .holdout_period_id
            .as_ref()
            .ok_or_else(|| invalid_reference("holdout_period_id"))?
            .value
            .clone(),
        canonical_period_sha256: require_digest(
            wire.canonical_period_sha256.as_ref(),
            "canonical_period_sha256",
        )?,
    };
    validate_domain_plan_reference(
        &reference,
        canonical_plan_bytes,
        expected_period,
        trusted_plan_schema_sha256,
        trusted_backtest_schema_sha256,
        resolved_backtest_artifacts,
    )
}

fn require_digest(
    value: Option<&crate::wire::v1::Sha256Digest>,
    field: &'static str,
) -> Result<[u8; 32], HoldoutValidationError> {
    value
        .ok_or_else(|| invalid_reference(field))?
        .value
        .as_slice()
        .try_into()
        .map_err(|_| invalid_reference(field))
}

fn format_date(value: &crate::wire::v1::CivilDate) -> String {
    format!("{:04}-{:02}-{:02}", value.year, value.month, value.day)
}

fn invalid_reference(field: &'static str) -> HoldoutValidationError {
    HoldoutValidationError {
        code: HoldoutValidationCode::ReferenceMismatch,
        field,
    }
}

fn encode_digest(digest: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(71);
    output.push_str("sha256:");
    for byte in digest {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}
