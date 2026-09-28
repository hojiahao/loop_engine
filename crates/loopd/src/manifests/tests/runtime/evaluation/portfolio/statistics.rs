//! Real report registration with authenticated full-population evidence.

use super::*;
use crate::manifests::statistics::{StatisticsDocument, StatisticsPolicy};
use crate::manifests::tests::fixture::{artifact, put};
use crate::runtime::StatisticsConfig;
use crate::store::{JobMutation, StoreError, SubmitJob};
use serde::Serialize;

pub(super) struct ReportCase {
    pub(super) portfolio: PortfolioCase,
    pub(super) job: JobSpecification,
    output: PathBuf,
}

impl ReportCase {
    pub(super) async fn new(multiple: bool) -> Self {
        let mut portfolio = PortfolioCase::with_history(true).await;
        if multiple {
            // Finish the first strategy before registering the second. The
            // report must refresh search statistics without rewriting this
            // original result or requiring its old trial count to stay current.
            portfolio
                .base
                .client()
                .await
                .execute_backtest(portfolio.request().await)
                .await
                .unwrap();
            second_strategy(&mut portfolio).await;
        }
        let policy = StatisticsPolicy {
            schema: "loop.global-statistics-policy/v1".to_owned(),
            policy_id: "policy.global".to_owned(),
            revision: "1".to_owned(),
            scope: "all-database-development-trials".to_owned(),
            minimum_sessions: 8,
            hac_lags: 1,
            pbo_blocks: 4,
        };
        let input = artifact(
            &portfolio.base.fixture.root,
            "loop.global_statistics_policy",
            "application/json",
            &serde_json::to_vec(&policy).unwrap(),
            &[],
        )
        .wire()
        .unwrap();
        let mut job = portfolio.job.clone();
        job.job_id.as_mut().unwrap().value = "job.statistics".to_owned();
        job.idempotency_key.as_mut().unwrap().value = "submit.statistics".to_owned();
        job.kind = JobKind::Report as i32;
        let Some(job_specification::Input::Backtest(backtest)) = &portfolio.job.input else {
            unreachable!()
        };
        job.input = Some(job_specification::Input::Artifact(ArtifactJobInput {
            policy: Some(PolicyReference {
                policy_id: Some(PolicyId {
                    value: policy.policy_id,
                }),
                revision: policy.revision,
                sha256: input.sha256.clone(),
            }),
            input: Some(input),
            budget: backtest.budget.clone(),
        }));
        let output = portfolio
            .base
            .fixture
            .directory
            .path()
            .join("statistics-output");
        std::fs::create_dir(&output).unwrap();
        std::fs::set_permissions(&output, std::fs::Permissions::from_mode(0o700)).unwrap();
        portfolio.statistics = Some(StatisticsConfig {
            output_store: output.clone(),
        });
        portfolio.jobs.push(job.clone());
        portfolio.restart().await;
        portfolio
            .base
            .store
            .submit(SubmitJob {
                specification: job.clone(),
                request_id: "request.statistics".to_owned(),
            })
            .await
            .unwrap();
        Self {
            portfolio,
            job,
            output,
        }
    }

    pub(super) async fn client(
        &self,
    ) -> job_service_client::JobServiceClient<tonic::transport::Channel> {
        self.portfolio
            .base
            .tls
            .timed_client(
                self.portfolio.base.address,
                Some("client"),
                Duration::from_secs(240),
            )
            .await
            .unwrap()
    }

    pub(super) async fn run_portfolios(&self) {
        for job in self
            .portfolio
            .jobs
            .iter()
            .filter(|job| job.kind == JobKind::Backtest as i32)
        {
            let id = &job.job_id.as_ref().unwrap().value;
            if self
                .portfolio
                .base
                .store
                .get(id)
                .await
                .unwrap()
                .unwrap()
                .state
                == JobState::Succeeded as i32
            {
                continue;
            }
            let record = self
                .client()
                .await
                .acquire_job_lease(AcquireJobLeaseRequest {
                    context: Some(context(&format!("acquire.{id}"))),
                    job_id: job.job_id.clone(),
                    expected_revision: 1,
                    requested_duration: Some(prost_types::Duration {
                        seconds: 180,
                        nanos: 0,
                    }),
                })
                .await
                .unwrap()
                .into_inner()
                .job
                .unwrap();
            self.client()
                .await
                .execute_backtest(ExecuteBacktestRequest {
                    context: Some(context(&format!("execute.{id}"))),
                    job_id: job.job_id.clone(),
                    lease_id: record.active_lease.unwrap().lease_id,
                    expected_revision: record.revision,
                })
                .await
                .unwrap();
        }
    }

