mod processes;

use super::*;
use crate::store::{AdmissionPolicy, JobRepository, StoreOptions, SubmitJob};
use crate::test_support as fixture;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use loop_protocol::wire::v1::*;
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

struct Admission;
impl AdmissionPolicy for Admission {
    fn validate_submission(&self, job: &JobSpecification) -> StoreResult<()> {
        if job.kind == JobKind::Discovery as i32
            && job.submitted_by.as_ref() == Some(&fixture::actor())
        {
            Ok(())
        } else {
            Err(StoreError::AdmissionDenied)
        }
    }
    fn authorize_job_command(&self, _: &str, actor: &Actor, _: &JobRecord) -> StoreResult<()> {
        if *actor == fixture::actor() {
            Ok(())
        } else {
            Err(StoreError::AdmissionDenied)
        }
    }
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
    request.context = Some(fixture::context("provider.original"));
    let invocation = request.invocation.as_mut().unwrap();
    invocation.request_id = request.context.as_ref().unwrap().request_id.clone();
    let model = invocation.model.as_mut().unwrap();
    model.resolved_at = Some(fixture::timestamp(fixture::NOW - 5_000));
    // The codec golden deliberately exceeds the job's token bounds to exercise
    // uint64 encoding. Storage workflows need a deployable capability snapshot.
    model.capabilities.as_mut().unwrap().context_window_tokens = 65_536;
    invocation.budget.as_mut().unwrap().maximum_wall_time = Some(prost_types::Duration {
        seconds: 10,
        nanos: 0,
    });
    request
}

fn submission() -> SubmitJob {
    let mut command = fixture::command(1);
    let request = invocation();
    let invocation = request.invocation.unwrap();
    let (_, input) = fixture::research::inputs().remove(0);
    let job_specification::Input::Discovery(mut input) = input else {
        panic!("fixture discovery")
    };
    input.maximum_candidates = 1;
    input.maker_model = invocation.model.clone();
    input.checker_model = invocation.model;
    // Provider transport policy and research-plan policy are distinct pins.
    input.budget = Some(JobBudget {
        maximum_steps: 1,
        maximum_input_tokens: 1024,
        maximum_output_tokens: 64,
        maximum_cost: invocation.budget.unwrap().maximum_cost,
        maximum_wall_time: Some(prost_types::Duration {
            seconds: 120,
            nanos: 0,
        }),
    });
    command.specification.kind = JobKind::Discovery as i32;
    command.specification.input = Some(job_specification::Input::Discovery(input));
    command
}

fn options(path: &Path, clock: Arc<fixture::FixtureClock>) -> StoreOptions {
    let mut options = fixture::options(path, clock);
    options.admission = Arc::new(Admission);
    options
}

async fn setup() -> (tempfile::TempDir, PgJobStore, Arc<fixture::FixtureClock>) {
    let submission = submission();
    loop_protocol::job::validate_job_specification(&submission.specification).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let clock = Arc::new(fixture::FixtureClock(AtomicI64::new(fixture::NOW)));
    let store = PgJobStore::open(options(&directory.path().join("state"), clock.clone()))
        .await
        .unwrap();
    store.submit(submission).await.unwrap();
    (directory, store, clock)
}

fn command(job: &JobRecord, key: &str) -> ModelStepCommand {
    ModelStepCommand {
        context: Some(fixture::context(key)),
        job_id: job.specification.as_ref().unwrap().job_id.clone(),
        lease_id: job
            .active_lease
            .as_ref()
            .and_then(|lease| lease.lease_id.clone()),
        expected_revision: job.revision,
    }
}

async fn reserve(store: &PgJobStore) -> ModelStep {
    let job = store.get("job.1").await.unwrap().unwrap();
    store
        .reserve_model(&fixture::actor(), command(&job, "reserve"), invocation())
        .await
        .unwrap()
}

