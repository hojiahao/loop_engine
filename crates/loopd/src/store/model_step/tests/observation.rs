use super::*;
use loop_protocol::wire::discovery::v1::DiscoveryOperation;

struct StopAdmission;

impl AdmissionPolicy for StopAdmission {
    fn validate_submission(&self, _: &JobSpecification) -> StoreResult<()> {
        Err(StoreError::AdmissionDenied)
    }

    fn authorize_job_command(
        &self,
        operation: &str,
        actor: &Actor,
        job: &JobRecord,
    ) -> StoreResult<()> {
        if operation == "loop.discovery.read_control"
            && actor == &fixture::actor()
            && job.specification.as_ref().is_some_and(|spec| {
                spec.submitted_by.as_ref() == Some(actor)
                    && spec
                        .run_id
                        .as_ref()
                        .is_some_and(|run| run.value == "run.fixture")
            })
        {
            Ok(())
        } else {
            Err(StoreError::AdmissionDenied)
        }
    }
}

async fn second_job(store: &PgJobStore) {
    let mut command = submission();
    command.request_id = "second".into();
    command.specification.job_id.as_mut().unwrap().value = "job.2".into();
    command
        .specification
        .idempotency_key
        .as_mut()
        .unwrap()
        .value = "second".into();
    store.submit(command).await.unwrap();
}

#[tokio::test]
async fn metadata_unreserved() {
    let (_directory, mut store, _) = setup().await;
    store.admission = Arc::new(StopAdmission);
    let before = store.audit_events(0, 100).await.unwrap();
    let (job, history) = store
        .discovery_status(&fixture::actor(), "job.1")
        .await
        .unwrap();
    assert_eq!(job.state, JobState::Queued as i32);
    assert!(history.is_empty());
    assert!(matches!(
        store.model_history(&fixture::actor(), "job.1").await,
        Err(StoreError::AdmissionDenied)
    ));
    assert_eq!(store.audit_events(0, 100).await.unwrap(), before);
    store.close().await;
}

#[tokio::test]
async fn metadata_paused() {
    let (_directory, mut store, _) = setup().await;
    let step = reserve(&store).await;
    let paused = store
        .control_model(
            &fixture::actor(),
            lifecycle::unleased(&step.job, "pause"),
            ModelControl::Pause,
        )
        .await
        .unwrap();
    store.admission = Arc::new(StopAdmission);
    let (job, history) = store
        .discovery_status(&fixture::actor(), "job.1")
        .await
        .unwrap();
    assert_eq!(job, paused);
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].job, job);
    assert_eq!(history[0].reserved_nano_usd, step.reserved_nano_usd);
    store.close().await;
}

#[tokio::test]
async fn metadata_corrupt() {
    let (directory, mut store, _) = setup().await;
    reserve(&store).await;
    store.admission = Arc::new(StopAdmission);
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
        store.discovery_status(&fixture::actor(), "job.1").await,
        Err(StoreError::Corrupt(_))
    ));
    store.close().await;
}

#[tokio::test]
async fn page_gaps() {
    let (_directory, mut store, _) = setup().await;
    second_job(&store).await;
    reserve(&store).await;
    store.admission = Arc::new(StopAdmission);
    let before = store.audit_events(0, 100).await.unwrap();
    let page = store
        .discovery_events(&fixture::actor(), "job.1", 0, 1)
        .await
        .unwrap();
    assert!(page.has_more);
    assert_eq!(page.next_after_sequence, 1);
    assert_eq!(page.events[0].operation(), DiscoveryOperation::Start);
    let page = store
        .discovery_events(&fixture::actor(), "job.1", 1, 1)
        .await
        .unwrap();
    assert!(!page.has_more);
    assert_eq!(page.next_after_sequence, 3);
    assert_eq!(page.events[0].operation(), DiscoveryOperation::Reserve);
    let page = store
        .discovery_events(&fixture::actor(), "job.1", 3, 100)
        .await
        .unwrap();
    assert!(!page.has_more);
    assert_eq!(page.next_after_sequence, 3);
    assert!(page.events.is_empty());
    assert_eq!(store.audit_events(0, 100).await.unwrap(), before);
    store.close().await;
}

