use loop_protocol::provenance::{ProvenanceSnapshot, assess_provenance};
use loop_protocol::wire::v1::{
    Actor, BacktestEngineKind, FactorSpecId, JobRecord, PerturbationSpace, PerturbationState,
    PerturbationStep, PerturbationWork, WindowObservation, job_outcome, job_specification,
};
use prost::Message;
use prost_types::Timestamp;
use sqlx::{Postgres, Row, Transaction, postgres::PgRow};

use super::super::postgres::{record_from_row, timestamp_millis, verified_blob};
use super::super::{PgJobStore, StoreError, StoreResult, backtest, rejection, validate_id};
use super::{AdvancePerturbation, OPERATION, PerturbationResult, validation};

pub(super) struct Prepared {
    pub record: JobRecord,
    pub space: PerturbationSpace,
    pub work: PerturbationWork,
    pub revision: u64,
}

#[derive(Clone, PartialEq, Message)]
pub(super) struct Receipt {
    #[prost(message, optional, tag = "1")]
    pub record: Option<JobRecord>,
    #[prost(message, optional, tag = "2")]
    pub space: Option<PerturbationSpace>,
    #[prost(string, tag = "3")]
    pub context_id: String,
    #[prost(uint64, tag = "4")]
    pub revision: u64,
    #[prost(message, optional, tag = "5")]
    pub step: Option<PerturbationStep>,
    #[prost(message, optional, tag = "6")]
    pub accepted_at: Option<Timestamp>,
    #[prost(string, tag = "7")]
    pub request_id: String,
}

impl Receipt {
    pub(super) fn result(&self, replayed: bool) -> StoreResult<PerturbationResult> {
        Ok(PerturbationResult {
            revision: self.revision,
            step: self
                .step
                .clone()
                .ok_or(StoreError::Corrupt("receipt step"))?,
            accepted_at: self
                .accepted_at
                .ok_or(StoreError::Corrupt("receipt time"))?,
            replayed,
        })
    }
}

