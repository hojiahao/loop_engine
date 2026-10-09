mod processes;

use super::*;
use crate::store::{
    AdmissionPolicy, JobRepository, ModelControl, ModelStepCommand, StoreOptions, SubmitJob,
};
use crate::test_support as fixture;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use loop_protocol::wire::provider::v1::InvokeModelRequest;
use loop_protocol::wire::runs::v1::RunBudget;
use loop_protocol::wire::v1::*;
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

struct Admission;
impl AdmissionPolicy for Admission {
    fn validate_submission(&self, job: &JobSpecification) -> StoreResult<()> {
        if job.kind == JobKind::Discovery as i32 && job.submitted_by.as_ref() == Some(&executor()) {
            Ok(())
        } else {
            Err(StoreError::AdmissionDenied)
        }
    }
    fn authorize_job_command(&self, _: &str, actor: &Actor, _: &JobRecord) -> StoreResult<()> {
        if *actor == executor() {
            Ok(())
        } else {
            Err(StoreError::AdmissionDenied)
        }
    }
    fn authorize_run(&self, actor: &Actor, specification: &RunSpecification) -> StoreResult<()> {
        if *actor == owner() && specification.owner.as_ref() == Some(actor) {
            Ok(())
        } else {
            Err(StoreError::AdmissionDenied)
        }
    }
}

fn owner() -> Actor {
    Actor {
        actor_id: Some(ActorId {
            value: "human.run-owner".into(),
        }),
        kind: ActorKind::Human as i32,
        display_name: "Run owner".into(),
        authenticated_subject: "human:run-owner".into(),
    }
}

fn executor() -> Actor {
    Actor {
        actor_id: Some(ActorId {
            value: "agent.run-executor".into(),
        }),
        kind: ActorKind::Agent as i32,
        display_name: "Run executor".into(),
        authenticated_subject: "agent:run-executor".into(),
    }
}

fn context(key: &str, actor: Actor) -> CommandContext {
    let mut context = fixture::context(key);
    context.actor = Some(actor);
    context
}

fn invocation() -> InvokeModelRequest {
    let value: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/contracts/protocol/v1/harness_invocation.json"
    ))
    .unwrap();
    let mut request = InvokeModelRequest::decode(
        STANDARD
            .decode(value["request_base64"].as_str().unwrap())
            .unwrap()
            .as_slice(),
    )
    .unwrap();
    request.context = Some(context("model.original", executor()));
    let input = request.invocation.as_mut().unwrap();
    input.request_id = request.context.as_ref().unwrap().request_id.clone();
    input.model.as_mut().unwrap().resolved_at = Some(fixture::timestamp(fixture::NOW - 5_000));
    input
        .model
        .as_mut()
        .unwrap()
        .capabilities
        .as_mut()
        .unwrap()
        .context_window_tokens = 65_536;
    input.budget.as_mut().unwrap().maximum_wall_time = Some(prost_types::Duration {
        seconds: 10,
        nanos: 0,
    });
    request
}

fn specification() -> RunSpecification {
    let (_, job_specification::Input::Discovery(input)) = fixture::research::inputs().remove(0)
    else {
        panic!("discovery fixture")
    };
    let model = invocation().invocation.unwrap().model;
    RunSpecification {
        plan: input.research_policy.clone(),
        run_id: Some(RunId {
            value: "run.managed".into(),
        }),
        owner: Some(owner()),
        executor: Some(executor()),
        maximum_rounds: 2,
        protocol_selection: fixture::command(1).specification.protocol_selection,
        discovery: Some(discovery::DiscoveryJobInput {
            dataset: input.dataset,
            research_policy: input.research_policy,
            maker_model: model.clone(),
            checker_model: model,
            maximum_candidates: 1,
            budget: Some(discovery::DiscoveryJobBudget {
                maximum_steps: 1,
                maximum_input_tokens: 1024,
                maximum_output_tokens: 64,
                maximum_cost: Some(storage::money(10_000_000)),
                maximum_wall_time: Some(prost_types::Duration {
                    seconds: 120,
                    nanos: 0,
                }),
            }),
        }),
        budget: Some(RunBudget {
            maximum_steps: 2,
            maximum_input_tokens: 2048,
            maximum_output_tokens: 128,
            maximum_cost: Some(storage::money(20_000_000)),
            maximum_wall_time: Some(prost_types::Duration {
                seconds: 3600,
                nanos: 0,
            }),
        }),
    }
}