    pub(super) async fn request(&self) -> ExecuteStatisticsRequest {
        let record = self
            .client()
            .await
            .acquire_job_lease(AcquireJobLeaseRequest {
                context: Some(context("statistics.acquire")),
                job_id: self.job.job_id.clone(),
                expected_revision: 1,
                requested_duration: Some(prost_types::Duration {
                    seconds: 180,
                    nanos: 0,
                }),
            })
            .await
            .unwrap()
            .into_inner()
            .job
            .unwrap();
        ExecuteStatisticsRequest {
            context: Some(context("statistics.execute")),
            job_id: self.job.job_id.clone(),
            lease_id: record.active_lease.unwrap().lease_id,
            expected_revision: record.revision,
        }
    }

    fn summary(&self, record: &JobRecord) -> serde_json::Value {
        let Some(job_outcome::Outcome::Success(success)) = record
            .outcome
            .as_ref()
            .and_then(|outcome| outcome.outcome.as_ref())
        else {
            panic!("report success")
        };
        let reference = &success.outputs[0].artifact_id.as_ref().unwrap().value;
        let document: StatisticsDocument =
            serde_json::from_slice(&std::fs::read(self.output.join(&reference[7..])).unwrap())
                .unwrap();
        assert!(!document.production_eligible);
        serde_json::from_slice(
            &std::fs::read(self.output.join(&document.summary.sha256[7..])).unwrap(),
        )
        .unwrap()
    }
}

