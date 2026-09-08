mod support;

use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

use loop_core::audit::verify_audit_chain;
use loop_protocol::wire::{discovery::v1 as discovery, research::v1 as research, v1::*};
use loopd::store::{
    Clock, JobRepository, PgJobStore, RoleCommand, RoleJobHandle, StoreError, StoreResult,
    SubmissionMetadata,
};
use prost::Message;
use sha2::{Digest, Sha256};
use sqlx::Connection;
use support::research as fixtures;
use support::*;

struct TickClock(AtomicI64);

impl Clock for TickClock {
    fn now_millis(&self) -> StoreResult<i64> {
        Ok(self.0.fetch_add(1, Ordering::SeqCst))
    }
}

fn request(key: &str) -> RoleCommand {
    fixtures::command(key)
}

fn metadata() -> SubmissionMetadata {
    let source = command(1).specification;
    SubmissionMetadata {
        run_id: source.run_id.unwrap(),
        protocol_selection: source.protocol_selection.unwrap(),
    }
}

async fn setup() -> (tempfile::TempDir, PgJobStore, Arc<TickClock>) {
    let directory = tempfile::tempdir().unwrap();
    let clock = Arc::new(TickClock(AtomicI64::new(NOW)));
    let mut options = base_options(&directory.path().join("state"));
    options.clock = clock.clone();
    options.admission = Arc::new(fixtures::Admission);
    let store = PgJobStore::open(options).await.unwrap();
    (directory, store, clock)
}

fn role_command(input: job_specification::Input, key: &str) -> RoleCommand {
    let context = Some(context(key));
    match input {
        job_specification::Input::Discovery(DiscoveryJobInput {
            dataset,
            research_policy,
            maker_model,
            checker_model,
            budget,
            maximum_candidates,
        }) => RoleCommand::Discovery(discovery::StartDiscoveryRequest {
            context,
            discovery: Some(discovery::DiscoveryJobInput {
                dataset,
                research_policy,
                maker_model,
                checker_model,
                budget: budget.map(|value| discovery::DiscoveryJobBudget {
                    maximum_steps: value.maximum_steps,
                    maximum_input_tokens: value.maximum_input_tokens,
                    maximum_output_tokens: value.maximum_output_tokens,
                    maximum_cost: value.maximum_cost,
                    maximum_wall_time: value.maximum_wall_time,
                }),
                maximum_candidates,
            }),
        }),
        job_specification::Input::FactorEvaluation(FactorEvaluationJobInput {
            factor,
            dataset,
            budget,
        }) => RoleCommand::FactorEvaluation(research::EnqueueFactorEvaluationRequest {
            context,
            input: Some(research::FactorEvaluationInput {
                factor,
                dataset,
                budget: budget.map(research_budget),
            }),
        }),
        job_specification::Input::Backtest(BacktestJobInput {
            factor_spec_id,
            dataset,
            return_definition,
            provenance,
            deterministic_seed,
            budget,
        }) => RoleCommand::Backtest(research::EnqueueBacktestRequest {
            context,
            input: Some(research::BacktestInput {
                factor_spec_id,
                dataset,
                return_definition,
                provenance,
                deterministic_seed,
                budget: budget.map(research_budget),
            }),
        }),
        job_specification::Input::Reconciliation(ReconciliationJobInput {
            primary_backtest_id,
            independent_backtest_id,
            reconciliation_policy,
            budget,
        }) => RoleCommand::Reconciliation(research::EnqueueReconciliationRequest {
            context,
            input: Some(research::ReconciliationInput {
                primary_backtest_id,
                independent_backtest_id,
                reconciliation_policy,
                budget: budget.map(research_budget),
            }),
        }),
        _ => panic!("not a role-owned input"),
    }
}

fn research_budget(value: JobBudget) -> research::ResearchJobBudget {
    research::ResearchJobBudget {
        maximum_steps: value.maximum_steps,
        maximum_input_tokens: value.maximum_input_tokens,
        maximum_output_tokens: value.maximum_output_tokens,
        maximum_cost: value.maximum_cost,
        maximum_wall_time: value.maximum_wall_time,
    }
}