fn start() -> StartRunRequest {
    StartRunRequest {
        context: Some(context("run.start", owner())),
        plan: specification().plan,
    }
}

fn step(revision: u64) -> StepRunRequest {
    StepRunRequest {
        context: Some(context(&format!("run.step.{revision}"), owner())),
        run_id: specification().run_id,
        expected_revision: revision,
    }
}

fn options(path: &Path, clock: Arc<fixture::FixtureClock>) -> StoreOptions {
    let mut options = fixture::options(path, clock);
    options.admission = Arc::new(Admission);
    options
}

async fn setup() -> (tempfile::TempDir, PgJobStore, Arc<fixture::FixtureClock>) {
    let directory = tempfile::tempdir().unwrap();
    let clock = Arc::new(fixture::FixtureClock(AtomicI64::new(fixture::NOW)));
    let store = PgJobStore::open(options(&directory.path().join("state"), clock.clone()))
        .await
        .unwrap();
    (directory, store, clock)
}

fn model_command(job: &JobRecord, key: &str) -> ModelStepCommand {
    ModelStepCommand {
        context: Some(context(key, executor())),
        job_id: job.specification.as_ref().unwrap().job_id.clone(),
        expected_revision: job.revision,
        lease_id: job
            .active_lease
            .as_ref()
            .and_then(|lease| lease.lease_id.clone()),
        ordinal: 0,
    }
}

