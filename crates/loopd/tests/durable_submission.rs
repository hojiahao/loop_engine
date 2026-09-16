mod support;

use loop_core::audit::verify_audit_chain;
use loop_protocol::job::protocol_selection_sha256;
use loop_protocol::wire::v1::*;
use loopd::store::{JobRepository, PgJobStore, StoreError};
use sqlx::Connection;
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};
use support::*;

#[tokio::test]
// Scenario: migration configures durable storage and reopens.
async fn migration_durable_storage() {
    let (directory, store, clock) = fixture().await;
    store.verify_configuration().await.unwrap();
    store.submit(command(1)).await.unwrap();
    store.close().await;
    let store = PgJobStore::open(options(&directory.path().join("state"), clock))
        .await
        .unwrap();
    store.verify_configuration().await.unwrap();
    assert_eq!(store.get("job.1").await.unwrap().unwrap().revision, 1);
    assert!(store.submit(command(1)).await.unwrap().replayed);
    assert_eq!(store.audit_events(0, 500).await.unwrap().len(), 1);
    store.close().await;
}

#[tokio::test]
// Scenario: concurrent startup serializes migrations.
async fn concurrent_startup_migrations() {
    let directory = tempfile::tempdir().unwrap();
    let clock = Arc::new(FixtureClock(AtomicI64::new(NOW)));
    let path = directory.path().join("state");
    let (first, second) = tokio::join!(
        PgJobStore::open(options(&path, clock.clone())),
        PgJobStore::open(options(&path, clock)),
    );
    let first = first.unwrap();
    let second = second.unwrap();
    first.verify_configuration().await.unwrap();
    second.verify_configuration().await.unwrap();
    first.close().await;
    second.close().await;
}

#[tokio::test]
// Scenario: default policy denies submission without creating state.
async fn default_policy_submission() {
    let directory = tempfile::tempdir().unwrap();
    let store = PgJobStore::open(base_options(&directory.path().join("state")))
        .await
        .unwrap();
    assert!(matches!(
        store.submit(command(1)).await,
        Err(StoreError::AdmissionDenied)
    ));
    assert!(store.get("job.1").await.unwrap().is_none());
    assert!(store.audit_events(0, 500).await.unwrap().is_empty());
    store.close().await;
}

#[tokio::test]
// Scenario: a replay returns the original response without another event.
async fn replay_original_response() {
    let (_directory, store, clock) = fixture().await;
    let first = store.submit(command(1)).await.unwrap();
    clock.0.store(NOW + 10, Ordering::SeqCst);
    let mut retry = command(1);
    retry.request_id = "transport.retry".to_owned();
    let second = store.submit(retry).await.unwrap();
    assert!(!first.replayed);
    assert!(second.replayed);
    assert_eq!(first.job, second.job);
    assert_eq!(store.audit_events(0, 500).await.unwrap().len(), 1);
    store.close().await;
}

#[tokio::test]
// Scenario: changed semantics under the same key fail closed.
async fn changed_semantics_key() {
    let (_directory, store, _) = fixture().await;
    store.submit(command(1)).await.unwrap();
    let mut changed = command(1);
    let Some(job_specification::Input::Artifact(input)) = &mut changed.specification.input else {
        unreachable!()
    };
    input.budget.as_mut().unwrap().maximum_steps += 1;
    assert!(matches!(
        store.submit(changed).await,
        Err(StoreError::IdempotencyConflict)
    ));
    let mut duplicate = command(1);
    duplicate
        .specification
        .idempotency_key
        .as_mut()
        .unwrap()
        .value = "different.key".to_owned();
    assert!(matches!(
        store.submit(duplicate).await,
        Err(StoreError::DuplicateJob)
    ));
    assert_eq!(store.audit_events(0, 500).await.unwrap().len(), 1);
    store.close().await;
}

#[tokio::test]
// Scenario: concurrent writers commit one job receipt and event.
async fn concurrent_writers_job() {
    let (directory, first, clock) = fixture().await;
    let second = PgJobStore::open(options(&directory.path().join("state"), clock))
        .await
        .unwrap();
    let mut tasks = Vec::new();
    for index in 0..20 {
        let store = if index % 2 == 0 {
            first.clone()
        } else {
            second.clone()
        };
        tasks.push(tokio::spawn(async move {
            store.submit(command(1)).await.unwrap()
        }));
    }
    let mut created = 0;
    for task in tasks {
        created += usize::from(!task.await.unwrap().replayed);
    }
    assert_eq!(created, 1);
    assert_eq!(first.audit_events(0, 500).await.unwrap().len(), 1);
    let mut database = connection(&directory).await;
    let receipts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM command_receipts")
        .fetch_one(&mut database)
        .await
        .unwrap();
    assert_eq!(receipts, 1);
    database.close().await.unwrap();
    first.close().await;
    second.close().await;
}