fn response(step: &ModelStep) -> ModelResponse {
    let invocation = step.request.invocation.as_ref().unwrap();
    ModelResponse {
        request_id: invocation.request_id.clone(),
        resolution_id: invocation.model.as_ref().unwrap().resolution_id.clone(),
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
    }
}

fn outcome() -> JobOutcome {
    JobOutcome {
        outcome: Some(job_outcome::Outcome::Success(JobSuccess {
            outputs: vec![],
        })),
    }
}

#[tokio::test]
async fn atomic_reservation() {
    let (_directory, store, _) = setup().await;
    let step = reserve(&store).await;
    assert_eq!(step.job.state, JobState::Running as i32);
    assert_eq!(step.job.revision, 2);
    assert_eq!(step.job.attempt, 1);
    assert_eq!(step.state, ModelStepState::Reserved);
    assert_eq!(step.reserved_input, 1024);
    assert_eq!(step.reserved_output, 64);
    assert!(step.reserved_nano_usd > 0);
    assert_eq!(store.audit_events(0, 100).await.unwrap().len(), 2);
    store.close().await;
}

#[tokio::test]
async fn dispatch_once() {
    let (_directory, store, _) = setup().await;
    let reserved = reserve(&store).await;
    let command = command(&reserved.job, "dispatch");
    let first = store
        .dispatch_model(&fixture::actor(), command.clone())
        .await
        .unwrap();
    assert!(first.send);
    assert!(
        !store
            .dispatch_model(&fixture::actor(), command)
            .await
            .unwrap()
            .send
    );
    let changed = super::tests::command(&first.step.job, "new.dispatch");
    assert!(
        !store
            .dispatch_model(&fixture::actor(), changed)
            .await
            .unwrap()
            .send
    );
    assert_eq!(store.audit_events(0, 100).await.unwrap().len(), 3);
    store.close().await;
}

#[tokio::test]
async fn immutable_completion() {
    let (directory, store, clock) = setup().await;
    let reserved = reserve(&store).await;
    let dispatch = store
        .dispatch_model(&fixture::actor(), command(&reserved.job, "dispatch"))
        .await
        .unwrap();
    let command = command(&dispatch.step.job, "finish");
    let completed = store
        .finish_model(
            &fixture::actor(),
            command.clone(),
            response(&dispatch.step),
            outcome(),
        )
        .await
        .unwrap();
    assert_eq!(completed.state, ModelStepState::Completed);
    assert_eq!(completed.job.state, JobState::Succeeded as i32);
    assert!(completed.job.active_lease.is_none());
    store.close().await;
    let reopened = PgJobStore::open(options(&directory.path().join("state"), clock))
        .await
        .unwrap();
    assert_eq!(
        reopened
            .model_step(&fixture::actor(), "job.1")
            .await
            .unwrap(),
        Some(completed.clone())
    );
    assert_eq!(
        reopened
            .finish_model(
                &fixture::actor(),
                command.clone(),
                response(&dispatch.step),
                outcome()
            )
            .await
            .unwrap(),
        completed
    );
    let mut changed = response(&dispatch.step);
    changed.usage.as_mut().unwrap().output_tokens += 1;
    assert!(matches!(
        reopened
            .finish_model(&fixture::actor(), command, changed, outcome())
            .await,
        Err(StoreError::IdempotencyConflict)
    ));
    reopened.close().await;
}

