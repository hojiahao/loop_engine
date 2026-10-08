mod lifecycle;
mod observation;
mod processes;

use super::*;
use crate::store::{AdmissionPolicy, JobRepository, StoreOptions, SubmitJob};
use crate::test_support as fixture;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use loop_protocol::wire::v1::*;
use sha2::{Digest, Sha256};
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
        ordinal: 0,
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

fn tool_request() -> InvokeModelRequest {
    let mut request = invocation();
    let input = request.invocation.as_mut().unwrap();
    input.tools = vec![crate::runtime::model_codec::describe_tool().unwrap()];
    input.tool_choice = Some(ToolChoice {
        mode: ToolChoiceMode::Required as i32,
        named_tool: String::new(),
    });
    input.structured_output = None;
    input
        .model
        .as_mut()
        .unwrap()
        .capabilities
        .as_mut()
        .unwrap()
        .supports_tools = true;
    request
}

async fn tool_setup() -> (tempfile::TempDir, PgJobStore, Arc<fixture::FixtureClock>) {
    let mut submission = submission();
    let Some(job_specification::Input::Discovery(input)) = submission.specification.input.as_mut()
    else {
        panic!("discovery fixture");
    };
    input.maker_model = tool_request().invocation.unwrap().model;
    input.checker_model = input.maker_model.clone();
    let budget = input.budget.as_mut().unwrap();
    budget.maximum_steps = 3;
    budget.maximum_input_tokens = 2048;
    budget.maximum_output_tokens = 128;
    budget
        .maximum_cost
        .as_mut()
        .unwrap()
        .amount
        .as_mut()
        .unwrap()
        .value = "0.02".into();
    loop_protocol::job::validate_job_specification(&submission.specification).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let clock = Arc::new(fixture::FixtureClock(AtomicI64::new(fixture::NOW)));
    let store = PgJobStore::open(options(&directory.path().join("state"), clock.clone()))
        .await
        .unwrap();
    store.submit(submission).await.unwrap();
    (directory, store, clock)
}

fn call_response(step: &ModelStep) -> ModelResponse {
    let mut output = response(step);
    let schema = crate::runtime::model_codec::describe_tool()
        .unwrap()
        .input_schema
        .unwrap();
    output.finish_reason = ModelFinishReason::ToolCall as i32;
    output.content = vec![ContentBlock {
        content: Some(content_block::Content::ToolCall(ToolCallContent {
            tool_call_id: "call_1".into(),
            tool_name: "research_describe".into(),
            arguments: Some(JsonDocument {
                utf8_json: b"{}".to_vec(),
                canonical_sha256: Some(Sha256Digest {
                    value: Sha256::digest(b"{}").to_vec(),
                }),
                schema_id: schema.schema_id,
                schema_sha256: schema.schema_sha256,
            }),
        })),
    }];
    output
}

fn tool_result() -> ToolResultContent {
    ToolResultContent {
        tool_call_id: "call_1".into(),
        status: ToolResultStatus::Success as i32,
        result: Some(tool_result_content::Result::Json(JsonDocument {
            utf8_json: b"{}".to_vec(),
            canonical_sha256: Some(Sha256Digest {
                value: Sha256::digest(b"{}").to_vec(),
            }),
            schema_id: "loop.research-description/v1".into(),
            schema_sha256: Some(Sha256Digest {
                value: vec![42; 32],
            }),
        })),
    }
}

async fn tool_called(store: &PgJobStore) -> ModelStep {
    let job = store.get("job.1").await.unwrap().unwrap();
    let step = store
        .reserve_model(&fixture::actor(), command(&job, "reserve"), tool_request())
        .await
        .unwrap();
    let step = store
        .dispatch_model(&fixture::actor(), command(&step.job, "dispatch"))
        .await
        .unwrap()
        .step;
    store
        .finish_call(
            &fixture::actor(),
            command(&step.job, "call"),
            call_response(&step),
        )
        .await
        .unwrap()
}

async fn tool_ready(store: &PgJobStore) -> ModelStep {
    let step = tool_called(store).await;
    store
        .record_tool(&fixture::actor(), command(&step.job, "tool"), tool_result())
        .await
        .unwrap()
}