#[tokio::test]
// Scenario: audit failure rolls back job receipt and clock watermark.
async fn audit_failure_job() {
    let (directory, store, _) = fixture().await;
    let mut database = connection(&directory).await;
    sqlx::query("CREATE TRIGGER injected_failure BEFORE INSERT ON audit_events FOR EACH ROW EXECUTE FUNCTION reject_immutable_change()")
        .execute(&mut database).await.unwrap();
    assert!(matches!(
        store.submit(command(1)).await,
        Err(StoreError::Database(_))
    ));
    assert!(store.get("job.1").await.unwrap().is_none());
    let receipts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM command_receipts")
        .fetch_one(&mut database)
        .await
        .unwrap();
    let watermark: i64 = sqlx::query_scalar("SELECT last_observed_at_ms FROM store_metadata")
        .fetch_one(&mut database)
        .await
        .unwrap();
    assert_eq!((receipts, watermark), (0, 0));
    database.close().await.unwrap();
    store.close().await;
}

#[tokio::test]
// Scenario: backward clock and invalid time never add a job.
async fn backward_clock_invalid() {
    let (_directory, store, clock) = fixture().await;
    store.submit(command(1)).await.unwrap();
    clock.0.store(NOW - 1, Ordering::SeqCst);
    assert!(matches!(
        store.submit(command(2)).await,
        Err(StoreError::ClockRegression)
    ));
    clock.0.store(NOW, Ordering::SeqCst);
    let mut future = command(2);
    future.specification.submitted_at = Some(timestamp(NOW + 1));
    assert!(matches!(
        store.submit(future).await,
        Err(StoreError::Invalid(_))
    ));
    clock.0.store(NOW + 3_600_000, Ordering::SeqCst);
    assert!(matches!(
        store.submit(command(2)).await,
        Err(StoreError::Invalid(_))
    ));
    assert!(store.get("job.2").await.unwrap().is_none());
    store.close().await;
}

#[tokio::test]
// Scenario: unavailable pinned protocol is denied.
async fn unavailable_pinned_protocol() {
    let (_directory, store, _) = fixture().await;
    let mut request = command(1);
    let selection = request.specification.protocol_selection.as_mut().unwrap();
    selection.client_build_sha256 = Some(digest(99));
    selection.selection_sha256 = Some(Sha256Digest {
        value: protocol_selection_sha256(selection).unwrap().to_vec(),
    });
    assert!(matches!(
        store.submit(request).await,
        Err(StoreError::AdmissionDenied)
    ));
    assert!(store.audit_events(0, 1).await.unwrap().is_empty());
    store.close().await;
}

#[tokio::test]
// Scenario: audit chain is verified and rows are immutable.
async fn audit_chain_rows() {
    let (directory, store, _) = fixture().await;
    for index in 1..=3 {
        store.submit(command(index)).await.unwrap();
    }
    let events = store.audit_events(0, 3).await.unwrap();
    verify_audit_chain(&events).unwrap();
    assert_eq!(store.audit_events(2, 1).await.unwrap(), events[2..]);
    assert!(store.audit_events(0, 501).await.is_err());
    assert!(store.audit_events(99, 1).await.is_err());
    let mut database = connection(&directory).await;
    for query in [
        "DELETE FROM audit_events",
        "UPDATE audit_events SET actor_display_name = 'changed'",
        "DELETE FROM command_receipts",
        "UPDATE command_receipts SET request_id = 'changed'",
    ] {
        assert!(sqlx::query(query).execute(&mut database).await.is_err());
    }
    database.close().await.unwrap();
    store.close().await;
}

