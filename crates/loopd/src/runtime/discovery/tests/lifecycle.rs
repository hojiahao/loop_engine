use super::*;

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

async fn cancel(case: &Case, job: &wire::DiscoveryJobHandle) -> wire::DiscoveryJobHandle {
    case.client("client")
        .await
        .cancel_discovery(timed(wire::CancelDiscoveryRequest {
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

async fn reconcile(case: &Case, job: &wire::DiscoveryJobHandle) -> wire::DiscoveryStepView {
    case.client("client")
        .await
        .reconcile_discovery(timed(wire::ReconcileDiscoveryRequest {
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

#[tokio::test]
async fn queued_resume() {
    let mut case = Case::open().await;
    let job = case.start().await;
    let paused = pause(&case, &job).await;
    assert_eq!(paused.status, wire::DiscoveryJobStatus::Paused as i32);
    assert!(case.execute(&paused).await.is_err());
    assert_eq!(case.calls(), 0);
    case.restart().await;
    let mut client = case.client("client").await;
    let request = wire::ResumeDiscoveryRequest {
        context: Some(case.context()),
        job_id: paused.job_id.clone(),
        expected_revision: paused.revision,
    };
    let result = client
        .resume_discovery(timed(request.clone()))
        .await
        .unwrap()
        .into_inner()
        .step
        .unwrap();
    assert!(result.candidate.is_some());
    assert_eq!(
        client
            .resume_discovery(timed(request))
            .await
            .unwrap()
            .into_inner()
            .step
            .unwrap(),
        result
    );
    assert_eq!(case.calls(), 1);
    assert_eq!(result.reserved_input_tokens, 4096);
    case.close().await;
}

#[tokio::test]
async fn tool_resume() {
    let mut case = Case::controlled().await;
    let job = case.start().await;
    let first = complete_call(&case, &job).await;
    let current = case.read(&job).await;
    let paused = pause(&case, current.job.as_ref().unwrap()).await;
    case.restart().await;
    let result = resume(&case, &paused).await;
    assert!(result.candidate.is_some());
    assert_eq!(case.calls(), 2);
    let history = case
        .store
        .model_history(&case.actor(), &job.job_id.unwrap().value)
        .await
        .unwrap();
    assert_eq!(history[0].request, first.request);
    assert_eq!(history[0].response, first.response);
    assert_eq!(history[0].tool_attempts, 1);
    case.close().await;
}

#[tokio::test]
async fn cancelled_receipt() {
    let mut case = Case::open().await;
    let job = case.start().await;
    let dispatched = dispatch(&case, &job).await;
    let response = invoke(&case, &dispatched).await;
    let current = case.read(&job).await;
    let cancelled = cancel(&case, current.job.as_ref().unwrap()).await;
    case.restart_provider().await;
    // Data corruption must not block evidence-only lookup or grant execution.
    case.corrupt_data();
    let result = reconcile(&case, &cancelled).await;
    assert_eq!(
        result.job.as_ref().unwrap().status,
        wire::DiscoveryJobStatus::Cancelled as i32
    );
    assert_eq!(result.state, wire::DiscoveryStepState::Completed as i32);
    assert!(result.candidate.is_none());
    assert_eq!(result.reserved_input_tokens, 4096);
    assert_eq!(case.calls(), 1);
    assert_eq!(
        case.store
            .model_step(&case.actor(), &job.job_id.unwrap().value)
            .await
            .unwrap()
            .unwrap()
            .response,
        Some(response)
    );
    case.close().await;
}

#[tokio::test]
async fn reconciled_resume() {
    let mut case = Case::open().await;
    let job = case.start().await;
    let dispatched = dispatch(&case, &job).await;
    invoke(&case, &dispatched).await;
    let current = case.read(&job).await;
    let paused = pause(&case, current.job.as_ref().unwrap()).await;
    let evidence = reconcile(&case, &paused).await;
    assert!(evidence.candidate.is_none());
    let result = resume(&case, evidence.job.as_ref().unwrap()).await;
    assert!(result.candidate.is_some());
    assert_eq!(case.calls(), 1);
    case.close().await;
}

#[tokio::test]
async fn disabled_cancel() {
    let mut case = Case::controlled().await;
    let job = case.start().await;
    case.corrupt_plan();
    case.corrupt_data();
    case.stop_only().await;
    let paused = pause(&case, &job).await;
    let cancelled = cancel(&case, &paused).await;
    assert_eq!(cancelled.status, wire::DiscoveryJobStatus::Cancelled as i32);
    assert_eq!(case.calls(), 0);
    assert!(case.execute(&cancelled).await.is_err());
    case.close().await;
}

#[tokio::test]
async fn expired_pause() {
    let mut case = Case::open().await;
    let job = case.start().await;
    let paused = pause(&case, &job).await;
    case.clock.0.store(121_000, Ordering::SeqCst);
    let result = resume(&case, &paused).await;
    assert_eq!(
        result.job.as_ref().unwrap().status,
        wire::DiscoveryJobStatus::BudgetExhausted as i32
    );
    assert_eq!(case.calls(), 0);
    assert!(result.reserved_cost.is_none());
    case.close().await;
}

#[tokio::test]
async fn live_cancellation() {
    let mut case = Case::open().await;
    let job = case.start().await;
    case.delay_supplier();
    let mut client = case.client("client").await;
    let request = timed(wire::ExecuteDiscoveryRequest {
        context: Some(case.context()),
        job_id: job.job_id.clone(),
        expected_revision: job.revision,
    });
    let running = tokio::spawn(async move { client.execute_discovery(request).await });
    let limit = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while case.calls() == 0 {
        assert!(std::time::Instant::now() < limit);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let current = case.read(&job).await;
    let cancelled = cancel(&case, current.job.as_ref().unwrap()).await;
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(2), running)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    let result = case.read(&cancelled).await;
    assert_eq!(
        result.job.as_ref().unwrap().status,
        wire::DiscoveryJobStatus::Cancelled as i32
    );
    assert!(result.candidate.is_none());
    assert_eq!(case.calls(), 1);
    assert_eq!(result.reserved_input_tokens, 4096);
    case.close().await;
}

#[tokio::test]
async fn response_usage() {
    invalid_response("usage").await;
}

#[tokio::test]
async fn response_identity() {
    invalid_response("identity").await;
}

async fn invalid_response(mode: &str) {
    let mut case = Case::open().await;
    case.invalid_response(mode);
    let job = case.start().await;
    let result = case.execute(&job).await.unwrap();
    assert_eq!(
        result.job.as_ref().unwrap().status,
        wire::DiscoveryJobStatus::InfrastructureFailed as i32
    );
    assert!(result.candidate.is_none());
    assert_eq!(case.calls(), 1);
    assert_eq!(result.reserved_input_tokens, 4096);
    let current = case
        .store
        .get(&job.job_id.unwrap().value)
        .await
        .unwrap()
        .unwrap();
    let Some(v1::job_outcome::Outcome::InfrastructureFailure(failure)) =
        current.outcome.unwrap().outcome
    else {
        panic!("expected infrastructure outcome")
    };
    assert_eq!(failure.error.unwrap().code, "model_contract_invalid");
    case.close().await;
}
