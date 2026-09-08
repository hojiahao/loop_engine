mod support;

use std::sync::atomic::Ordering;

use loop_core::audit::verify_audit_chain;
use loop_protocol::wire::jobs::v1::*;
use loop_protocol::wire::v1::*;
use loopd::store::{JobMutation, JobRepository, RecoveryCommand, SqliteJobStore, StoreError};
use prost_types::Duration;
use sqlx::Connection;
use support::*;

fn acquire(key: &str, revision: u64) -> JobMutation {
    JobMutation::Acquire(AcquireJobLeaseRequest {
        context: Some(context(key)),
        job_id: Some(JobId {
            value: "job.1".to_owned(),
        }),
        expected_revision: revision,
        requested_duration: Some(Duration {
            seconds: 30,
            nanos: 0,
        }),
    })
}

fn heartbeat(job: &JobRecord, key: &str) -> JobMutation {
    JobMutation::Heartbeat(HeartbeatJobLeaseRequest {
        context: Some(context(key)),
        job_id: job.specification.as_ref().unwrap().job_id.clone(),
        expected_revision: job.revision,
        lease_id: job.active_lease.as_ref().unwrap().lease_id.clone(),
        requested_extension: Some(Duration {
            seconds: 60,
            nanos: 0,
        }),
    })
}

fn complete(job: &JobRecord, key: &str) -> JobMutation {
    JobMutation::Complete(CompleteJobRequest {
        context: Some(context(key)),
        job_id: job.specification.as_ref().unwrap().job_id.clone(),
        expected_revision: job.revision,
        lease_id: job.active_lease.as_ref().unwrap().lease_id.clone(),
        outcome: Some(JobOutcome {
            outcome: Some(job_outcome::Outcome::Success(JobSuccess {
                outputs: vec![artifact()],
            })),
        }),
    })
}

fn cancel(revision: u64, key: &str) -> JobMutation {
    JobMutation::Cancel(CancelJobRequest {
        context: Some(context(key)),
        job_id: Some(JobId {
            value: "job.1".to_owned(),
        }),
        expected_revision: revision,
        reason: "operator requested stop".to_owned(),
    })
}

fn recover(revision: u64, key: &str) -> JobMutation {
    JobMutation::Recover(RecoveryCommand {
        context: Some(context(key)),
        job_id: Some(JobId {
            value: "job.1".to_owned(),
        }),
        expected_revision: revision,
    })
}

#[tokio::test]
async fn lease_heartbeat_completion_and_replay_survive_restart() {
    let (directory, store, clock) = fixture().await;
    store.submit(command(1)).await.unwrap();
    let leased = store.mutate(&actor(), acquire("acquire", 1)).await.unwrap();
    assert_eq!(
        (leased.job.revision, leased.job.attempt, leased.job.state),
        (2, 1, JobState::Leased as i32)
    );
    clock.0.store(NOW + 10_000, Ordering::SeqCst);
    let running = store
        .mutate(&actor(), heartbeat(&leased.job, "heartbeat"))
        .await
        .unwrap();
    assert_eq!(running.job.state, JobState::Running as i32);
    let finish = complete(&running.job, "finish");
    let result = store.mutate(&actor(), finish.clone()).await.unwrap();
    assert_eq!(
        (result.job.revision, result.job.state),
        (4, JobState::Succeeded as i32)
    );
    assert!(result.job.active_lease.is_none());
    store.close().await;
    let reopened = SqliteJobStore::open(options(&directory.path().join("state.sqlite3"), clock))
        .await
        .unwrap();
    let replay = reopened.mutate(&actor(), finish).await.unwrap();
    assert!(replay.replayed);
    assert_eq!(result.job, replay.job);
    assert!(matches!(
        reopened.mutate(&actor(), cancel(4, "late.cancel")).await,
        Err(StoreError::InvalidTransition)
    ));
    let events = reopened.audit_events(0, 500).await.unwrap();
    assert_eq!(events.len(), 4);
    verify_audit_chain(&events).unwrap();
    reopened.close().await;
}

