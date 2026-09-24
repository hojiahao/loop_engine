use std::path::Path;
use std::sync::{Arc, Mutex};

use loop_protocol::wire::jobs::v1::{AcquireJobLeaseRequest, CompleteJobRequest};
use loop_protocol::wire::v1::*;
use loopd::research_worker::PythonPerturber;
use loopd::store::{
    AdmissionPolicy, AdvancePerturbation, BacktestPolicy, JobMutation, JobRepository, PgJobStore,
    StoreError, StoreOptions, StoreResult,
};

use super::{FixtureClock, NOW, actor, backtest, context, digest, research, timestamp};

pub fn candidate(window: u32) -> WindowCandidate {
    WindowCandidate {
        window,
        factor_spec_id: Some(FactorSpecId {
            value: format!("sha256:{window:064x}"),
        }),
    }
}

pub fn space() -> PerturbationSpace {
    let (_, job_specification::Input::Backtest(input)) = research::inputs().remove(2) else {
        unreachable!()
    };
    PerturbationSpace {
        algorithm: "window.ema-gradient-pcg64.v1".to_owned(),
        dataset: input.dataset,
        provenance: input.provenance,
        backtest_seed: input.deterministic_seed,
        random_seed: Some(digest(0)),
        candidates: [5, 10, 15, 20, 25].map(candidate).to_vec(),
    }
}

pub struct Admission;

impl AdmissionPolicy for Admission {
    fn validate_submission(&self, job: &JobSpecification) -> StoreResult<()> {
        let mut normalized = job.clone();
        let Some(job_specification::Input::Backtest(input)) = &mut normalized.input else {
            return backtest::Admission.validate_submission(job);
        };
        if !space()
            .candidates
            .iter()
            .any(|candidate| candidate.factor_spec_id == input.factor_spec_id)
        {
            return Err(StoreError::AdmissionDenied);
        }
        let (_, job_specification::Input::Backtest(original)) = research::inputs().remove(2) else {
            unreachable!()
        };
        input.factor_spec_id = original.factor_spec_id;
        research::Admission.validate_submission(&normalized)
    }

    fn authorize_job_command(
        &self,
        operation: &str,
        principal: &Actor,
        job: &JobRecord,
    ) -> StoreResult<()> {
        backtest::Admission.authorize_job_command(operation, principal, job)
    }
}

/// Fixed IS-only test family and immutable fabricated result metadata, not real data.
pub struct Policy {
    pub space: Mutex<PerturbationSpace>,
    pub current: Mutex<Option<ResearchProvenanceFingerprint>>,
    pub metric_override: Mutex<Option<(String, String)>>,
    pub denied: Mutex<bool>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            space: Mutex::new(space()),
            current: Mutex::new(space().provenance),
            metric_override: Mutex::new(None),
            denied: Mutex::new(false),
        }
    }
}

impl BacktestPolicy for Policy {
    fn resolve_perturbation_space(
        &self,
        _: &Actor,
        job: &JobSpecification,
        context: &str,
    ) -> StoreResult<PerturbationSpace> {
        Admission.validate_submission(job)?;
        if *self.denied.lock().unwrap() || context != "context.perturbation" {
            return Err(StoreError::AdmissionDenied);
        }
        Ok(self.space.lock().unwrap().clone())
    }

    fn resolve_result(
        &self,
        job: &JobSpecification,
        _: &JobSuccess,
    ) -> StoreResult<BacktestResult> {
        Admission.validate_submission(job)?;
        let Some(job_specification::Input::Backtest(input)) = &job.input else {
            return Err(StoreError::AdmissionDenied);
        };
        let candidate = space()
            .candidates
            .into_iter()
            .find(|candidate| candidate.factor_spec_id == input.factor_spec_id)
            .unwrap();
        let mut result = backtest::result();
        result.backtest_id.as_mut().unwrap().value =
            format!("backtest.{}", job.job_id.as_ref().unwrap().value);
        let (value, estimator) =
            self.metric_override
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| {
                    (
                        format!("{}", f64::from(candidate.window) / 10.0),
                        "sample_std_ddof1_sqrt252_zero_rf.v1".to_owned(),
                    )
                });
        result.metrics = vec![BacktestMetric {
            name: "net_sharpe".to_owned(),
            value: Some(ExactDecimal { value }),
            unit: "dimensionless".to_owned(),
            estimator,
        }];
        Ok(result)
    }

    fn resolve_current(
        &self,
        _: &Actor,
        _: &JobSpecification,
        context: &str,
    ) -> StoreResult<Option<ResearchProvenanceFingerprint>> {
        if context != "context.perturbation" {
            return Err(StoreError::AdmissionDenied);
        }
        Ok(self.current.lock().unwrap().clone())
    }
}

pub fn options(path: &Path, clock: Arc<FixtureClock>, policy: Arc<Policy>) -> StoreOptions {
    let mut options = super::options(path, clock);
    options.admission = Arc::new(Admission);
    options.backtest_policy = policy;
    options
}

pub fn worker() -> PythonPerturber {
    PythonPerturber::new(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.venv/bin/python"))
        .unwrap()
}

pub fn command(index: u32, revision: u64, key: &str) -> AdvancePerturbation {
    AdvancePerturbation {
        context: Some(context(key)),
        source_job_id: Some(JobId {
            value: format!("job.{index}"),
        }),
        context_id: "context.perturbation".to_owned(),
        expected_revision: revision,
        deadline: Some(timestamp(NOW + 30_000)),
    }
}

pub async fn seed(store: &PgJobStore, index: u32, window: u32, rejected: bool) {
    let mut submission = super::rejection::command(index);
    let Some(job_specification::Input::Backtest(input)) = &mut submission.specification.input
    else {
        unreachable!()
    };
    input.factor_spec_id = candidate(window).factor_spec_id;
    let job_id = submission.specification.job_id.clone();
    store.submit(submission).await.unwrap();
    let leased = store
        .mutate(
            &actor(),
            JobMutation::Acquire(AcquireJobLeaseRequest {
                context: Some(context(&format!("acquire.{index}"))),
                job_id: job_id.clone(),
                expected_revision: 1,
                requested_duration: Some(prost_types::Duration {
                    seconds: 30,
                    nanos: 0,
                }),
            }),
        )
        .await
        .unwrap()
        .job;
    let outcome = if rejected {
        job_outcome::Outcome::FactorRejection(FactorRejection {
            factor_spec_id: candidate(window).factor_spec_id,
            code: FactorRejectionCode::Performance as i32,
            reason: "fixture IS rejection".to_owned(),
            evidence: vec![super::artifact()],
            rejected_at: Some(timestamp(NOW)),
        })
    } else {
        job_outcome::Outcome::Success(JobSuccess {
            outputs: backtest::outputs(),
        })
    };
    store
        .mutate(
            &actor(),
            JobMutation::Complete(CompleteJobRequest {
                context: Some(context(&format!("finish.{index}"))),
                job_id,
                expected_revision: leased.revision,
                lease_id: leased.active_lease.unwrap().lease_id,
                outcome: Some(JobOutcome {
                    outcome: Some(outcome),
                }),
            }),
        )
        .await
        .unwrap();
}
