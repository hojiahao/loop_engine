//! Real PostgreSQL, mTLS Discovery RPC, compiled Provider and local HTTP supplier.

mod fixture;
use crate::test_support::tls;

use loop_protocol::wire::{discovery::v1 as wire, provider::v1 as provider, v1};
use prost::Message;
use std::sync::atomic::Ordering;

use crate::store::{JobRepository, ModelStepCommand, ModelStepState};
use fixture::{Case, timed};

#[tokio::test]
async fn candidate_roundtrip() {
    let mut case = Case::open().await;
    let job = case.start().await;
    case.verify_deployment(&job).await;
    let queued = case.read(&job).await;
    assert_eq!(queued.state, wire::DiscoveryStepState::Unspecified as i32);
    assert!(queued.reserved_cost.is_none());
    let result = case.execute(&job).await.unwrap();
    assert_eq!(result.state, wire::DiscoveryStepState::Completed as i32);
    assert_eq!(
        result.job.as_ref().unwrap().status,
        wire::DiscoveryJobStatus::Succeeded as i32
    );
    assert_eq!(result.reserved_input_tokens, 4096);
    assert_eq!(result.reserved_output_tokens, 128);
    let candidate = result.candidate.as_ref().unwrap();
    assert!(
        candidate
            .expression_id
            .as_ref()
            .unwrap()
            .value
            .starts_with("sha256:")
    );
    assert!(String::from_utf8_lossy(&candidate.canonical_json).contains("market.close"));
    assert_eq!(case.calls(), 1);
    case.restart().await;
    assert_eq!(case.read(result.job.as_ref().unwrap()).await, result);
    let replay = case.execute(result.job.as_ref().unwrap()).await.unwrap();
    assert_eq!(replay, result);
    assert_eq!(case.calls(), 1);
    case.close().await;
}

