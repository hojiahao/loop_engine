use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicI64;

use loop_core::audit::Sha256Digest as CanonicalDigest;
use loop_core::holdout::{CanonicalHoldoutPeriod, parse_canonical_holdout_evaluation_plan};
use loop_protocol::wire::holdout::v1::RecordHoldoutApprovalRequest;
use loop_protocol::wire::v1::{Actor, ActorId, ActorKind, HoldoutEvaluationPlanId, Sha256Digest};
use loopd::store::{HoldoutPolicy, HoldoutRepository, PgJobStore, StoreError, StoreResult};

use super::{FixtureClock, NOW, actor, artifact, context, digest, holdout, options};

pub struct Policy {
    pub validity_ms: i64,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            validity_ms: 3_600_000,
        }
    }
}

impl HoldoutPolicy for Policy {
    fn authorize_period(
        &self,
        operation: &str,
        principal: &Actor,
        period_id: &str,
    ) -> StoreResult<()> {
        if matches!(
            operation,
            "loop.holdout.record-approval" | "loop.holdout.read-approval"
        ) && [human(1), human(2)].contains(principal)
            && [0, 1].into_iter().any(|index| {
                holdout::command(index, "fixture")
                    .period
                    .unwrap()
                    .holdout_period_id
                    .unwrap()
                    .value
                    == period_id
            })
        {
            return Ok(());
        }
        holdout::Policy.authorize_period(operation, principal, period_id)
    }

    fn validate_registration(&self, period: &CanonicalHoldoutPeriod) -> StoreResult<()> {
        holdout::Policy.validate_registration(period)
    }

    fn authorize_approval_read(&self, principal: &Actor, _: &str) -> StoreResult<()> {
        if [human(1), human(2)].contains(principal) {
            Ok(())
        } else {
            Err(StoreError::AdmissionDenied)
        }
    }

    fn approval_expiry(
        &self,
        request: &RecordHoldoutApprovalRequest,
        period: &CanonicalHoldoutPeriod,
        now: i64,
    ) -> StoreResult<i64> {
        let fixture = goldens();
        let index =
            usize::from(period.value.sample.role.as_str() == "second_locked_historical_holdout");
        let artifacts: BTreeMap<_, _> = fixture["backtest_artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| {
                (
                    item["sha256"].as_str().unwrap().to_owned(),
                    item["content"].as_str().unwrap().as_bytes().to_vec(),
                )
            })
            .collect();
        // These opaque BacktestSpec fixtures test storage bindings, not Phase 7 semantics.
        let plan = parse_canonical_holdout_evaluation_plan(
            fixture["plans"][index]["canonical_json"]
                .as_str()
                .unwrap()
                .as_bytes(),
            period,
            &[0xaa; 32],
            &artifacts,
        )?;
        if request.freeze_manifest_sha256 != Some(digest(90))
            || request
                .holdout_evaluation_plan_id
                .as_ref()
                .map(|id| id.value.as_str())
                != Some(plan.holdout_evaluation_plan_id.as_str())
            || request
                .evaluation_plan_sha256
                .as_ref()
                .map(|value| value.value.as_slice())
                != Some(plan.plan_sha256.as_slice())
            || request.evaluation_plan_entry_count as usize != plan.value.entries.len()
            || request.evidence != vec![artifact()]
        {
            return Err(StoreError::AdmissionDenied);
        }
        now.checked_add(self.validity_ms)
            .ok_or(StoreError::Invalid("fixture validity"))
    }
}

pub fn human(index: u32) -> Actor {
    Actor {
        actor_id: Some(ActorId {
            value: format!("human.{index}"),
        }),
        kind: ActorKind::Human as i32,
        display_name: format!("Fixture human {index}"),
        authenticated_subject: format!("test:human:{index}"),
    }
}

pub fn command(index: usize, key: &str, principal: &Actor) -> RecordHoldoutApprovalRequest {
    let period = holdout::command(index, "fixture").period.unwrap();
    let fixture = goldens();
    let plan = &fixture["plans"][index];
    let mut context = context(key);
    context.actor = Some(principal.clone());
    RecordHoldoutApprovalRequest {
        context: Some(context),
        holdout_period_id: period.holdout_period_id,
        freeze_manifest_sha256: Some(digest(90)),
        reason: "Reviewed the frozen plan and evidence".to_owned(),
        evidence: vec![artifact()],
        canonical_period_sha256: period.canonical_period_sha256,
        holdout_evaluation_plan_id: Some(HoldoutEvaluationPlanId {
            value: plan["holdout_evaluation_plan_id"]
                .as_str()
                .unwrap()
                .to_owned(),
        }),
        evaluation_plan_sha256: Some(Sha256Digest {
            value: CanonicalDigest::parse(plan["plan_sha256"].as_str().unwrap())
                .unwrap()
                .as_bytes()
                .to_vec(),
        }),
        evaluation_plan_entry_count: plan["entry_count"].as_u64().unwrap() as u32,
    }
}

fn goldens() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../../tests/contracts/holdout_identity_golden.json"
    ))
    .unwrap()
}

pub async fn setup() -> (tempfile::TempDir, PgJobStore, Arc<FixtureClock>) {
    let directory = tempfile::tempdir().unwrap();
    let clock = Arc::new(FixtureClock(AtomicI64::new(NOW)));
    let mut config = options(&directory.path().join("state"), clock.clone());
    config.holdout_policy = Arc::new(Policy::default());
    let store = PgJobStore::open(config).await.unwrap();
    store
        .register_period(&actor(), holdout::command(0, "period.0"))
        .await
        .unwrap();
    (directory, store, clock)
}
