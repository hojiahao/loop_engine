use std::path::Path;
use std::sync::{Arc, Mutex};

use loop_protocol::wire::v1::*;
use loopd::store::{
    AdmissionEvidence, AdmissionPolicy, BacktestPolicy, DecideFactor, FactorState, PgJobStore,
    StoreError, StoreOptions, StoreResult,
};
use sha2::{Digest, Sha256};

use super::{FixtureClock, NOW, actor, backtest, context, digest, perturbation, timestamp};

pub struct Admission;

impl AdmissionPolicy for Admission {
    fn validate_submission(&self, job: &JobSpecification) -> StoreResult<()> {
        perturbation::Admission.validate_submission(job)
    }
    fn authorize_job_command(&self, _: &str, principal: &Actor, _: &JobRecord) -> StoreResult<()> {
        if principal == &actor() || principal == &human() {
            Ok(())
        } else {
            Err(StoreError::AdmissionDenied)
        }
    }
}

/// Fabricated IS reports, deliberately not a production manifest resolver.
pub struct Policy {
    pub evidence: Mutex<AdmissionEvidence>,
    pub current: Mutex<Option<ResearchProvenanceFingerprint>>,
    pub available: Mutex<bool>,
    pub override_allowed: Mutex<bool>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            evidence: Mutex::new(evidence()),
            current: Mutex::new(perturbation::space().provenance),
            available: Mutex::new(true),
            override_allowed: Mutex::new(false),
        }
    }
}

impl BacktestPolicy for Policy {
    fn resolve_result(
        &self,
        job: &JobSpecification,
        _: &JobSuccess,
    ) -> StoreResult<BacktestResult> {
        Admission.validate_submission(job)?;
        let mut result = backtest::result();
        result.backtest_id.as_mut().unwrap().value =
            format!("backtest.{}", job.job_id.as_ref().unwrap().value);
        Ok(result)
    }
    fn resolve_current(
        &self,
        _: &Actor,
        _: &JobSpecification,
        id: &str,
    ) -> StoreResult<Option<ResearchProvenanceFingerprint>> {
        if id != "context.library" {
            return Err(StoreError::AdmissionDenied);
        }
        Ok(self.current.lock().unwrap().clone())
    }
    fn resolve_admission(
        &self,
        _: &Actor,
        _: &JobSpecification,
        id: &str,
    ) -> StoreResult<AdmissionEvidence> {
        if id != "context.library" {
            return Err(StoreError::AdmissionDenied);
        }
        if !*self.available.lock().unwrap() {
            return Err(StoreError::Unavailable("fixture review unavailable"));
        }
        Ok(self.evidence.lock().unwrap().clone())
    }
    fn authorize_factor_override(
        &self,
        principal: &Actor,
        command: &DecideFactor,
        _: &AdmissionEvidence,
    ) -> StoreResult<()> {
        if principal != &human()
            || !*self.override_allowed.lock().unwrap()
            || command.override_approval_id != "approval.semantic.1"
        {
            return Err(StoreError::AdmissionDenied);
        }
        Ok(())
    }
}

pub fn human() -> Actor {
    let mut principal = actor();
    principal.kind = ActorKind::Human as i32;
    principal.actor_id.as_mut().unwrap().value = "human.fixture".to_owned();
    principal.authenticated_subject = "user:fixture".to_owned();
    principal
}

pub fn evidence() -> AdmissionEvidence {
    AdmissionEvidence {
        report: Some(backtest::outputs()[0].clone()),
        policy: Some(PolicyReference {
            policy_id: Some(PolicyId {
                value: "policy.admission".to_owned(),
            }),
            revision: "1".to_owned(),
            sha256: Some(digest(50)),
        }),
        result_manifest_sha256: vec![39; 32],
        library_sha256: snapshot(&[]),
        eligible_observations: 1000,
        valid_observations: 950,
        minimum_coverage_bps: 9500,
        machine_rejection: String::new(),
        semantic_accepted: true,
        replacements: vec![],
    }
}

pub fn snapshot(states: &[FactorState]) -> Vec<u8> {
    let mut entries: Vec<_> = states
        .iter()
        .filter(|s| s.status == "admitted")
        .map(|s| (&s.factor_spec_id, s.revision))
        .collect();
    entries.sort();
    let mut hash = Sha256::new();
    hash.update(b"loop.active-library.v1\0");
    hash.update(serde_json::to_vec(&entries).unwrap());
    hash.finalize().to_vec()
}

pub fn options(path: &Path, clock: Arc<FixtureClock>, policy: Arc<Policy>) -> StoreOptions {
    let mut options = super::options(path, clock);
    options.admission = Arc::new(Admission);
    options.backtest_policy = policy;
    options
}

pub fn command(index: u32, revision: u64, key: &str) -> DecideFactor {
    DecideFactor {
        context: Some(context(key)),
        source_job_id: Some(JobId {
            value: format!("job.{index}"),
        }),
        context_id: "context.library".to_owned(),
        expected_revision: revision,
        reason: "fixture evidence decision".to_owned(),
        override_reason: String::new(),
        override_approval_id: String::new(),
        deadline: Some(timestamp(NOW + 30_000)),
    }
}

pub fn forced(index: u32, revision: u64, key: &str) -> DecideFactor {
    let mut command = command(index, revision, key);
    command.context.as_mut().unwrap().actor = Some(human());
    command.override_reason = "fixture semantic-only human exception".to_owned();
    command.override_approval_id = "approval.semantic.1".to_owned();
    command
}

pub async fn seed(store: &PgJobStore, index: u32, window: u32) {
    perturbation::seed(store, index, window, false).await;
}