#[tokio::test]
async fn denied_principal() {
    let mut case = Case::open().await;
    let job = case.start().await;
    let mut request = wire::GetDiscoveryRequest {
        context: Some(case.context()),
        job_id: job.job_id.clone(),
    };
    request
        .context
        .as_mut()
        .unwrap()
        .actor
        .as_mut()
        .unwrap()
        .actor_id
        .as_mut()
        .unwrap()
        .value = "agent.impersonator".into();
    let error = case
        .client("client")
        .await
        .get_discovery(timed(request))
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::PermissionDenied);
    let error = case
        .client("unknown")
        .await
        .get_discovery(timed(wire::GetDiscoveryRequest {
            context: Some(case.context()),
            job_id: job.job_id,
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::PermissionDenied);
    assert_eq!(case.calls(), 0);
    case.close().await;
}

#[tokio::test]
async fn denied_plan() {
    let mut case = Case::open().await;
    let mut input = case.input.clone();
    input.maximum_candidates = 2;
    let error = case
        .client("client")
        .await
        .start_discovery(timed(wire::StartDiscoveryRequest {
            context: Some(case.context()),
            discovery: Some(input),
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::PermissionDenied);
    assert_eq!(case.calls(), 0);
    assert!(case.store.audit_events(0, 10).await.unwrap().is_empty());
    case.close().await;
}

#[tokio::test]
async fn start_deadline_missing() {
    let mut case = Case::open().await;
    let error = case
        .client("client")
        .await
        .start_discovery(wire::StartDiscoveryRequest {
            context: Some(case.context()),
            discovery: Some(case.input.clone()),
        })
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::InvalidArgument);
    assert!(case.store.audit_events(0, 10).await.unwrap().is_empty());
    assert_eq!(case.calls(), 0);
    case.close().await;
}

#[tokio::test]
async fn execute_deadline_missing() {
    let mut case = Case::open().await;
    let job = case.start().await;
    let error = case
        .client("client")
        .await
        .execute_discovery(wire::ExecuteDiscoveryRequest {
            context: Some(case.context()),
            job_id: job.job_id.clone(),
            expected_revision: job.revision,
        })
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::InvalidArgument);
    assert!(
        case.store
            .model_step(&case.actor(), &job.job_id.unwrap().value)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(case.calls(), 0);
    case.close().await;
}

#[tokio::test]
async fn concurrent_execution() {
    let mut case = Case::open().await;
    let job = case.start().await;
    // Both authenticated commands carry the same observed job revision. Only
    // one may reserve/dispatch, including when the other reads a newer step.
    let (first, second) = tokio::join!(case.execute(&job), case.execute(&job));
    let (completed, conflict) = match (first, second) {
        (Ok(completed), Err(conflict)) | (Err(conflict), Ok(completed)) => (completed, conflict),
        other => panic!("expected one completion and one revision conflict: {other:?}"),
    };
    assert_eq!(conflict.code(), tonic::Code::Aborted);
    assert_eq!(completed.state, wire::DiscoveryStepState::Completed as i32);
    assert!(completed.candidate.is_some());
    assert_eq!(case.calls(), 1);
    assert_eq!(case.read(completed.job.as_ref().unwrap()).await, completed);
    case.close().await;
}

#[tokio::test]
async fn stale_execution() {
    let mut case = Case::open().await;
    let job = case.start().await;
    let stale = case
        .store
        .get(&job.job_id.as_ref().unwrap().value)
        .await
        .unwrap()
        .unwrap();
    let reserved = reserve(&case, &job).await;
    assert_eq!(reserved.job.revision, stale.revision + 1);
    assert!(matches!(
        case.stale_execute(stale).await,
        Err(crate::store::StoreError::RevisionConflict)
    ));
    assert_eq!(case.calls(), 0);
    assert_eq!(
        case.store
            .model_step(&case.actor(), &job.job_id.unwrap().value)
            .await
            .unwrap()
            .unwrap(),
        reserved
    );
    case.close().await;
}

#[tokio::test]
async fn corrupt_data() {
    let mut case = Case::open().await;
    let job = case.start().await;
    case.corrupt_data();
    assert!(case.execute(&job).await.is_err());
    assert_eq!(case.calls(), 0);
    assert!(
        case.store
            .model_step(&case.actor(), &job.job_id.unwrap().value)
            .await
            .unwrap()
            .is_none()
    );
    case.close().await;
}

#[tokio::test]
async fn protected_data() {
    let mut case = Case::protected().await;
    let job = case.start().await;
    let error = case.execute(&job).await.unwrap_err();
    assert_eq!(error.code(), tonic::Code::PermissionDenied);
    assert_eq!(case.calls(), 0);
    assert!(
        case.store
            .model_step(&case.actor(), &job.job_id.unwrap().value)
            .await
            .unwrap()
            .is_none()
    );
    case.close().await;
}

#[tokio::test]
async fn invalid_candidate() {
    let mut case = Case::open().await;
    let job = case.start().await;
    case.invalid_ast();
    let result = case.execute(&job).await.unwrap();
    assert_eq!(result.state, wire::DiscoveryStepState::Completed as i32);
    assert_eq!(
        result.job.as_ref().unwrap().status,
        wire::DiscoveryJobStatus::InfrastructureFailed as i32
    );
    assert!(result.candidate.is_none());
    assert_eq!(case.calls(), 1);
    case.close().await;
}

async fn reserve(case: &Case, job: &wire::DiscoveryJobHandle) -> crate::store::ModelStep {
    let record = case
        .store
        .get(&job.job_id.as_ref().unwrap().value)
        .await
        .unwrap()
        .unwrap();
    let mut invocation = case.invocation.clone();
    let context = case.context();
    invocation.request_id = context.request_id.clone();
    let request = provider::InvokeModelRequest {
        context: Some(context),
        invocation: Some(invocation),
    };
    case.store
        .reserve_model(
            &case.actor(),
            ModelStepCommand {
                ordinal: 0,
                context: Some(case.context()),
                job_id: job.job_id.clone(),
                lease_id: None,
                expected_revision: record.revision,
            },
            request,
        )
        .await
        .unwrap()
}

async fn dispatch(case: &Case, job: &wire::DiscoveryJobHandle) -> crate::store::ModelStep {
    let reserved = reserve(case, job).await;
    let dispatched = case
        .store
        .dispatch_model(
            &case.actor(),
            ModelStepCommand {
                ordinal: 0,
                context: Some(case.context()),
                job_id: job.job_id.clone(),
                lease_id: reserved.job.active_lease.as_ref().unwrap().lease_id.clone(),
                expected_revision: reserved.job.revision,
            },
        )
        .await
        .unwrap();
    assert!(dispatched.send);
    dispatched.step
}

#[tokio::test]
async fn absent_recovery() {
    let mut case = Case::open().await;
    let job = case.start().await;
    let dispatched = dispatch(&case, &job).await;
    case.clock.0.store(11_000, Ordering::SeqCst);
    case.restart().await;
    let current = case.read(&job).await;
    let result = case.execute(current.job.as_ref().unwrap()).await.unwrap();
    assert_eq!(result.state, wire::DiscoveryStepState::Ambiguous as i32);
    assert!(result.candidate.is_none());
    assert_eq!(result.reserved_input_tokens, dispatched.reserved_input);
    assert_eq!(result.reserved_output_tokens, dispatched.reserved_output);
    assert_eq!(case.calls(), 0);
    case.close().await;
}

#[tokio::test]
async fn receipt_recovery() {
    let mut case = Case::open().await;
    let job = case.start().await;
    let dispatched = dispatch(&case, &job).await;
    let mut request = tonic::Request::new(dispatched.request.clone());
    let budget = dispatched
        .request
        .invocation
        .as_ref()
        .unwrap()
        .budget
        .as_ref()
        .unwrap();
    let millis = crate::store::model_duration(budget.maximum_wall_time.as_ref()).unwrap();
    request.set_timeout(std::time::Duration::from_millis(millis as u64));
    let response = case.provider().await.invoke_model(request).await.unwrap();
    assert!(response.into_inner().response.is_some());
    assert_eq!(case.calls(), 1);
    // Crash cut: the Provider receipt exists, but no result reached PostgreSQL.
    // Both services restart; the Provider is now explicitly read-only.
    case.restart_provider().await;
    case.clock.0.store(11_000, Ordering::SeqCst);
    case.restart().await;
    let current = case.read(&job).await;
    assert_eq!(current.state, wire::DiscoveryStepState::Dispatched as i32);
    let result = case.execute(current.job.as_ref().unwrap()).await.unwrap();
    assert_eq!(result.state, wire::DiscoveryStepState::Completed as i32);
    assert!(result.candidate.is_some());
    assert_eq!(case.calls(), 1);
    let persisted = case
        .store
        .model_step(&case.actor(), &job.job_id.unwrap().value)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(persisted.state, ModelStepState::Completed);
    assert_eq!(
        persisted.request.encode_to_vec(),
        dispatched.request.encode_to_vec()
    );
    case.close().await;
}

#[tokio::test]
async fn controlled_roundtrip() {
    let mut case = Case::controlled().await;
    let job = case.start().await;
    let result = case.execute(&job).await.unwrap();
    assert_eq!(
        result.job.as_ref().unwrap().status,
        wire::DiscoveryJobStatus::Succeeded as i32
    );
    assert!(result.candidate.is_some());
    assert_eq!(result.reserved_input_tokens, 8192);
    assert_eq!(result.reserved_output_tokens, 256);
    assert_eq!(
        result
            .reserved_cost
            .as_ref()
            .unwrap()
            .amount
            .as_ref()
            .unwrap()
            .value,
        "2"
    );
    assert_eq!(case.calls(), 2);
    let history = case
        .store
        .model_history(&case.actor(), &job.job_id.as_ref().unwrap().value)
        .await
        .unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].ordinal, 0);
    assert_eq!(history[1].ordinal, 1);
    let messages = &history[1].request.invocation.as_ref().unwrap().messages;
    assert_eq!(messages.len(), 3);
    assert_eq!(
        messages[1].content,
        history[0].response.as_ref().unwrap().content
    );
    assert_eq!(
        messages[2].content[0].content,
        Some(v1::content_block::Content::ToolResult(
            history[0].tool_result.clone().unwrap()
        ))
    );
    let bodies = case.supplier_bodies();
    assert_eq!(bodies.len(), 2);
    assert_eq!(bodies[0]["tool_choice"], "required");
    assert_eq!(bodies[1]["tool_choice"], "none");
    let description: serde_json::Value =
        serde_json::from_str(bodies[1]["messages"][2]["content"].as_str().unwrap()).unwrap();
    assert_eq!(description["snapshot_ids"][0], "snapshot.discovery");
    assert_eq!(description["sample"]["role"], "in_sample");
    assert_eq!(
        description["artifacts"][0]["schema"]["name"],
        "loop.synthetic_prices"
    );
    case.restart().await;
    assert_eq!(case.read(result.job.as_ref().unwrap()).await, result);
    assert_eq!(
        case.execute(result.job.as_ref().unwrap()).await.unwrap(),
        result
    );
    assert_eq!(case.calls(), 2);
    case.close().await;
}

