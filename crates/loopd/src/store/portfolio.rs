//! Registered predecessor and global-search gates for the installed producer.

use loop_protocol::wire::v1::{
    Actor, BacktestResult, FactorEvaluationWork, JobRecord, job_specification,
};
use sqlx::{Postgres, Transaction};

use super::{PgJobStore, StoreError, StoreResult, TrialLedger};
use crate::manifests::ObjectRef;

/// Numerical lineage resolved by a service-owned portfolio policy. A client
/// cannot supply this to a command; runtime execution constructs the proof.
#[derive(Clone)]
pub struct PortfolioLineage {
    /// Full database-local trial commitment, including unsuccessful attempts.
    pub trials: TrialLedger,
    /// Exact prior installed evaluator input, checked against the persisted job.
    pub work: FactorEvaluationWork,
    /// Actual factor-evaluation manifest consumed by numerical reconstruction.
    pub evaluation: ObjectRef,
    /// Exact factor-value bytes consumed by the portfolio producer.
    pub values: ObjectRef,
}

impl PgJobStore {
    pub(crate) async fn portfolio_source(
        &self,
        principal: &Actor,
        job: &JobRecord,
        lineage: &PortfolioLineage,
    ) -> StoreResult<()> {
        let mut transaction = self.pool.begin().await?;
        validate(self, &mut transaction, principal, job, None, lineage).await?;
        transaction.commit().await?;
        Ok(())
    }
}

pub(super) async fn validate(
    store: &PgJobStore,
    transaction: &mut Transaction<'_, Postgres>,
    principal: &Actor,
    record: &JobRecord,
    result: Option<&BacktestResult>,
    lineage: &PortfolioLineage,
) -> StoreResult<()> {
    if super::research_ledger::capture(store, transaction, principal).await? != lineage.trials {
        return Err(StoreError::StaleTrials);
    }
    validate_lineage(store, transaction, principal, record, result, lineage).await
}

// A new global report recomputes search-adjusted statistics. Its numerical
// sources retain their original search commitment; this check never makes the
// historical adjusted statistics current or permits deletion of past trials.
pub(super) async fn validate_history(
    store: &PgJobStore,
    transaction: &mut Transaction<'_, Postgres>,
    principal: &Actor,
    record: &JobRecord,
    result: &BacktestResult,
    lineage: &PortfolioLineage,
    current: &TrialLedger,
) -> StoreResult<()> {
    for old in &lineage.trials.entries {
        let index = current
            .entries
            .binary_search_by(|entry| entry.job_id.cmp(&old.job_id))
            .map_err(|_| StoreError::StaleTrials)?;
        let new = &current.entries[index];
        if old.run_id != new.run_id
            || old.factor_spec_id != new.factor_spec_id
            || old.specification_sha256 != new.specification_sha256
            || old.attempts > new.attempts
        {
            return Err(StoreError::StaleTrials);
        }
    }
    validate_lineage(store, transaction, principal, record, Some(result), lineage).await
}

async fn validate_lineage(
    store: &PgJobStore,
    transaction: &mut Transaction<'_, Postgres>,
    principal: &Actor,
    record: &JobRecord,
    result: Option<&BacktestResult>,
    lineage: &PortfolioLineage,
) -> StoreResult<()> {
    let work = &lineage.work;
    let id = &work
        .job_id
        .as_ref()
        .ok_or(StoreError::Corrupt("portfolio predecessor"))?
        .value;
    let row = sqlx::query("SELECT * FROM jobs WHERE job_id = $1")
        .bind(id)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(StoreError::AdmissionDenied)?;
    let source = super::postgres::record_from_row(&row)?;
    store
        .admission
        .authorize_job_command("loop.factors.read_trials", principal, &source)?;
    let trial = super::library::trials::verify(transaction, &source).await?;
    let evaluation = trial.evaluation.ok_or(StoreError::AdmissionDenied)?;
    if evaluation.disposition != "ready_for_backtest" {
        return Err(StoreError::AdmissionDenied);
    }
    let computed = evaluation
        .result
        .as_ref()
        .ok_or(StoreError::Corrupt("portfolio evaluation"))?;
    let Some(job_specification::Input::FactorEvaluation(input)) = source
        .specification
        .as_ref()
        .and_then(|job| job.input.as_ref())
    else {
        return Err(StoreError::Corrupt("portfolio source kind"));
    };
    let Some(job_specification::Input::Backtest(target)) = record
        .specification
        .as_ref()
        .and_then(|job| job.input.as_ref())
    else {
        return Err(StoreError::AdmissionDenied);
    };
    let original = input
        .provenance
        .as_ref()
        .ok_or(StoreError::Corrupt("evaluation provenance"))?;
    let current = target
        .provenance
        .as_ref()
        .ok_or(StoreError::Corrupt("portfolio provenance"))?;
    let bound = |artifact: Option<&loop_protocol::wire::v1::ArtifactRef>, reference: &ObjectRef| {
        artifact.is_some_and(|artifact| {
            artifact
                .artifact_id
                .as_ref()
                .is_some_and(|id| id.value == reference.sha256)
                && artifact.byte_size == reference.byte_size
        })
    };
    if work.factor != input.factor
        || work.provenance != input.provenance
        || work.deterministic_seed != input.deterministic_seed
        || work.lease_id != computed.lease_id
        || work.sample_start != computed.sample_start
        || work.sample_end != computed.sample_end
        || input
            .factor
            .as_ref()
            .and_then(|factor| factor.factor_spec_id.as_ref())
            != target.factor_spec_id.as_ref()
        || input.dataset != target.dataset
        || input.deterministic_seed != target.deterministic_seed
        || original.source_code_sha256 != current.source_code_sha256
        || original.operator_registry_sha256 != current.operator_registry_sha256
        || original.environment_sha256 != current.environment_sha256
        || original.data_manifest_sha256 != current.data_manifest_sha256
        || original.trading_calendar_sha256 != current.trading_calendar_sha256
        || !bound(computed.manifest.as_ref(), &lineage.evaluation)
        || !bound(computed.values.as_ref(), &lineage.values)
        || result.is_some_and(|result| {
            !bound(
                result
                    .artifacts
                    .as_ref()
                    .and_then(|artifacts| artifacts.factor_values.as_ref()),
                &lineage.values,
            )
        })
    {
        return Err(StoreError::Corrupt("registered portfolio lineage"));
    }
    Ok(())
}
