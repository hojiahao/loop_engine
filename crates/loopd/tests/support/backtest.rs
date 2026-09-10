use std::path::Path;
use std::sync::{Arc, Mutex};

use loop_protocol::wire::jobs::v1::{AcquireJobLeaseRequest, CompleteJobRequest};
use loop_protocol::wire::v1::*;
use loopd::store::{
    AdmissionPolicy, BacktestPolicy, ExportBacktest, JobMutation, JobRepository, PgJobStore,
    StoreError, StoreOptions, StoreResult,
};

use super::{FixtureAdmission, FixtureClock, NOW, actor, context, digest, research, timestamp};

pub struct Admission;

impl AdmissionPolicy for Admission {
    fn validate_submission(&self, job: &JobSpecification) -> StoreResult<()> {
        research::Admission.validate_submission(job)
    }

    fn authorize_job_command(
        &self,
        operation: &str,
        actor: &Actor,
        job: &JobRecord,
    ) -> StoreResult<()> {
        FixtureAdmission.authorize_job_command(operation, actor, job)
    }
}

/// Deliberate in-memory fixture, not a production artifact materializer.
pub struct Policy {
    pub result: Mutex<BacktestResult>,
    pub current: Mutex<Option<ResearchProvenanceFingerprint>>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            result: Mutex::new(result()),
            current: Mutex::new(result().provenance),
        }
    }
}

impl BacktestPolicy for Policy {
    fn resolve_result(
        &self,
        job: &JobSpecification,
        _: &JobSuccess,
    ) -> StoreResult<BacktestResult> {
        research::Admission.validate_submission(job)?;
        Ok(self.result.lock().unwrap().clone())
    }

    fn resolve_current(
        &self,
        _: &Actor,
        _: &JobSpecification,
        context: &str,
    ) -> StoreResult<Option<ResearchProvenanceFingerprint>> {
        if context != "context.fixture" {
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

pub fn outputs() -> Vec<ArtifactRef> {
    (31..=39)
        .map(|byte| {
            let mut artifact = super::artifact();
            let hex = format!("{byte:02x}").repeat(32);
            artifact.artifact_id.as_mut().unwrap().value = format!("sha256:{hex}");
            artifact.uri = format!("artifact://sha256/{hex}");
            artifact.sha256 = Some(digest(byte));
            artifact
        })
        .collect()
}

pub fn result() -> BacktestResult {
    let (_, job_specification::Input::Backtest(input)) = research::inputs().remove(2) else {
        unreachable!()
    };
    let outputs = outputs();
    BacktestResult {
        backtest_id: Some(BacktestId {
            value: "backtest.fixture".to_owned(),
        }),
        engine: BacktestEngineKind::PrimaryCrossSectional as i32,
        engine_version: "fixture.1".to_owned(),
        provenance: input.provenance,
        metrics: vec![BacktestMetric {
            name: "rank_ic".to_owned(),
            value: Some(ExactDecimal {
                value: "0.031".to_owned(),
            }),
            unit: "dimensionless".to_owned(),
            estimator: "spearman.v1".to_owned(),
        }],
        artifacts: Some(BacktestArtifacts {
            factor_values: Some(outputs[0].clone()),
            target_positions: Some(outputs[1].clone()),
            orders: Some(outputs[2].clone()),
            fills: Some(outputs[3].clone()),
            nav: Some(outputs[4].clone()),
            simple_returns: Some(outputs[5].clone()),
            risk_exposures: Some(outputs[6].clone()),
            cost_ledger: Some(outputs[7].clone()),
        }),
        result_manifest_sha256: Some(digest(39)),
        completed_at: Some(timestamp(NOW)),
    }
}

pub fn export(key: &str) -> ExportBacktest {
    ExportBacktest {
        context: Some(context(key)),
        job_id: Some(JobId {
            value: "job.1".to_owned(),
        }),
        context_id: "context.fixture".to_owned(),
        deadline: Some(timestamp(NOW + 30_000)),
    }
}

pub async fn seed(store: &PgJobStore) -> CompleteJobRequest {
    let mut command = super::command(1);
    let (kind, input) = research::inputs().remove(2);
    command.specification.kind = kind as i32;
    command.specification.input = Some(input);
    store.submit(command).await.unwrap();
    let leased = store
        .mutate(
            &actor(),
            JobMutation::Acquire(AcquireJobLeaseRequest {
                context: Some(context("acquire")),
                job_id: Some(JobId {
                    value: "job.1".to_owned(),
                }),
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
    CompleteJobRequest {
        context: Some(context("finish")),
        job_id: Some(JobId {
            value: "job.1".to_owned(),
        }),
        expected_revision: leased.revision,
        lease_id: leased.active_lease.unwrap().lease_id,
        outcome: Some(JobOutcome {
            outcome: Some(job_outcome::Outcome::Success(JobSuccess {
                outputs: outputs(),
            })),
        }),
    }
}
