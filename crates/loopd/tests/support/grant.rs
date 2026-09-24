use std::sync::{Arc, atomic::AtomicI64};

use loop_core::audit::Sha256Digest as CanonicalDigest;
use loop_core::holdout::CanonicalHoldoutPeriod;
use loop_protocol::wire::holdout::v1::{RecordHoldoutApprovalRequest, RequestHoldoutGrantRequest};
use loop_protocol::wire::v1::*;
use loopd::store::{
    CloseGrant, GrantClosure, GrantResult, HoldoutPolicy, HoldoutRepository, PgJobStore,
    ResolvedFreeze, StoreError, StoreResult,
};

use super::{FixtureClock, NOW, actor, approval, artifact, context, digest, holdout, options};

pub struct Policy {
    pub required_approvers: usize,
    pub grant_validity_ms: i64,
    pub alias_subject: bool,
    pub resolved: Option<ResolvedFreeze>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            required_approvers: 2,
            grant_validity_ms: 60_000,
            alias_subject: false,
            resolved: Some(resolved(0)),
        }
    }
}

impl HoldoutPolicy for Policy {
    fn authorize_period(
        &self,
        operation: &str,
        principal: &Actor,
        period: &str,
    ) -> StoreResult<()> {
        if principal == &actor()
            && matches!(
                operation,
                "loop.holdout.issue-grant"
                    | "loop.holdout.read-grant"
                    | "loop.holdout.expire-grant"
                    | "loop.holdout.revoke-grant"
            )
            && period
                == holdout::command(0, "fixture")
                    .period
                    .unwrap()
                    .holdout_period_id
                    .unwrap()
                    .value
        {
            return Ok(());
        }
        if self.alias_subject
            && principal == &alias()
            && matches!(
                operation,
                "loop.holdout.record-approval" | "loop.holdout.read-approval"
            )
        {
            return approval::Policy::default().authorize_period(
                operation,
                &approval::human(2),
                period,
            );
        }
        approval::Policy::default().authorize_period(operation, principal, period)
    }

    fn validate_registration(&self, period: &CanonicalHoldoutPeriod) -> StoreResult<()> {
        holdout::Policy.validate_registration(period)
    }

    fn approval_expiry(
        &self,
        request: &RecordHoldoutApprovalRequest,
        period: &CanonicalHoldoutPeriod,
        now: i64,
    ) -> StoreResult<i64> {
        approval::Policy::default().approval_expiry(request, period, now)
    }

    fn authorize_approval_read(&self, principal: &Actor, id: &str) -> StoreResult<()> {
        approval::Policy::default().authorize_approval_read(principal, id)
    }

    fn resolve_freeze(&self, digest: &Sha256Digest) -> StoreResult<ResolvedFreeze> {
        let value = self.resolved.as_ref().ok_or(StoreError::AdmissionDenied)?;
        if value
            .reference
            .manifest
            .as_ref()
            .and_then(|v| v.sha256.as_ref())
            != Some(digest)
        {
            return Err(StoreError::AdmissionDenied);
        }
        // Synthetic pre-approved registry. This is not the future owning BacktestSpec parser.
        Ok(value.clone())
    }

    fn grant_expiry(
        &self,
        freeze: &FreezeManifestReference,
        approvals: &[HoldoutApprovalRecord],
        now: i64,
    ) -> StoreResult<i64> {
        if freeze != &resolved(0).reference || approvals.len() < self.required_approvers {
            return Err(StoreError::AdmissionDenied);
        }
        for record in approvals {
            let human = record
                .approved_by
                .as_ref()
                .ok_or(StoreError::AdmissionDenied)?;
            if !([approval::human(1), approval::human(2)].contains(human)
                || self.alias_subject && human == &alias())
            {
                return Err(StoreError::AdmissionDenied);
            }
        }
        now.checked_add(self.grant_validity_ms)
            .ok_or(StoreError::Invalid("fixture grant validity"))
    }

    fn authorize_grant_read(&self, principal: &Actor, _: &str) -> StoreResult<()> {
        if principal == &actor() {
            Ok(())
        } else {
            Err(StoreError::AdmissionDenied)
        }
    }
}

pub fn alias() -> Actor {
    Actor {
        authenticated_subject: approval::human(1).authenticated_subject,
        ..approval::human(2)
    }
}