#[tokio::test]
async fn report_replay() {
    let mut case = ReportCase::new(true).await;
    case.run_portfolios().await;
    let request = case.request().await;
    let first = case
        .client()
        .await
        .execute_statistics(request.clone())
        .await
        .unwrap()
        .into_inner()
        .job
        .unwrap();
    assert_eq!(first.state, JobState::Succeeded as i32);
    let summary = case.summary(&first);
    assert_eq!(summary["registered_jobs"], 4);
    assert_eq!(summary["distinct_strategies"], 2);
    assert_eq!(summary["complete_matrix"], true);
    assert_eq!(summary["pbo"]["status"], "available", "{summary}");
    let events = case
        .portfolio
        .base
        .store
        .audit_events(0, 100)
        .await
        .unwrap()
        .len();
    case.portfolio.restart().await;
    let replay = case
        .client()
        .await
        .execute_statistics(request)
        .await
        .unwrap()
        .into_inner()
        .job
        .unwrap();
    assert_eq!(first, replay);
    assert_eq!(
        case.portfolio
            .base
            .store
            .audit_events(0, 100)
            .await
            .unwrap()
            .len(),
        events
    );
    let current = case
        .portfolio
        .operator()
        .await
        .read_statistics(ReadStatisticsRequest {
            job_id: case.job.job_id.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(current.job, Some(first));
    assert!(current.report.is_some());
}

#[tokio::test]
async fn imported_statistics_denied() {
    let case = ReportCase::new(false).await;
    let request = case.request().await;
    let error = case
        .client()
        .await
        .complete_job(CompleteJobRequest {
            context: request.context,
            job_id: request.job_id,
            lease_id: request.lease_id,
            expected_revision: request.expected_revision,
            outcome: Some(JobOutcome {
                outcome: Some(job_outcome::Outcome::Success(
                    case.portfolio.base.fixture.success(),
                )),
            }),
        })
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::PermissionDenied);
    assert!(std::fs::read_dir(&case.output).unwrap().next().is_none());
}

#[tokio::test]
async fn pending_statistics() {
    let case = ReportCase::new(false).await;
    let record = case
        .client()
        .await
        .execute_statistics(case.request().await)
        .await
        .unwrap()
        .into_inner()
        .job
        .unwrap();
    let summary = case.summary(&record);
    assert_eq!(summary["registered_jobs"], 2);
    assert_eq!(summary["counted_attempts"], 2);
    assert_eq!(summary["complete_matrix"], false);
    assert_eq!(summary["pbo"]["status"], "unavailable");
    assert!(!summary["issues"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn operator_statistics_denied() {
    let case = ReportCase::new(false).await;
    let mut request = case.request().await;
    request.context = Some(case.portfolio.operator_context("operator.statistics"));
    let error = case
        .portfolio
        .operator()
        .await
        .execute_statistics(request)
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::PermissionDenied);
    assert!(std::fs::read_dir(&case.output).unwrap().next().is_none());
}

#[tokio::test]
async fn expired_statistics_denied() {
    let case = ReportCase::new(false).await;
    let request = case.request().await;
    case.portfolio
        .base
        .clock
        .0
        .fetch_add(181_000, Ordering::SeqCst);
    let error = case
        .client()
        .await
        .execute_statistics(request)
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::FailedPrecondition);
    assert!(std::fs::read_dir(&case.output).unwrap().next().is_none());
}

#[tokio::test]
async fn snapshot_fences_commit() {
    let case = ReportCase::new(false).await;
    let request = case.request().await;
    let snapshot = case
        .portfolio
        .base
        .store
        .trial_snapshot(&actor())
        .await
        .unwrap();
    let primary = Arc::new(
        PortfolioExecutor::open(
            &python(),
            &case.portfolio.base.fixture.root,
            &case.portfolio.output,
            vec![case.portfolio.pin.clone()],
            case.portfolio.base.broker.clone(),
        )
        .unwrap(),
    );
    let executor = crate::runtime::StatisticsExecutor::open(
        case.portfolio.statistics.clone().unwrap(),
        primary,
    )
    .unwrap();
    let before = case
        .portfolio
        .base
        .store
        .get("job.statistics")
        .await
        .unwrap()
        .unwrap();
    let proof = executor
        .execute(crate::runtime::StatisticsTask {
            job: &case.job,
            lease: &request.lease_id.as_ref().unwrap().value,
            started_ms: crate::store::timestamp_millis(
                before
                    .active_lease
                    .as_ref()
                    .unwrap()
                    .issued_at
                    .as_ref()
                    .unwrap(),
                false,
            )
            .unwrap(),
            snapshot: snapshot.clone(),
            portfolios: vec![],
            prior: None,
        })
        .await
        .unwrap();
    case.portfolio.request().await;
    let after = case
        .portfolio
        .base
        .store
        .trial_snapshot(&actor())
        .await
        .unwrap();
    // Acquiring a registered job changes state/revision but not the conservative
    // attempt count of one. A ledger-only comparison would miss this change.
    assert_eq!(snapshot.ledger, after.ledger);
    assert_ne!(snapshot.records, after.records);
    let events = case
        .portfolio
        .base
        .store
        .audit_events(0, 100)
        .await
        .unwrap();
    let success = proof.success().unwrap();
    let error = case
        .portfolio
        .base
        .store
        .with_statistics_evidence(Arc::new(proof))
        .mutate(
            &actor(),
            JobMutation::Complete(CompleteJobRequest {
                context: request.context,
                job_id: request.job_id,
                lease_id: request.lease_id,
                expected_revision: request.expected_revision,
                outcome: Some(JobOutcome {
                    outcome: Some(job_outcome::Outcome::Success(success)),
                }),
            }),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, StoreError::StaleTrials));
    assert_eq!(
        case.portfolio
            .base
            .store
            .get("job.statistics")
            .await
            .unwrap(),
        Some(before)
    );
    assert_eq!(
        case.portfolio
            .base
            .store
            .audit_events(0, 100)
            .await
            .unwrap(),
        events
    );
}

async fn second_strategy(case: &mut PortfolioCase) {
    let mut source = case.base.fixture.job.specification.clone();
    source.job_id.as_mut().unwrap().value = "job.second_factor".to_owned();
    source.idempotency_key.as_mut().unwrap().value = "submit.second_factor".to_owned();
    let Some(job_specification::Input::FactorEvaluation(input)) = &mut source.input else {
        unreachable!()
    };
    let factor = input.factor.as_mut().unwrap();
    factor.direction = FactorDirection::LowerIsBetter as i32;
    factor.factor_spec_id.as_mut().unwrap().value = format!(
        "sha256:{}",
        loop_protocol::job::factor_identity_hash(factor)
            .unwrap()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    case.jobs.push(source.clone());
    let f = &case.base.fixture;
    let resolver = Arc::new(
        EvaluationResolver::open(
            &f.root,
            case.jobs
                .iter()
                .filter(|job| job.kind == JobKind::FactorEvaluation as i32)
                .map(|job| EvaluationPin {
                    job_id: job.job_id.as_ref().unwrap().value.clone(),
                    context: f.json(&f.context),
                })
                .collect(),
        )
        .unwrap(),
    );
    case.base.executor =
        Arc::new(FactorExecutor::open(&python(), &case.base.output, resolver.clone()).unwrap());
    case.restart().await;
    case.base
        .store
        .submit(SubmitJob {
            specification: source.clone(),
            request_id: "request.second_factor".to_owned(),
        })
        .await
        .unwrap();
    let leased = case
        .base
        .client()
        .await
        .acquire_job_lease(AcquireJobLeaseRequest {
            context: Some(context("second.acquire")),
            job_id: source.job_id.clone(),
            expected_revision: 1,
            requested_duration: Some(prost_types::Duration {
                seconds: 120,
                nanos: 0,
            }),
        })
        .await
        .unwrap()
        .into_inner()
        .job
        .unwrap();
    case.base
        .client()
        .await
        .evaluate_factor(EvaluateFactorRequest {
            context: Some(context("second.evaluate")),
            job_id: source.job_id.clone(),
            lease_id: leased.active_lease.unwrap().lease_id,
            expected_revision: leased.revision,
        })
        .await
        .unwrap();
    let trial = case
        .base
        .store
        .factor_trials(&actor(), &source.run_id.as_ref().unwrap().value, "", 10)
        .await
        .unwrap()
        .into_iter()
        .find(|trial| trial.job_id == "job.second_factor")
        .unwrap()
        .evaluation
        .unwrap();
    let evaluated = trial.result.as_ref().unwrap();
    let work = resolver
        .prepare(&source, evaluated.lease_id.as_ref().unwrap())
        .await
        .unwrap()
        .work;
    let f = &case.base.fixture;
    let copy = |reference: &ArtifactRef| {
        put(
            &f.root,
            &std::fs::read(
                case.base
                    .output
                    .join(&reference.artifact_id.as_ref().unwrap().value[7..]),
            )
            .unwrap(),
        )
    };
    // Materializer requires declared field order; changing values retains it by
    // round-tripping through this explicit schema rather than a sorted map.
    #[derive(Serialize, Deserialize)]
    struct Recipe {
        schema: String,
        evaluation_work: ObjectRef,
        evaluation_result: ObjectRef,
        factor_values: ObjectRef,
        execution_tape: ObjectRef,
        policies: std::collections::BTreeMap<String, model::PolicyDocument>,
    }
    let mut recipe: Recipe =
        serde_json::from_slice(&std::fs::read(f.path(&case.pin.request)).unwrap()).unwrap();
    recipe.evaluation_work = put(&f.root, &work.encode_to_vec());
    recipe.evaluation_result = copy(evaluated.manifest.as_ref().unwrap());
    recipe.factor_values = copy(evaluated.values.as_ref().unwrap());
    let request = f.json(&recipe);
    let mut backtest: model::Backtest =
        serde_json::from_slice(&std::fs::read(f.path(&case.pin.specification)).unwrap()).unwrap();
    let mut manifest: model::Context =
        serde_json::from_slice(&std::fs::read(f.path(&backtest.context)).unwrap()).unwrap();
    let mut configuration: model::Configuration =
        serde_json::from_slice(&std::fs::read(f.path(&manifest.configuration)).unwrap()).unwrap();
    configuration.portfolio_request = Some(request.clone());
    manifest.configuration = f.json(&configuration);
    backtest.context = f.json(&manifest);
    backtest.backtest_id = "backtest.second".to_owned();
    let factor = work.factor.as_ref().unwrap();
    backtest.factor = f.json(&model::Factor {
        schema: "loop.factor-manifest/v1".to_owned(),
        factor_spec_id: factor.factor_spec_id.as_ref().unwrap().value.clone(),
        specification: put(
            &f.root,
            &loop_protocol::job::factor_identity_bytes(factor).unwrap(),
        ),
        expression: put(&f.root, &factor.expression.as_ref().unwrap().canonical_json),
    });
    let mut job = case.job.clone();
    job.job_id.as_mut().unwrap().value = "job.second_portfolio".to_owned();
    job.idempotency_key.as_mut().unwrap().value = "submit.second_portfolio".to_owned();
    let Some(job_specification::Input::Backtest(input)) = &mut job.input else {
        unreachable!()
    };
    input.factor_spec_id = factor.factor_spec_id.clone();
    input.provenance = Some(
        manifest
            .provenance(*f.registry.identity().as_bytes())
            .unwrap(),
    );
    case.extra_pins.push(PortfolioPin {
        job_id: job.job_id.as_ref().unwrap().value.clone(),
        specification: f.json(&backtest),
        request,
        evaluation_job_id: "job.second_factor".to_owned(),
    });
    case.jobs.push(job.clone());
    case.restart().await;
    case.base
        .store
        .submit(SubmitJob {
            specification: job,
            request_id: "request.second_portfolio".to_owned(),
        })
        .await
        .unwrap();
}