fn next_request(step: &ModelStep) -> InvokeModelRequest {
    let mut request = invocation();
    request.context = Some(fixture::context("provider.second"));
    let input = request.invocation.as_mut().unwrap();
    input.request_id = request.context.as_ref().unwrap().request_id.clone();
    let original = step.request.invocation.as_ref().unwrap();
    input.model = original.model.clone();
    input.messages = original.messages.clone();
    input.messages.push(ModelMessage {
        role: ModelRole::Assistant as i32,
        content: step.response.as_ref().unwrap().content.clone(),
    });
    input.messages.push(ModelMessage {
        role: ModelRole::Tool as i32,
        content: vec![ContentBlock {
            content: Some(content_block::Content::ToolResult(
                step.tool_result.as_ref().unwrap().clone(),
            )),
        }],
    });
    input.tools = original.tools.clone();
    input.tool_choice = Some(ToolChoice {
        mode: ToolChoiceMode::None as i32,
        named_tool: String::new(),
    });
    input.structured_output.as_mut().unwrap().json_schema =
        Some(crate::runtime::model_codec::ast_schema().unwrap());
    request
}

fn next_command(job: &JobRecord, key: &str) -> ModelStepCommand {
    let mut command = command(job, key);
    command.ordinal = 1;
    command
}

#[tokio::test]
async fn context_roundtrip() {
    let (directory, store, clock) = tool_setup().await;
    let first = tool_ready(&store).await;
    assert_eq!(first.job.state, JobState::Running as i32);
    let request = next_request(&first);
    let second = store
        .reserve_model(
            &fixture::actor(),
            next_command(&first.job, "reserve.second"),
            request.clone(),
        )
        .await
        .unwrap();
    assert_eq!(second.ordinal, 1);
    assert_ne!(second.job.active_lease, first.job.active_lease);
    let second = store
        .dispatch_model(
            &fixture::actor(),
            next_command(&second.job, "dispatch.second"),
        )
        .await
        .unwrap()
        .step;
    let second = store
        .finish_model(
            &fixture::actor(),
            next_command(&second.job, "finish.second"),
            response(&second),
            outcome(),
        )
        .await
        .unwrap();
    store.close().await;
    let reopened = PgJobStore::open(options(&directory.path().join("state"), clock))
        .await
        .unwrap();
    let history = reopened
        .model_history(&fixture::actor(), "job.1")
        .await
        .unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].request, first.request);
    assert_eq!(history[0].response, first.response);
    assert_eq!(history[0].tool_result, Some(tool_result()));
    assert_eq!(history[1].request, request);
    assert_eq!(history[1], second);
    assert_eq!(
        history.iter().map(|step| step.reserved_input).sum::<u64>(),
        2048
    );
    reopened.close().await;
}

#[tokio::test]
async fn cumulative_limits() {
    for dimension in ["input", "output", "cost"] {
        let (_directory, store, _) = tool_setup().await;
        let first = tool_ready(&store).await;
        let mut request = next_request(&first);
        let budget = request
            .invocation
            .as_mut()
            .unwrap()
            .budget
            .as_mut()
            .unwrap();
        match dimension {
            "input" => budget.maximum_input_tokens += 1,
            "output" => budget.maximum_output_tokens += 1,
            "cost" => {
                budget
                    .maximum_cost
                    .as_mut()
                    .unwrap()
                    .amount
                    .as_mut()
                    .unwrap()
                    .value = "0.010000001".into()
            }
            _ => unreachable!(),
        }
        assert!(matches!(
            store
                .reserve_model(
                    &fixture::actor(),
                    next_command(&first.job, "excess"),
                    request
                )
                .await,
            Err(StoreError::Invalid(_))
        ));
        assert_eq!(
            store
                .model_history(&fixture::actor(), "job.1")
                .await
                .unwrap(),
            vec![first]
        );
        store.close().await;
    }
}