#[tokio::test]
async fn failed_reservation() {
    let (_directory, store, _) = setup().await;
    let job = store.get("job.1").await.unwrap().unwrap();
    let mut request = invocation();
    request
        .invocation
        .as_mut()
        .unwrap()
        .budget
        .as_mut()
        .unwrap()
        .maximum_input_tokens = 1025;
    assert!(matches!(
        store
            .reserve_model(&fixture::actor(), command(&job, "reserve"), request)
            .await,
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(store.get("job.1").await.unwrap().unwrap(), job);
    assert!(
        store
            .model_step(&fixture::actor(), "job.1")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(store.audit_events(0, 100).await.unwrap().len(), 1);
    store.close().await;
}

#[tokio::test]
async fn budget_retained() {
    let (_directory, store, _) = setup().await;
    let reserved = reserve(&store).await;
    let sent = store
        .dispatch_model(&fixture::actor(), command(&reserved.job, "dispatch"))
        .await
        .unwrap();
    let uncertain = store
        .uncertain_model(&fixture::actor(), command(&sent.step.job, "uncertain"))
        .await
        .unwrap();
    assert_eq!(uncertain.state, ModelStepState::Ambiguous);
    assert_eq!(uncertain.reserved_nano_usd, reserved.reserved_nano_usd);
    assert!(
        !store
            .dispatch_model(&fixture::actor(), command(&uncertain.job, "retry"))
            .await
            .unwrap()
            .send
    );
    store.close().await;
}

#[tokio::test]
async fn lease_takeover() {
    let (_directory, store, clock) = setup().await;
    let reserved = reserve(&store).await;
    let sent = store
        .dispatch_model(&fixture::actor(), command(&reserved.job, "dispatch"))
        .await
        .unwrap();
    clock.0.store(fixture::NOW + 15_000, Ordering::SeqCst);
    let mut takeover = command(&sent.step.job, "takeover");
    takeover.lease_id = None;
    takeover.context.as_mut().unwrap().requested_at =
        Some(fixture::timestamp(fixture::NOW + 15_000));
    let taken = store
        .takeover_model(
            &fixture::actor(),
            takeover,
            prost_types::Duration {
                seconds: 15,
                nanos: 0,
            },
        )
        .await
        .unwrap();
    assert_eq!(taken.state, ModelStepState::Dispatched);
    assert_eq!(taken.job.attempt, 2);
    assert_ne!(taken.job.active_lease, sent.step.job.active_lease);
    let mut resend = command(&taken.job, "retry");
    resend.context.as_mut().unwrap().requested_at = Some(fixture::timestamp(fixture::NOW + 15_000));
    assert!(
        !store
            .dispatch_model(&fixture::actor(), resend)
            .await
            .unwrap()
            .send
    );
    assert!(matches!(
        store
            .finish_model(
                &fixture::actor(),
                command(&sent.step.job, "old.finish"),
                response(&sent.step),
                outcome()
            )
            .await,
        Err(StoreError::RevisionConflict)
    ));
    store.close().await;
}

#[tokio::test]
async fn legacy_fenced() {
    let (directory, store, _) = setup().await;
    let reserved = reserve(&store).await;
    let generic = loop_protocol::wire::jobs::v1::CancelJobRequest {
        context: Some(fixture::context("cancel")),
        job_id: Some(JobId {
            value: "job.1".into(),
        }),
        expected_revision: reserved.job.revision,
        reason: "cancel".into(),
    };
    assert!(matches!(
        store
            .mutate(
                &fixture::actor(),
                crate::store::JobMutation::Cancel(generic)
            )
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    assert!(
        sqlx::query("UPDATE jobs SET revision=revision+1 WHERE job_id='job.1'")
            .execute(&mut fixture::connection(&directory).await)
            .await
            .is_err()
    );
    assert_eq!(store.get("job.1").await.unwrap().unwrap(), reserved.job);
    store.close().await;
}

#[tokio::test]
async fn corrupt_receipt() {
    let (directory, store, _) = setup().await;
    reserve(&store).await;
    let mut connection = fixture::connection(&directory).await;
    sqlx::query("ALTER TABLE model_steps DISABLE TRIGGER model_steps_protected")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("UPDATE model_steps SET request_blob=decode('00','hex')")
        .execute(&mut connection)
        .await
        .unwrap();
    assert!(matches!(
        store.model_step(&fixture::actor(), "job.1").await,
        Err(StoreError::Corrupt(_))
    ));
    store.close().await;
}

#[tokio::test]
async fn authority_denied() {
    let (_directory, store, _) = setup().await;
    let job = store.get("job.1").await.unwrap().unwrap();
    let mut stranger = fixture::actor();
    stranger.authenticated_subject = "service:other".into();
    assert!(matches!(
        store
            .reserve_model(&stranger, command(&job, "reserve"), invocation())
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    assert!(matches!(
        store.model_step(&stranger, "job.1").await,
        Err(StoreError::AdmissionDenied)
    ));
    assert_eq!(store.audit_events(0, 100).await.unwrap().len(), 1);
    store.close().await;
}

#[tokio::test]
async fn clock_regression() {
    let (_directory, store, clock) = setup().await;
    let step = reserve(&store).await;
    clock.0.store(fixture::NOW - 1, Ordering::SeqCst);
    assert!(matches!(
        store
            .dispatch_model(&fixture::actor(), command(&step.job, "dispatch"))
            .await,
        Err(StoreError::ClockRegression)
    ));
    clock.0.store(fixture::NOW, Ordering::SeqCst);
    assert_eq!(
        store
            .model_step(&fixture::actor(), "job.1")
            .await
            .unwrap()
            .unwrap()
            .state,
        ModelStepState::Reserved
    );
    store.close().await;
}

#[tokio::test]
async fn expired_dispatch() {
    let (_directory, store, clock) = setup().await;
    let step = reserve(&store).await;
    clock.0.store(fixture::NOW + 15_000, Ordering::SeqCst);
    assert!(matches!(
        store
            .dispatch_model(&fixture::actor(), command(&step.job, "dispatch"))
            .await,
        Err(StoreError::LeaseFenced)
    ));
    assert_eq!(
        store
            .model_step(&fixture::actor(), "job.1")
            .await
            .unwrap()
            .unwrap()
            .state,
        ModelStepState::Reserved
    );
    store.close().await;
}

#[tokio::test]
async fn deadline_takeover() {
    let (_directory, store, clock) = setup().await;
    let step = reserve(&store).await;
    clock.0.store(fixture::NOW + 120_000, Ordering::SeqCst);
    let mut takeover = command(&step.job, "takeover");
    takeover.lease_id = None;
    takeover.context.as_mut().unwrap().requested_at =
        Some(fixture::timestamp(fixture::NOW + 120_000));
    assert!(matches!(
        store
            .takeover_model(
                &fixture::actor(),
                takeover,
                prost_types::Duration {
                    seconds: 15,
                    nanos: 0
                }
            )
            .await,
        Err(StoreError::LeaseFenced)
    ));
    assert_eq!(
        store
            .model_step(&fixture::actor(), "job.1")
            .await
            .unwrap()
            .unwrap()
            .reserved_nano_usd,
        step.reserved_nano_usd
    );
    store.close().await;
}

#[tokio::test]
async fn response_budget() {
    let (_directory, store, _) = setup().await;
    let reserved = reserve(&store).await;
    let sent = store
        .dispatch_model(&fixture::actor(), command(&reserved.job, "dispatch"))
        .await
        .unwrap();
    let mut invalid = response(&sent.step);
    invalid.usage.as_mut().unwrap().output_tokens = 65;
    assert!(matches!(
        store
            .finish_model(
                &fixture::actor(),
                command(&sent.step.job, "finish"),
                invalid,
                outcome()
            )
            .await,
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(
        store
            .model_step(&fixture::actor(), "job.1")
            .await
            .unwrap()
            .unwrap(),
        sent.step
    );
    store.close().await;
}

#[test]
fn exact_money() {
    let money = |text: &str| Money {
        amount: Some(ExactDecimal { value: text.into() }),
        currency_code: "USD".into(),
    };
    assert_eq!(validation::money(Some(&money("0.000000001"))).unwrap(), 1);
    assert_eq!(
        validation::money(Some(&money("18.42"))).unwrap(),
        18_420_000_000
    );
    for text in ["1.0", "01", "1e2", "-1", "0.0000000001"] {
        assert!(validation::money(Some(&money(text))).is_err());
    }
}