async fn finish(store: &PgJobStore) {
    let snapshot = store.read_run(&owner(), "run.managed").await.unwrap();
    let job = snapshot.current_job;
    let id = &job
        .specification
        .as_ref()
        .unwrap()
        .job_id
        .as_ref()
        .unwrap()
        .value;
    let mut request = invocation();
    request.context = Some(context(&format!("invoke.{id}"), executor()));
    request.invocation.as_mut().unwrap().request_id =
        request.context.as_ref().unwrap().request_id.clone();
    let reserved = store
        .reserve_model(
            &executor(),
            model_command(&job, &format!("reserve.{id}")),
            request,
        )
        .await
        .unwrap();
    let dispatched = store
        .dispatch_model(
            &executor(),
            model_command(&reserved.job, &format!("dispatch.{id}")),
        )
        .await
        .unwrap()
        .step;
    let input = dispatched.request.invocation.as_ref().unwrap();
    let response = ModelResponse {
        request_id: input.request_id.clone(),
        resolution_id: input.model.as_ref().unwrap().resolution_id.clone(),
        content: vec![ContentBlock {
            content: Some(content_block::Content::Text(TextContent {
                text: "candidate".into(),
            })),
        }],
        finish_reason: ModelFinishReason::Stop as i32,
        usage: Some(ModelUsage {
            input_tokens: 12,
            output_tokens: 8,
            ..Default::default()
        }),
    };
    store
        .finish_model(
            &executor(),
            model_command(&dispatched.job, &format!("finish.{id}")),
            response,
            JobOutcome {
                outcome: Some(job_outcome::Outcome::Success(JobSuccess {
                    outputs: vec![],
                })),
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn two_rounds() {
    let (_directory, store, _) = setup().await;
    let first = store
        .start_run(&owner(), &start(), &specification())
        .await
        .unwrap();
    assert_eq!(
        (first.revision, first.reserved_steps, first.completed_rounds),
        (1, 1, 0)
    );
    finish(&store).await;
    let second = store
        .advance_run(&owner(), &step(1), &specification())
        .await
        .unwrap();
    assert_eq!(
        (
            second.revision,
            second.reserved_steps,
            second.completed_rounds
        ),
        (2, 2, 1)
    );
    assert_ne!(
        first.current_job.as_ref().unwrap().job_id,
        second.current_job.as_ref().unwrap().job_id
    );
    finish(&store).await;
    let final_view = store
        .advance_run(&owner(), &step(2), &specification())
        .await
        .unwrap();
    assert_eq!(final_view.status, RunStatus::Completed as i32);
    assert_eq!(
        (
            final_view.revision,
            final_view.reserved_steps,
            final_view.completed_rounds
        ),
        (3, 2, 2)
    );
    assert!(matches!(
        store
            .advance_run(&owner(), &step(3), &specification())
            .await,
        Err(StoreError::InvalidTransition)
    ));
    let events = store.audit_events(0, 100).await.unwrap();
    loop_core::audit::verify_audit_chain(&events).unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.target.kind == loop_core::audit::AuditTargetKind::RunId)
            .count(),
        3
    );
    assert!(
        events
            .iter()
            .filter(|event| event.target.kind == loop_core::audit::AuditTargetKind::RunId)
            .all(|event| event.actor.kind == loop_core::audit::ActorKind::Human)
    );
    store.close().await;
}

#[tokio::test]
async fn immutable_replay() {
    let (directory, store, _) = setup().await;
    let first = store
        .start_run(&owner(), &start(), &specification())
        .await
        .unwrap();
    assert_eq!(
        store
            .start_run(&owner(), &start(), &specification())
            .await
            .unwrap(),
        first
    );
    finish(&store).await;
    let second = store
        .advance_run(&owner(), &step(1), &specification())
        .await
        .unwrap();
    store.close().await;
    let reopened = PgJobStore::open(options(
        &directory.path().join("state"),
        Arc::new(fixture::FixtureClock(AtomicI64::new(fixture::NOW))),
    ))
    .await
    .unwrap();
    assert_eq!(
        reopened.replay_step(&owner(), &step(1)).await.unwrap(),
        Some(second.clone())
    );
    assert_eq!(
        reopened
            .advance_run(&owner(), &step(1), &specification())
            .await
            .unwrap(),
        second
    );
    assert_eq!(
        reopened
            .start_run(&owner(), &start(), &specification())
            .await
            .unwrap(),
        first
    );
    assert_eq!(
        reopened
            .read_run(&owner(), "run.managed")
            .await
            .unwrap()
            .view
            .reserved_steps,
        2
    );
    let mut changed = step(1);
    changed.expected_revision = 2;
    assert!(matches!(
        reopened.replay_step(&owner(), &changed).await,
        Err(StoreError::IdempotencyConflict)
    ));
    reopened.close().await;
}

#[tokio::test]
async fn budget_dimensions() {
    for dimension in ["steps", "input", "output", "cost", "time"] {
        let (_directory, store, clock) = setup().await;
        let mut spec = specification();
        let budget = spec.budget.as_mut().unwrap();
        match dimension {
            "steps" => budget.maximum_steps = 1,
            "input" => budget.maximum_input_tokens = 1024,
            "output" => budget.maximum_output_tokens = 64,
            "cost" => budget.maximum_cost = Some(storage::money(10_000_000)),
            "time" => {
                budget.maximum_wall_time = Some(prost_types::Duration {
                    seconds: 120,
                    nanos: 0,
                })
            }
            _ => unreachable!(),
        }
        store.start_run(&owner(), &start(), &spec).await.unwrap();
        finish(&store).await;
        let mut command = step(1);
        if dimension == "time" {
            clock.0.store(fixture::NOW + 1, Ordering::SeqCst);
            command.context.as_mut().unwrap().requested_at =
                Some(fixture::timestamp(fixture::NOW + 1));
        }
        let result = store.advance_run(&owner(), &command, &spec).await.unwrap();
        assert_eq!(
            result.status,
            RunStatus::BudgetExhausted as i32,
            "{dimension}"
        );
        assert_eq!((result.completed_rounds, result.reserved_steps), (1, 1));
        store.close().await;
    }
}

#[tokio::test]
async fn first_budget() {
    let (_directory, store, _) = setup().await;
    let mut spec = specification();
    spec.budget.as_mut().unwrap().maximum_input_tokens = 1;
    assert!(store.start_run(&owner(), &start(), &spec).await.is_err());
    let counts: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM research_runs), (SELECT count(*) FROM jobs), (SELECT count(*) FROM command_receipts)").fetch_one(&store.pool).await.unwrap();
    assert_eq!(counts, (0, 0, 0));
    store.close().await;
}

