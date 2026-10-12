use super::*;
use crate::store::{ModelRetry, ModelStep};

async fn pause(case: &Case, job: &wire::DiscoveryJobHandle) -> wire::DiscoveryJobHandle {
    case.client("client")
        .await
        .pause_discovery(timed(wire::PauseDiscoveryRequest {
            context: Some(case.context()),
            job_id: job.job_id.clone(),
            expected_revision: job.revision,
        }))
        .await
        .unwrap()
        .into_inner()
        .job
        .unwrap()
}

async fn resume(case: &Case, job: &wire::DiscoveryJobHandle) -> wire::DiscoveryStepView {
    case.client("client")
        .await
        .resume_discovery(timed(wire::ResumeDiscoveryRequest {
            context: Some(case.context()),
            job_id: job.job_id.clone(),
            expected_revision: job.revision,
        }))
        .await
        .unwrap()
        .into_inner()
        .step
        .unwrap()
}

async fn latest(case: &Case, job: &wire::DiscoveryJobHandle) -> ModelStep {
    case.store
        .model_step(&case.actor(), &job.job_id.as_ref().unwrap().value)
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn transient_lookup() {
    let mut case = Case::open().await;
    let job = case.start().await;
    let dispatched = dispatch(&case, &job).await;
    let response = invoke(&case, &dispatched).await;
    let current = case.read(&job).await;
    let paused = pause(&case, current.job.as_ref().unwrap()).await;
    case.lookup_faults(2);
    case.restart().await;
    let completed = resume(&case, &paused).await;
    assert!(completed.candidate.is_some());
    assert_eq!(case.lookups(), 3);
    assert_eq!(case.calls(), 1);
    let step = latest(&case, &job).await;
    assert_eq!(step.lookup_attempts, 3);
    assert_eq!(step.request, dispatched.request);
    assert_eq!(step.response, Some(response));
    case.close().await;
}

#[tokio::test]
async fn transient_exhaustion() {
    let mut case = Case::open().await;
    let job = case.start().await;
    let dispatched = dispatch(&case, &job).await;
    let current = case.read(&job).await;
    let paused = pause(&case, current.job.as_ref().unwrap()).await;
    case.lookup_faults(10);
    let failed = resume(&case, &paused).await;
    assert_eq!(
        failed.job.as_ref().unwrap().status,
        wire::DiscoveryJobStatus::InfrastructureFailed as i32,
    );
    assert_eq!(case.lookups(), 3);
    assert_eq!(case.calls(), 0);
    assert_eq!(failed.reserved_input_tokens, dispatched.reserved_input);
    assert!(failed.candidate.is_none());
    let step = latest(&case, &job).await;
    assert_eq!(step.lookup_attempts, 3);
    assert_eq!(step.request, dispatched.request);
    assert!(step.response.is_none());
    case.close().await;
}

#[tokio::test]
async fn paused_retry_budget() {
    let mut case = Case::open().await;
    let job = case.start().await;
    let dispatched = dispatch(&case, &job).await;
    // Pause after one durable attempt and actual ABSENT response. The resumed
    // service has only two attempts left, even after rebuilding its runtime.
    let attempted = case
        .store
        .retry_model(
            &case.actor(),
            step_command(&case, &dispatched),
            ModelRetry::Lookup,
        )
        .await
        .unwrap();
    let original = attempted.request.context.as_ref().unwrap();
    let mut lookup = tonic::Request::new(provider::LookupInvocationRequest {
        context: Some(case.context()),
        original_request_id: original.request_id.clone(),
        original_idempotency_key: original.idempotency_key.clone(),
        request_sha256: Some(v1::Sha256Digest {
            value: attempted.request_sha256.to_vec(),
        }),
    });
    lookup.set_timeout(std::time::Duration::from_secs(5));
    let absent = case
        .provider()
        .await
        .lookup_invocation(lookup)
        .await
        .unwrap()
        .into_inner();
    assert_eq!(absent.state, provider::InvocationState::Absent as i32);
    let current = case.read(&job).await;
    let paused = pause(&case, current.job.as_ref().unwrap()).await;
    case.restart().await;
    assert_eq!(latest(&case, &job).await.lookup_attempts, 1);
    let failed = resume(&case, &paused).await;
    assert_eq!(
        failed.job.as_ref().unwrap().status,
        wire::DiscoveryJobStatus::InfrastructureFailed as i32
    );
    assert_eq!(case.lookups(), 3);
    assert_eq!(case.calls(), 0);
    assert_eq!(latest(&case, &job).await.lookup_attempts, 3);
    assert_eq!(failed.reserved_input_tokens, dispatched.reserved_input);
    case.close().await;
}

async fn final_dispatch(case: &Case, job: &wire::DiscoveryJobHandle) -> ModelStep {
    let completed = complete_call(case, job).await;
    let committed = commit_tool(case, &completed).await;
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
    let mut command = step_command(case, &committed);
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
        .dispatch_model(&case.actor(), step_command(case, &reserved))
        .await
        .unwrap();
    assert!(dispatched.send);
    dispatched.step
}

#[tokio::test]
async fn final_resume() {
    let mut case = Case::controlled().await;
    let job = case.start().await;
    let dispatched = final_dispatch(&case, &job).await;
    let response = invoke(&case, &dispatched).await;
    let current = case.read(&job).await;
    let paused = pause(&case, current.job.as_ref().unwrap()).await;
    case.restart().await;
    let completed = resume(&case, &paused).await;
    assert!(completed.candidate.is_some());
    assert_eq!(case.calls(), 2);
    assert_eq!(case.lookups(), 1);
    let step = latest(&case, &job).await;
    assert_eq!(step.ordinal, 1);
    assert_eq!(step.request, dispatched.request);
    assert_eq!(step.response, Some(response));
    assert_eq!(completed.reserved_input_tokens, 8192);
    case.close().await;
}

#[tokio::test]
async fn reconcile_replay() {
    let mut case = Case::open().await;
    let job = case.start().await;
    let dispatched = dispatch(&case, &job).await;
    invoke(&case, &dispatched).await;
    let current = case.read(&job).await;
    let paused = pause(&case, current.job.as_ref().unwrap()).await;
    let request = wire::ReconcileDiscoveryRequest {
        context: Some(case.context()),
        job_id: paused.job_id.clone(),
        expected_revision: paused.revision,
    };
    let first = case
        .client("client")
        .await
        .reconcile_discovery(timed(request.clone()))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(case.lookups(), 1);
    case.restart().await;
    let replay = case
        .client("client")
        .await
        .reconcile_discovery(timed(request.clone()))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(replay, first);
    assert_eq!(case.lookups(), 1);
    assert_eq!(case.calls(), 1);
    let completed = resume(&case, first.step.as_ref().unwrap().job.as_ref().unwrap()).await;
    assert!(completed.candidate.is_some());
    let replay = case
        .client("client")
        .await
        .reconcile_discovery(timed(request))
        .await
        .unwrap()
        .into_inner()
        .step
        .unwrap();
    assert_eq!(replay.job, completed.job);
    assert_eq!(
        replay.reserved_input_tokens,
        completed.reserved_input_tokens
    );
    assert_eq!(case.lookups(), 1);
    assert_eq!(case.calls(), 1);
    assert!(replay.candidate.is_none());
    case.close().await;
}

async fn prior_profile(case: &mut Case, expected_calls: u64) {
    case.prior_descriptor().await;
    let job = case.start().await;
    let id = &job.job_id.as_ref().unwrap().value;
    let before = case
        .store
        .get(id)
        .await
        .unwrap()
        .unwrap()
        .specification
        .unwrap();
    let paused = pause(case, &job).await;
    case.restart().await;
    let result = resume(case, &paused).await;
    assert!(result.candidate.is_some());
    assert_eq!(case.calls(), expected_calls);
    let after = case
        .store
        .get(id)
        .await
        .unwrap()
        .unwrap()
        .specification
        .unwrap();
    assert_eq!(after, before);
    assert_ne!(
        after
            .protocol_selection
            .unwrap()
            .schema_descriptor_sha256
            .unwrap()
            .value,
        <sha2::Sha256 as sha2::Digest>::digest(loop_protocol::FILE_DESCRIPTOR_SET).as_slice(),
    );
}

#[tokio::test]
async fn prior_single_profile() {
    let mut case = Case::open().await;
    prior_profile(&mut case, 1).await;
    case.close().await;
}

#[tokio::test]
async fn prior_tool_profile() {
    let mut case = Case::controlled().await;
    prior_profile(&mut case, 2).await;
    case.close().await;
}