#[tokio::test]
async fn foreign_cursor() {
    let (_directory, store, _) = setup().await;
    second_job(&store).await;
    assert!(matches!(
        store
            .discovery_events(&fixture::actor(), "job.1", 2, 100)
            .await,
        Err(StoreError::Invalid(_))
    ));
    store.close().await;
}

#[tokio::test]
async fn page_bounds() {
    let (_directory, store, _) = setup().await;
    for (after, limit) in [(0, 0), (0, 101), (u64::MAX, 1), (99, 1)] {
        assert!(matches!(
            store
                .discovery_events(&fixture::actor(), "job.1", after, limit)
                .await,
            Err(StoreError::Invalid(_))
        ));
    }
    store.close().await;
}

#[tokio::test]
async fn foreign_actor() {
    let (_directory, mut store, _) = setup().await;
    store.admission = Arc::new(StopAdmission);
    let mut actor = fixture::actor();
    actor.actor_id.as_mut().unwrap().value = "agent.other".into();
    assert!(matches!(
        store.discovery_events(&actor, "job.1", 0, 100).await,
        Err(StoreError::AdmissionDenied)
    ));
    assert!(matches!(
        store.discovery_status(&actor, "job.1").await,
        Err(StoreError::AdmissionDenied)
    ));
    store.close().await;
}