#[tokio::test]
async fn fractional_child() {
    let (_directory, store, _) = setup().await;
    let mut spec = specification();
    spec.discovery
        .as_mut()
        .unwrap()
        .budget
        .as_mut()
        .unwrap()
        .maximum_wall_time = Some(prost_types::Duration {
        seconds: 119,
        nanos: 1,
    });
    spec.budget.as_mut().unwrap().maximum_wall_time = Some(prost_types::Duration {
        seconds: 120,
        nanos: 0,
    });
    store.start_run(&owner(), &start(), &spec).await.unwrap();
    let observed = store.read_run(&owner(), "run.managed").await.unwrap();
    assert_eq!(observed.specification, spec);
    assert_eq!(observed.view.reserved_steps, 1);
    store.close().await;
}

#[tokio::test]
async fn denied_owners() {
    let (_directory, store, _) = setup().await;
    store
        .start_run(&owner(), &start(), &specification())
        .await
        .unwrap();
    assert!(matches!(
        store.read_run(&executor(), "run.managed").await,
        Err(StoreError::AdmissionDenied)
    ));
    let mut spoof = step(1);
    spoof.context.as_mut().unwrap().actor = Some(executor());
    assert!(store.replay_step(&executor(), &spoof).await.is_err());
    let mut changed = specification();
    changed.executor.as_mut().unwrap().authenticated_subject = "agent:replacement".into();
    assert!(matches!(
        store.advance_run(&owner(), &step(1), &changed).await,
        Err(StoreError::AdmissionDenied)
    ));
    let mut changed = specification();
    changed.budget.as_mut().unwrap().maximum_steps = 3;
    assert!(matches!(
        store.start_run(&owner(), &start(), &changed).await,
        Err(StoreError::IdempotencyConflict)
    ));
    store.close().await;
}

#[tokio::test]
async fn cas_and_clock() {
    let (_directory, store, clock) = setup().await;
    store
        .start_run(&owner(), &start(), &specification())
        .await
        .unwrap();
    assert!(matches!(
        store
            .advance_run(&owner(), &step(2), &specification())
            .await,
        Err(StoreError::RevisionConflict)
    ));
    assert!(matches!(
        store
            .advance_run(&owner(), &step(1), &specification())
            .await,
        Err(StoreError::InvalidTransition)
    ));
    clock.0.store(fixture::NOW - 1, Ordering::SeqCst);
    assert!(matches!(
        store
            .advance_run(&owner(), &step(1), &specification())
            .await,
        Err(StoreError::ClockRegression)
    ));
    clock.0.store(fixture::NOW + 3_600_000, Ordering::SeqCst);
    let mut request = step(1);
    request.context.as_mut().unwrap().requested_at =
        Some(fixture::timestamp(fixture::NOW + 3_600_000));
    let result = store
        .advance_run(&owner(), &request, &specification())
        .await
        .unwrap();
    assert_eq!(result.status, RunStatus::DeadlineExceeded as i32);
    assert_eq!(result.reserved_steps, 1);
    store.close().await;
}

#[tokio::test]
async fn failed_child() {
    let (_directory, store, _) = setup().await;
    store
        .start_run(&owner(), &start(), &specification())
        .await
        .unwrap();
    let job = store
        .read_run(&owner(), "run.managed")
        .await
        .unwrap()
        .current_job;
    store
        .control_model(
            &executor(),
            model_command(&job, "cancel.child"),
            ModelControl::Cancel,
        )
        .await
        .unwrap();
    let result = store
        .advance_run(&owner(), &step(1), &specification())
        .await
        .unwrap();
    assert_eq!(result.status, RunStatus::InfrastructureFailed as i32);
    assert_eq!((result.reserved_steps, result.completed_rounds), (1, 0));
    store.close().await;
}