async fn accepts_role(kind: JobKind) {
    let (_directory, store, _) = setup().await;
    let (_, expected) = fixtures::inputs()
        .into_iter()
        .find(|(candidate, _)| *candidate == kind)
        .unwrap();
    let command = role_command(expected.clone(), "accept");
    let result = store
        .submit_role(&actor(), command.clone(), metadata())
        .await
        .unwrap();
    assert!(!result.replayed);
    let job_id = match &result.handle {
        RoleJobHandle::Discovery(handle) => {
            assert_eq!(kind, JobKind::Discovery);
            assert_eq!(handle.status, discovery::DiscoveryJobStatus::Queued as i32);
            handle.job_id.as_ref().unwrap()
        }
        RoleJobHandle::Research(handle) => {
            assert_ne!(kind, JobKind::Discovery);
            assert_eq!(handle.status, research::ResearchJobStatus::Queued as i32);
            handle.job_id.as_ref().unwrap()
        }
    };
    let stored = store.get(&job_id.value).await.unwrap().unwrap();
    let spec = stored.specification.unwrap();
    assert_eq!(spec.kind, kind as i32);
    assert_eq!(spec.input, Some(expected));
    assert_eq!(spec.run_id, Some(metadata().run_id));
    assert_eq!(spec.submitted_by, Some(actor()));
    assert_eq!(stored.revision, 1);
    assert_eq!(stored.attempt, 0);
    assert!(stored.active_lease.is_none());
    let replay = store
        .submit_role(&actor(), command, metadata())
        .await
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.handle, result.handle);
    assert_eq!(store.audit_events(0, 500).await.unwrap().len(), 1);
    store.close().await;
}

#[tokio::test]
async fn accepts_discovery() {
    accepts_role(JobKind::Discovery).await;
}

#[tokio::test]
async fn accepts_factor_evaluation() {
    accepts_role(JobKind::FactorEvaluation).await;
}

#[tokio::test]
async fn accepts_backtest() {
    accepts_role(JobKind::Backtest).await;
}

#[tokio::test]
async fn accepts_reconciliation() {
    accepts_role(JobKind::IndependentReconciliation).await;
}

#[tokio::test]
async fn replay_preserves_receipt() {
    let (directory, store, clock) = setup().await;
    let result = store
        .submit_role(&actor(), request("reconcile"), metadata())
        .await
        .unwrap();
    let RoleJobHandle::Research(handle) = &result.handle else {
        panic!("wrong role projection")
    };
    assert_eq!(handle.status, research::ResearchJobStatus::Queued as i32);
    assert_eq!(handle.revision, 1);
    let stored = store
        .get(&handle.job_id.as_ref().unwrap().value)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.specification.as_ref().unwrap().kind,
        JobKind::IndependentReconciliation as i32
    );
    assert_eq!(handle.submitted_at, Some(timestamp(NOW)));
    store.close().await;
    let mut options = base_options(&directory.path().join("state"));
    options.clock = clock;
    options.admission = Arc::new(fixtures::Admission);
    let reopened = PgJobStore::open(options).await.unwrap();
    let mut retry = request("reconcile");
    let RoleCommand::Reconciliation(request) = &mut retry else {
        unreachable!()
    };
    request
        .context
        .as_mut()
        .unwrap()
        .request_id
        .as_mut()
        .unwrap()
        .value = "request.retry".to_owned();
    request.context.as_mut().unwrap().requested_at = Some(timestamp(NOW + 1));
    let mut new_negotiation = metadata();
    new_negotiation.protocol_selection.client_build_version = "new.negotiation".to_owned();
    let replay = reopened
        .submit_role(&actor(), retry, new_negotiation)
        .await
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.handle, result.handle);
    assert_eq!(reopened.audit_events(0, 500).await.unwrap().len(), 1);
    reopened.close().await;
}

#[tokio::test]
async fn concurrent_retries_commit_once() {
    let (directory, first, clock) = setup().await;
    let mut options = base_options(&directory.path().join("state"));
    options.clock = clock;
    options.admission = Arc::new(fixtures::Admission);
    let second = PgJobStore::open(options).await.unwrap();
    let mut tasks = vec![];
    for index in 0..20 {
        let store = if index % 2 == 0 {
            first.clone()
        } else {
            second.clone()
        };
        tasks.push(tokio::spawn(async move {
            store
                .submit_role(&actor(), request("same.command"), metadata())
                .await
                .unwrap()
        }));
    }
    let mut created = 0;
    let mut handle = None;
    for task in tasks {
        let result = task.await.unwrap();
        created += usize::from(!result.replayed);
        if let Some(previous) = &handle {
            assert_eq!(previous, &result.handle);
        }
        handle = Some(result.handle);
    }
    assert_eq!(created, 1);
    let events = first.audit_events(0, 500).await.unwrap();
    assert_eq!(events.len(), 1);
    verify_audit_chain(&events).unwrap();
    first.close().await;
    second.close().await;
}

