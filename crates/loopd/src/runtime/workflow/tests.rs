//! Real immutable-file guards must survive the complete start transaction.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use loop_protocol::wire::{
    discovery::v1 as discovery, research::v1 as research, runs::v1 as runs, v1,
};
use prost::Message;
use serde::Serialize;

use super::*;
use crate::manifests::numerical::NumericalResolver;
use crate::store::{AdmissionPolicy, Clock, PgJobStore};
use crate::test_support as support;

#[derive(Serialize)]
struct Recipe {
    schema: &'static str,
    id: &'static str,
    revision: &'static str,
    evaluation_context: ObjectRef,
    direction: &'static str,
    deterministic_seed: String,
    execution_tape: ObjectRef,
    policies: BTreeMap<String, String>,
    hypothesis: &'static str,
    required_observations: Vec<&'static str>,
    falsification_criteria: Vec<&'static str>,
}

struct DriftClock {
    reads: AtomicUsize,
    path: PathBuf,
    changed: Vec<u8>,
}

impl Clock for DriftClock {
    fn now_millis(&self) -> StoreResult<i64> {
        // Observe-clock and the pre-write fence see intact evidence. The final
        // fence runs after the job, run, transition and receipts were inserted.
        if self.reads.fetch_add(1, Ordering::SeqCst) == 2 {
            replace(&self.path, &self.changed)?;
        }
        Ok(support::NOW)
    }
}

struct FrozenAdmission(runs::RunSpecification);

impl AdmissionPolicy for FrozenAdmission {
    fn authorize_run(&self, actor: &v1::Actor, run: &runs::RunSpecification) -> StoreResult<()> {
        if run == &self.0 && run.owner.as_ref() == Some(actor) {
            Ok(())
        } else {
            Err(StoreError::AdmissionDenied)
        }
    }

    fn validate_submission(&self, job: &v1::JobSpecification) -> StoreResult<()> {
        let Some(job_specification::Input::Discovery(actual)) = &job.input else {
            return Err(StoreError::AdmissionDenied);
        };
        let expected = self.0.discovery.as_ref().unwrap();
        if job.kind != v1::JobKind::Discovery as i32
            || job.run_id != self.0.run_id
            || job.submitted_by != self.0.executor
            || job.protocol_selection != self.0.protocol_selection
            || actual.dataset != expected.dataset
            || actual.research_policy != expected.research_policy
            || actual.maker_model != expected.maker_model
            || actual.checker_model != expected.checker_model
            || actual.maximum_candidates != expected.maximum_candidates
            || actual.budget.as_ref().map(Message::encode_to_vec)
                != expected.budget.as_ref().map(Message::encode_to_vec)
        {
            return Err(StoreError::AdmissionDenied);
        }
        loop_protocol::job::validate_job_specification(job)?;
        Ok(())
    }
}

fn replace(path: &Path, bytes: &[u8]) -> StoreResult<()> {
    let mut file = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path)
        .map_err(|error| StoreError::Io(error.error))?;
    Ok(())
}

fn actor(name: &str, kind: v1::ActorKind) -> v1::Actor {
    v1::Actor {
        actor_id: Some(v1::ActorId { value: name.into() }),
        authenticated_subject: format!("fixture:{name}"),
        display_name: name.into(),
        kind: kind as i32,
    }
}

fn specification(
    dataset: v1::DevelopmentDatasetReference,
    recipe: v1::PolicyReference,
) -> runs::RunSpecification {
    let (_, job_specification::Input::Discovery(mut input)) = support::research::inputs().remove(0)
    else {
        panic!("discovery fixture");
    };
    input.dataset = Some(dataset);
    input.maximum_candidates = 1;
    let budget = input.budget.as_mut().unwrap();
    budget.maximum_steps = 1;
    budget.maximum_wall_time = Some(prost_types::Duration {
        seconds: 120,
        nanos: 0,
    });
    let protocol = support::command(1).specification.protocol_selection;
    let numerical = research::ResearchJobBudget {
        maximum_steps: 1,
        maximum_cost: Some(v1::Money {
            currency_code: "USD".into(),
            amount: Some(v1::ExactDecimal { value: "0".into() }),
        }),
        maximum_wall_time: Some(prost_types::Duration {
            seconds: 180,
            nanos: 0,
        }),
        ..Default::default()
    };
    runs::RunSpecification {
        plan: input.research_policy.clone(),
        run_id: Some(v1::RunId {
            value: "run.guarded".into(),
        }),
        owner: Some(actor("human.guard", v1::ActorKind::Human)),
        executor: Some(actor("agent.guard", v1::ActorKind::Agent)),
        maximum_rounds: 1,
        protocol_selection: protocol.clone(),
        budget: Some(runs::RunBudget {
            maximum_steps: 3,
            maximum_input_tokens: budget.maximum_input_tokens,
            maximum_output_tokens: budget.maximum_output_tokens,
            maximum_cost: budget.maximum_cost.clone(),
            maximum_wall_time: Some(prost_types::Duration {
                seconds: 600,
                nanos: 0,
            }),
        }),
        discovery: Some(discovery::DiscoveryJobInput {
            dataset: input.dataset,
            research_policy: input.research_policy,
            maker_model: input.maker_model,
            checker_model: input.checker_model,
            maximum_candidates: 1,
            budget: Some(discovery::DiscoveryJobBudget {
                maximum_steps: budget.maximum_steps,
                maximum_input_tokens: budget.maximum_input_tokens,
                maximum_output_tokens: budget.maximum_output_tokens,
                maximum_cost: budget.maximum_cost.clone(),
                maximum_wall_time: budget.maximum_wall_time,
            }),
        }),
        numerical: Some(runs::NumericalWorkflow {
            research_executor: Some(actor("service.guard", v1::ActorKind::Service)),
            recipe: Some(recipe),
            evaluation_budget: Some(numerical.clone()),
            portfolio_budget: Some(numerical),
            protocol_selection: protocol,
            perturbation_budget: None,
        }),
        feedback_policy: None,
    }
}