#[tokio::test]
async fn paused_deadline() {
    let (_directory, store, clock) = setup().await;
    store
        .start_run(&owner(), &start(), &specification())
        .await
        .unwrap();
    let job = store
        .read_run(&owner(), "run.managed")
        .await
        .unwrap()
        .current_job;
    store
        .control_model(
            &executor(),
            model_command(&job, "pause.child"),
            ModelControl::Pause,
        )
        .await
        .unwrap();
    assert!(matches!(
        store
            .advance_run(&owner(), &step(1), &specification())
            .await,
        Err(StoreError::InvalidTransition)
    ));
    clock.0.store(fixture::NOW + 3_600_000, Ordering::SeqCst);
    let mut command = step(1);
    command.context.as_mut().unwrap().requested_at =
        Some(fixture::timestamp(fixture::NOW + 3_600_000));
    let view = store
        .advance_run(&owner(), &command, &specification())
        .await
        .unwrap();
    assert_eq!(view.status, RunStatus::DeadlineExceeded as i32);
    assert_eq!(
        view.current_job.as_ref().unwrap().status,
        discovery::DiscoveryJobStatus::Paused as i32
    );
    assert_eq!(view.reserved_steps, 1);
    assert_eq!(
        store.replay_step(&owner(), &command).await.unwrap(),
        Some(view)
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM jobs")
            .fetch_one(&store.pool)
            .await
            .unwrap(),
        1
    );
    store.close().await;
}

