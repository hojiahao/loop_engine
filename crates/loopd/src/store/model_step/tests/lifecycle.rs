use super::*;
use std::sync::atomic::AtomicUsize;

pub(super) fn unleased(job: &JobRecord, key: &str) -> ModelStepCommand {
    let mut command = command(job, key);
    command.lease_id = None;
    command
}

async fn dispatched(store: &PgJobStore) -> ModelStep {
    let step = reserve(store).await;
    store
        .dispatch_model(&fixture::actor(), command(&step.job, "dispatch"))
        .await
        .unwrap()
        .step
}

#[tokio::test]
async fn pause_preserves() {
    let (_directory, store, _) = setup().await;
    let step = dispatched(&store).await;
    let paused = store
        .control_model(
            &fixture::actor(),
            unleased(&step.job, "pause"),
            ModelControl::Pause,
        )
        .await
        .unwrap();
    assert_eq!(paused.state, JobState::Paused as i32);
    assert_eq!(paused.attempt, step.job.attempt);
    assert!(paused.active_lease.is_none());
    assert!(paused.outcome.is_none());
    let history = store
        .model_history(&fixture::actor(), "job.1")
        .await
        .unwrap();
    let mut expected = step.clone();
    expected.job = paused;
    assert_eq!(history, vec![expected]);
    assert!(matches!(
        store
            .finish_model(
                &fixture::actor(),
                command(&step.job, "late"),
                response(&step),
                outcome()
            )
            .await,
        Err(StoreError::RevisionConflict)
    ));
    store.close().await;
}

#[tokio::test]
async fn pause_unreserved() {
    let (_directory, store, _) = setup().await;
    let job = store.get("job.1").await.unwrap().unwrap();
    let paused = store
        .control_model(
            &fixture::actor(),
            unleased(&job, "pause"),
            ModelControl::Pause,
        )
        .await
        .unwrap();
    assert_eq!(paused.attempt, 0);
    assert_eq!(paused.state, JobState::Paused as i32);
    assert!(
        store
            .model_history(&fixture::actor(), "job.1")
            .await
            .unwrap()
            .is_empty()
    );
    let cmd = unleased(&paused, "resume");
    let resumed = store
        .resume_model(&fixture::actor(), cmd.clone(), Some(invocation()))
        .await
        .unwrap();
    assert!(resumed.execute);
    let mut changed = invocation();
    changed
        .context
        .as_mut()
        .unwrap()
        .request_id
        .as_mut()
        .unwrap()
        .value = "regenerated".into();
    let replay = store
        .resume_model(&fixture::actor(), cmd, Some(changed))
        .await
        .unwrap();
    assert!(!replay.execute);
    assert_eq!(replay.step, resumed.step);
    assert_eq!(resumed.job.attempt, 1);
    store.close().await;
}

#[tokio::test]
async fn control_replay() {
    let (_directory, store, _) = setup().await;
    let step = reserve(&store).await;
    let cmd = unleased(&step.job, "pause");
    let paused = store
        .control_model(&fixture::actor(), cmd.clone(), ModelControl::Pause)
        .await
        .unwrap();
    let resumed = store
        .resume_model(&fixture::actor(), unleased(&paused, "resume"), None)
        .await
        .unwrap();
    let replay = store
        .control_model(&fixture::actor(), cmd.clone(), ModelControl::Pause)
        .await
        .unwrap();
    assert_eq!(replay, resumed.job);
    let mut conflict = cmd;
    conflict.expected_revision += 1;
    assert!(matches!(
        store
            .control_model(&fixture::actor(), conflict, ModelControl::Pause)
            .await,
        Err(StoreError::IdempotencyConflict)
    ));
    assert_eq!(store.audit_events(0, 100).await.unwrap().len(), 4);
    store.close().await;
}