pub(super) async fn prepare(
    store: &PgJobStore,
    transaction: &mut Transaction<'_, Postgres>,
    principal: &Actor,
    command: &AdvancePerturbation,
) -> StoreResult<Prepared> {
    let job_id = &command
        .source_job_id
        .as_ref()
        .ok_or(StoreError::Invalid("source job"))?
        .value;
    let row = sqlx::query("SELECT * FROM jobs WHERE job_id = $1")
        .bind(job_id)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(StoreError::NotFound)?;
    let record = record_from_row(&row)?;
    store
        .admission
        .authorize_job_command(OPERATION, principal, &record)?;
    let job = record
        .specification
        .as_ref()
        .ok_or(StoreError::Corrupt("source specification"))?;
    // Reject protected job kinds before any result or holdout resolver is called.
    let Some(job_specification::Input::Backtest(input)) = &job.input else {
        return Err(StoreError::AdmissionDenied);
    };
    store.admission.validate_submission(job)?;
    let space =
        store
            .backtest_policy
            .resolve_perturbation_space(principal, job, &command.context_id)?;
    validation::space(&space, input)?;
    let candidate = space
        .candidates
        .iter()
        .find(|candidate| candidate.factor_spec_id == input.factor_spec_id)
        .ok_or(StoreError::AdmissionDenied)?;
    let observation = match record
        .outcome
        .as_ref()
        .and_then(|outcome| outcome.outcome.as_ref())
    {
        Some(job_outcome::Outcome::Success(_)) => {
            let (_, result) = backtest::current_in_transaction(
                store,
                transaction,
                principal,
                job_id,
                &command.context_id,
                OPERATION,
            )
            .await?;
            if result.engine != BacktestEngineKind::PrimaryCrossSectional as i32 {
                return Err(StoreError::Invalid("perturbation source engine"));
            }
            let metric = result
                .metrics
                .iter()
                .find(|metric| metric.name == "net_sharpe")
                .filter(|metric| {
                    metric.unit == "dimensionless"
                        && metric.estimator == "sample_std_ddof1_sqrt252_zero_rf.v1"
                })
                .ok_or(StoreError::Invalid("IS net Sharpe metric"))?;
            let value = &metric
                .value
                .as_ref()
                .ok_or(StoreError::Invalid("Sharpe value"))?
                .value;
            let net_sharpe: f64 = value
                .parse()
                .map_err(|_| StoreError::Invalid("Sharpe value"))?;
            if !net_sharpe.is_finite()
                || net_sharpe.abs() > 1_000_000.0
                || (net_sharpe == 0.0 && value != "0")
            {
                return Err(StoreError::Invalid("Sharpe range"));
            }
            Some(WindowObservation {
                source_job_id: command.source_job_id.clone(),
                candidate: Some(candidate.clone()),
                net_sharpe,
            })
        }
        Some(job_outcome::Outcome::FactorRejection(rejected)) if matches!(rejected.code, 4..=6) => {
            rejection::verify_stored(transaction, &record).await?;
            let current =
                store
                    .backtest_policy
                    .resolve_current(principal, job, &command.context_id)?;
            let current = current
                .as_ref()
                .map(ProvenanceSnapshot::try_from)
                .transpose()?;
            let frozen = ProvenanceSnapshot::try_from(
                input
                    .provenance
                    .as_ref()
                    .ok_or(StoreError::Corrupt("source provenance"))?,
            )?;
            assess_provenance(&frozen, &frozen, current.as_ref())?.require_current()?;
            None
        }
        _ => return Err(StoreError::InvalidTransition),
    };
    let (revision, state) = load_state(transaction, &command.context_id, &space).await?;
    validation::state(&state, &space)?;
    let mut failed_factor_ids = Vec::<FactorSpecId>::new();
    for candidate in &space.candidates {
        let mut lookup = job.clone();
        let Some(job_specification::Input::Backtest(input)) = &mut lookup.input else {
            return Err(StoreError::Corrupt("backtest input"));
        };
        input.factor_spec_id = candidate.factor_spec_id.clone();
        match rejection::check_previous(transaction, &lookup).await {
            Ok(()) => {}
            Err(StoreError::PreviouslyRejected) => failed_factor_ids.push(
                candidate
                    .factor_spec_id
                    .clone()
                    .ok_or(StoreError::Invalid("candidate id"))?,
            ),
            Err(error) => return Err(error),
        }
    }
    let work = PerturbationWork {
        state: Some(state),
        candidates: space.candidates.clone(),
        observation,
        failed_factor_ids,
        current_window: candidate.window,
    };
    Ok(Prepared {
        record,
        space,
        work,
        revision,
    })
}

async fn load_state(
    transaction: &mut Transaction<'_, Postgres>,
    context_id: &str,
    space: &PerturbationSpace,
) -> StoreResult<(u64, PerturbationState)> {
    let row = sqlx::query("SELECT * FROM perturbation_states WHERE context_id = $1")
        .bind(context_id)
        .fetch_optional(&mut **transaction)
        .await?;
    let Some(row) = row else {
        return Ok((
            0,
            PerturbationState {
                version: 1,
                random_seed: space.random_seed.clone(),
                ..Default::default()
            },
        ));
    };
    let stored_space =
        PerturbationSpace::decode(verified_blob(&row, "space_blob", "space_sha256")?.as_slice())
            .map_err(|_| StoreError::Corrupt("perturbation space"))?;
    if stored_space != *space {
        return Err(StoreError::Invalid("immutable perturbation space changed"));
    }
    let state =
        PerturbationState::decode(verified_blob(&row, "state_blob", "state_sha256")?.as_slice())
            .map_err(|_| StoreError::Corrupt("perturbation state encoding"))?;
    let revision = row.try_get::<i64, _>("revision")?;
    let receipt_row = sqlx::query("SELECT * FROM command_receipts WHERE actor_id = $1 AND operation = $2 AND idempotency_key = $3")
        .bind(row.try_get::<String, _>("actor_id")?).bind(OPERATION)
        .bind(row.try_get::<String, _>("idempotency_key")?)
        .fetch_optional(&mut **transaction).await?.ok_or(StoreError::Corrupt("state receipt missing"))?;
    let receipt = decode_receipt(&receipt_row)?;
    if revision <= 0
        || receipt.revision != revision as u64
        || receipt.context_id != context_id
        || receipt.space.as_ref() != Some(space)
        || receipt.step.as_ref().and_then(|step| step.state.as_ref()) != Some(&state)
        || row.try_get::<i64, _>("updated_at_ms")?
            != receipt_row.try_get::<i64, _>("committed_at_ms")?
    {
        return Err(StoreError::Corrupt("state receipt binding"));
    }
    Ok((revision as u64, state))
}