pub fn resolved(index: usize) -> ResolvedFreeze {
    let fixtures: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/contracts/holdout_identity_golden.json"
    ))
    .unwrap();
    let value = &fixtures["plans"][index];
    let plan_bytes = value["canonical_json"]
        .as_str()
        .unwrap()
        .as_bytes()
        .to_vec();
    let plan_digest = CanonicalDigest::parse(value["plan_sha256"].as_str().unwrap()).unwrap();
    let plan_digest = Sha256Digest {
        value: plan_digest.as_bytes().to_vec(),
    };
    let period = holdout::command(index, "fixture").period.unwrap();
    let mut plan_artifact = artifact();
    plan_artifact.artifact_id = Some(ArtifactId {
        value: value["plan_sha256"].as_str().unwrap().to_owned(),
    });
    plan_artifact.uri = format!(
        "artifact://sha256/{}",
        &value["plan_sha256"].as_str().unwrap()[7..]
    );
    plan_artifact.sha256 = Some(plan_digest.clone());
    plan_artifact.schema = Some(ArtifactSchemaReference {
        name: "loop.holdout_evaluation_plan".to_owned(),
        version: 1,
        schema_sha256: Some(digest(0xbb)),
    });
    plan_artifact.byte_size = plan_bytes.len() as u64;
    let mut manifest = artifact();
    manifest.artifact_id = Some(ArtifactId {
        value: format!("sha256:{}", "5a".repeat(32)),
    });
    manifest.uri = format!("artifact://sha256/{}", "5a".repeat(32));
    manifest.sha256 = Some(digest(90));
    manifest.schema.as_mut().unwrap().name = "loop.freeze_manifest".to_owned();
    ResolvedFreeze {
        reference: FreezeManifestReference {
            manifest: Some(manifest),
            configuration_sha256: Some(digest(41)),
            model_catalog_sha256: Some(digest(42)),
            data_manifest_sha256: Some(digest(43)),
            source_tree_sha256: Some(digest(44)),
            source_commit: Some(VcsObjectId {
                algorithm: VcsObjectAlgorithm::Sha1 as i32,
                value: vec![45; 20],
            }),
            holdout_approval_policy: Some(PolicyReference {
                policy_id: Some(PolicyId {
                    value: "fixture.approvers".to_owned(),
                }),
                revision: "1".to_owned(),
                sha256: Some(digest(46)),
            }),
            holdout_evaluation_plan: Some(HoldoutEvaluationPlanReference {
                holdout_evaluation_plan_id: Some(HoldoutEvaluationPlanId {
                    value: value["holdout_evaluation_plan_id"]
                        .as_str()
                        .unwrap()
                        .to_owned(),
                }),
                canonical_plan: Some(plan_artifact),
                plan_sha256: Some(plan_digest),
                entry_count: value["entry_count"].as_u64().unwrap() as u32,
                holdout_period_id: period.holdout_period_id,
                canonical_period_sha256: period.canonical_period_sha256,
            }),
        },
        canonical_plan: plan_bytes,
        plan_schema_sha256: [0xbb; 32],
        backtest_schema_sha256: [0xaa; 32],
        backtest_artifacts: fixtures["backtest_artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| {
                (
                    item["sha256"].as_str().unwrap().to_owned(),
                    item["content"].as_str().unwrap().as_bytes().to_vec(),
                )
            })
            .collect(),
    }
}

pub async fn seed(store: &PgJobStore, count: u32, aliased: bool) -> RequestHoldoutGrantRequest {
    store
        .register_period(&actor(), holdout::command(0, "period.0"))
        .await
        .unwrap();
    let mut ids = Vec::new();
    for index in 1..=count {
        let human = if aliased && index == 2 {
            alias()
        } else {
            approval::human(index)
        };
        ids.push(
            store
                .record_approval(
                    &human,
                    approval::command(0, &format!("approval.{index}"), &human),
                )
                .await
                .unwrap()
                .record
                .holdout_approval_record_id
                .unwrap(),
        );
    }
    ids.sort_by(|a, b| a.value.cmp(&b.value));
    let period = holdout::command(0, "fixture").period.unwrap();
    RequestHoldoutGrantRequest {
        context: Some(context("grant.issue")),
        freeze_manifest: Some(resolved(0).reference),
        approval_record_ids: ids,
        holdout_period_id: period.holdout_period_id,
        expected_period_revision: 1,
        canonical_period_sha256: period.canonical_period_sha256,
    }
}

pub async fn setup_with(
    policy: Policy,
    count: u32,
) -> (
    tempfile::TempDir,
    PgJobStore,
    Arc<FixtureClock>,
    RequestHoldoutGrantRequest,
) {
    let directory = tempfile::tempdir().unwrap();
    let clock = Arc::new(FixtureClock(AtomicI64::new(NOW)));
    let mut config = options(&directory.path().join("state"), clock.clone());
    let aliased = policy.alias_subject;
    config.holdout_policy = Arc::new(policy);
    let store = PgJobStore::open(config).await.unwrap();
    let request = seed(&store, count, aliased).await;
    (directory, store, clock, request)
}

pub async fn setup() -> (
    tempfile::TempDir,
    PgJobStore,
    Arc<FixtureClock>,
    RequestHoldoutGrantRequest,
) {
    setup_with(Policy::default(), 2).await
}

pub fn close(result: &GrantResult, disposition: GrantClosure, key: &str) -> CloseGrant {
    CloseGrant {
        context: Some(context(key)),
        grant_reference: result.grant.reference.clone(),
        expected_grant_revision: result.grant.revision,
        expected_period_revision: result.period.revision,
        disposition: disposition as i32,
        reason: "Fixture terminal decision".to_owned(),
    }
}
