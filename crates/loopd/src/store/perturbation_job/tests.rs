use std::sync::Arc;

use loop_protocol::wire::jobs::v1::{AcquireJobLeaseRequest, CancelJobRequest, CompleteJobRequest};
use loop_protocol::wire::v1::*;
use prost::Message;
use sha2::{Digest, Sha256};

use super::*;
use crate::store::{AdmissionPolicy, JobMutation, JobRepository, SubmitJob};
use crate::test_support as fixture;

struct Admission;

impl AdmissionPolicy for Admission {
    fn validate_submission(&self, _: &JobSpecification) -> StoreResult<()> {
        Ok(())
    }
}

fn specification() -> JobSpecification {
    let mut job = fixture::command(1).specification;
    let (_, job_specification::Input::Discovery(discovery)) = fixture::research::inputs().remove(0)
    else {
        unreachable!()
    };
    let mut work = fixture::artifact();
    work.schema.as_mut().unwrap().name = "loop.perturbation_work".to_owned();
    work.media_type = "application/x-protobuf".to_owned();
    job.kind = JobKind::Perturbation as i32;
    job.input = Some(job_specification::Input::Perturbation(
        PerturbationJobInput {
            dataset: discovery.dataset,
            feedback_policy: discovery.research_policy,
            parent_binding_sha256: Some(fixture::digest(31)),
            family_sha256: Some(fixture::digest(32)),
            work: Some(work),
            source_code_sha256: Some(fixture::digest(33)),
            environment_sha256: Some(fixture::digest(34)),
            budget: Some(JobBudget {
                maximum_steps: 1,
                maximum_cost: Some(Money {
                    amount: Some(ExactDecimal {
                        value: "0".to_owned(),
                    }),
                    currency_code: "USD".to_owned(),
                }),
                maximum_wall_time: Some(prost_types::Duration {
                    seconds: 20,
                    nanos: 0,
                }),
                ..Default::default()
            }),
            algorithm: "window.ema-gradient-pcg64.v1".to_owned(),
        },
    ));
    let selection = job.protocol_selection.as_mut().unwrap();
    selection
        .enabled_features
        .push("jobs.perturbation.v1".to_owned());
    selection.enabled_features.sort_unstable();
    selection.selection_sha256 = Some(Sha256Digest {
        value: loop_protocol::job::protocol_selection_sha256(selection)
            .unwrap()
            .to_vec(),
    });
    loop_protocol::job::validate_job_specification(&job).unwrap();
    job
}

fn queued() -> JobRecord {
    let job = specification();
    JobRecord {
        updated_at: job.submitted_at,
        specification: Some(job),
        state: JobState::Queued as i32,
        revision: 1,
        ..Default::default()
    }
}

async fn empty(store: &PgJobStore) {
    let writes: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM jobs) + (SELECT count(*) FROM factor_trials)
         + (SELECT count(*) FROM command_receipts) + (SELECT count(*) FROM audit_events)",
    )
    .fetch_one(&store.pool)
    .await
    .unwrap();
    assert_eq!(writes, 0);
}

#[tokio::test]
async fn generic_denied() {
    let (_directory, mut store, _) = fixture::fixture().await;
    // Even a permissive administrative policy cannot bypass the private parent
    // reservation with the generic submission entry point.
    store.admission = Arc::new(Admission);
    let error = store
        .submit(SubmitJob {
            request_id: "request.perturbation".to_owned(),
            specification: specification(),
        })
        .await
        .unwrap_err();
    assert!(matches!(error, StoreError::AdmissionDenied));
    empty(&store).await;
}

#[tokio::test]
async fn unreserved_denied() {
    let (_directory, store, _) = fixture::fixture().await;
    let mut transaction = store.pool.begin().await.unwrap();
    assert!(matches!(
        super::super::postgres::insert_job(&mut transaction, &specification(), fixture::NOW).await,
        Err(StoreError::AdmissionDenied)
    ));
    transaction.rollback().await.unwrap();
    empty(&store).await;
}

#[tokio::test]
async fn database_fence() {
    let (_directory, store, _) = fixture::fixture().await;
    let record = queued();
    let job = record.specification.as_ref().unwrap();
    let bytes = record.encode_to_vec();
    let mut transaction = store.pool.begin().await.unwrap();
    let error = sqlx::query(
        "INSERT INTO jobs(job_id,run_id,kind,state,revision,attempt,submitted_at_ms,
         updated_at_ms,deadline_ms,record_blob,record_sha256)
         VALUES ($1,$2,8,1,1,0,$3,$3,$4,$5,$6)",
    )
    .bind(&job.job_id.as_ref().unwrap().value)
    .bind(&job.run_id.as_ref().unwrap().value)
    .bind(fixture::NOW - 1_000)
    .bind(fixture::NOW + 19_000)
    .bind(&bytes)
    .bind(Sha256::digest(&bytes).as_slice())
    .execute(&mut *transaction)
    .await
    .unwrap_err();
    let database = error.as_database_error().unwrap();
    assert_eq!(database.code().as_deref(), Some("23514"));
    assert!(
        database
            .message()
            .contains("perturbation requires an adaptive parent")
    );
    transaction.rollback().await.unwrap();
    empty(&store).await;
}

