//! Storage boundary fixtures deliberately omit file-backed runtime authority.
//! Installed numerical execution is separately covered by runtime acceptance.

use super::*;
use crate::store::runs::transitions::{self, RunBinding};
use loop_protocol::wire::research::v1::ResearchJobBudget;
use loop_protocol::wire::runs::v1::{
    NumericalWorkflow, ResearchRunView, RunStage, research_run_view::CurrentChild,
};
use sha2::{Digest, Sha256};

pub(super) fn numerical_spec() -> RunSpecification {
    let mut specification = specification();
    let numerical = ResearchJobBudget {
        maximum_steps: 1,
        maximum_input_tokens: 0,
        maximum_output_tokens: 0,
        maximum_cost: Some(storage::money(0)),
        maximum_wall_time: Some(prost_types::Duration {
            seconds: 120,
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