#[tokio::test]
// Scenario: envelope checksum and projection corruption are rejected.
async fn envelope_checksum_projection() {
    let (directory, store, _) = fixture().await;
    store.submit(command(1)).await.unwrap();
    store.submit(command(2)).await.unwrap();
    let mut database = connection(&directory).await;
    sqlx::query(
        "UPDATE jobs SET record_sha256 = decode(repeat('00', 32), 'hex') WHERE job_id = 'job.1'",
    )
    .execute(&mut database)
    .await
    .unwrap();
    sqlx::query("UPDATE jobs SET kind = 1 WHERE job_id = 'job.2'")
        .execute(&mut database)
        .await
        .unwrap();
    assert!(matches!(
        store.get("job.1").await,
        Err(StoreError::Corrupt(_))
    ));
    assert!(matches!(
        store.get("job.2").await,
        Err(StoreError::Corrupt(_))
    ));
    database.close().await.unwrap();
    store.close().await;
}

#[tokio::test]
// Scenario: changing the audit ledger identity on reopen is rejected.
async fn audit_ledger_identity() {
    let (directory, store, clock) = fixture().await;
    store.close().await;
    let mut changed = options(&directory.path().join("state"), clock);
    changed.audit_ledger_id = "ledger.other".to_owned();
    assert!(matches!(
        PgJobStore::open(changed).await,
        Err(StoreError::Corrupt(_))
    ));
}

#[tokio::test]
// Scenario: migration lock wait is bounded and cancellable.
async fn migration_lock_wait() {
    use std::time::Duration;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state");
    let mut lock = connection(&directory).await;
    sqlx::query("SELECT pg_advisory_lock(hashtextextended($1, 0))")
        .bind(format!("loop.migrations.{}", schema(&path)))
        .execute(&mut lock)
        .await
        .unwrap();
    let clock = Arc::new(FixtureClock(AtomicI64::new(NOW)));
    let mut configuration = options(&path, clock.clone());
    configuration.migration_lock_timeout = Duration::from_millis(25);
    assert!(matches!(
        PgJobStore::open(configuration).await,
        Err(StoreError::Unavailable("migration timeout"))
    ));
    assert!(
        tokio::time::timeout(
            Duration::from_millis(25),
            PgJobStore::open(options(&path, clock.clone()))
        )
        .await
        .is_err()
    );
    lock.close().await.unwrap();
    let store = tokio::time::timeout(
        Duration::from_secs(5),
        PgJobStore::open(options(&path, clock)),
    )
    .await
    .unwrap()
    .unwrap();
    store.verify_configuration().await.unwrap();
    store.close().await;
}

#[tokio::test]
// Scenario: modified migration checksum is rejected on reopen.
async fn modified_migration_checksum() {
    let (directory, store, clock) = fixture().await;
    let mut database = connection(&directory).await;
    sqlx::query(
        "UPDATE _sqlx_migrations SET checksum = decode(repeat('00', 48), 'hex') WHERE version = 1",
    )
    .execute(&mut database)
    .await
    .unwrap();
    database.close().await.unwrap();
    store.close().await;
    assert!(matches!(
        PgJobStore::open(options(&directory.path().join("state"), clock)).await,
        Err(StoreError::Migration(_))
    ));
}

#[tokio::test]
// Scenario: a prelease terminal capability is required.
async fn prelease_terminal_capability() {
    let (_directory, store, _) = fixture().await;
    let mut request = command(1);
    let selection = request.specification.protocol_selection.as_mut().unwrap();
    selection
        .enabled_features
        .retain(|value| value != "jobs.prelease-terminal.v1");
    selection.selection_sha256 = Some(Sha256Digest {
        value: protocol_selection_sha256(selection).unwrap().to_vec(),
    });
    assert!(matches!(
        store.submit(request).await,
        Err(StoreError::AdmissionDenied)
    ));
    store.close().await;
}

#[tokio::test]
// Scenario: strict table checks reject partial leases and invalid revisions.
async fn strict_table_leases() {
    let (directory, store, _) = fixture().await;
    store.submit(command(1)).await.unwrap();
    let mut database = connection(&directory).await;
    for query in [
        "UPDATE jobs SET revision = 0",
        "UPDATE jobs SET attempt = -1",
        "UPDATE jobs SET state = 2, attempt = 1, lease_id = 'lease.partial', lease_owner_id = 'worker.partial'",
        "UPDATE jobs SET state = 3, attempt = 1, lease_expires_at_ms = 1788761620000",
        "UPDATE jobs SET lease_id = 'lease.without.active.state'",
        "UPDATE jobs SET state = 9",
    ] {
        assert!(
            sqlx::query(query).execute(&mut database).await.is_err(),
            "constraint allowed: {query}"
        );
    }
    assert_eq!(store.get("job.1").await.unwrap().unwrap().revision, 1);
    database.close().await.unwrap();
    store.close().await;
}