#[tokio::test]
async fn concurrent_revision_race_has_exactly_one_winner() {
    let (directory, store, clock) = fixture().await;
    store.submit(command(1)).await.unwrap();
    let second = SqliteJobStore::open(options(&directory.path().join("state.sqlite3"), clock))
        .await
        .unwrap();
    let mut tasks = vec![];
    for index in 0..32 {
        let store = if index % 2 == 0 {
            store.clone()
        } else {
            second.clone()
        };
        tasks.push(tokio::spawn(async move {
            store
                .mutate(&actor(), acquire(&format!("claim.{index}"), 1))
                .await
        }));
    }
    let mut successes = 0;
    for task in tasks {
        match task.await.unwrap() {
            Ok(_) => successes += 1,
            Err(StoreError::RevisionConflict) => (),
            other => panic!("unexpected race outcome: {other:?}"),
        }
    }
    assert_eq!(successes, 1);
    assert_eq!(store.audit_events(0, 500).await.unwrap().len(), 2);
    store.close().await;
    second.close().await;
}

#[tokio::test]
async fn lease_owner_id_expiry_and_transport_actor_are_fenced() {
    let (_directory, store, clock) = fixture().await;
    store.submit(command(1)).await.unwrap();
    let leased = store
        .mutate(&actor(), acquire("claim", 1))
        .await
        .unwrap()
        .job;
    let mut wrong = heartbeat(&leased, "wrong");
    let JobMutation::Heartbeat(request) = &mut wrong else {
        unreachable!()
    };
    request.lease_id = Some(LeaseId {
        value: "lease.other".to_owned(),
    });
    assert!(matches!(
        store.mutate(&actor(), wrong).await,
        Err(StoreError::LeaseFenced)
    ));
    let mut other = actor();
    other.actor_id.as_mut().unwrap().value = "worker.other".to_owned();
    assert!(matches!(
        store.mutate(&other, heartbeat(&leased, "actor")).await,
        Err(StoreError::AdmissionDenied)
    ));
    let mut owner = heartbeat(&leased, "owner");
    let JobMutation::Heartbeat(request) = &mut owner else {
        unreachable!()
    };
    request.context.as_mut().unwrap().actor = Some(other.clone());
    assert!(matches!(
        store.mutate(&other, owner).await,
        Err(StoreError::LeaseFenced)
    ));
    clock.0.store(NOW + 30_000, Ordering::SeqCst);
    assert!(matches!(
        store.mutate(&actor(), heartbeat(&leased, "expired")).await,
        Err(StoreError::LeaseFenced)
    ));
    assert!(matches!(
        store.mutate(&actor(), complete(&leased, "late")).await,
        Err(StoreError::LeaseFenced)
    ));
    let failed = store
        .mutate(&actor(), recover(2, "recover"))
        .await
        .unwrap()
        .job;
    assert_eq!(failed.state, JobState::InfrastructureFailed as i32);
    let Some(job_outcome::Outcome::InfrastructureFailure(failure)) =
        failed.outcome.unwrap().outcome
    else {
        unreachable!()
    };
    assert!(!failure.error.unwrap().retryable);
    store.close().await;
}

#[tokio::test]
async fn queued_cancel_and_budget_exhaustion_do_not_invent_attempts() {
    let (_directory, store, clock) = fixture().await;
    store.submit(command(1)).await.unwrap();
    let cancelled = store
        .mutate(&actor(), cancel(1, "cancel"))
        .await
        .unwrap()
        .job;
    assert_eq!(
        (cancelled.state, cancelled.attempt),
        (JobState::Cancelled as i32, 0)
    );
    store.submit(command(2)).await.unwrap();
    clock.0.store(NOW + 3_599_000, Ordering::SeqCst);
    let mut recovery = recover(1, "deadline");
    let JobMutation::Recover(request) = &mut recovery else {
        unreachable!()
    };
    request.job_id.as_mut().unwrap().value = "job.2".to_owned();
    let exhausted = store.mutate(&actor(), recovery).await.unwrap().job;
    assert_eq!(
        (exhausted.state, exhausted.attempt),
        (JobState::BudgetExhausted as i32, 0)
    );
    store.close().await;
}