#[tokio::test]
async fn foreign_run() {
    let (_directory, mut store, _) = setup().await;
    let mut command = submission();
    command.request_id = "foreign".into();
    command.specification.job_id.as_mut().unwrap().value = "job.foreign".into();
    command.specification.run_id.as_mut().unwrap().value = "run.foreign".into();
    command
        .specification
        .idempotency_key
        .as_mut()
        .unwrap()
        .value = "foreign".into();
    store.submit(command).await.unwrap();
    store.admission = Arc::new(StopAdmission);
    assert!(matches!(
        store
            .discovery_events(&fixture::actor(), "job.foreign", 0, 100)
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    assert!(matches!(
        store
            .discovery_status(&fixture::actor(), "job.foreign")
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    store.close().await;
}

#[tokio::test]
async fn event_corrupt() {
    let (directory, store, _) = setup().await;
    let mut connection = fixture::connection(&directory).await;
    sqlx::query("ALTER TABLE audit_events DISABLE TRIGGER audit_events_no_update")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("UPDATE audit_events SET payload_blob=decode('00','hex') WHERE sequence=1")
        .execute(&mut connection)
        .await
        .unwrap();
    assert!(
        store
            .discovery_events(&fixture::actor(), "job.1", 0, 100)
            .await
            .is_err()
    );
    store.close().await;
}

#[tokio::test]
async fn predecessor_corrupt() {
    let (directory, store, _) = setup().await;
    second_job(&store).await;
    reserve(&store).await;
    let mut connection = fixture::connection(&directory).await;
    sqlx::query("ALTER TABLE audit_events DISABLE TRIGGER audit_events_no_update")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("UPDATE audit_events SET payload_blob=decode('00','hex') WHERE sequence=2")
        .execute(&mut connection)
        .await
        .unwrap();
    assert!(
        store
            .discovery_events(&fixture::actor(), "job.1", 1, 100)
            .await
            .is_err()
    );
    store.close().await;
}

#[tokio::test]
async fn unsupported_event() {
    use crate::store::audit::{EventInput, append};
    use loop_core::audit::{AuditAction, AuditTarget, AuditTargetKind, canonicalize_audit_payload};
    let (_directory, store, _) = setup().await;
    let actor = fixture::actor();
    let mut transaction = store.pool.begin().await.unwrap();
    store.observe_clock(&mut transaction).await.unwrap();
    append(&mut transaction, &store.ledger_id, fixture::NOW, EventInput {
        actor: &actor,
        correlation_id: "correlation.fixture",
        causation_id: "causation.fixture",
        action: AuditAction::CommandAccepted,
        target: AuditTarget { kind: AuditTargetKind::JobId, value: "job.1".into() },
        payload: canonicalize_audit_payload("loop.audit.command_accepted", 1,
            br#"{"command":"loop.unknown.secret","request_id":"unknown","summary":"unsupported"}"#).unwrap(),
    }).await.unwrap();
    transaction.commit().await.unwrap();
    assert!(matches!(
        store.discovery_events(&actor, "job.1", 0, 100).await,
        Err(StoreError::Corrupt(_))
    ));
    store.close().await;
}

#[tokio::test]
async fn observation_clock() {
    let (_directory, store, clock) = setup().await;
    clock.0.store(fixture::NOW - 1, Ordering::SeqCst);
    assert!(matches!(
        store.discovery_status(&fixture::actor(), "job.1").await,
        Err(StoreError::ClockRegression)
    ));
    assert!(matches!(
        store
            .discovery_events(&fixture::actor(), "job.1", 0, 100)
            .await,
        Err(StoreError::ClockRegression)
    ));
    store.close().await;
}

#[tokio::test]
async fn predecessor_link() {
    use loop_core::audit::{Sha256Digest, audit_event_sha256};
    let (directory, store, _) = setup().await;
    reserve(&store).await;
    let mut connection = fixture::connection(&directory).await;
    sqlx::query("ALTER TABLE audit_events DISABLE TRIGGER audit_events_no_update")
        .execute(&mut connection)
        .await
        .unwrap();
    let row = sqlx::query("SELECT * FROM audit_events WHERE sequence=2")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    let mut event = crate::store::audit::event_from_row(&row, &store.ledger_id).unwrap();
    event.previous_event_sha256 = Sha256Digest::ZERO;
    let digest = audit_event_sha256(&event).unwrap();
    sqlx::query("UPDATE audit_events SET previous_sha256=$1,event_sha256=$2 WHERE sequence=2")
        .bind(Sha256Digest::ZERO.as_bytes().as_slice())
        .bind(digest.as_bytes().as_slice())
        .execute(&mut connection)
        .await
        .unwrap();
    assert!(matches!(
        store
            .discovery_events(&fixture::actor(), "job.1", 1, 100)
            .await,
        Err(StoreError::Corrupt("discovery event predecessor"))
    ));
    store.close().await;
}

#[tokio::test]
async fn event_target() {
    let (directory, store, _) = setup().await;
    second_job(&store).await;
    let mut connection = fixture::connection(&directory).await;
    sqlx::query("ALTER TABLE audit_events DISABLE TRIGGER audit_events_no_update")
        .execute(&mut connection)
        .await
        .unwrap();
    // job_id is an indexed projection, independent of the signed target. A
    // forged projection must not expose the foreign event even with valid bytes.
    sqlx::query("UPDATE audit_events SET job_id='job.2' WHERE sequence=1")
        .execute(&mut connection)
        .await
        .unwrap();
    assert!(matches!(
        store
            .discovery_events(&fixture::actor(), "job.2", 0, 100)
            .await,
        Err(StoreError::Corrupt("discovery event target"))
    ));
    store.close().await;
}

#[tokio::test]
async fn indexed_page() {
    let (_directory, store, _) = setup().await;
    let mut transaction = store.pool.begin().await.unwrap();
    sqlx::query("SET LOCAL enable_seqscan=off")
        .execute(&mut *transaction)
        .await
        .unwrap();
    let plan: Vec<String> = sqlx::query_scalar(
        "EXPLAIN (COSTS OFF) SELECT * FROM audit_events WHERE job_id=$1 AND sequence>$2 ORDER BY sequence LIMIT $3",
    ).bind("job.1").bind(0_i64).bind(101_i64)
        .fetch_all(&mut *transaction).await.unwrap();
    assert!(
        plan.iter()
            .any(|line| line.contains("audit_events_job_sequence")),
        "{plan:?}"
    );
    transaction.rollback().await.unwrap();
    store.close().await;
}