#[tokio::test]
async fn unknown_tool() {
    let mut case = Case::controlled().await;
    case.invalid_tool("unknown");
    denied_tool(&case).await;
    case.close().await;
}

#[tokio::test]
async fn extra_arguments() {
    let mut case = Case::controlled().await;
    case.invalid_tool("arguments");
    denied_tool(&case).await;
    case.close().await;
}

async fn denied_tool(case: &Case) {
    let job = case.start().await;
    if let Ok(result) = case.execute(&job).await {
        assert!(result.candidate.is_none());
        assert_ne!(
            result.job.as_ref().unwrap().status,
            wire::DiscoveryJobStatus::Succeeded as i32
        );
    }
    assert_eq!(case.calls(), 1);
    let history = case
        .store
        .model_history(&case.actor(), &job.job_id.unwrap().value)
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert!(history[0].tool_result.is_none());
}

#[tokio::test]
async fn protected_tool() {
    let mut case = Case::controlled_protected().await;
    let job = case.start().await;
    assert_eq!(
        case.execute(&job).await.unwrap_err().code(),
        tonic::Code::PermissionDenied
    );
    assert_eq!(case.calls(), 0);
    assert!(
        case.store
            .model_history(&case.actor(), &job.job_id.unwrap().value)
            .await
            .unwrap()
            .is_empty()
    );
    case.close().await;
}

