use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

use loop_core::audit::Sha256Digest as CanonicalDigest;
use loop_core::holdout::{CanonicalHoldoutPeriod, HoldoutEvaluationPlanEntry};
use loop_protocol::negotiation::{ProtocolBuildIdentity, validate_protocol_selection_availability};
use loop_protocol::wire::holdout::v1::{
    ConsumeGrantAndEnqueueBacktestRequest, RecordHoldoutApprovalRequest,
};
use loop_protocol::wire::v1::*;
use loopd::store::{
    AdmissionPolicy, GrantResult, HoldoutPolicy, HoldoutRepository, PgJobStore, ResolvedFreeze,
    StoreError, StoreResult,
};

use super::{
    FixtureAdmission, FixtureClock, NOW, actor, context, digest, grant, holdout, options,
    protocol_info, timestamp,
};

#[derive(Default)]
pub struct Policy {
    pub grant: grant::Policy,
    pub wrong_field: Option<&'static str>,
    pub advance_clock: Option<(Arc<FixtureClock>, i64)>,
}

impl HoldoutPolicy for Policy {
    fn authorize_period(
        &self,
        operation: &str,
        principal: &Actor,
        period: &str,
    ) -> StoreResult<()> {
        if operation == "loop.holdout.consume_grant" {
            self.grant
                .authorize_period("loop.holdout.issue-grant", principal, period)
        } else {
            self.grant.authorize_period(operation, principal, period)
        }
    }
    fn validate_registration(&self, period: &CanonicalHoldoutPeriod) -> StoreResult<()> {
        self.grant.validate_registration(period)
    }
    fn approval_expiry(
        &self,
        request: &RecordHoldoutApprovalRequest,
        period: &CanonicalHoldoutPeriod,
        now: i64,
    ) -> StoreResult<i64> {
        self.grant.approval_expiry(request, period, now)
    }
    fn grant_expiry(
        &self,
        freeze: &FreezeManifestReference,
        records: &[HoldoutApprovalRecord],
        now: i64,
    ) -> StoreResult<i64> {
        self.grant.grant_expiry(freeze, records, now)
    }
    fn authorize_approval_read(&self, principal: &Actor, id: &str) -> StoreResult<()> {
        self.grant.authorize_approval_read(principal, id)
    }
    fn authorize_grant_read(&self, principal: &Actor, id: &str) -> StoreResult<()> {
        self.grant.authorize_grant_read(principal, id)
    }
    fn resolve_freeze(&self, digest: &Sha256Digest) -> StoreResult<ResolvedFreeze> {
        self.grant.resolve_freeze(digest)
    }
    fn materialize_backtest(
        &self,
        freeze: &FreezeManifestReference,
        entry: &HoldoutEvaluationPlanEntry,
        bytes: &[u8],
    ) -> StoreResult<BacktestSpec> {
        if self.wrong_field == Some("unavailable") {
            return Err(StoreError::Unavailable("fixture parser unavailable"));
        }
        let expected = grant::resolved(0);
        if freeze != &expected.reference
            || expected
                .backtest_artifacts
                .get(&entry.backtest_spec_artifact.sha256)
                .map(Vec::as_slice)
                != Some(bytes)
        {
            return Err(StoreError::AdmissionDenied);
        }
        let period = holdout::command(0, "fixture").period.unwrap();
        let mut spec = BacktestSpec {
            backtest_id: Some(BacktestId {
                value: format!("backtest.fixture.{}", entry.entry_index),
            }),
            schema_version: 1,
            factor_spec_id: Some(FactorSpecId {
                value: entry.factor_spec_id.clone(),
            }),
            snapshot_ids: period.snapshot_ids,
            sample: period.sample,
            return_definition: ReturnDefinition::SimpleNavReturn as i32,
            provenance: Some(ResearchProvenanceFingerprint {
                source_code_sha256: freeze.source_tree_sha256.clone(),
                operator_registry_sha256: Some(digest(51)),
                configuration_sha256: freeze.configuration_sha256.clone(),
                data_manifest_sha256: freeze.data_manifest_sha256.clone(),
                trading_calendar_sha256: Some(digest(52)),
                environment_sha256: Some(digest(53)),
            }),
            canonical_spec_sha256: Some(Sha256Digest {
                value: CanonicalDigest::parse(&entry.backtest_spec_artifact.sha256)
                    .unwrap()
                    .as_bytes()
                    .to_vec(),
            }),
            created_at: Some(timestamp(NOW - 5_000)),
            deterministic_seed: Some(digest(54)),
        };
        // Explicitly synthetic projection for storage tests, not a production parser.
        if entry.entry_index == "2" {
            match self.wrong_field {
                Some("factor") => {
                    spec.factor_spec_id.as_mut().unwrap().value =
                        format!("sha256:{}", "fe".repeat(32))
                }
                Some("sample") => {
                    spec.sample.as_mut().unwrap().role =
                        SampleRole::SecondLockedHistoricalHoldout as i32
                }
                Some("snapshots") => {
                    spec.snapshot_ids.remove(0);
                }
                Some("digest") => spec.canonical_spec_sha256 = Some(digest(55)),
                Some("configuration") => {
                    spec.provenance.as_mut().unwrap().configuration_sha256 = Some(digest(56))
                }
                Some("source") => {
                    spec.provenance.as_mut().unwrap().source_code_sha256 = Some(digest(57))
                }
                Some("data") => {
                    spec.provenance.as_mut().unwrap().data_manifest_sha256 = Some(digest(58))
                }
                Some("seed") => spec.deterministic_seed = None,
                Some(_) | None => (),
            }
            if let Some((clock, value)) = &self.advance_clock {
                clock.0.store(*value, Ordering::SeqCst);
            }
        }
        Ok(spec)
    }
}

