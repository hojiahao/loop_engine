//! Registered independent evidence uses the existing job/receipt/audit transaction.

use loop_protocol::wire::v1::{Actor, JobRecord, JobState, job_outcome, job_specification};
use sqlx::{Postgres, Transaction};

use super::{PgJobStore, StoreError, StoreResult, backtest};
use crate::manifests::reconciliation::{Disposition, ValidationEvidence};

pub(super) fn completed(record: &JobRecord) -> bool {
    record.state == JobState::Succeeded as i32
        && matches!(
            record.specification.as_ref().and_then(|job| job.input.as_ref()),
            Some(job_specification::Input::Reconciliation(input)) if input.validation.is_some()
        )
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
    let evidence = store
        .validation_evidence
        .as_ref()
        .ok_or(StoreError::AdmissionDenied)?;
    evidence.check_record(record)?;
    if let Some(statistics) = &evidence.statistics {
        let prepared = store.with_statistics_evidence(statistics.proof.clone());
        let row = sqlx::query("SELECT * FROM jobs WHERE job_id = $1")
            .bind(&statistics.proof.document.job_id)
            .fetch_optional(&mut **transaction)
            .await?
            .ok_or(StoreError::StatisticsPending)?;
        let registered = super::postgres::record_from_row(&row)?;
        store.admission.authorize_job_command(
            "loop.statistics.read_current",
            principal,
            &registered,
        )?;
        if registered != statistics.record || registered.state != JobState::Succeeded as i32 {
            return Err(StoreError::StatisticsPending);
        }
        // This rechecks every source and exact state/revision in the same
        // transaction. Old primary adjusted statistics do not become current.
        super::statistics::verify(&prepared, transaction, principal, &registered).await?;
        statistics.binding(&evidence.primary_record)?;
        return evidence.check();
    }
    let prepared = store.with_backtest_policy(evidence.primary.clone());
    let (source, _) = backtest::current_in_transaction(
        &prepared,
        transaction,
        principal,
        &evidence.document.primary_job_id,
        &evidence.document.context_id,
        "loop.backtests.read_current",
    )
    .await?;
    if source != evidence.primary_record {
        return Err(StoreError::Corrupt("validation registered primary changed"));
    }
    evidence.check()?;
    Ok(())
}

impl PgJobStore {
    pub(crate) async fn current_validation(
        &self,
        principal: &Actor,
        evidence: &ValidationEvidence,
    ) -> StoreResult<JobRecord> {
        evidence.check()?;
        let mut transaction = self.pool.begin().await?;
        if evidence.statistics.is_some() {
            self.observe_clock(&mut transaction).await?;
        }
        let id = &evidence.document.job_id;
        let row = sqlx::query("SELECT * FROM jobs WHERE job_id = $1")
            .bind(id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or(StoreError::AdmissionDenied)?;
        let record = super::postgres::record_from_row(&row)?;
        self.admission.authorize_job_command(
            "loop.reconciliation.read_current",
            principal,
            &record,
        )?;
        if !completed(&record) {
            return Err(StoreError::InvalidTransition);
        }
        verify(self, &mut transaction, principal, &record).await?;
        transaction.commit().await?;
        Ok(record)
    }
}

pub(super) async fn admission(
    store: &PgJobStore,
    transaction: &mut Transaction<'_, Postgres>,
    principal: &Actor,
    source: &JobRecord,
) -> StoreResult<()> {
    let Some(evidence) = &store.validation_evidence else {
        return Ok(());
    };
    if *source != evidence.primary_record {
        return Err(StoreError::Corrupt("admission validation source"));
    }
    let row = sqlx::query("SELECT * FROM jobs WHERE job_id = $1")
        .bind(&evidence.document.job_id)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(StoreError::IndependentPending)?;
    let record = super::postgres::record_from_row(&row)?;
    store.admission.authorize_job_command(
        "loop.reconciliation.read_current",
        principal,
        &record,
    )?;
    if !completed(&record) {
        return Err(StoreError::IndependentPending);
    }
    verify(store, transaction, principal, &record).await?;
    // Current supported data profiles are explicitly synthetic/public-development.
    // This gate remains inside the shared command, ahead of semantic overrides.
    match evidence.document.disposition {
        Disposition::Rejected => Err(StoreError::IndependentMismatch),
        Disposition::Unavailable => Err(StoreError::IndependentUnavailable),
        Disposition::Accepted => match &evidence.document.global_statistics {
            None => Err(StoreError::StatisticsPending),
            Some(binding) if !binding.available => Err(StoreError::StatisticsUnavailable),
            Some(_) => Err(StoreError::AdmissionPrerequisite),
        },
    }
}

pub(super) fn check_lease(
    store: &PgJobStore,
    record: &JobRecord,
    request: &loop_protocol::wire::jobs::v1::CompleteJobRequest,
) -> StoreResult<()> {
    if !matches!(record.specification.as_ref().and_then(|job| job.input.as_ref()),
        Some(job_specification::Input::Reconciliation(input)) if input.validation.is_some())
        || !matches!(
            request
                .outcome
                .as_ref()
                .and_then(|outcome| outcome.outcome.as_ref()),
            Some(job_outcome::Outcome::Success(_))
        )
    {
        return Ok(());
    }
    let proof = store
        .validation_evidence
        .as_ref()
        .ok_or(StoreError::AdmissionDenied)?;
    if record.specification.as_ref() != Some(&proof.job)
        || request
            .lease_id
            .as_ref()
            .is_none_or(|lease| lease.value != proof.document.lease_id)
    {
        return Err(StoreError::LeaseFenced);
    }
    if let Some(lease) = &record.active_lease
        && super::timestamp_millis(
            lease.issued_at.as_ref().ok_or(StoreError::LeaseFenced)?,
            false,
        )? != proof.document.started_at_ms
    {
        return Err(StoreError::LeaseFenced);
    }
    proof.check()
}
