//! Leased proposal evidence; perturbations never become numerical factor trials.

use std::collections::BTreeSet;

use loop_protocol::wire::jobs::v1::CompleteJobRequest;
use loop_protocol::wire::v1::{
    Actor, BacktestEngineKind, JobRecord, JobState, job_outcome, job_specification,
};
use sqlx::{Postgres, Transaction};

use super::postgres::{record_from_row, timestamp_millis};
use super::{PgJobStore, StoreError, StoreResult};
use crate::manifests::perturbation::PerturbationEvidence;

fn completed(record: &JobRecord) -> bool {
    record.state == JobState::Succeeded as i32
        && matches!(
            record
                .specification
                .as_ref()
                .and_then(|job| job.input.as_ref()),
            Some(job_specification::Input::Perturbation(_))
        )
}

pub(super) fn check_lease(
    store: &PgJobStore,
    record: &JobRecord,
    request: &CompleteJobRequest,
) -> StoreResult<()> {
    if !matches!(
        record
            .specification
            .as_ref()
            .and_then(|job| job.input.as_ref()),
        Some(job_specification::Input::Perturbation(_))
    ) || !matches!(
        request
            .outcome
            .as_ref()
            .and_then(|outcome| outcome.outcome.as_ref()),
        Some(job_outcome::Outcome::Success(_))
    ) {
        return Ok(());
    }
    let proof = store
        .perturbation_proof
        .as_ref()
        .ok_or(StoreError::AdmissionDenied)?;
    if record.specification.as_ref() != Some(proof.job())
        || request.lease_id.as_ref().map(|id| id.value.as_str()) != Some(proof.lease_id())
    {
        return Err(StoreError::LeaseFenced);
    }
    if let Some(lease) = &record.active_lease
        && timestamp_millis(
            lease.issued_at.as_ref().ok_or(StoreError::LeaseFenced)?,
            false,
        )? != proof.started_at()
    {
        return Err(StoreError::LeaseFenced);
    }
    if record.active_lease.is_some() {
        proof.check_lease(record)?;
    }
    proof.check()
}

pub(super) async fn verify(
    store: &PgJobStore,
    transaction: &mut Transaction<'_, Postgres>,
    principal: &Actor,
    record: &JobRecord,
) -> StoreResult<()> {
    if !completed(record) {
        return Ok(());
    }
    let proof = store
        .perturbation_proof
        .as_ref()
        .ok_or(StoreError::AdmissionDenied)?;
    proof.check_record(record)?;
    verify_sources(store, transaction, principal, proof).await?;
    proof.check()
}

async fn verify_sources(
    store: &PgJobStore,
    transaction: &mut Transaction<'_, Postgres>,
    principal: &Actor,
    proof: &PerturbationEvidence,
) -> StoreResult<()> {
    let job = proof.job();
    let Some(job_specification::Input::Perturbation(input)) = &job.input else {
        return Err(StoreError::AdmissionDenied);
    };
    let child_id = &job
        .job_id
        .as_ref()
        .ok_or(StoreError::AdmissionDenied)?
        .value;
    let submitted = timestamp_millis(
        job.submitted_at
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?,
        true,
    )?;
    if proof.sources().is_empty() || proof.sources().len() > 64 {
        return Err(StoreError::Invalid("perturbation source bounds"));
    }
    let mut seen = BTreeSet::new();
    let mut cache = super::runs::TrialCache::default();
    for (original, result) in proof.sources() {
        let source = original
            .specification
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?;
        let id = &source
            .job_id
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?
            .value;
        let Some(job_specification::Input::Backtest(backtest)) = &source.input else {
            return Err(StoreError::AdmissionDenied);
        };
        if !seen.insert(id)
            || source.run_id != job.run_id
            || backtest.dataset != input.dataset
            || original.state != JobState::Succeeded as i32
            || result.engine != BacktestEngineKind::PrimaryCrossSectional as i32
            || timestamp_millis(
                original
                    .updated_at
                    .as_ref()
                    .ok_or(StoreError::AdmissionDenied)?,
                true,
            )? > submitted
        {
            return Err(StoreError::AdmissionDenied);
        }
        let row = sqlx::query("SELECT * FROM jobs WHERE job_id=$1")
            .bind(id)
            .fetch_optional(&mut **transaction)
            .await?
            .ok_or(StoreError::AdmissionDenied)?;
        let current = record_from_row(&row)?;
        if current != *original {
            return Err(StoreError::Corrupt("perturbation source changed"));
        }
        let earlier: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM run_transitions s JOIN run_transitions c
             ON s.run_id=c.run_id WHERE s.current_job_id=$1 AND c.current_job_id=$2
             AND s.created_child AND c.created_child AND s.stage=3 AND c.stage=4
             AND s.round<c.round AND s.revision<c.revision)",
        )
        .bind(id)
        .bind(child_id)
        .fetch_one(&mut **transaction)
        .await?;
        if !earlier {
            return Err(StoreError::AdmissionDenied);
        }
        super::runs::authorize_trial(store, transaction, principal, &current, &mut cache).await?;
        if super::backtest::verify_stored(transaction, &current)
            .await?
            .as_ref()
            != Some(result)
        {
            return Err(StoreError::Corrupt("perturbation registered result"));
        }
    }
    Ok(())
}

impl PgJobStore {
    /// Verify a successful perturbation under one bounded read snapshot.
    /// Missing, inaccessible, stale or corrupt source/result evidence denies;
    /// replay never launches a worker or applies random state. Cancellation
    /// rolls back the read, leaving immutable job and parent history untouched.
    pub(crate) async fn current_perturbation(
        &self,
        principal: &Actor,
        proof: &PerturbationEvidence,
    ) -> StoreResult<JobRecord> {
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            proof.check()?;
            let mut transaction = super::research_ledger::begin_snapshot(self).await?;
            let id = &proof
                .job()
                .job_id
                .as_ref()
                .ok_or(StoreError::AdmissionDenied)?
                .value;
            let row = sqlx::query("SELECT * FROM jobs WHERE job_id=$1")
                .bind(id)
                .fetch_optional(&mut *transaction)
                .await?
                .ok_or(StoreError::AdmissionDenied)?;
            let record = record_from_row(&row)?;
            self.admission
                .authorize_job_command("loop.jobs.read", principal, &record)?;
            if !completed(&record) {
                return Err(StoreError::InvalidTransition);
            }
            proof.check_record(&record)?;
            verify_sources(self, &mut transaction, principal, proof).await?;
            self.admission
                .authorize_job_command("loop.jobs.read", principal, &record)?;
            proof.check_record(&record)?;
            transaction.commit().await?;
            Ok(record)
        })
        .await
        .map_err(|_| StoreError::Unavailable("perturbation read deadline"))?
    }
}

#[cfg(test)]
mod tests;