pub struct Admission;
impl AdmissionPolicy for Admission {
    fn validate_submission(&self, job: &JobSpecification) -> StoreResult<()> {
        if job.kind == JobKind::Report as i32 {
            return FixtureAdmission.validate_submission(job);
        }
        if job.kind != JobKind::HoldoutBacktest as i32
            || job.submitted_by.as_ref() != Some(&actor())
        {
            return Err(StoreError::AdmissionDenied);
        }
        validate_protocol_selection_availability(
            job.protocol_selection
                .as_ref()
                .ok_or(StoreError::AdmissionDenied)?,
            &protocol_info(),
            &[ProtocolBuildIdentity {
                build_version: "fixture.1".to_owned(),
                build_sha256: [23; 32],
            }],
            &[[22; 32]],
            "loop.v1",
            &[
                "jobs.envelope.v1",
                "jobs.kind-input.v1",
                "jobs.prelease-terminal.v1",
            ],
        )
        .map_err(|_| StoreError::AdmissionDenied)
    }

    fn authorize_job_command(
        &self,
        operation: &str,
        principal: &Actor,
        record: &JobRecord,
    ) -> StoreResult<()> {
        FixtureAdmission.authorize_job_command(operation, principal, record)
    }
}

pub fn command(issued: &GrantResult, key: &str) -> ConsumeGrantAndEnqueueBacktestRequest {
    ConsumeGrantAndEnqueueBacktestRequest {
        context: Some(context(key)),
        grant_reference: issued.grant.reference.clone(),
        expected_grant_revision: issued.grant.revision,
        expected_period_revision: issued.period.revision,
    }
}

pub async fn seed(store: &PgJobStore) -> (GrantResult, ConsumeGrantAndEnqueueBacktestRequest) {
    let request = grant::seed(store, 2, false).await;
    let issued = store.issue_grant(&actor(), request).await.unwrap();
    let request = command(&issued, "batch.consume");
    (issued, request)
}

pub async fn setup_with(
    policy: Policy,
    clock: Arc<FixtureClock>,
) -> (
    tempfile::TempDir,
    PgJobStore,
    GrantResult,
    ConsumeGrantAndEnqueueBacktestRequest,
) {
    let directory = tempfile::tempdir().unwrap();
    let mut config = options(&directory.path().join("state"), clock);
    config.holdout_policy = Arc::new(policy);
    config.admission = Arc::new(Admission);
    let store = PgJobStore::open(config).await.unwrap();
    let (issued, request) = seed(&store).await;
    (directory, store, issued, request)
}

pub async fn setup() -> (
    tempfile::TempDir,
    PgJobStore,
    Arc<FixtureClock>,
    GrantResult,
    ConsumeGrantAndEnqueueBacktestRequest,
) {
    let clock = Arc::new(FixtureClock(AtomicI64::new(NOW)));
    let (directory, store, issued, request) = setup_with(Policy::default(), clock.clone()).await;
    (directory, store, clock, issued, request)
}