#[tokio::test]
async fn corrupt_tool_data() {
    let mut case = Case::controlled().await;
    let job = case.start().await;
    case.corrupt_data();
    assert!(case.execute(&job).await.is_err());
    assert_eq!(case.calls(), 0);
    assert!(
        case.store
            .model_history(&case.actor(), &job.job_id.unwrap().value)
            .await
            .unwrap()
            .is_empty()
    );
    case.close().await;
}

fn step_command(case: &Case, step: &crate::store::ModelStep) -> ModelStepCommand {
    ModelStepCommand {
        ordinal: step.ordinal,
        context: Some(case.context()),
        job_id: step.job.specification.as_ref().unwrap().job_id.clone(),
        lease_id: step.job.active_lease.as_ref().unwrap().lease_id.clone(),
        expected_revision: step.job.revision,
    }
}

async fn invoke(case: &Case, step: &crate::store::ModelStep) -> v1::ModelResponse {
    let mut request = tonic::Request::new(step.request.clone());
    request.set_timeout(std::time::Duration::from_secs(5));
    case.provider()
        .await
        .invoke_model(request)
        .await
        .unwrap()
        .into_inner()
        .response
        .unwrap()
}

async fn complete_call(case: &Case, job: &wire::DiscoveryJobHandle) -> crate::store::ModelStep {
    let step = dispatch(case, job).await;
    let response = invoke(case, &step).await;
    case.store
        .finish_call(&case.actor(), step_command(case, &step), response)
        .await
        .unwrap()
}