#[tokio::test]
async fn heartbeat_never_shortens_lease_or_exceeds_frozen_deadline() {
    let (_directory, store, clock) = fixture().await;
    store.submit(command(1)).await.unwrap();
    clock.0.store(NOW + 3_590_000, Ordering::SeqCst);
    let leased = store
        .mutate(&actor(), acquire("claim", 1))
        .await
        .unwrap()
        .job;
    assert_eq!(
        leased.active_lease.as_ref().unwrap().expires_at,
        Some(timestamp(NOW + 3_599_000))
    );
    let running = store
        .mutate(&actor(), heartbeat(&leased, "beat"))
        .await
        .unwrap()
        .job;
    assert_eq!(
        running.active_lease.as_ref().unwrap().expires_at,
        Some(timestamp(NOW + 3_599_000))
    );
    clock.0.store(NOW + 3_599_000, Ordering::SeqCst);
    let exhausted = store
        .mutate(&actor(), recover(3, "recover"))
        .await
        .unwrap()
        .job;
    assert_eq!(exhausted.state, JobState::BudgetExhausted as i32);
    store.close().await;
}

#[tokio::test]
async fn malformed_outcome_and_audit_failure_leave_revision_unchanged() {
    let (directory, store, _) = fixture().await;
    store.submit(command(1)).await.unwrap();
    let leased = store
        .mutate(&actor(), acquire("claim", 1))
        .await
        .unwrap()
        .job;
    let mut malformed = complete(&leased, "bad");
    let JobMutation::Complete(request) = &mut malformed else {
        unreachable!()
    };
    request.outcome = Some(JobOutcome {
        outcome: Some(job_outcome::Outcome::FactorRejection(
            FactorRejection::default(),
        )),
    });
    assert!(matches!(
        store.mutate(&actor(), malformed).await,
        Err(StoreError::Job(_))
    ));
    let mut database = connection(&directory).await;
    sqlx::query("CREATE TRIGGER injected_failure BEFORE INSERT ON command_receipts BEGIN SELECT RAISE(ABORT, 'injected receipt failure'); END")
        .execute(&mut database).await.unwrap();
    assert!(matches!(
        store.mutate(&actor(), complete(&leased, "finish")).await,
        Err(StoreError::Database(_))
    ));
    assert_eq!(store.get("job.1").await.unwrap().unwrap(), leased);
    assert_eq!(store.audit_events(0, 500).await.unwrap().len(), 2);
    database.close().await.unwrap();
    store.close().await;
}

#[tokio::test]
async fn changed_replay_semantics_and_early_recovery_are_denied() {
    let (_directory, store, clock) = fixture().await;
    store.submit(command(1)).await.unwrap();
    assert!(matches!(
        store.mutate(&actor(), recover(1, "early")).await,
        Err(StoreError::InvalidTransition)
    ));
    let first = acquire("claim", 1);
    let original = store.mutate(&actor(), first.clone()).await.unwrap();
    let mut retry = first.clone();
    let JobMutation::Acquire(request) = &mut retry else {
        unreachable!()
    };
    request
        .context
        .as_mut()
        .unwrap()
        .request_id
        .as_mut()
        .unwrap()
        .value = "new.transport".to_owned();
    assert_eq!(
        store.mutate(&actor(), retry).await.unwrap().job,
        original.job
    );
    let mut changed = first;
    let JobMutation::Acquire(request) = &mut changed else {
        unreachable!()
    };
    request.requested_duration.as_mut().unwrap().seconds = 31;
    assert!(matches!(
        store.mutate(&actor(), changed).await,
        Err(StoreError::IdempotencyConflict)
    ));
    clock.0.store(NOW - 1, Ordering::SeqCst);
    assert!(matches!(
        store.mutate(&actor(), cancel(2, "cancel")).await,
        Err(StoreError::ClockRegression)
    ));
    store.close().await;
}

#[tokio::test]
async fn cancelled_worker_cannot_complete_and_heartbeat_replay_cannot_extend() {
    let (_directory, store, clock) = fixture().await;
    store.submit(command(1)).await.unwrap();
    let leased = store
        .mutate(&actor(), acquire("claim", 1))
        .await
        .unwrap()
        .job;
    let beat = heartbeat(&leased, "beat");
    let running = store.mutate(&actor(), beat.clone()).await.unwrap().job;
    clock.0.store(NOW + 10_000, Ordering::SeqCst);
    let replay = store.mutate(&actor(), beat).await.unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.job, running);
    store.mutate(&actor(), cancel(3, "cancel")).await.unwrap();
    assert!(matches!(
        store.mutate(&actor(), complete(&running, "finish")).await,
        Err(StoreError::RevisionConflict)
    ));
    assert_eq!(
        store.get("job.1").await.unwrap().unwrap().state,
        JobState::Cancelled as i32
    );
    store.close().await;
}