#[tokio::test]
async fn changed_input_conflicts() {
    let (_directory, store, _) = setup().await;
    store
        .submit_role(&actor(), request("reconcile"), metadata())
        .await
        .unwrap();
    let mut changed = request("reconcile");
    let RoleCommand::Reconciliation(request) = &mut changed else {
        unreachable!()
    };
    request
        .input
        .as_mut()
        .unwrap()
        .budget
        .as_mut()
        .unwrap()
        .maximum_steps += 1;
    assert!(matches!(
        store.submit_role(&actor(), changed, metadata()).await,
        Err(StoreError::IdempotencyConflict)
    ));
    assert_eq!(store.audit_events(0, 500).await.unwrap().len(), 1);
    store.close().await;
}

#[tokio::test]
async fn changed_run_conflicts() {
    let (_directory, store, _) = setup().await;
    store
        .submit_role(&actor(), request("reconcile"), metadata())
        .await
        .unwrap();
    let mut other_run = metadata();
    other_run.run_id.value = "run.other".to_owned();
    assert!(matches!(
        store
            .submit_role(&actor(), request("reconcile"), other_run)
            .await,
        Err(StoreError::IdempotencyConflict)
    ));
    assert_eq!(store.audit_events(0, 500).await.unwrap().len(), 1);
    store.close().await;
}