async fn commit_tool(case: &Case, step: &crate::store::ModelStep) -> crate::store::ModelStep {
    let result = case.tool_result(step).await;
    case.store
        .record_tool(&case.actor(), step_command(case, step), result)
        .await
        .unwrap()
}

#[tokio::test]
async fn tool_reply_recovery() {
    let mut case = Case::controlled().await;
    let job = case.start().await;
    let completed = complete_call(&case, &job).await;
    assert!(completed.tool_result.is_none());
    case.resume_provider().await;
    case.clock.0.store(36_000, Ordering::SeqCst);
    case.restart().await;
    let current = case.read(&job).await;
    let result = case.execute(current.job.as_ref().unwrap()).await.unwrap();
    assert!(result.candidate.is_some());
    assert_eq!(case.calls(), 2);
    let history = case
        .store
        .model_history(&case.actor(), &job.job_id.unwrap().value)
        .await
        .unwrap();
    assert_eq!(history[0].request, completed.request);
    assert_eq!(history[0].response, completed.response);
    assert!(history[0].tool_result.is_some());
    case.close().await;
}

#[tokio::test]
async fn tool_commit_recovery() {
    let mut case = Case::controlled().await;
    let job = case.start().await;
    let completed = complete_call(&case, &job).await;
    let committed = commit_tool(&case, &completed).await;
    case.resume_provider().await;
    case.clock.0.store(36_000, Ordering::SeqCst);
    case.restart().await;
    let current = case.read(&job).await;
    let result = case.execute(current.job.as_ref().unwrap()).await.unwrap();
    assert!(result.candidate.is_some());
    assert_eq!(case.calls(), 2);
    let history = case
        .store
        .model_history(&case.actor(), &job.job_id.unwrap().value)
        .await
        .unwrap();
    assert_eq!(history[0].tool_result, committed.tool_result);
    assert_eq!(history[0].request, committed.request);
    case.close().await;
}

#[tokio::test]
async fn final_receipt_recovery() {
    let mut case = Case::controlled().await;
    let job = case.start().await;
    let completed = complete_call(&case, &job).await;
    let committed = commit_tool(&case, &completed).await;
    let mut invocation = case.final_invocation();
    let context = case.context();
    invocation.request_id = context.request_id.clone();
    invocation.messages.extend([
        v1::ModelMessage {
            role: v1::ModelRole::Assistant as i32,
            content: committed.response.as_ref().unwrap().content.clone(),
        },
        v1::ModelMessage {
            role: v1::ModelRole::Tool as i32,
            content: vec![v1::ContentBlock {
                content: Some(v1::content_block::Content::ToolResult(
                    committed.tool_result.clone().unwrap(),
                )),
            }],
        },
    ]);
    let mut command = step_command(&case, &committed);
    command.ordinal = 1;
    let reserved = case
        .store
        .reserve_model(
            &case.actor(),
            command,
            provider::InvokeModelRequest {
                context: Some(context),
                invocation: Some(invocation),
            },
        )
        .await
        .unwrap();
    let dispatched = case
        .store
        .dispatch_model(&case.actor(), step_command(&case, &reserved))
        .await
        .unwrap();
    assert!(dispatched.send);
    let response = invoke(&case, &dispatched.step).await;
    assert_eq!(case.calls(), 2);
    // The final Provider receipt exists; its Rust result transaction never ran.
    case.restart_provider().await;
    case.clock.0.store(11_000, Ordering::SeqCst);
    case.restart().await;
    let current = case.read(&job).await;
    let result = case.execute(current.job.as_ref().unwrap()).await.unwrap();
    assert!(result.candidate.is_some());
    assert_eq!(case.calls(), 2);
    let history = case
        .store
        .model_history(&case.actor(), &job.job_id.unwrap().value)
        .await
        .unwrap();
    assert_eq!(history[1].response, Some(response));
    assert_eq!(history[1].request, dispatched.step.request);
    case.close().await;
}
