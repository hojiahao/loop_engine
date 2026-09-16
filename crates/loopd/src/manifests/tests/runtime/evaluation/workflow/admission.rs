//! Genuine factor values plus explicitly synthetic portfolio/review fixtures.
//! This proves wiring, not an implemented portfolio producer or profitability.
use super::*;
use crate::manifests::tests::fixture::{artifact, put};
use crate::store::{DecideFactor, EvaluationTrial};

struct Linked {
    case: Case,
    evaluation: JobSpecification,
    numerical: EvaluationTrial,
    jobs: Vec<JobSpecification>,
}

impl Linked {
    async fn new() -> Self {
        let case = Case::open(fixture_with_minimum(false, 6666).await).await;
        case.client()
            .await
            .evaluate_factor(case.request().await)
            .await
            .unwrap();
        let numerical = trials(&case).await.remove(0).evaluation.unwrap();
        let evaluation = case.fixture.job.specification.clone();
        Self {
            case,
            jobs: vec![evaluation.clone()],
            evaluation,
            numerical,
        }
    }

    async fn backtest(&mut self, id: &str, accepted: bool, reported_valid: u64) -> PgJobStore {
        let f = &mut self.case.fixture;
        let computed = self.numerical.result.as_ref().unwrap();
        let Some(job_specification::Input::FactorEvaluation(input)) = &self.evaluation.input else {
            unreachable!()
        };
        let factor = input.factor.as_ref().unwrap();
        let mut config: model::Configuration =
            serde_json::from_slice(&std::fs::read(f.path(&f.context.configuration)).unwrap())
                .unwrap();
        config.backtest_engine_version = "synthetic-producer.1".to_owned();
        f.context.configuration = f.json(&config);
        let context_ref = f.json(&f.context);
        let factor_ref = f.json(&model::Factor {
            schema: "loop.factor-manifest/v1".to_owned(),
            factor_spec_id: factor.factor_spec_id.as_ref().unwrap().value.clone(),
            specification: put(
                &f.root,
                &loop_protocol::job::factor_identity_bytes(factor).unwrap(),
            ),
            expression: put(&f.root, &factor.expression.as_ref().unwrap().canonical_json),
        });
        let data: model::Dataset =
            serde_json::from_slice(&std::fs::read(f.path(&f.context.data)).unwrap()).unwrap();
        let spec = f.json(&model::Backtest {
            schema: "loop.backtest-spec/v1".to_owned(),
            backtest_id: format!("backtest.{id}"),
            context: context_ref.clone(),
            factor: factor_ref,
            engine: model::Engine::PrimaryCrossSectional,
            engine_version: "synthetic-producer.1".to_owned(),
            sample: data.sample,
            return_definition: "simple_nav_return".to_owned(),
            deterministic_seed: format!("sha256:{}", "01".repeat(32)),
        });
        let values = computed.values.as_ref().unwrap();
        let copy = |id: &str| {
            put(
                &f.root,
                &std::fs::read(self.case.output.join(&id[7..])).unwrap(),
            )
        };
        let values_ref = copy(&values.artifact_id.as_ref().unwrap().value);
        let manifest_ref = copy(
            &computed
                .manifest
                .as_ref()
                .unwrap()
                .artifact_id
                .as_ref()
                .unwrap()
                .value,
        );
        let value_schema = values.schema.as_ref().unwrap();
        let schema_id = format!(
            "sha256:{}",
            value_schema
                .schema_sha256
                .as_ref()
                .unwrap()
                .value
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        let schema_ref = copy(&schema_id);
        let now = SystemClock.now_millis().unwrap();
        f.result.job_id = id.to_owned();
        f.result.specification = spec.clone();
        f.result.completed_at_ms = now;
        f.result.artifacts.factor_values = model::Artifact {
            object: values_ref,
            schema: model::SchemaRef {
                name: value_schema.name.clone(),
                version: value_schema.version,
                document: schema_ref,
            },
            media_type: values.media_type.clone(),
            created_at_ms: now,
        };
        f.result_artifact = artifact(
            &f.root,
            "loop.backtest_result",
            "application/json",
            &serde_json::to_vec(&f.result).unwrap(),
            &[],
        );
        f.result_artifact.created_at_ms = now;
        f.review.schema = "loop.admission-review/v2".to_owned();
        f.review.job_id = id.to_owned();
        f.review.factor_spec_id = factor.factor_spec_id.as_ref().unwrap().value.clone();
        f.review.result = f.result_artifact.object.clone();
        f.review.policy = config
            .policies
            .iter()
            .find(|p| p.policy_id == "policy.evaluation")
            .unwrap()
            .clone();
        f.review.eligible_observations = computed.eligible_observations;
        f.review.valid_observations = reported_valid;
        f.review.minimum_coverage_bps = self.numerical.minimum_coverage_bps;
        f.review.semantic_accepted = accepted;
        f.review.evaluation = Some(model::EvaluationSource {
            job_id: self.evaluation.job_id.as_ref().unwrap().value.clone(),
            manifest: manifest_ref,
        });
        let schema = model::SchemaDocument {
            schema: "loop.artifact-schema/v1".to_owned(),
            name: "loop.admission_review".to_owned(),
            version: 2,
            media_type: "application/json".to_owned(),
            columns: vec![],
        };
        let review = model::Artifact {
            object: f.json(&f.review),
            schema: model::SchemaRef {
                name: schema.name.clone(),
                version: 2,
                document: f.json(&schema),
            },
            media_type: schema.media_type,
            created_at_ms: now,
        };
        f.catalog.contexts = vec![model::ContextEntry {
            context_id: context_ref.sha256.clone(),
            manifest: context_ref,
        }];
        f.catalog.backtests = vec![model::BacktestEntry {
            job_id: id.to_owned(),
            specification: spec,
            result: Some(f.result_artifact.clone()),
            review: Some(review),
        }];
        f.job.specification = self.evaluation.clone();
        f.job.specification.job_id.as_mut().unwrap().value = id.to_owned();
        f.job.specification.idempotency_key.as_mut().unwrap().value = format!("submit.{id}");
        f.job.specification.kind = JobKind::Backtest as i32;
        f.job.specification.input = Some(job_specification::Input::Backtest(BacktestJobInput {
            factor_spec_id: factor.factor_spec_id.clone(),
            dataset: input.dataset.clone(),
            return_definition: ReturnDefinition::SimpleNavReturn as i32,
            provenance: Some(
                f.context
                    .provenance(*f.registry.identity().as_bytes())
                    .unwrap(),
            ),
            deterministic_seed: input.deterministic_seed.clone(),
            budget: input.budget.clone(),
        }));
        self.jobs.push(f.job.specification.clone());
        let mut options = support::base_options(&f.directory.path().join("state"));
        options.clock = self.case.clock.clone();
        options.admission = Arc::new(Pinned(self.jobs.clone()));
        options.backtest_policy = f.policy().await;
        let store = PgJobStore::open(options).await.unwrap();
        store.submit(f.job.clone()).await.unwrap();
        let leased = store
            .mutate(
                &actor(),
                crate::store::JobMutation::Acquire(AcquireJobLeaseRequest {
                    context: Some(context(&format!("acquire.{id}"))),
                    job_id: f.job.specification.job_id.clone(),
                    expected_revision: 1,
                    requested_duration: Some(prost_types::Duration {
                        seconds: 120,
                        nanos: 0,
                    }),
                }),
            )
            .await
            .unwrap()
            .job;
        store
            .mutate(
                &actor(),
                crate::store::JobMutation::Complete(CompleteJobRequest {
                    context: Some(context(&format!("complete.{id}"))),
                    job_id: f.job.specification.job_id.clone(),
                    expected_revision: leased.revision,
                    lease_id: leased.active_lease.unwrap().lease_id,
                    outcome: Some(JobOutcome {
                        outcome: Some(job_outcome::Outcome::Success(f.success())),
                    }),
                }),
            )
            .await
            .unwrap();
        store
    }

    fn command(&self, revision: u64, key: &str) -> DecideFactor {
        DecideFactor {
            context: Some(context(key)),
            source_job_id: self.case.fixture.job.specification.job_id.clone(),
            context_id: self.case.fixture.context_id(),
            expected_revision: revision,
            reason: "synthetic portfolio fixture following genuine numerical evaluation".to_owned(),
            deadline: Some(support::timestamp(
                SystemClock.now_millis().unwrap() + 30_000,
            )),
            ..Default::default()
        }
    }
}

#[tokio::test]
// Scenario: linked review uses shared readmission.
async fn linked_review_shared() {
    let mut linked = Linked::new().await;
    let store = linked.backtest("job.review1", false, 4).await;
    let rejected = store
        .decide_factor(&actor(), linked.command(0, "review.reject"))
        .await
        .unwrap();
    assert_eq!(rejected.rejection_code, "semantic_review");
    assert_eq!(rejected.states[0].admissions, 0);
    store.close().await;
    let store = linked.backtest("job.review2", true, 4).await;
    let command = linked.command(1, "review.readmit");
    let accepted = store
        .decide_factor(&actor(), command.clone())
        .await
        .unwrap();
    assert_eq!(accepted.states[0].status, "admitted");
    assert_eq!(accepted.states[0].revision, 2);
    assert_eq!(accepted.states[0].admissions, 1);
    assert!(
        store
            .decide_factor(&actor(), command)
            .await
            .unwrap()
            .replayed
    );
    let trials = store
        .factor_trials(
            &actor(),
            &linked.evaluation.run_id.as_ref().unwrap().value,
            "",
            10,
        )
        .await
        .unwrap();
    assert_eq!(trials.len(), 3);
    assert_eq!(trials.iter().filter(|t| t.evaluation.is_some()).count(), 1);
    loop_core::audit::verify_audit_chain(&store.audit_events(0, 100).await.unwrap()).unwrap();
    store.close().await;
}

#[tokio::test]
// Scenario: review cannot invent coverage.
async fn review_coverage() {
    let mut linked = Linked::new().await;
    let store = linked.backtest("job.forged", true, 6).await;
    assert!(matches!(
        store
            .decide_factor(&actor(), linked.command(0, "review.forged"))
            .await,
        Err(StoreError::Corrupt("admission numerical lineage"))
    ));
    store.close().await;
}