#[tokio::test]
async fn actor_spoofing_is_denied() {
    let (_directory, store, _) = setup().await;
    let mut other = actor();
    other.actor_id.as_mut().unwrap().value = "actor.spoofed".to_owned();
    assert!(matches!(
        store
            .submit_role(&other, request("reconcile"), metadata())
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    assert!(store.audit_events(0, 500).await.unwrap().is_empty());
    store.close().await;
}

#[tokio::test]
async fn unresolved_references_are_denied() {
    let (_directory, store, _) = setup().await;
    let mut unavailable = request("unavailable");
    let RoleCommand::Reconciliation(request) = &mut unavailable else {
        unreachable!()
    };
    request
        .input
        .as_mut()
        .unwrap()
        .primary_backtest_id
        .as_mut()
        .unwrap()
        .value = "backtest.absent".to_owned();
    assert!(matches!(
        store.submit_role(&actor(), unavailable, metadata()).await,
        Err(StoreError::AdmissionDenied)
    ));
    assert!(store.audit_events(0, 500).await.unwrap().is_empty());
    store.close().await;
}

#[tokio::test]
async fn operations_scope_idempotency_keys() {
    let (_directory, store, _) = setup().await;
    store.submit(command(1)).await.unwrap();
    store
        .submit_role(&actor(), request("submit.1"), metadata())
        .await
        .unwrap();
    assert_eq!(store.audit_events(0, 500).await.unwrap().len(), 2);
    store.close().await;
}

#[tokio::test]
async fn audit_failure_rolls_back_submission() {
    let (directory, store, _) = setup().await;
    let mut database = connection(&directory).await;
    sqlx::query("CREATE TRIGGER injected_failure BEFORE INSERT ON audit_events FOR EACH ROW EXECUTE FUNCTION reject_immutable_change()")
        .execute(&mut database).await.unwrap();
    assert!(matches!(
        store
            .submit_role(&actor(), request("rollback"), metadata())
            .await,
        Err(StoreError::Database(_))
    ));
    for table in [
        "SELECT count(*) FROM jobs",
        "SELECT count(*) FROM command_receipts",
        "SELECT count(*) FROM audit_events",
    ] {
        assert_eq!(
            sqlx::query_scalar::<_, i64>(table)
                .fetch_one(&mut database)
                .await
                .unwrap(),
            0
        );
    }
    database.close().await.unwrap();
    store.close().await;
}

#[tokio::test]
async fn rejects_missing_inputs() {
    let (_directory, store, _) = setup().await;
    for command in [
        RoleCommand::Discovery(discovery::StartDiscoveryRequest {
            context: Some(context("missing")),
            discovery: None,
        }),
        RoleCommand::FactorEvaluation(research::EnqueueFactorEvaluationRequest {
            context: Some(context("missing")),
            input: None,
        }),
        RoleCommand::Backtest(research::EnqueueBacktestRequest {
            context: Some(context("missing")),
            input: None,
        }),
        RoleCommand::Reconciliation(research::EnqueueReconciliationRequest {
            context: Some(context("missing")),
            input: None,
        }),
    ] {
        assert!(matches!(
            store.submit_role(&actor(), command, metadata()).await,
            Err(StoreError::Invalid("role input"))
        ));
    }
    assert!(store.audit_events(0, 500).await.unwrap().is_empty());
    store.close().await;
}

#[tokio::test]
async fn default_policy_denies_submission() {
    let directory = tempfile::tempdir().unwrap();
    let mut options = base_options(&directory.path().join("state"));
    options.clock = Arc::new(TickClock(AtomicI64::new(NOW)));
    let denied = PgJobStore::open(options).await.unwrap();
    for (_, input) in fixtures::inputs() {
        assert!(matches!(
            denied
                .submit_role(&actor(), role_command(input, "denied"), metadata())
                .await,
            Err(StoreError::AdmissionDenied)
        ));
    }
    assert!(denied.audit_events(0, 500).await.unwrap().is_empty());
    denied.close().await;
}

#[tokio::test]
async fn rejects_unresolved_datasets() {
    let (_directory, store, _) = setup().await;
    for (_, mut input) in fixtures::inputs() {
        let dataset = match &mut input {
            job_specification::Input::Discovery(input) => &mut input.dataset,
            job_specification::Input::FactorEvaluation(input) => &mut input.dataset,
            job_specification::Input::Backtest(input) => &mut input.dataset,
            job_specification::Input::Reconciliation(_) => continue,
            _ => unreachable!(),
        };
        dataset.as_mut().unwrap().snapshot_ids[0].value = "snapshot.holdout".to_owned();
        assert!(matches!(
            store
                .submit_role(&actor(), role_command(input, "unresolved"), metadata())
                .await,
            Err(StoreError::AdmissionDenied)
        ));
    }
    assert!(store.audit_events(0, 500).await.unwrap().is_empty());
    store.close().await;
}

#[tokio::test]
async fn rejects_unavailable_protocol() {
    let (_directory, store, _) = setup().await;
    for (_, input) in fixtures::inputs() {
        let mut metadata = metadata();
        metadata.protocol_selection.client_build_sha256 = Some(digest(99));
        metadata.protocol_selection.selection_sha256 = Some(Sha256Digest {
            value: loop_protocol::job::protocol_selection_sha256(&metadata.protocol_selection)
                .unwrap()
                .to_vec(),
        });
        assert!(matches!(
            store
                .submit_role(&actor(), role_command(input, "protocol"), metadata)
                .await,
            Err(StoreError::AdmissionDenied)
        ));
    }
    assert!(store.audit_events(0, 500).await.unwrap().is_empty());
    store.close().await;
}

#[tokio::test]
async fn rejects_rehashed_receipt_state() {
    let (directory, store, _) = setup().await;
    let result = store
        .submit_role(&actor(), request("receipt"), metadata())
        .await
        .unwrap();
    let RoleJobHandle::Research(handle) = result.handle else {
        unreachable!()
    };
    let mut record = store
        .get(&handle.job_id.unwrap().value)
        .await
        .unwrap()
        .unwrap();
    record.state = JobState::Cancelled as i32;
    record.outcome = Some(JobOutcome {
        outcome: Some(job_outcome::Outcome::Cancellation(JobCancellation {
            cancelled_by: Some(actor()),
            cancelled_at: record.updated_at,
            reason: "not an acceptance receipt".to_owned(),
        })),
    });
    loop_protocol::job::validate_job_record(&record).unwrap();
    let blob = record.encode_to_vec();
    let mut database = connection(&directory).await;
    sqlx::query("DROP TRIGGER command_receipts_no_update ON command_receipts")
        .execute(&mut database)
        .await
        .unwrap();
    sqlx::query("UPDATE command_receipts SET response_blob = $1, response_sha256 = $2")
        .bind(&blob)
        .bind(Sha256::digest(&blob).to_vec())
        .execute(&mut database)
        .await
        .unwrap();
    assert!(matches!(
        store
            .submit_role(&actor(), request("receipt"), metadata())
            .await,
        Err(StoreError::Corrupt("role receipt binding"))
    ));
    assert_eq!(store.audit_events(0, 500).await.unwrap().len(), 1);
    database.close().await.unwrap();
    store.close().await;
}

#[tokio::test]
async fn rejects_crosslinked_receipt() {
    let (directory, store, _) = setup().await;
    store.submit(command(1)).await.unwrap();
    store
        .submit_role(&actor(), request("receipt"), metadata())
        .await
        .unwrap();
    let mut database = connection(&directory).await;
    sqlx::query("DROP TRIGGER command_receipts_no_update ON command_receipts")
        .execute(&mut database)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE command_receipts SET job_id = 'job.1' WHERE operation = 'loop.research.reconcile'",
    )
    .execute(&mut database)
    .await
    .unwrap();
    assert!(matches!(
        store
            .submit_role(&actor(), request("receipt"), metadata())
            .await,
        Err(StoreError::Corrupt("role receipt binding"))
    ));
    assert_eq!(store.audit_events(0, 500).await.unwrap().len(), 2);
    database.close().await.unwrap();
    store.close().await;
}
