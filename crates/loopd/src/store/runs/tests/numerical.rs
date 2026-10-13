//! Storage boundary fixtures deliberately omit file-backed runtime authority.
//! Installed numerical execution is separately covered by runtime acceptance.

use super::*;
use crate::store::runs::transitions::{self, RunBinding};
use loop_protocol::wire::research::v1::ResearchJobBudget;
use loop_protocol::wire::runs::v1::{
    NumericalWorkflow, ResearchRunView, RunStage, research_run_view::CurrentChild,
};
use sha2::{Digest, Sha256};
use std::sync::atomic::AtomicUsize;

struct CommitClock {
    reads: AtomicUsize,
    stable_reads: usize,
    final_value: i64,
}

impl crate::store::Clock for CommitClock {
    fn now_millis(&self) -> StoreResult<i64> {
        // Keep the configured earlier clock/authority checks stable, then
        // change time at the final boundary after transactional writes.
        Ok(
            if self.reads.fetch_add(1, Ordering::SeqCst) < self.stable_reads {
                fixture::NOW
            } else {
                self.final_value
            },
        )
    }
}

async fn cancelled_run(store: &PgJobStore) -> RunView {
    seed(store).await;
    let job = store
        .read_run(&owner(), "run.managed")
        .await
        .unwrap()
        .current_job;
    store
        .control_model(
            &executor(),
            model_command(&job, "cancel.clock"),
            ModelControl::Cancel,
        )
        .await
        .unwrap();
    store.read_run(&owner(), "run.managed").await.unwrap().view
}

pub(super) fn numerical_spec() -> RunSpecification {
    let mut specification = specification();
    let numerical = ResearchJobBudget {
        maximum_steps: 1,
        maximum_input_tokens: 0,
        maximum_output_tokens: 0,
        maximum_cost: Some(storage::money(0)),
        maximum_wall_time: Some(prost_types::Duration {
            seconds: 180,
            nanos: 0,
        }),
    };
    specification.budget.as_mut().unwrap().maximum_steps = 6;
    let mut research = executor();
    research.kind = ActorKind::Service as i32;
    research.actor_id.as_mut().unwrap().value = "service.numerical".into();
    research.authenticated_subject = "service:numerical".into();
    let mut protocol = specification.protocol_selection.clone().unwrap();
    protocol.selected_package = "loop.v1".into();
    protocol.enabled_features = [
        "jobs.envelope.v1",
        "jobs.kind-input.v1",
        "jobs.prelease-terminal.v1",
    ]
    .map(str::to_owned)
    .to_vec();
    protocol.selection_sha256 = Some(Sha256Digest {
        value: loop_protocol::job::protocol_selection_sha256(&protocol)
            .unwrap()
            .to_vec(),
    });
    specification.numerical = Some(NumericalWorkflow {
        research_executor: Some(research),
        recipe: specification.plan.clone(),
        evaluation_budget: Some(numerical.clone()),
        portfolio_budget: Some(numerical),
        protocol_selection: Some(protocol),
        perturbation_budget: None,
    });
    specification
}

#[test]
fn research_role() {
    let mut specification = numerical_spec();
    loop_protocol::runs::validate_specification(&specification, &timestamp(fixture::NOW)).unwrap();
    specification
        .numerical
        .as_mut()
        .unwrap()
        .research_executor
        .as_mut()
        .unwrap()
        .kind = ActorKind::Agent as i32;
    assert!(
        loop_protocol::runs::validate_specification(&specification, &timestamp(fixture::NOW))
            .is_err()
    );
}

