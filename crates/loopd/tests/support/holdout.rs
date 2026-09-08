use chrono::{Datelike, NaiveDate};
use loop_core::audit::Sha256Digest as CanonicalDigest;
use loop_core::holdout::{CanonicalHoldoutPeriod, parse_canonical_holdout_period};
use loop_protocol::wire::v1::{
    Actor, CivilDate, HoldoutPeriod, HoldoutPeriodId, SampleRole, SampleWindow, Sha256Digest,
    SnapshotId,
};
use loopd::store::{HoldoutPolicy, RegisterPeriod, StoreError, StoreResult};

use super::{actor, context};

pub struct Policy;

impl HoldoutPolicy for Policy {
    fn authorize_period(&self, operation: &str, principal: &Actor, _: &str) -> StoreResult<()> {
        if principal == &actor()
            && matches!(
                operation,
                "loop.holdout.register-period" | "loop.holdout.read-period"
            )
        {
            Ok(())
        } else {
            Err(StoreError::AdmissionDenied)
        }
    }

    fn validate_registration(&self, period: &CanonicalHoldoutPeriod) -> StoreResult<()> {
        if [0, 1]
            .into_iter()
            .any(|index| command(index, "fixture").canonical_bytes == period.canonical_bytes)
        {
            Ok(())
        } else {
            Err(StoreError::AdmissionDenied)
        }
    }
}

pub fn command(index: usize, key: &str) -> RegisterPeriod {
    let fixtures: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/contracts/holdout_identity_golden.json"
    ))
    .unwrap();
    let bytes = fixtures["periods"][index]["canonical_json"]
        .as_str()
        .unwrap()
        .as_bytes();
    let canonical = parse_canonical_holdout_period(bytes).unwrap();
    RegisterPeriod {
        context: Some(context(key)),
        period: Some(HoldoutPeriod {
            holdout_period_id: Some(HoldoutPeriodId {
                value: canonical.holdout_period_id,
            }),
            sample: Some(SampleWindow {
                role: if index == 0 {
                    SampleRole::FirstLockedConfirmation
                } else {
                    SampleRole::SecondLockedHistoricalHoldout
                } as i32,
                start_inclusive: Some(date(&canonical.value.sample.start_inclusive)),
                end_inclusive: Some(date(&canonical.value.sample.end_inclusive)),
            }),
            snapshot_ids: canonical
                .value
                .snapshot_ids
                .into_iter()
                .map(|value| SnapshotId { value })
                .collect(),
            snapshot_manifest_sha256: Some(Sha256Digest {
                value: CanonicalDigest::parse(&canonical.value.snapshot_manifest_sha256)
                    .unwrap()
                    .as_bytes()
                    .to_vec(),
            }),
            canonical_period_sha256: Some(Sha256Digest {
                value: canonical.canonical_period_sha256.to_vec(),
            }),
        }),
        canonical_bytes: bytes.to_vec(),
    }
}

fn date(value: &str) -> CivilDate {
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap();
    CivilDate {
        year: date.year(),
        month: date.month(),
        day: date.day(),
    }
}