#[tokio::test]
async fn control_denied() {
    let (_directory, store, _) = setup().await;
    let step = reserve(&store).await;
    assert!(matches!(
        store
            .control_model(
                &fixture::actor(),
                command(&step.job, "leased"),
                ModelControl::Pause
            )
            .await,
        Err(StoreError::Invalid(_))
    ));
    let mut wrong = unleased(&step.job, "ordinal");
    wrong.ordinal = 1;
    assert!(matches!(
        store
            .control_model(&fixture::actor(), wrong, ModelControl::Pause)
            .await,
        Err(StoreError::Invalid(_))
    ));
    assert!(matches!(
        store
            .control_model(
                &fixture::actor(),
                unleased(&step.job, "early"),
                ModelControl::Expire
            )
            .await,
        Err(StoreError::InvalidTransition)
    ));
    let mut stranger = fixture::actor();
    stranger.authenticated_subject = "service:foreign".into();
    assert!(matches!(
        store
            .control_model(
                &stranger,
                unleased(&step.job, "foreign"),
                ModelControl::Cancel
            )
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    let cancelled = store
        .control_model(
            &fixture::actor(),
            unleased(&step.job, "cancel"),
            ModelControl::Cancel,
        )
        .await
        .unwrap();
    assert!(matches!(
        store
            .resume_model(&fixture::actor(), unleased(&cancelled, "resume"), None)
            .await,
        Err(StoreError::InvalidTransition)
    ));
    assert!(matches!(
        store
            .control_model(
                &fixture::actor(),
                unleased(&cancelled, "pause"),
                ModelControl::Pause
            )
            .await,
        Err(StoreError::InvalidTransition)
    ));
    store.close().await;
}

#[tokio::test]
async fn corrupt_stop() {
    let (directory, store, _) = setup().await;
    let step = reserve(&store).await;
    let mut connection = fixture::connection(&directory).await;
    sqlx::query("ALTER TABLE model_steps DISABLE TRIGGER model_steps_protected")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("UPDATE model_steps SET request_blob=decode('00','hex')")
        .execute(&mut connection)
        .await
        .unwrap();
    let cancelled = store
        .control_model(
            &fixture::actor(),
            unleased(&step.job, "cancel"),
            ModelControl::Cancel,
        )
        .await
        .unwrap();
    assert_eq!(cancelled.state, JobState::Cancelled as i32);
    assert!(matches!(
        store.model_history(&fixture::actor(), "job.1").await,
        Err(StoreError::Corrupt(_))
    ));
    store.close().await;
}

#[tokio::test]
async fn resume_preserves() {
    let (_directory, store, _) = setup().await;
    let step = dispatched(&store).await;
    let paused = store
        .control_model(
            &fixture::actor(),
            unleased(&step.job, "pause"),
            ModelControl::Pause,
        )
        .await
        .unwrap();
    assert!(matches!(
        store
            .resume_model(
                &fixture::actor(),
                unleased(&paused, "replacement"),
                Some(invocation())
            )
            .await,
        Err(StoreError::Invalid(_))
    ));
    let cmd = unleased(&paused, "resume");
    let resumed = store
        .resume_model(&fixture::actor(), cmd.clone(), None)
        .await
        .unwrap();
    let mut expected = step;
    expected.job = resumed.job.clone();
    assert_eq!(resumed.step, Some(expected));
    assert!(resumed.execute);
    assert!(
        !store
            .resume_model(&fixture::actor(), cmd, None)
            .await
            .unwrap()
            .execute
    );
    let send = store
        .dispatch_model(&fixture::actor(), command(&resumed.job, "redispatch"))
        .await
        .unwrap();
    assert!(!send.send);
    store.close().await;
}

#[tokio::test]
async fn resume_second() {
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
    let paused = store
        .control_model(
            &fixture::actor(),
            unleased(&second.job, "pause"),
            ModelControl::Pause,
        )
        .await
        .unwrap();
    let mut wrong = unleased(&paused, "wrong");
    wrong.ordinal = 1;
    assert!(matches!(
        store.resume_model(&fixture::actor(), wrong, None).await,
        Err(StoreError::Invalid(_))
    ));
    let resumed = store
        .resume_model(&fixture::actor(), unleased(&paused, "resume"), None)
        .await
        .unwrap();
    assert_eq!(resumed.step.as_ref().unwrap().ordinal, 1);
    assert_eq!(resumed.step.as_ref().unwrap().request, second.request);
    assert_eq!(
        store
            .model_history(&fixture::actor(), "job.1")
            .await
            .unwrap()[0]
            .tool_result,
        first.tool_result
    );
    store.close().await;
}

#[tokio::test]
async fn pause_expired() {
    let (_directory, store, clock) = setup().await;
    let step = dispatched(&store).await;
    clock.0.store(fixture::NOW + 120_000, Ordering::SeqCst);
    let mut cmd = unleased(&step.job, "pause");
    cmd.context.as_mut().unwrap().requested_at = Some(fixture::timestamp(fixture::NOW + 120_000));
    let expired = store
        .control_model(&fixture::actor(), cmd, ModelControl::Pause)
        .await
        .unwrap();
    assert_eq!(expired.state, JobState::BudgetExhausted as i32);
    assert_eq!(
        store
            .model_step(&fixture::actor(), "job.1")
            .await
            .unwrap()
            .unwrap()
            .request,
        step.request
    );
    store.close().await;
}

#[tokio::test]
async fn resume_expired() {
    let (_directory, store, clock) = setup().await;
    let step = reserve(&store).await;
    let paused = store
        .control_model(
            &fixture::actor(),
            unleased(&step.job, "pause"),
            ModelControl::Pause,
        )
        .await
        .unwrap();
    clock.0.store(fixture::NOW + 120_000, Ordering::SeqCst);
    let mut cmd = unleased(&paused, "resume");
    cmd.context.as_mut().unwrap().requested_at = Some(fixture::timestamp(fixture::NOW + 120_000));
    let mut stale = cmd.clone();
    stale.expected_revision -= 1;
    assert!(matches!(
        store.resume_model(&fixture::actor(), stale, None).await,
        Err(StoreError::RevisionConflict)
    ));
    let mut zero = cmd.clone();
    zero.expected_revision = 0;
    assert!(matches!(
        store.resume_model(&fixture::actor(), zero, None).await,
        Err(StoreError::Invalid(_))
    ));
    let expired = store
        .resume_model(&fixture::actor(), cmd.clone(), None)
        .await
        .unwrap();
    assert_eq!(expired.job.state, JobState::BudgetExhausted as i32);
    assert!(!expired.execute);
    assert!(expired.step.is_none());
    let replay = store
        .resume_model(&fixture::actor(), cmd, None)
        .await
        .unwrap();
    assert_eq!(replay.job, expired.job);
    assert!(!replay.execute);
    assert_eq!(replay.step.as_ref().unwrap().request, step.request);
    store.close().await;
}

#[tokio::test]
async fn retry_bounded() {
    let (directory, store, clock) = setup().await;
    let step = dispatched(&store).await;
    let cmd = command(&step.job, "lookup.1");
    let first = store
        .retry_model(&fixture::actor(), cmd.clone(), ModelRetry::Lookup)
        .await
        .unwrap();
    assert_eq!(first.lookup_attempts, 1);
    assert_eq!(first.retry_after_ms, fixture::NOW + 250);
    assert!(matches!(
        store
            .retry_model(&fixture::actor(), cmd, ModelRetry::Lookup)
            .await,
        Err(StoreError::Invalid("model retry replay"))
    ));
    assert!(matches!(
        store
            .retry_model(
                &fixture::actor(),
                command(&first.job, "early"),
                ModelRetry::Lookup
            )
            .await,
        Err(StoreError::Invalid("model retry backoff"))
    ));
    let paused = store
        .control_model(
            &fixture::actor(),
            unleased(&first.job, "pause"),
            ModelControl::Pause,
        )
        .await
        .unwrap();
    store.close().await;
    clock.0.store(fixture::NOW + 250, Ordering::SeqCst);
    let reopened = PgJobStore::open(options(&directory.path().join("state"), clock.clone()))
        .await
        .unwrap();
    let resumed = reopened
        .resume_model(&fixture::actor(), unleased(&paused, "resume"), None)
        .await
        .unwrap();
    assert_eq!(resumed.step.as_ref().unwrap().lookup_attempts, 1);
    let second = reopened
        .retry_model(
            &fixture::actor(),
            command(&resumed.job, "lookup.2"),
            ModelRetry::Lookup,
        )
        .await
        .unwrap();
    clock.0.store(fixture::NOW + 500, Ordering::SeqCst);
    let third = reopened
        .retry_model(
            &fixture::actor(),
            command(&second.job, "lookup.3"),
            ModelRetry::Lookup,
        )
        .await
        .unwrap();
    assert_eq!(third.lookup_attempts, 3);
    clock.0.store(fixture::NOW + 750, Ordering::SeqCst);
    assert!(matches!(
        reopened
            .retry_model(
                &fixture::actor(),
                command(&third.job, "lookup.4"),
                ModelRetry::Lookup
            )
            .await,
        Err(StoreError::Invalid("model retries exhausted"))
    ));
    assert_eq!(third.request, step.request);
    reopened.close().await;
}

#[tokio::test]
async fn tool_retries() {
    let (_directory, store, clock) = tool_setup().await;
    let mut step = tool_called(&store).await;
    for index in 0..3 {
        clock.0.store(fixture::NOW + index * 250, Ordering::SeqCst);
        step = store
            .retry_model(
                &fixture::actor(),
                command(&step.job, &format!("tool.{index}")),
                ModelRetry::Tool,
            )
            .await
            .unwrap();
    }
    assert_eq!(step.tool_attempts, 3);
    assert!(matches!(
        store
            .retry_model(
                &fixture::actor(),
                command(&step.job, "tool.excess"),
                ModelRetry::Tool
            )
            .await,
        Err(StoreError::Invalid("model retries exhausted"))
    ));
    let result = store
        .record_tool(
            &fixture::actor(),
            command(&step.job, "record"),
            tool_result(),
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .model_step(&fixture::actor(), "job.1")
            .await
            .unwrap()
            .unwrap(),
        result
    );
    assert!(matches!(
        store
            .retry_model(
                &fixture::actor(),
                command(&result.job, "repeat"),
                ModelRetry::Tool
            )
            .await,
        Err(StoreError::InvalidTransition)
    ));
    store.close().await;
}

#[tokio::test]
async fn retry_stages() {
    let (_directory, store, _) = setup().await;
    let step = reserve(&store).await;
    for kind in [ModelRetry::Lookup, ModelRetry::Tool] {
        assert!(matches!(
            store
                .retry_model(&fixture::actor(), command(&step.job, "retry"), kind)
                .await,
            Err(StoreError::InvalidTransition)
        ));
    }
    let step = store
        .dispatch_model(&fixture::actor(), command(&step.job, "dispatch"))
        .await
        .unwrap()
        .step;
    assert!(matches!(
        store
            .retry_model(
                &fixture::actor(),
                command(&step.job, "tool"),
                ModelRetry::Tool
            )
            .await,
        Err(StoreError::InvalidTransition)
    ));
    let complete = store
        .finish_model(
            &fixture::actor(),
            command(&step.job, "finish"),
            response(&step),
            outcome(),
        )
        .await
        .unwrap();
    assert!(matches!(
        store
            .retry_model(
                &fixture::actor(),
                command(&complete.job, "lookup"),
                ModelRetry::Lookup
            )
            .await,
        Err(StoreError::LeaseFenced)
    ));
    store.close().await;
}

#[tokio::test]
async fn late_evidence() {
    let (_directory, store, clock) = setup().await;
    let step = dispatched(&store).await;
    let cancelled = store
        .control_model(
            &fixture::actor(),
            unleased(&step.job, "cancel"),
            ModelControl::Cancel,
        )
        .await
        .unwrap();
    clock.0.store(fixture::NOW + 180_000, Ordering::SeqCst);
    let mut cmd = unleased(&cancelled, "reconcile");
    cmd.context.as_mut().unwrap().requested_at = Some(fixture::timestamp(fixture::NOW + 180_000));
    let reconciled = store
        .reconcile_model(&fixture::actor(), cmd.clone(), response(&step))
        .await
        .unwrap();
    assert_eq!(reconciled.job.state, JobState::Cancelled as i32);
    assert_eq!(reconciled.job.outcome, cancelled.outcome);
    assert_eq!(reconciled.state, ModelStepState::Completed);
    assert_eq!(reconciled.reserved_nano_usd, step.reserved_nano_usd);
    assert_eq!(reconciled.lookup_attempts, 0);
    assert_eq!(
        store
            .reconcile_model(&fixture::actor(), cmd.clone(), response(&step))
            .await
            .unwrap(),
        reconciled
    );
    let mut wrong = response(&step);
    wrong.usage.as_mut().unwrap().output_tokens += 1;
    assert!(matches!(
        store.reconcile_model(&fixture::actor(), cmd, wrong).await,
        Err(StoreError::IdempotencyConflict)
    ));
    store.close().await;
}

#[tokio::test]
async fn reconciled_resume() {
    let (_directory, store, _) = setup().await;
    let step = dispatched(&store).await;
    let paused = store
        .control_model(
            &fixture::actor(),
            unleased(&step.job, "pause"),
            ModelControl::Pause,
        )
        .await
        .unwrap();
    let cmd = unleased(&paused, "reconcile");
    let reconciled = store
        .reconcile_model(&fixture::actor(), cmd.clone(), response(&step))
        .await
        .unwrap();
    let resumed = store
        .resume_model(&fixture::actor(), unleased(&reconciled.job, "resume"), None)
        .await
        .unwrap();
    assert_eq!(
        resumed.step.as_ref().unwrap().state,
        ModelStepState::Completed
    );
    let completed = store
        .finish_model(
            &fixture::actor(),
            command(&resumed.job, "finish"),
            response(&step),
            outcome(),
        )
        .await
        .unwrap();
    assert_eq!(completed.job.state, JobState::Succeeded as i32);
    assert_eq!(completed.response, reconciled.response);
    assert_eq!(completed.request, step.request);
    let events = store.audit_events(0, 100).await.unwrap();
    assert_eq!(
        store
            .reconcile_replay(&fixture::actor(), cmd)
            .await
            .unwrap(),
        Some(completed.job)
    );
    assert_eq!(store.audit_events(0, 100).await.unwrap(), events);
    store.close().await;
}

#[tokio::test]
async fn replay_ordinal() {
    let (_directory, store, clock) = tool_setup().await;
    let first = tool_ready(&store).await;
    let second = store
        .reserve_model(
            &fixture::actor(),
            next_command(&first.job, "reserve.second"),
            next_request(&first),
        )
        .await
        .unwrap();
    let second = store
        .dispatch_model(
            &fixture::actor(),
            next_command(&second.job, "dispatch.second"),
        )
        .await
        .unwrap()
        .step;
    let paused = store
        .control_model(
            &fixture::actor(),
            unleased(&second.job, "pause"),
            ModelControl::Pause,
        )
        .await
        .unwrap();
    let mut cmd = unleased(&paused, "reconcile");
    assert!(
        store
            .reconcile_replay(&fixture::actor(), cmd.clone())
            .await
            .unwrap()
            .is_none()
    );
    let mut persisted = cmd.clone();
    persisted.ordinal = 1;
    let reconciled = store
        .reconcile_model(&fixture::actor(), persisted, response(&second))
        .await
        .unwrap();
    let resumed = store
        .resume_model(&fixture::actor(), unleased(&reconciled.job, "resume"), None)
        .await
        .unwrap();
    let completed = store
        .finish_model(
            &fixture::actor(),
            next_command(&resumed.job, "finish"),
            response(&second),
            outcome(),
        )
        .await
        .unwrap();
    clock.0.store(fixture::NOW + 180_000, Ordering::SeqCst);
    let context = cmd.context.as_mut().unwrap();
    context.requested_at = Some(fixture::timestamp(fixture::NOW + 180_000));
    context.request_id.as_mut().unwrap().value = "fresh.transport".into();
    assert_eq!(
        store
            .reconcile_replay(&fixture::actor(), cmd.clone())
            .await
            .unwrap(),
        Some(completed.job.clone())
    );
    // RPCs carry ordinal zero; even another internal value cannot select history.
    cmd.ordinal = u32::MAX;
    assert_eq!(
        store
            .reconcile_replay(&fixture::actor(), cmd)
            .await
            .unwrap(),
        Some(completed.job)
    );
    store.close().await;
}

#[tokio::test]
async fn replay_progressed() {
    let (_directory, store, _) = tool_setup().await;
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
    let paused = store
        .control_model(
            &fixture::actor(),
            unleased(&first.job, "pause"),
            ModelControl::Pause,
        )
        .await
        .unwrap();
    let cmd = unleased(&paused, "reconcile");
    let reconciled = store
        .reconcile_model(&fixture::actor(), cmd.clone(), call_response(&first))
        .await
        .unwrap();
    let resumed = store
        .resume_model(&fixture::actor(), unleased(&reconciled.job, "resume"), None)
        .await
        .unwrap();
    let first = store
        .record_tool(
            &fixture::actor(),
            command(&resumed.job, "tool"),
            tool_result(),
        )
        .await
        .unwrap();
    let second = store
        .reserve_model(
            &fixture::actor(),
            next_command(&first.job, "reserve.second"),
            next_request(&first),
        )
        .await
        .unwrap();
    let events = store.audit_events(0, 100).await.unwrap();
    assert_eq!(
        store
            .reconcile_replay(&fixture::actor(), cmd)
            .await
            .unwrap(),
        Some(second.job)
    );
    assert_eq!(store.audit_events(0, 100).await.unwrap(), events);
    store.close().await;
}

#[tokio::test]
async fn replay_binding() {
    let case = super::processes::lifecycle_case("reconcile").await;
    super::processes::execute(&case.store, "reconcile", case.command.clone())
        .await
        .unwrap();
    let current = case.store.get("job.1").await.unwrap().unwrap();
    let mut other = submission();
    other.request_id = "request.other".into();
    other.specification.job_id.as_mut().unwrap().value = "job.other".into();
    other.specification.idempotency_key.as_mut().unwrap().value = "other".into();
    case.store.submit(other).await.unwrap();
    for change in ["revision", "job", "context"] {
        let mut cmd = case.command.clone();
        match change {
            "revision" => cmd.expected_revision += 1,
            "job" => cmd.job_id.as_mut().unwrap().value = "job.other".into(),
            "context" => {
                cmd.context
                    .as_mut()
                    .unwrap()
                    .correlation_id
                    .as_mut()
                    .unwrap()
                    .value = "other".into();
            }
            _ => unreachable!(),
        }
        assert!(
            matches!(
                case.store.reconcile_replay(&fixture::actor(), cmd).await,
                Err(StoreError::IdempotencyConflict)
            ),
            "{change}"
        );
    }
    let mut missing = case.command.clone();
    missing
        .context
        .as_mut()
        .unwrap()
        .idempotency_key
        .as_mut()
        .unwrap()
        .value = "missing".into();
    assert!(
        case.store
            .reconcile_replay(&fixture::actor(), missing)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(case.store.get("job.1").await.unwrap().unwrap(), current);
    case.store.close().await;
}

#[tokio::test]
async fn replay_authority() {
    struct Revoked;
    impl AdmissionPolicy for Revoked {
        fn validate_submission(&self, _: &JobSpecification) -> StoreResult<()> {
            Err(StoreError::AdmissionDenied)
        }

        fn authorize_job_command(
            &self,
            operation: &str,
            _: &Actor,
            _: &JobRecord,
        ) -> StoreResult<()> {
            assert_eq!(operation, "loop.model.reconcile");
            Err(StoreError::AdmissionDenied)
        }
    }
    let case = super::processes::lifecycle_case("reconcile").await;
    super::processes::execute(&case.store, "reconcile", case.command.clone())
        .await
        .unwrap();
    let mut stranger = fixture::actor();
    stranger.authenticated_subject = "service:foreign".into();
    let mut forged = case.command.clone();
    forged.context.as_mut().unwrap().actor = Some(stranger.clone());
    assert!(matches!(
        case.store.reconcile_replay(&stranger, forged).await,
        Err(StoreError::AdmissionDenied)
    ));
    let mut revoked = case.store.clone();
    revoked.admission = Arc::new(Revoked);
    assert!(matches!(
        revoked
            .reconcile_replay(&fixture::actor(), case.command)
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    case.store.close().await;
}

#[tokio::test]
async fn replay_corrupt() {
    let case = super::processes::lifecycle_case("reconcile").await;
    super::processes::execute(&case.store, "reconcile", case.command.clone())
        .await
        .unwrap();
    let mut connection = fixture::connection(&case.directory).await;
    sqlx::query("ALTER TABLE command_receipts DISABLE TRIGGER command_receipts_no_update")
        .execute(&mut connection)
        .await
        .unwrap();
    for field in ["request_sha256", "response_sha256"] {
        let original: Vec<u8> = sqlx::query_scalar(&format!(
            "SELECT {field} FROM command_receipts WHERE operation='loop.model.reconcile'"
        ))
        .fetch_one(&mut connection)
        .await
        .unwrap();
        sqlx::query(&format!(
            "UPDATE command_receipts SET {field}=$1 WHERE operation='loop.model.reconcile'"
        ))
        .bind(vec![0_u8; 32])
        .execute(&mut connection)
        .await
        .unwrap();
        assert!(matches!(
            case.store
                .reconcile_replay(&fixture::actor(), case.command.clone())
                .await,
            Err(StoreError::Corrupt(_))
        ));
        sqlx::query(&format!(
            "UPDATE command_receipts SET {field}=$1 WHERE operation='loop.model.reconcile'"
        ))
        .bind(original)
        .execute(&mut connection)
        .await
        .unwrap();
    }
    case.store.close().await;
}

#[tokio::test]
async fn reconcile_denied() {
    let (_directory, store, _) = setup().await;
    let step = dispatched(&store).await;
    assert!(matches!(
        store
            .reconcile_model(
                &fixture::actor(),
                unleased(&step.job, "running"),
                response(&step)
            )
            .await,
        Err(StoreError::InvalidTransition)
    ));
    assert!(matches!(
        store
            .reconcile_model(
                &fixture::actor(),
                command(&step.job, "leased"),
                response(&step)
            )
            .await,
        Err(StoreError::Invalid(_))
    ));
    let paused = store
        .control_model(
            &fixture::actor(),
            unleased(&step.job, "pause"),
            ModelControl::Pause,
        )
        .await
        .unwrap();
    let mut wrong = response(&step);
    wrong.request_id.as_mut().unwrap().value = "different".into();
    assert!(matches!(
        store
            .reconcile_model(&fixture::actor(), unleased(&paused, "wrong"), wrong)
            .await,
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(store.get("job.1").await.unwrap().unwrap(), paused);
    store.close().await;
}

#[tokio::test]
async fn failure_retains() {
    let (_directory, store, _) = setup().await;
    let step = dispatched(&store).await;
    assert!(matches!(
        store
            .fail_model(
                &fixture::actor(),
                command(&step.job, "raw"),
                "secret raw failure"
            )
            .await,
        Err(StoreError::Invalid(_))
    ));
    let failed = store
        .fail_model(
            &fixture::actor(),
            command(&step.job, "fail"),
            "retry_exhausted",
        )
        .await
        .unwrap();
    assert_eq!(failed.state, JobState::InfrastructureFailed as i32);
    assert!(failed.active_lease.is_none());
    let preserved = store
        .model_step(&fixture::actor(), "job.1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(preserved.state, ModelStepState::Dispatched);
    assert_eq!(preserved.request, step.request);
    assert_eq!(preserved.reserved_nano_usd, step.reserved_nano_usd);
    store.close().await;
}

#[tokio::test]
async fn failure_expired() {
    let (_directory, store, clock) = setup().await;
    let step = dispatched(&store).await;
    clock.0.store(fixture::NOW + 120_000, Ordering::SeqCst);
    let mut cmd = command(&step.job, "fail");
    cmd.context.as_mut().unwrap().requested_at = Some(fixture::timestamp(fixture::NOW + 120_000));
    let failed = store
        .fail_model(&fixture::actor(), cmd, "model_deadline")
        .await
        .unwrap();
    assert_eq!(failed.state, JobState::BudgetExhausted as i32);
    store.close().await;
}

#[tokio::test]
async fn retry_trigger() {
    use sqlx::Connection;
    let (directory, store, _) = tool_setup().await;
    let first = tool_called(&store).await;
    store
        .retry_model(
            &fixture::actor(),
            command(&first.job, "tool"),
            ModelRetry::Tool,
        )
        .await
        .unwrap();
    for update in [
        "tool_attempts=0,retry_after_ms=0",
        "tool_attempts=4,retry_after_ms=updated_at_ms+500",
        "tool_attempts=tool_attempts+1,retry_after_ms=updated_at_ms+500,response_blob=decode('00','hex')",
        "response_revision=response_revision+1",
    ] {
        let mut connection = fixture::connection(&directory).await;
        let mut transaction = connection.begin().await.unwrap();
        sqlx::query("SELECT set_config('loop.model_step_writer','v3',true)")
            .execute(&mut *transaction)
            .await
            .unwrap();
        let query = format!(
            "UPDATE model_steps SET {update},updated_revision=updated_revision+1,updated_at_ms=updated_at_ms+250 WHERE job_id='job.1'"
        );
        assert!(
            sqlx::query(&query)
                .execute(&mut *transaction)
                .await
                .is_err(),
            "{update}"
        );
        transaction.rollback().await.unwrap();
    }
    assert_eq!(
        store
            .model_step(&fixture::actor(), "job.1")
            .await
            .unwrap()
            .unwrap()
            .tool_attempts,
        1
    );
    store.close().await;
}

#[tokio::test]
async fn lifecycle_writer() {
    use sqlx::Connection;
    let (directory, store, _) = setup().await;
    // Even an unreserved Discovery job must not be revived by an older writer.
    let mut connection = fixture::connection(&directory).await;
    let mut transaction = connection.begin().await.unwrap();
    sqlx::query("SELECT set_config('loop.model_step_writer','v2',true)")
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
async fn lifecycle_migration() {
    use sqlx::Connection;
    let (directory, store, _) = setup().await;
    let step = dispatched(&store).await;
    store
        .finish_model(
            &fixture::actor(),
            command(&step.job, "finish"),
            response(&step),
            outcome(),
        )
        .await
        .unwrap();
    let source = fixture::schema(&directory.path().join("state"));
    let schema = format!("loop_lifecycle_migration_{}", uuid::Uuid::new_v4().simple());
    let mut connection = fixture::connection(&directory).await;
    let mut transaction = connection.begin().await.unwrap();
    sqlx::raw_sql(&format!(
        "CREATE SCHEMA {schema}; SET LOCAL search_path TO {schema};"
    ))
    .execute(&mut *transaction)
    .await
    .unwrap();
    sqlx::raw_sql(include_str!(
        "../../../../../../migrations/postgres/0001_durable_jobs.sql"
    ))
    .execute(&mut *transaction)
    .await
    .unwrap();
    sqlx::raw_sql(include_str!(
        "../../../../../../migrations/postgres/0011_model_steps.sql"
    ))
    .execute(&mut *transaction)
    .await
    .unwrap();
    sqlx::raw_sql(include_str!(
        "../../../../../../migrations/postgres/0012_tool_context.sql"
    ))
    .execute(&mut *transaction)
    .await
    .unwrap();
    sqlx::query("SELECT set_config('loop.model_step_writer','v2',true)")
        .execute(&mut *transaction)
        .await
        .unwrap();
    sqlx::raw_sql(&format!("INSERT INTO jobs SELECT * FROM {source}.jobs;"))
        .execute(&mut *transaction)
        .await
        .unwrap();
    let columns = "job_id,actor_id,request_id,idempotency_key,invocation_sha256,request_blob,request_sha256,reserved_input,reserved_output,reserved_nano_usd,state,created_revision,updated_revision,created_at_ms,updated_at_ms,response_blob,response_sha256,ordinal";
    sqlx::raw_sql(&format!(
        "INSERT INTO model_steps({columns}) SELECT {columns} FROM {source}.model_steps;"
    ))
    .execute(&mut *transaction)
    .await
    .unwrap();
    let original: String =
        sqlx::query_scalar("SELECT to_jsonb(model_steps)::text FROM model_steps")
            .fetch_one(&mut *transaction)
            .await
            .unwrap();
    let jobs: String = sqlx::query_scalar("SELECT to_jsonb(jobs)::text FROM jobs")
        .fetch_one(&mut *transaction)
        .await
        .unwrap();
    let constraints = "SELECT conname::text, pg_get_constraintdef(oid) FROM pg_constraint WHERE conrelid='jobs'::regclass AND contype='c' AND conname NOT IN ('jobs_state_check','jobs_check3') ORDER BY conname";
    let before: Vec<(String, String)> = sqlx::query_as(constraints)
        .fetch_all(&mut *transaction)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!(
        "../../../../../../migrations/postgres/0013_discovery_lifecycle.sql"
    ))
    .execute(&mut *transaction)
    .await
    .unwrap();
    let after: Vec<(String, String)> = sqlx::query_as(constraints)
        .fetch_all(&mut *transaction)
        .await
        .unwrap();
    assert_eq!(
        after, before,
        "unrelated clock and lease constraints must survive"
    );
    let migrated: String = sqlx::query_scalar("SELECT (to_jsonb(model_steps)-'lookup_attempts'-'tool_attempts'-'retry_after_ms'-'response_revision'-'response_at_ms')::text FROM model_steps")
        .fetch_one(&mut *transaction).await.unwrap();
    assert_eq!(migrated, original);
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT to_jsonb(jobs)::text FROM jobs")
            .fetch_one(&mut *transaction)
            .await
            .unwrap(),
        jobs
    );
    assert!(sqlx::query_scalar::<_, bool>("SELECT response_revision=updated_revision AND response_at_ms=updated_at_ms AND lookup_attempts=0 AND tool_attempts=0 AND retry_after_ms=0 FROM model_steps")
        .fetch_one(&mut *transaction).await.unwrap());
    transaction.rollback().await.unwrap();
    store.close().await;
}

struct RegressingClock {
    now: i64,
    before: usize,
    calls: AtomicUsize,
}

impl crate::store::Clock for RegressingClock {
    fn now_millis(&self) -> StoreResult<i64> {
        Ok(self.now - i64::from(self.calls.fetch_add(1, Ordering::SeqCst) >= self.before))
    }
}

#[tokio::test]
async fn commit_regression() {
    for mode in [
        "pause",
        "cancel",
        "expire",
        "resume",
        "retry",
        "tool_retry",
        "fail",
        "reconcile",
    ] {
        let case = super::processes::lifecycle_case(mode).await;
        let mut faulty = case.store.clone();
        faulty.clock = Arc::new(RegressingClock {
            now: case.clock.0.load(Ordering::SeqCst),
            before: if mode == "reconcile" { 2 } else { 1 },
            calls: AtomicUsize::new(0),
        });
        assert!(
            matches!(
                super::processes::execute(&faulty, mode, case.command.clone()).await,
                Err(StoreError::ClockRegression)
            ),
            "{mode}"
        );
        assert_eq!(
            case.store.get("job.1").await.unwrap().unwrap(),
            case.job,
            "{mode}"
        );
        let events = case.store.audit_events(0, 100).await.unwrap();
        assert_eq!(events.len(), case.job.revision as usize);
        case.store.close().await;
    }
}