#[tokio::test]
async fn proof_required() {
    let (_directory, store, _) = fixture::fixture().await;
    let mut record = queued();
    let lease_id = LeaseId {
        value: "lease.perturbation".to_owned(),
    };
    record.state = JobState::Leased as i32;
    record.revision = 2;
    record.attempt = 1;
    record.updated_at = Some(fixture::timestamp(fixture::NOW));
    record.active_lease = Some(JobLease {
        lease_id: Some(lease_id.clone()),
        job_id: record.specification.as_ref().unwrap().job_id.clone(),
        owner: Some(fixture::actor()),
        acquired_revision: 2,
        issued_at: Some(fixture::timestamp(fixture::NOW)),
        heartbeat_at: Some(fixture::timestamp(fixture::NOW)),
        expires_at: Some(fixture::timestamp(fixture::NOW + 10_000)),
    });
    loop_protocol::job::validate_job_record(&record).unwrap();
    let mut receipt = fixture::artifact();
    receipt.schema.as_mut().unwrap().name = "loop.perturbation_receipt".to_owned();
    receipt.media_type = "application/x-protobuf".to_owned();
    let request = CompleteJobRequest {
        context: Some(fixture::context("complete.perturbation")),
        job_id: record.specification.as_ref().unwrap().job_id.clone(),
        expected_revision: 2,
        lease_id: Some(lease_id),
        outcome: Some(JobOutcome {
            outcome: Some(job_outcome::Outcome::Success(JobSuccess {
                outputs: vec![receipt],
            })),
        }),
    };
    assert!(matches!(
        check_lease(&store, &record, &request),
        Err(StoreError::AdmissionDenied)
    ));
    let mut forged = record;
    forged.state = JobState::Succeeded as i32;
    forged.revision = 3;
    forged.active_lease = None;
    forged.outcome = request.outcome;
    loop_protocol::job::validate_job_record(&forged).unwrap();
    let mut transaction = store.pool.begin().await.unwrap();
    assert!(matches!(
        verify(&store, &mut transaction, &fixture::actor(), &forged).await,
        Err(StoreError::AdmissionDenied)
    ));
    transaction.rollback().await.unwrap();
    empty(&store).await;
}

#[test]
fn no_factor_trial() {
    let job = specification();
    assert!(
        super::super::library::trials::factor_id(&job)
            .unwrap()
            .is_none()
    );
    let budget = super::super::postgres::budget(&job).unwrap();
    assert_eq!(budget.maximum_steps, 1);
    assert_eq!(budget.maximum_input_tokens, 0);
    assert_eq!(budget.maximum_output_tokens, 0);
}

#[tokio::test]
async fn lease_replay() {
    let (_directory, store, _) = fixture::fixture().await;
    // The generic lease contract is shared by numerical jobs. A receipt may
    // describe an old lease, so its replay flag can never grant worker dispatch.
    let submitted = store.submit(fixture::command(1)).await.unwrap();
    let id = submitted.job.specification.as_ref().unwrap().job_id.clone();
    let acquire = JobMutation::Acquire(AcquireJobLeaseRequest {
        context: Some(fixture::context("numerical.acquire")),
        job_id: id.clone(),
        expected_revision: 1,
        requested_duration: Some(prost_types::Duration {
            seconds: 20,
            nanos: 0,
        }),
    });
    let claimed = store
        .mutate(&fixture::actor(), acquire.clone())
        .await
        .unwrap();
    assert!(!claimed.replayed);
    let cancelled = store
        .mutate(
            &fixture::actor(),
            JobMutation::Cancel(CancelJobRequest {
                context: Some(fixture::context("numerical.cancel")),
                job_id: id.clone(),
                expected_revision: claimed.job.revision,
                reason: "test cancellation".into(),
            }),
        )
        .await
        .unwrap();
    let events = store.audit_events(0, 10).await.unwrap();
    let replay = store.mutate(&fixture::actor(), acquire).await.unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.job, claimed.job);
    assert_eq!(replay.job.state, JobState::Leased as i32);
    let actual = store.get(&id.unwrap().value).await.unwrap().unwrap();
    assert_eq!(actual, cancelled.job);
    assert_eq!(actual.state, JobState::Cancelled as i32);
    assert_eq!(actual.attempt, 1);
    assert!(actual.active_lease.is_none());
    assert_eq!(store.audit_events(0, 10).await.unwrap(), events);
}
