//! Real file/ledger lifecycle with explicitly synthetic frozen projections.
use std::sync::Arc;

use crate::store::{
    AdmissionPolicy, HoldoutPolicy, HoldoutRepository, PgJobStore, RegisterPeriod, ResolvedFreeze,
    StoreError, StoreResult,
};
use loop_core::audit::Sha256Digest as DigestId;
use loop_core::holdout::{self as canonical, CanonicalHoldoutPeriod, HoldoutEvaluationPlanEntry};
use loop_protocol::wire::holdout::v1::{RecordHoldoutApprovalRequest, RequestHoldoutGrantRequest};
use loop_protocol::wire::v1::*;

use super::{Fixture, model, support};
use support::{NOW, actor, approval, batch, context, grant, holdout};

struct Policy {
    period: CanonicalHoldoutPeriod,
    wire: HoldoutPeriod,
    freeze: ResolvedFreeze,
}

impl Policy {
    fn new(
        fixture: &Fixture,
        data: &model::Dataset,
        reference: &crate::manifests::ObjectRef,
    ) -> Self {
        let original = holdout::command(0, "original");
        let mut value = canonical::parse_canonical_holdout_period(&original.canonical_bytes)
            .unwrap()
            .value;
        value.snapshot_ids = data
            .snapshots
            .iter()
            .map(|snapshot| snapshot.snapshot_id.clone())
            .collect();
        value.snapshot_manifest_sha256 = reference.sha256.clone();
        let period = canonical::canonicalize_holdout_period(value).unwrap();
        let mut wire = original.period.unwrap();
        wire.holdout_period_id.as_mut().unwrap().value = period.holdout_period_id.clone();
        wire.canonical_period_sha256 = Some(Sha256Digest {
            value: period.canonical_period_sha256.to_vec(),
        });
        wire.snapshot_ids = data.reference(reference).unwrap().snapshot_ids;
        wire.snapshot_manifest_sha256 = Some(Sha256Digest {
            value: reference.digest().unwrap().to_vec(),
        });

        let mut freeze = grant::resolved(0);
        let original_period =
            canonical::parse_canonical_holdout_period(&original.canonical_bytes).unwrap();
        let mut plan = canonical::parse_canonical_holdout_evaluation_plan(
            &freeze.canonical_plan,
            &original_period,
            &freeze.backtest_schema_sha256,
            &freeze.backtest_artifacts,
        )
        .unwrap()
        .value;
        plan.holdout_period_id = period.holdout_period_id.clone();
        plan.canonical_period_sha256 = format!(
            "sha256:{}",
            period
                .canonical_period_sha256
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        let plan = canonical::canonicalize_holdout_evaluation_plan(
            plan,
            &period,
            &freeze.backtest_schema_sha256,
            &freeze.backtest_artifacts,
        )
        .unwrap();
        freeze.canonical_plan = plan.canonical_bytes;
        let plan_object = fixture::put(&fixture.root, &freeze.canonical_plan);
        let pinned = freeze.reference.holdout_evaluation_plan.as_mut().unwrap();
        pinned.holdout_period_id = wire.holdout_period_id.clone();
        pinned.canonical_period_sha256 = wire.canonical_period_sha256.clone();
        pinned.holdout_evaluation_plan_id.as_mut().unwrap().value = plan.holdout_evaluation_plan_id;
        pinned.plan_sha256 = Some(Sha256Digest {
            value: plan.plan_sha256.to_vec(),
        });
        let artifact = pinned.canonical_plan.as_mut().unwrap();
        artifact.artifact_id.as_mut().unwrap().value = plan_object.sha256.clone();
        artifact.sha256 = pinned.plan_sha256.clone();
        artifact.uri = format!("artifact://sha256/{}", &plan_object.sha256[7..]);
        artifact.byte_size = plan_object.byte_size;
        freeze.reference.data_manifest_sha256 = wire.snapshot_manifest_sha256.clone();
        Self {
            period,
            wire,
            freeze,
        }
    }
}

use super::super::fixture;

impl HoldoutPolicy for Policy {
    fn authorize_period(&self, _: &str, principal: &Actor, period: &str) -> StoreResult<()> {
        if period == self.period.holdout_period_id
            && [actor(), approval::human(1), approval::human(2)].contains(principal)
        {
            Ok(())
        } else {
            Err(StoreError::AdmissionDenied)
        }
    }
    fn validate_registration(&self, period: &CanonicalHoldoutPeriod) -> StoreResult<()> {
        if period == &self.period {
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
        if period == &self.period
            && request.freeze_manifest_sha256
                == self
                    .freeze
                    .reference
                    .manifest
                    .as_ref()
                    .and_then(|value| value.sha256.clone())
        {
            Ok(now + 3_600_000)
        } else {
            Err(StoreError::AdmissionDenied)
        }
    }
    fn grant_expiry(
        &self,
        freeze: &FreezeManifestReference,
        approvals: &[HoldoutApprovalRecord],
        now: i64,
    ) -> StoreResult<i64> {
        if freeze == &self.freeze.reference && approvals.len() == 2 {
            Ok(now + 60_000)
        } else {
            Err(StoreError::AdmissionDenied)
        }
    }
    fn resolve_freeze(&self, digest: &Sha256Digest) -> StoreResult<ResolvedFreeze> {
        if self
            .freeze
            .reference
            .manifest
            .as_ref()
            .and_then(|value| value.sha256.as_ref())
            == Some(digest)
        {
            Ok(self.freeze.clone())
        } else {
            Err(StoreError::AdmissionDenied)
        }
    }
    fn materialize_backtest(
        &self,
        freeze: &FreezeManifestReference,
        entry: &HoldoutEvaluationPlanEntry,
        bytes: &[u8],
    ) -> StoreResult<BacktestSpec> {
        if freeze != &self.freeze.reference {
            return Err(StoreError::AdmissionDenied);
        }
        let mut spec = batch::Policy::default().materialize_backtest(
            &grant::resolved(0).reference,
            entry,
            bytes,
        )?;
        spec.sample = self.wire.sample;
        spec.snapshot_ids = self.wire.snapshot_ids.clone();
        spec.provenance.as_mut().unwrap().data_manifest_sha256 =
            self.wire.snapshot_manifest_sha256.clone();
        Ok(spec)
    }
}

struct SeedAuthority;
impl AdmissionPolicy for SeedAuthority {
    fn validate_submission(&self, job: &JobSpecification) -> StoreResult<()> {
        loop_protocol::job::validate_job_specification(job)?;
        if job.kind == JobKind::HoldoutBacktest as i32
            && job.submitted_by.as_ref() == Some(&actor())
        {
            Ok(())
        } else {
            Err(StoreError::AdmissionDenied)
        }
    }
    fn authorize_job_command(&self, _: &str, principal: &Actor, _: &JobRecord) -> StoreResult<()> {
        if principal == &actor() {
            Ok(())
        } else {
            Err(StoreError::AdmissionDenied)
        }
    }
}

pub(super) async fn seed(fixture: &mut Fixture, clock: Arc<support::FixtureClock>) -> PgJobStore {
    let mut data: model::Dataset =
        serde_json::from_slice(&std::fs::read(fixture.path(&fixture.context.data)).unwrap())
            .unwrap();
    data.schema = "loop.protected-dataset/v1".to_owned();
    data.sample = model::Sample {
        role: model::SampleRole::FirstLockedConfirmation,
        start: "2021-01-01".to_owned(),
        end: "2024-12-31".to_owned(),
    };
    for (index, snapshot) in data.snapshots.iter_mut().enumerate() {
        snapshot.snapshot_id = format!("sha256:{:064x}", index + 1);
    }
    fixture.context.data = fixture.json(&data);
    let policy = Arc::new(Policy::new(fixture, &data, &fixture.context.data));
    let mut options = support::options(&fixture.directory.path().join("state"), clock);
    options.admission = Arc::new(SeedAuthority);
    options.holdout_policy = policy.clone();
    let store = PgJobStore::open(options).await.unwrap();
    store
        .register_period(
            &actor(),
            RegisterPeriod {
                context: Some(context("tls-period")),
                period: Some(policy.wire.clone()),
                canonical_bytes: policy.period.canonical_bytes.clone(),
            },
        )
        .await
        .unwrap();
    let plan = policy
        .freeze
        .reference
        .holdout_evaluation_plan
        .as_ref()
        .unwrap();
    let mut ids = Vec::new();
    for index in 1..=2 {
        let human = approval::human(index);
        let mut request = approval::command(0, &format!("tls-approval-{index}"), &human);
        request.holdout_period_id = policy.wire.holdout_period_id.clone();
        request.canonical_period_sha256 = policy.wire.canonical_period_sha256.clone();
        request.holdout_evaluation_plan_id = plan.holdout_evaluation_plan_id.clone();
        request.evaluation_plan_sha256 = plan.plan_sha256.clone();
        ids.push(
            store
                .record_approval(&human, request)
                .await
                .unwrap()
                .record
                .holdout_approval_record_id
                .unwrap(),
        );
    }
    ids.sort_by(|left, right| left.value.cmp(&right.value));
    let issued = store
        .issue_grant(
            &actor(),
            RequestHoldoutGrantRequest {
                context: Some(context("tls-grant")),
                freeze_manifest: Some(policy.freeze.reference.clone()),
                approval_record_ids: ids,
                holdout_period_id: policy.wire.holdout_period_id.clone(),
                canonical_period_sha256: policy.wire.canonical_period_sha256.clone(),
                expected_period_revision: 1,
            },
        )
        .await
        .unwrap();
    let mut metadata = support::research::metadata();
    metadata.protocol_selection = fixture
        .job
        .specification
        .protocol_selection
        .clone()
        .unwrap();
    let batch = store
        .consume_grant(&actor(), batch::command(&issued, "tls-consume"), metadata)
        .await
        .unwrap();
    let job_id = &batch.response.job_batch.unwrap().job_ids[0].value;
    use crate::store::JobRepository;
    fixture.job.specification = store
        .get(job_id)
        .await
        .unwrap()
        .unwrap()
        .specification
        .unwrap();
    assert_eq!(
        fixture.job.specification.submitted_at,
        Some(support::timestamp(NOW))
    );
    assert_eq!(
        DigestId::parse(&fixture.context.data.sha256)
            .unwrap()
            .as_bytes()
            .as_slice(),
        fixture.context.data.digest().unwrap()
    );
    store
}