fn decode_receipt(row: &PgRow) -> StoreResult<Receipt> {
    let receipt =
        Receipt::decode(verified_blob(row, "response_blob", "response_sha256")?.as_slice())
            .map_err(|_| StoreError::Corrupt("perturbation receipt"))?;
    let request = AdvancePerturbation::decode(
        verified_blob(row, "request_blob", "request_sha256")?.as_slice(),
    )
    .map_err(|_| StoreError::Corrupt("perturbation receipt request"))?;
    let committed = timestamp_millis(
        receipt
            .accepted_at
            .as_ref()
            .ok_or(StoreError::Corrupt("receipt time"))?,
        false,
    )?;
    let record = receipt
        .record
        .as_ref()
        .ok_or(StoreError::Corrupt("receipt source"))?;
    let completed = timestamp_millis(
        record
            .updated_at
            .as_ref()
            .ok_or(StoreError::Corrupt("source time"))?,
        false,
    )?;
    validate_id(&receipt.request_id).map_err(|_| StoreError::Corrupt("receipt request id"))?;
    if receipt.revision == 0
        || receipt.revision != request.expected_revision.saturating_add(1)
        || receipt.context_id != request.context_id
        || committed < completed
        || request.source_job_id
            != record
                .specification
                .as_ref()
                .and_then(|job| job.job_id.clone())
        || request.source_job_id.as_ref().map(|id| &id.value)
            != Some(&row.try_get::<String, _>("job_id")?)
        || row.try_get::<String, _>("request_id")? != receipt.request_id
        || row.try_get::<i64, _>("committed_at_ms")? != committed
    {
        return Err(StoreError::Corrupt("perturbation receipt binding"));
    }
    Ok(receipt)
}

pub(super) async fn replay(
    transaction: &mut Transaction<'_, Postgres>,
    command: &AdvancePerturbation,
    prepared: &Prepared,
    now: i64,
) -> StoreResult<Option<PerturbationResult>> {
    let context = command
        .context
        .as_ref()
        .ok_or(StoreError::Invalid("context"))?;
    let actor = &context
        .actor
        .as_ref()
        .and_then(|actor| actor.actor_id.as_ref())
        .ok_or(StoreError::AdmissionDenied)?
        .value;
    let key = &context
        .idempotency_key
        .as_ref()
        .ok_or(StoreError::Invalid("key"))?
        .value;
    let row = sqlx::query("SELECT * FROM command_receipts WHERE actor_id = $1 AND operation = $2 AND idempotency_key = $3")
        .bind(actor).bind(OPERATION).bind(key).fetch_optional(&mut **transaction).await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let original = AdvancePerturbation::decode(
        verified_blob(&row, "request_blob", "request_sha256")?.as_slice(),
    )
    .map_err(|_| StoreError::Corrupt("perturbation request"))?;
    if original != command.normalized() {
        return Err(StoreError::IdempotencyConflict);
    }
    let receipt = decode_receipt(&row)?;
    let state = receipt
        .step
        .as_ref()
        .and_then(|step| step.state.as_ref())
        .ok_or(StoreError::Corrupt("receipt state"))?;
    validation::state(state, &prepared.space)?;
    let current = prepared
        .work
        .state
        .as_ref()
        .ok_or(StoreError::Corrupt("current state"))?;
    if receipt.record.as_ref() != Some(&prepared.record)
        || receipt.space.as_ref() != Some(&prepared.space)
        || prepared.revision < receipt.revision
        || current.random_draws < state.random_draws
        || !current.history.starts_with(&state.history)
        || !current
            .proposed_factor_ids
            .starts_with(&state.proposed_factor_ids)
        || timestamp_millis(
            receipt
                .accepted_at
                .as_ref()
                .ok_or(StoreError::Corrupt("receipt time"))?,
            false,
        )? > now
    {
        return Err(StoreError::Corrupt("perturbation replay binding"));
    }
    Ok(Some(receipt.result(true)?))
}
