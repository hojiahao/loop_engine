//! Bind complete-population statistics to existing receipts and transactions.

use loop_protocol::wire::v1::{Actor, JobRecord, JobState, job_outcome};
use sqlx::{Postgres, Transaction};

use super::{PgJobStore, StoreError, StoreResult, TrialLedger};
use crate::manifests::portfolio::PreparedPortfolio;
use crate::manifests::statistics::{StatisticsEvidence, is_statistics};

fn completed(record: &JobRecord) -> bool {
    record.state == JobState::Succeeded as i32
        && record.specification.as_ref().is_some_and(is_statistics)
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
        .statistics_evidence
        .as_ref()
        .ok_or(StoreError::AdmissionDenied)?;
    proof.check_record(record)?;
    if super::research_ledger::capture_records(store, transaction, principal).await?
        != proof.snapshot
    {
        return Err(StoreError::Invalid("global trial snapshot changed"));
    }
    for (original, portfolio) in &proof.portfolios {
        let prepared = store.with_backtest_policy(portfolio.clone());
        verify_source(
            &prepared,
            transaction,
            principal,
            original,
            portfolio,
            &proof.snapshot.ledger,
        )
        .await?;
    }
    proof.check()
}

async fn verify_source(
    store: &PgJobStore,
    transaction: &mut Transaction<'_, Postgres>,
    principal: &Actor,
    original: &JobRecord,
    portfolio: &PreparedPortfolio,
    ledger: &TrialLedger,
) -> StoreResult<()> {
    let id = &portfolio.inputs.pin.job_id;
    let row = sqlx::query("SELECT * FROM jobs WHERE job_id = $1")
        .bind(id)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(StoreError::AdmissionDenied)?;
    let current = super::postgres::record_from_row(&row)?;
    if current != *original {
        return Err(StoreError::Corrupt("statistics registered source changed"));
    }
    store
        .admission
        .authorize_job_command("loop.backtests.read_current", principal, &current)?;
    let result = super::backtest::verify_stored(transaction, &current)
        .await?
        .ok_or(StoreError::InvalidTransition)?;
    super::portfolio::validate_history(
        store,
        transaction,
        principal,
        &current,
        &result,
        portfolio
            .lineage
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?,
        ledger,
    )
    .await?;
    super::backtest::resolve_current_result(
        store,
        principal,
        &current,
        portfolio.inputs.context_id(),
        &result,
    )
}

pub(super) fn check_lease(
    store: &PgJobStore,
    record: &JobRecord,
    request: &loop_protocol::wire::jobs::v1::CompleteJobRequest,
) -> StoreResult<()> {
    if !record.specification.as_ref().is_some_and(is_statistics)
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
        .statistics_evidence
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

impl PgJobStore {
    /// Preflight registered numerical lineage under a current whole-registry
    /// snapshot within 30 seconds. Missing/stale/corrupt sources deny; a later
    /// supervised report transaction must recheck every source. Original
    /// adjusted statistics remain historical. Cancellation leaves no writes.
    pub(crate) async fn statistics_source(
        &self,
        principal: &Actor,
        original: &JobRecord,
        portfolio: &PreparedPortfolio,
        ledger: &TrialLedger,
    ) -> StoreResult<()> {
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            let mut transaction = self.pool.begin().await?;
            verify_source(
                self,
                &mut transaction,
                principal,
                original,
                portfolio,
                ledger,
            )
            .await?;
            transaction.commit().await?;
            Ok(())
        })
        .await
        .map_err(|_| StoreError::Unavailable("statistics source deadline"))?
    }

    /// Recheck an actual reconstructed report under the writer lock, including
    /// exact registry revisions and registered sources. Deny absent or changed
    /// evidence; never repair files or append a replay receipt. Cancellation or
    /// the 30-second deadline rolls back the read transaction.
    pub(crate) async fn current_statistics(
        &self,
        principal: &Actor,
        proof: &StatisticsEvidence,
    ) -> StoreResult<JobRecord> {
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            let mut transaction = self.pool.begin().await?;
            self.observe_clock(&mut transaction).await?;
            let row = sqlx::query("SELECT * FROM jobs WHERE job_id = $1")
                .bind(&proof.document.job_id)
                .fetch_optional(&mut *transaction)
                .await?
                .ok_or(StoreError::AdmissionDenied)?;
            let record = super::postgres::record_from_row(&row)?;
            self.admission.authorize_job_command(
                "loop.statistics.read_current",
                principal,
                &record,
            )?;
            if !completed(&record) {
                return Err(StoreError::InvalidTransition);
            }
            verify(self, &mut transaction, principal, &record).await?;
            transaction.commit().await?;
            Ok(record)
        })
        .await
        .map_err(|_| StoreError::Unavailable("statistics read deadline"))?
    }
}