#[tokio::test]
async fn start_guard_rollback() {
    let fixture = crate::manifests::tests::run_fixture().await;
    let (dataset, execution_tape) = fixture.numerical_data();
    let recipe = Recipe {
        schema: "loop.run-numerical/v1",
        id: "recipe.guard",
        revision: "1",
        evaluation_context: fixture.json(&fixture.context),
        direction: "higher_is_better",
        deterministic_seed: format!("sha256:{}", "01".repeat(32)),
        execution_tape,
        policies: [
            "universe",
            "data",
            "calendar",
            "preprocess",
            "neutralization",
            "portfolio",
            "execution",
            "cost",
            "evaluation",
        ]
        .into_iter()
        .map(|role| (format!("{role}_policy"), format!("policy.{role}")))
        .collect(),
        hypothesis: "The synthetic close ranks the declared cross section.",
        required_observations: vec!["Causal closing prices."],
        falsification_criteria: vec!["Insufficient frozen coverage."],
    };
    let object = fixture.json(&recipe);
    let reference = v1::PolicyReference {
        policy_id: Some(v1::PolicyId {
            value: recipe.id.into(),
        }),
        revision: recipe.revision.into(),
        sha256: Some(v1::Sha256Digest {
            value: object.digest().unwrap().to_vec(),
        }),
    };
    let resolver = NumericalResolver::open(&fixture.root, &fixture.root).unwrap();
    let frozen = resolver
        .prepare(&reference, &object, &dataset)
        .await
        .unwrap();
    let prepared = PreparedRun::new(Arc::new(frozen)).unwrap();
    let specification = specification(dataset.clone(), reference.clone());
    let owner = specification.owner.as_ref().unwrap();
    let mut context = support::context("start.guarded");
    context.actor = Some(owner.clone());
    let request = runs::StartRunRequest {
        context: Some(context),
        plan: specification.plan.clone(),
        workflow_version: 2,
    };
    let original = std::fs::read(fixture.path(&object)).unwrap();
    let mut changed = original.clone();
    changed[0] ^= 1;
    let clock = Arc::new(DriftClock {
        reads: AtomicUsize::new(0),
        path: fixture.path(&object),
        changed,
    });
    let mut options = support::base_options(&fixture.directory.path().join("state"));
    options.clock = clock.clone();
    options.admission = Arc::new(FrozenAdmission(specification.clone()));
    let store = PgJobStore::open(options).await.unwrap();
    assert!(matches!(
        store
            .start_numerical(owner, &request, &specification, &prepared)
            .await,
        Err(StoreError::Corrupt("artifact changed after verification"))
    ));
    assert_eq!(clock.reads.load(Ordering::SeqCst), 3);
    let mut connection = support::connection(&fixture.directory).await;
    let counts: (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM jobs),
                (SELECT count(*) FROM research_runs),
                (SELECT count(*) FROM run_transitions),
                (SELECT count(*) FROM command_receipts),
                (SELECT count(*) FROM audit_events)",
    )
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(counts, (0, 0, 0, 0, 0));

    // Restoring bytes requires a newly resolved guard; the stale proof remains
    // invalid. The failed transaction consumed neither run ID nor request key.
    replace(&fixture.path(&object), &original).unwrap();
    assert!(prepared.check_for(&specification).is_err());
    let frozen = resolver
        .prepare(&reference, &object, &dataset)
        .await
        .unwrap();
    let restored = PreparedRun::new(Arc::new(frozen)).unwrap();
    let run = store
        .start_numerical(owner, &request, &specification, &restored)
        .await
        .unwrap();
    assert_eq!(run.revision, 1);
    assert_eq!(run.reserved_steps, 1);
    assert_eq!(run.status, runs::RunStatus::Active as i32);
    drop(connection);
    store.close().await;
}