#[tokio::test]
async fn submission_bypass() {
    let (_directory, store, _) = setup().await;
    store
        .start_run(&owner(), &start(), &specification())
        .await
        .unwrap();
    let current = store.read_run(&owner(), "run.managed").await.unwrap();
    let mut job = current.current_job.specification.unwrap();
    job.job_id = Some(JobId {
        value: "job.unbudgeted".into(),
    });
    job.idempotency_key = Some(IdempotencyKey {
        value: "unbudgeted".into(),
    });
    assert!(matches!(
        store
            .submit(SubmitJob {
                request_id: "request.unbudgeted".into(),
                specification: job
            })
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    let spec = specification();
    let request = discovery::StartDiscoveryRequest {
        context: Some(context("legacy.start", executor())),
        discovery: spec.discovery,
    };
    assert!(matches!(
        store
            .submit_role(
                &executor(),
                RoleCommand::Discovery(request),
                SubmissionMetadata {
                    run_id: spec.run_id.unwrap(),
                    protocol_selection: spec.protocol_selection.unwrap()
                }
            )
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM jobs")
            .fetch_one(&store.pool)
            .await
            .unwrap(),
        1
    );
    store.close().await;
}

#[tokio::test]
async fn legacy_collision() {
    let (_directory, store, _) = setup().await;
    let spec = specification();
    store
        .submit_role(
            &executor(),
            RoleCommand::Discovery(discovery::StartDiscoveryRequest {
                context: Some(context("legacy.first", executor())),
                discovery: spec.discovery.clone(),
            }),
            SubmissionMetadata {
                run_id: spec.run_id.clone().unwrap(),
                protocol_selection: spec.protocol_selection.clone().unwrap(),
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        store.start_run(&owner(), &start(), &spec).await,
        Err(StoreError::DuplicateJob)
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM research_runs")
            .fetch_one(&store.pool)
            .await
            .unwrap(),
        0
    );
    store.close().await;
}

#[tokio::test]
async fn corrupt_projection() {
    let (_directory, store, _) = setup().await;
    store
        .start_run(&owner(), &start(), &specification())
        .await
        .unwrap();
    sqlx::query("ALTER TABLE research_runs DISABLE TRIGGER research_runs_protected")
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE research_runs SET view_sha256=decode(repeat('ff',32),'hex')")
        .execute(&store.pool)
        .await
        .unwrap();
    assert!(matches!(
        store.read_run(&owner(), "run.managed").await,
        Err(StoreError::Corrupt(_))
    ));
    store.close().await;
}

#[tokio::test]
async fn writer_fence() {
    let (_directory, store, _) = setup().await;
    store
        .start_run(&owner(), &start(), &specification())
        .await
        .unwrap();
    let inserted = sqlx::query("INSERT INTO jobs(job_id,run_id,kind,state,revision,attempt,submitted_at_ms,updated_at_ms,deadline_ms,record_blob,record_sha256) SELECT 'job.old-writer',run_id,kind,state,revision,attempt,submitted_at_ms,updated_at_ms,deadline_ms,record_blob,record_sha256 FROM jobs LIMIT 1")
        .execute(&store.pool).await;
    assert!(inserted.is_err());
    let changed = sqlx::query("UPDATE research_runs SET revision=revision+1")
        .execute(&store.pool)
        .await;
    assert!(changed.is_err());
    let deleted = sqlx::query("DELETE FROM research_runs")
        .execute(&store.pool)
        .await;
    assert!(deleted.is_err());
    assert_eq!(
        store
            .read_run(&owner(), "run.managed")
            .await
            .unwrap()
            .view
            .revision,
        1
    );
    store.close().await;
}

#[tokio::test]
async fn receipt_corruption() {
    use sha2::{Digest, Sha256};
    let (_directory, store, _) = setup().await;
    let mut original = store
        .start_run(&owner(), &start(), &specification())
        .await
        .unwrap();
    original.budget.as_mut().unwrap().maximum_steps = 3;
    let bytes = original.encode_to_vec();
    receipt_writes(&store).await;
    sqlx::query("UPDATE command_receipts SET response_blob=$1,response_sha256=$2 WHERE operation='loop.runs.start'")
        .bind(&bytes).bind(Sha256::digest(&bytes).to_vec()).execute(&store.pool).await.unwrap();
    assert!(matches!(
        store.start_run(&owner(), &start(), &specification()).await,
        Err(StoreError::Corrupt(_))
    ));
    store.close().await;
}

async fn receipt_writes(store: &PgJobStore) {
    sqlx::query("ALTER TABLE command_receipts DISABLE TRIGGER command_receipts_no_update")
        .execute(&store.pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn receipt_child() {
    use sha2::{Digest, Sha256};
    let (_directory, store, _) = setup().await;
    let mut first = store
        .start_run(&owner(), &start(), &specification())
        .await
        .unwrap();
    finish(&store).await;
    let second = store
        .advance_run(&owner(), &step(1), &specification())
        .await
        .unwrap();
    first.current_job = second.current_job;
    let bytes = first.encode_to_vec();
    receipt_writes(&store).await;
    sqlx::query("UPDATE command_receipts SET response_blob=$1,response_sha256=$2 WHERE operation='loop.runs.start'")
        .bind(&bytes).bind(Sha256::digest(&bytes).to_vec()).execute(&store.pool).await.unwrap();
    assert!(matches!(
        store.start_run(&owner(), &start(), &specification()).await,
        Err(StoreError::Corrupt(_))
    ));
    store.close().await;
}

#[tokio::test]
async fn receipt_terminal() {
    use sha2::{Digest, Sha256};
    let (_directory, store, _) = setup().await;
    let mut first = store
        .start_run(&owner(), &start(), &specification())
        .await
        .unwrap();
    finish(&store).await;
    first.current_job = Some(
        child_handle(
            &store
                .read_run(&owner(), "run.managed")
                .await
                .unwrap()
                .current_job,
        )
        .unwrap(),
    );
    store
        .advance_run(&owner(), &step(1), &specification())
        .await
        .unwrap();
    first.revision = 2;
    first.completed_rounds = 1;
    first.status = RunStatus::BudgetExhausted as i32;
    let bytes = first.encode_to_vec();
    receipt_writes(&store).await;
    sqlx::query("UPDATE command_receipts SET response_blob=$1,response_sha256=$2 WHERE operation='loop.runs.step'")
        .bind(&bytes).bind(Sha256::digest(&bytes).to_vec()).execute(&store.pool).await.unwrap();
    assert!(matches!(
        store.replay_step(&owner(), &step(1)).await,
        Err(StoreError::Corrupt(_))
    ));
    store.close().await;
}

#[tokio::test]
async fn default_denial() {
    let (_directory, store, _) = setup().await;
    store
        .start_run(&owner(), &start(), &specification())
        .await
        .unwrap();
    let mut denied = store.clone();
    denied.admission = Arc::new(crate::store::DenySubmission);
    assert!(matches!(
        denied.read_run(&owner(), "run.managed").await,
        Err(StoreError::AdmissionDenied)
    ));
    assert!(matches!(
        denied.start_run(&owner(), &start(), &specification()).await,
        Err(StoreError::AdmissionDenied)
    ));
    assert!(matches!(
        denied.replay_step(&owner(), &step(1)).await,
        Err(StoreError::AdmissionDenied)
    ));
    store.close().await;
}