// Populate only the initial persisted state through the shared writer helpers.
// This tests encoding/fencing, and is not a substitute for PreparedRun execution.
pub(super) async fn seed(store: &PgJobStore) -> RunView {
    let specification = numerical_spec();
    let mut transaction = store.pool.begin().await.unwrap();
    let now = store.observe_clock(&mut transaction).await.unwrap();
    let permit = new_permit("run.managed");
    let mut context = context("seed", executor());
    context.idempotency_key.as_mut().unwrap().value =
        format!("run.{:x}.1.1", Sha256::digest(b"run.managed"));
    let command = RoleCommand::Discovery(discovery::StartDiscoveryRequest {
        context: Some(context),
        discovery: specification.discovery.clone(),
    });
    let metadata = SubmissionMetadata {
        run_id: specification.run_id.clone().unwrap(),
        protocol_selection: specification.protocol_selection.clone().unwrap(),
    };
    let job = crate::store::submission::build_job(
        &command,
        &executor(),
        metadata.clone(),
        now,
        Some(&permit),
    )
    .unwrap();
    let handle = queued_handle(&permit, now);
    let mut view = RunView {
        run_id: specification.run_id.clone(),
        status: RunStatus::Active as i32,
        revision: 1,
        maximum_rounds: 2,
        budget: specification.budget.clone(),
        reserved_cost: Some(storage::money(0)),
        submitted_at: Some(timestamp(now)),
        updated_at: Some(timestamp(now)),
        deadline: Some(timestamp(now + 3_600_000)),
        research: Some(ResearchRunView {
            round: 1,
            stage: RunStage::Discovery as i32,
            current_child: Some(CurrentChild::Discovery(handle)),
            discovery_job_id: job.job_id.clone(),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(reserve(&mut view, &specification, now).unwrap());
    let mut binding = RunBinding {
        current_specification: Some(job),
        created_child: true,
        evidence: br#"{"fixture":"storage-only"}"#.to_vec(),
        ..Default::default()
    };
    let bytes = transitions::seal(&specification, &mut view, &mut binding).unwrap();
    persist(
        &mut transaction,
        &specification,
        &view,
        Some(permit.job_id()),
    )
    .await
    .unwrap();
    crate::store::submission::submit_in(
        store,
        &mut transaction,
        &executor(),
        command,
        metadata,
        now,
        Some(&permit),
    )
    .await
    .unwrap();
    transitions::insert(&mut transaction, &view, &binding, &bytes)
        .await
        .unwrap();
    transaction.commit().await.unwrap();
    view
}

#[tokio::test]
async fn numerical_history() {
    let (_directory, store, _) = setup().await;
    let view = seed(&store).await;
    let snapshot = store.read_run(&owner(), "run.managed").await.unwrap();
    assert_eq!(snapshot.view, view);
    assert!(snapshot.binding.is_some());
    assert!(snapshot.view.current_job.is_none());
    assert_eq!(snapshot.specification, numerical_spec());
    store.close().await;
}

#[tokio::test]
async fn snapshot_consistency() {
    let (_directory, store, clock) = setup().await;
    let initial = seed(&store).await;
    let mut reader = crate::store::research_ledger::begin_snapshot(&store)
        .await
        .unwrap();
    assert_eq!(
        storage::load(&mut reader, "run.managed")
            .await
            .unwrap()
            .view,
        initial
    );
    // A different pool connection commits a new parent and transition while
    // the old population reader is still authorizing its trial observations.
    clock.0.store(fixture::NOW + 3_600_000, Ordering::SeqCst);
    let mut request = step(1);
    request.workflow_version = 2;
    let advanced = store
        .advance_numerical(&owner(), &request, &numerical_spec(), None)
        .await
        .unwrap();
    assert_eq!(advanced.revision, 2);
    assert_eq!(
        storage::load(&mut reader, "run.managed")
            .await
            .unwrap()
            .view,
        initial
    );
    reader.commit().await.unwrap();
    assert_eq!(
        store.read_run(&owner(), "run.managed").await.unwrap().view,
        advanced
    );
    store.close().await;
}

#[tokio::test]
async fn zero_reservation() {
    let (_directory, store, _) = setup().await;
    let mut view = seed(&store).await;
    let specification = numerical_spec();
    let allowance = JobBudget::decode(
        specification
            .numerical
            .as_ref()
            .unwrap()
            .evaluation_budget
            .as_ref()
            .unwrap()
            .encode_to_vec()
            .as_slice(),
    )
    .unwrap();
    let prior = (
        view.reserved_input_tokens,
        view.reserved_output_tokens,
        view.reserved_cost.clone(),
    );
    assert!(storage::reserve_budget(&mut view, &specification, &allowance, fixture::NOW).unwrap());
    assert_eq!(view.reserved_steps, 2);
    assert_eq!(
        (
            view.reserved_input_tokens,
            view.reserved_output_tokens,
            view.reserved_cost
        ),
        prior
    );
    store.close().await;
}

#[test]
fn numerical_deadline() {
    let specification = numerical_spec();
    let child = JobBudget::decode(
        specification
            .numerical
            .as_ref()
            .unwrap()
            .evaluation_budget
            .as_ref()
            .unwrap()
            .encode_to_vec()
            .as_slice(),
    )
    .unwrap();
    let original = RunView {
        deadline: Some(timestamp(fixture::NOW + 180_000)),
        reserved_cost: Some(storage::money(0)),
        ..Default::default()
    };
    let mut exact = original.clone();
    assert!(storage::reserve_budget(&mut exact, &specification, &child, fixture::NOW).unwrap());
    assert_eq!(exact.reserved_steps, 1);

    // Exceeding the parent's remaining time does not consume any reservation.
    let mut late = original.clone();
    assert!(!storage::reserve_budget(&mut late, &specification, &child, fixture::NOW + 1).unwrap());
    assert_eq!(late, original);
    assert!(!storage::reserve_budget(&mut late, &specification, &child, i64::MAX).unwrap());
    assert_eq!(late, original);
}

#[test]
fn duration_limits() {
    let mut specification = numerical_spec();
    specification
        .budget
        .as_mut()
        .unwrap()
        .maximum_wall_time
        .as_mut()
        .unwrap()
        .seconds = 604_800;
    let original = RunView {
        deadline: Some(timestamp(fixture::NOW + 604_800_000)),
        reserved_cost: Some(storage::money(0)),
        ..Default::default()
    };
    for (seconds, nanos, valid) in [
        (0, 1, true),
        (604_800, 0, true),
        (0, 0, false),
        (-1, 0, false),
        (1, -1, false),
        (1, 1_000_000_000, false),
        (604_800, 1, false),
        (i64::MAX, 0, false),
    ] {
        let child = JobBudget {
            maximum_steps: 1,
            maximum_cost: Some(storage::money(0)),
            maximum_wall_time: Some(prost_types::Duration { seconds, nanos }),
            ..Default::default()
        };
        let mut view = original.clone();
        let result = storage::reserve_budget(&mut view, &specification, &child, fixture::NOW);
        if valid {
            assert!(result.unwrap(), "{seconds}s {nanos}ns");
        } else {
            assert!(matches!(result, Err(StoreError::Invalid(_))));
            assert_eq!(view, original);
        }
    }
}

#[test]
fn discovery_ceiling() {
    let mut specification = numerical_spec();
    specification
        .discovery
        .as_mut()
        .unwrap()
        .budget
        .as_mut()
        .unwrap()
        .maximum_wall_time
        .as_mut()
        .unwrap()
        .seconds = 180;
    let original = RunView {
        deadline: Some(timestamp(fixture::NOW + 3_600_000)),
        reserved_cost: Some(storage::money(0)),
        ..Default::default()
    };
    let mut view = original.clone();
    assert!(matches!(
        storage::reserve(&mut view, &specification, fixture::NOW),
        Err(StoreError::Invalid("model wall time range"))
    ));
    assert_eq!(view, original);
}

#[tokio::test]
async fn numerical_expiry() {
    let (_directory, store, clock) = setup().await;
    seed(&store).await;
    clock.0.store(fixture::NOW + 3_600_000, Ordering::SeqCst);
    let mut request = step(1);
    request.workflow_version = 2;
    let view = store
        .advance_numerical(&owner(), &request, &numerical_spec(), None)
        .await
        .unwrap();
    assert_eq!(view.status, RunStatus::DeadlineExceeded as i32);
    assert_eq!(view.completed_rounds, 0);
    assert_eq!(view.revision, 2);
    assert_eq!(
        store.replay_step(&owner(), &request).await.unwrap(),
        Some(view.clone())
    );
    assert_eq!(
        store.read_run(&owner(), "run.managed").await.unwrap().view,
        view
    );
    store.close().await;
}

#[tokio::test]
async fn commit_deadline() {
    let (_directory, store, clock) = setup().await;
    let before = cancelled_run(&store).await;
    let audit = store.audit_events(0, 100).await.unwrap();
    let changed = Arc::new(CommitClock {
        reads: AtomicUsize::new(0),
        stable_reads: 2,
        final_value: fixture::NOW + 3_600_000,
    });
    let mut late = store.clone();
    late.clock = changed.clone();
    let mut request = step(1);
    request.workflow_version = 2;
    assert!(matches!(
        late.advance_numerical(&owner(), &request, &numerical_spec(), None)
            .await,
        Err(StoreError::Unavailable("run commit deadline"))
    ));
    assert_eq!(changed.reads.load(Ordering::SeqCst), 3);
    assert_eq!(
        store.read_run(&owner(), "run.managed").await.unwrap().view,
        before
    );
    assert_eq!(store.audit_events(0, 100).await.unwrap(), audit);
    assert_eq!(store.replay_step(&owner(), &request).await.unwrap(), None);

    // The failed commit consumed neither its CAS revision nor idempotency key.
    clock.0.store(fixture::NOW + 3_600_000, Ordering::SeqCst);
    let ended = store
        .advance_numerical(&owner(), &request, &numerical_spec(), None)
        .await
        .unwrap();
    assert_eq!(ended.status, RunStatus::DeadlineExceeded as i32);
    assert_eq!(ended.revision, 2);
    assert_eq!(ended.reserved_steps, before.reserved_steps);
    store.close().await;
}

#[tokio::test]
async fn commit_regression() {
    let (_directory, store, _) = setup().await;
    let before = cancelled_run(&store).await;
    let audit = store.audit_events(0, 100).await.unwrap();
    let changed = Arc::new(CommitClock {
        reads: AtomicUsize::new(0),
        stable_reads: 2,
        final_value: fixture::NOW - 1,
    });
    let mut regressed = store.clone();
    regressed.clock = changed.clone();
    let mut request = step(1);
    request.workflow_version = 2;
    assert!(matches!(
        regressed
            .advance_numerical(&owner(), &request, &numerical_spec(), None)
            .await,
        Err(StoreError::ClockRegression)
    ));
    assert_eq!(changed.reads.load(Ordering::SeqCst), 3);
    assert_eq!(
        store.read_run(&owner(), "run.managed").await.unwrap().view,
        before
    );
    assert_eq!(store.audit_events(0, 100).await.unwrap(), audit);
    assert_eq!(store.replay_step(&owner(), &request).await.unwrap(), None);
    store.close().await;
}

#[tokio::test]
async fn commit_authority() {
    let (_directory, store, _) = setup().await;
    let before = cancelled_run(&store).await;
    let audit = store.audit_events(0, 100).await.unwrap();
    let changed = Arc::new(CommitClock {
        reads: AtomicUsize::new(0),
        // Ledger, initial owner authorization and both clock fences pass;
        // the final owner authorization alone observes application expiry.
        stable_reads: 4,
        final_value: fixture::NOW + 1,
    });
    let actor = owner();
    let authority = crate::runtime::RuntimeAuthority::new(
        vec![crate::runtime::Identity {
            actor_id: actor.actor_id.as_ref().unwrap().value.clone(),
            subject: actor.authenticated_subject.clone(),
            display_name: actor.display_name.clone(),
            role: crate::runtime::Role::Operator,
            certificate_sha256: vec![format!("sha256:{}", "01".repeat(32))],
            not_before_ms: fixture::NOW - 1,
            expires_at_ms: fixture::NOW + 1,
            run_ids: vec!["run.managed".into()],
        }],
        vec![],
        changed.clone(),
    )
    .unwrap();
    let mut expiring = store.clone();
    expiring.clock = changed.clone();
    expiring.admission = Arc::new(authority);
    let mut request = step(1);
    request.workflow_version = 2;
    assert!(matches!(
        expiring
            .advance_numerical(&actor, &request, &numerical_spec(), None)
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    assert_eq!(changed.reads.load(Ordering::SeqCst), 5);
    assert_eq!(
        store.read_run(&actor, "run.managed").await.unwrap().view,
        before
    );
    assert_eq!(store.audit_events(0, 100).await.unwrap(), audit);
    assert_eq!(store.replay_step(&actor, &request).await.unwrap(), None);
    store.close().await;
}

#[tokio::test]
async fn source_required() {
    let (_directory, store, _) = setup().await;
    seed(&store).await;
    finish(&store).await;
    let mut request = step(1);
    request.workflow_version = 2;
    assert!(matches!(
        store
            .advance_numerical(&owner(), &request, &numerical_spec(), None)
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    assert_eq!(
        store
            .read_run(&owner(), "run.managed")
            .await
            .unwrap()
            .view
            .revision,
        1
    );
    store.close().await;
}

#[tokio::test]
async fn version_denied() {
    let (_directory, store, _) = setup().await;
    seed(&store).await;
    assert!(matches!(
        store
            .advance_run(&owner(), &step(1), &numerical_spec())
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    assert!(matches!(
        store.replay_step(&owner(), &step(1)).await,
        Err(StoreError::AdmissionDenied)
    ));
    let mut request = step(1);
    request.workflow_version = 3;
    assert!(matches!(
        store
            .advance_numerical(&owner(), &request, &numerical_spec(), None)
            .await,
        Err(StoreError::AdmissionDenied)
    ));
    assert_eq!(
        store
            .read_run(&owner(), "run.managed")
            .await
            .unwrap()
            .view
            .revision,
        1
    );
    store.close().await;
}

#[tokio::test]
async fn immutable_transition() {
    let (_directory, store, _) = setup().await;
    seed(&store).await;
    assert!(
        sqlx::query("UPDATE run_transitions SET stage=2")
            .execute(&store.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM run_transitions")
            .execute(&store.pool)
            .await
            .is_err()
    );
    assert_eq!(
        store
            .read_run(&owner(), "run.managed")
            .await
            .unwrap()
            .view
            .revision,
        1
    );
    store.close().await;
}

#[tokio::test]
async fn legacy_writer() {
    let (_directory, store, _) = setup().await;
    seed(&store).await;
    let mut transaction = store.pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('loop.run_writer','v1',true)")
        .execute(&mut *transaction)
        .await
        .unwrap();
    assert!(
        sqlx::query("UPDATE research_runs SET status=5,revision=2")
            .execute(&mut *transaction)
            .await
            .is_err()
    );
    transaction.rollback().await.unwrap();
    assert_eq!(
        store
            .read_run(&owner(), "run.managed")
            .await
            .unwrap()
            .view
            .revision,
        1
    );
    store.close().await;
}

#[tokio::test]
async fn missing_transition() {
    let (_directory, store, _) = setup().await;
    seed(&store).await;
    let mut transaction = store.pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('loop.run_writer','v2',true)")
        .execute(&mut *transaction)
        .await
        .unwrap();
    sqlx::query("UPDATE research_runs SET status=5,revision=2")
        .execute(&mut *transaction)
        .await
        .unwrap();
    assert!(transaction.commit().await.is_err());
    assert_eq!(
        store
            .read_run(&owner(), "run.managed")
            .await
            .unwrap()
            .view
            .revision,
        1
    );
    store.close().await;
}

#[tokio::test]
async fn transition_corruption() {
    let (_directory, store, _) = setup().await;
    seed(&store).await;
    sqlx::query("ALTER TABLE run_transitions DISABLE TRIGGER run_transitions_protected")
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE run_transitions SET binding_sha256=decode(repeat('ff',32),'hex')")
        .execute(&store.pool)
        .await
        .unwrap();
    assert!(matches!(
        store.read_run(&owner(), "run.managed").await,
        Err(StoreError::Corrupt(_))
    ));
    store.close().await;
}

#[tokio::test]
async fn historical_authority() {
    let (_directory, store, _) = setup().await;
    seed(&store).await;
    let record = store
        .read_run(&owner(), "run.managed")
        .await
        .unwrap()
        .current_job;
    let principal = numerical_spec()
        .numerical
        .unwrap()
        .research_executor
        .unwrap();
    let mut transaction = store.pool.begin().await.unwrap();
    assert!(matches!(
        transitions::authorize_trial(
            &store,
            &mut transaction,
            &principal,
            &record,
            &mut transitions::TrialCache::default()
        )
        .await,
        Err(StoreError::AdmissionDenied)
    ));
    transaction.rollback().await.unwrap();
    store.close().await;
}