#[tokio::test]
async fn context_conflict() {
    let (_directory, store, _) = tool_setup().await;
    let first = tool_ready(&store).await;
    let mut request = next_request(&first);
    request.invocation.as_mut().unwrap().messages[1].content[0].content =
        Some(content_block::Content::Text(TextContent {
            text: "changed frozen input".into(),
        }));
    assert!(matches!(
        store
            .reserve_model(
                &fixture::actor(),
                next_command(&first.job, "changed"),
                request
            )
            .await,
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(
        store
            .model_history(&fixture::actor(), "job.1")
            .await
            .unwrap(),
        vec![first]
    );
    store.close().await;
}

#[tokio::test]
async fn tool_replay() {
    let (_directory, store, _) = tool_setup().await;
    let first = tool_called(&store).await;
    let command = command(&first.job, "tool");
    let recorded = store
        .record_tool(&fixture::actor(), command.clone(), tool_result())
        .await
        .unwrap();
    assert_eq!(
        store
            .record_tool(&fixture::actor(), command.clone(), tool_result())
            .await
            .unwrap(),
        recorded
    );
    let mut changed = tool_result();
    let Some(tool_result_content::Result::Json(json)) = &mut changed.result else {
        panic!("tool fixture")
    };
    json.utf8_json = b"{\"changed\":true}".to_vec();
    json.canonical_sha256 = Some(Sha256Digest {
        value: Sha256::digest(&json.utf8_json).to_vec(),
    });
    assert!(matches!(
        store.record_tool(&fixture::actor(), command, changed).await,
        Err(StoreError::IdempotencyConflict)
    ));
    assert_eq!(store.audit_events(0, 100).await.unwrap().len(), 5);
    store.close().await;
}

#[tokio::test]
async fn missing_tool() {
    let (_directory, store, _) = tool_setup().await;
    let mut first = tool_called(&store).await;
    first.tool_result = Some(tool_result());
    assert!(matches!(
        store
            .reserve_model(
                &fixture::actor(),
                next_command(&first.job, "missing"),
                next_request(&first)
            )
            .await,
        Err(StoreError::Invalid(_))
    ));
    assert!(
        store
            .model_step(&fixture::actor(), "job.1")
            .await
            .unwrap()
            .unwrap()
            .tool_result
            .is_none()
    );
    store.close().await;
}

#[tokio::test]
async fn ordinal_replay() {
    let (_directory, store, _) = tool_setup().await;
    let first = tool_ready(&store).await;
    let second = store
        .reserve_model(
            &fixture::actor(),
            next_command(&first.job, "reserve.second"),
            next_request(&first),
        )
        .await
        .unwrap();
    let mut original = command(&first.job, "tool");
    original.expected_revision -= 1;
    let replay = store
        .record_tool(&fixture::actor(), original, tool_result())
        .await
        .unwrap();
    assert_eq!(replay.ordinal, 0);
    assert_eq!(replay.request, first.request);
    assert_eq!(replay.job, second.job);
    let mut stale = next_command(&second.job, "old.ordinal");
    stale.ordinal = 0;
    assert!(matches!(
        store.dispatch_model(&fixture::actor(), stale).await,
        Err(StoreError::Invalid(_))
    ));
    store.close().await;
}

#[tokio::test]
async fn tool_lease() {
    let (_directory, store, clock) = tool_setup().await;
    let job = store.get("job.1").await.unwrap().unwrap();
    let first = store
        .reserve_model(&fixture::actor(), command(&job, "reserve"), tool_request())
        .await
        .unwrap();
    let first = store
        .dispatch_model(&fixture::actor(), command(&first.job, "dispatch"))
        .await
        .unwrap()
        .step;
    clock.0.store(fixture::NOW + 14_000, Ordering::SeqCst);
    let mut cmd = command(&first.job, "call");
    cmd.context.as_mut().unwrap().requested_at = Some(fixture::timestamp(fixture::NOW + 14_000));
    let called = store
        .finish_call(&fixture::actor(), cmd, call_response(&first))
        .await
        .unwrap();
    assert_ne!(called.job.active_lease, first.job.active_lease);
    clock.0.store(fixture::NOW + 40_000, Ordering::SeqCst);
    let mut cmd = command(&called.job, "tool");
    cmd.context.as_mut().unwrap().requested_at = Some(fixture::timestamp(fixture::NOW + 40_000));
    let recorded = store
        .record_tool(&fixture::actor(), cmd, tool_result())
        .await
        .unwrap();
    assert!(recorded.tool_result.is_some());
    store.close().await;
}

#[tokio::test]
async fn tool_corruption() {
    let (directory, store, _) = tool_setup().await;
    tool_ready(&store).await;
    let mut connection = fixture::connection(&directory).await;
    sqlx::query("ALTER TABLE tool_results DISABLE TRIGGER tool_results_protected")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("UPDATE tool_results SET result_blob=decode('00','hex')")
        .execute(&mut connection)
        .await
        .unwrap();
    assert!(matches!(
        store.model_history(&fixture::actor(), "job.1").await,
        Err(StoreError::Corrupt(_))
    ));
    store.close().await;
}

#[tokio::test]
async fn previous_writer() {
    use sqlx::Connection;
    let (directory, store, _) = setup().await;
    reserve(&store).await;
    let mut connection = fixture::connection(&directory).await;
    let mut transaction = connection.begin().await.unwrap();
    sqlx::query("SELECT set_config('loop.model_step_writer', 'v1', true)")
        .execute(&mut *transaction)
        .await
        .unwrap();
    assert!(
        sqlx::query("UPDATE jobs SET revision=revision+1 WHERE job_id='job.1'")
            .execute(&mut *transaction)
            .await
            .is_err()
    );
    transaction.rollback().await.unwrap();
    store.close().await;
}

#[tokio::test]
async fn deployment_privileges() {
    use sqlx::Connection;
    let (directory, store, _) = setup().await;
    reserve(&store).await;
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/postgres-production-bundle.mjs");
    let bundle = std::process::Command::new("node")
        .arg(script)
        .output()
        .unwrap();
    assert!(bundle.status.success());
    let bundle = String::from_utf8(bundle.stdout).unwrap();
    let role = format!("loop_model_role_{}", uuid::Uuid::new_v4().simple());
    let schema = fixture::schema(&directory.path().join("state"));
    let mut connection = fixture::connection(&directory).await;
    let mut transaction = connection.begin().await.unwrap();
    // Role and grants are transaction-local test objects; rollback removes both.
    sqlx::raw_sql(&format!("CREATE ROLE {role} NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT; GRANT USAGE ON SCHEMA {schema} TO {role};"))
        .execute(&mut *transaction).await.unwrap();
    for statement in bundle
        .lines()
        .filter(|line| line.starts_with("GRANT ") || line.starts_with("REVOKE "))
    {
        let scoped = statement
            .replace("loop_engine_app", &role)
            .replace("IN SCHEMA public", &format!("IN SCHEMA {schema}"));
        sqlx::raw_sql(&scoped)
            .execute(&mut *transaction)
            .await
            .unwrap();
    }
    sqlx::query(&format!("SET LOCAL ROLE {role}"))
        .execute(&mut *transaction)
        .await
        .unwrap();
    sqlx::query("SELECT set_config('loop.model_step_writer', 'v3', true)")
        .execute(&mut *transaction)
        .await
        .unwrap();
    let updated = sqlx::query("UPDATE model_steps SET state='dispatched',updated_revision=updated_revision+1 WHERE job_id='job.1' AND ordinal=0")
        .execute(&mut *transaction).await.unwrap();
    assert_eq!(updated.rows_affected(), 1);
    let denied = sqlx::query("UPDATE tool_results SET result_blob=result_blob WHERE false")
        .execute(&mut *transaction)
        .await
        .unwrap_err();
    assert_eq!(
        denied
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("42501")
    );
    transaction.rollback().await.unwrap();
    store.close().await;
}

#[tokio::test]
async fn migration_preserves() {
    use sqlx::Connection;
    let (directory, store, _) = setup().await;
    let mut connection = fixture::connection(&directory).await;
    let mut transaction = connection.begin().await.unwrap();
    let schema = format!("loop_model_migration_{}", uuid::Uuid::new_v4().simple());
    // The probe schema and all DDL belong to this transaction and are rolled
    // back even if a test assertion interrupts before the explicit rollback.
    sqlx::raw_sql(&format!("CREATE SCHEMA {schema}; SET LOCAL search_path TO {schema}; CREATE TABLE jobs(job_id TEXT PRIMARY KEY);"))
        .execute(&mut *transaction).await.unwrap();
    sqlx::raw_sql(include_str!(
        "../../../../../migrations/postgres/0011_model_steps.sql"
    ))
    .execute(&mut *transaction)
    .await
    .unwrap();
    sqlx::query("SELECT set_config('loop.model_step_writer', 'v1', true)")
        .execute(&mut *transaction)
        .await
        .unwrap();
    sqlx::query("INSERT INTO jobs(job_id) VALUES ('old.job')")
        .execute(&mut *transaction)
        .await
        .unwrap();
    sqlx::query("INSERT INTO model_steps(job_id,actor_id,request_id,idempotency_key,invocation_sha256,request_blob,request_sha256,reserved_input,reserved_output,reserved_nano_usd,state,created_revision,updated_revision,created_at_ms,updated_at_ms) VALUES ('old.job','old.actor','old.request','old.key',decode(repeat('01',32),'hex'),decode('ab','hex'),decode(repeat('02',32),'hex'),10,20,30,'reserved',2,2,100,100)")
        .execute(&mut *transaction).await.unwrap();
    let original: String =
        sqlx::query_scalar("SELECT to_jsonb(model_steps)::text FROM model_steps")
            .fetch_one(&mut *transaction)
            .await
            .unwrap();
    sqlx::raw_sql(include_str!(
        "../../../../../migrations/postgres/0012_tool_context.sql"
    ))
    .execute(&mut *transaction)
    .await
    .unwrap();
    let migrated: String =
        sqlx::query_scalar("SELECT (to_jsonb(model_steps)-'ordinal')::text FROM model_steps")
            .fetch_one(&mut *transaction)
            .await
            .unwrap();
    assert_eq!(original, migrated);
    let ordinal: i32 = sqlx::query_scalar("SELECT ordinal FROM model_steps")
        .fetch_one(&mut *transaction)
        .await
        .unwrap();
    assert_eq!(ordinal, 0);
    transaction.rollback().await.unwrap();
    store.close().await;
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
