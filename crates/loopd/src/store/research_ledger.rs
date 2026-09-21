//! Complete, bounded development-trial accounting across all runs.

use loop_protocol::wire::v1::{Actor, JobKind, JobRecord};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction};

use super::postgres::{encode_message, record_from_row};
use super::{PgJobStore, StoreError, StoreResult};

/// One immutable trial identity and its conservative acquired-attempt count.
/// Terminal status is retained in job/audit history, not confused with a loss.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialEntry {
    /// Exact registered job, sorted globally rather than selected by a caller.
    pub job_id: String,
    /// Original research run; changing runs cannot reset search accounting.
    pub run_id: String,
    /// Canonical, frozen factor identity.
    pub factor_spec_id: String,
    /// Checksum of the immutable original job specification.
    pub specification_sha256: String,
    /// At least one for accepted work, including cancellation before acquisition.
    pub attempts: u32,
}

/// Complete database-local development trial commitment. No subset/filter API.
/// Counts are conservative search accounting, not an independent-trial estimate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialLedger {
    /// Versioned encoding, shared with the installed numerical producer.
    pub schema: String,
    /// Every registered development trial, bounded to 4096 jobs/65536 attempts.
    pub entries: Vec<TrialEntry>,
}

/// Exact whole-population records retained only inside the trusted runtime.
/// Reports serialize a bounded revision/state projection; final registration
/// compares these complete records inside the existing ledger transaction.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TrialSnapshot {
    pub ledger: TrialLedger,
    pub records: Vec<JobRecord>,
}

impl PgJobStore {
    /// Read the entire authorized development population with exact revisions
    /// and states, within 30 seconds. Missing/corrupt/unauthorized members deny
    /// the whole snapshot. Cancellation rolls back; no writes or receipts occur.
    /// Report registration must recapture it under the writer lock.
    pub(crate) async fn trial_snapshot(&self, principal: &Actor) -> StoreResult<TrialSnapshot> {
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            let mut transaction = self.pool.begin().await?;
            let snapshot = capture_records(self, &mut transaction, principal).await?;
            transaction.commit().await?;
            Ok(snapshot)
        })
        .await
        .map_err(|_| StoreError::Unavailable("trial snapshot deadline"))?
    }
    /// Capture all development trials after authenticating the principal at the
    /// transport. Authorize every member; an inaccessible run or scan overflow
    /// denies the whole result. No trial, receipt or audit mutation occurs.
    /// Cancellation rolls back the read; the completion transaction must check
    /// this snapshot again because new work can be registered during execution.
    pub async fn trial_ledger(&self, principal: &Actor) -> StoreResult<TrialLedger> {
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            let mut transaction = self.pool.begin().await?;
            let ledger = capture(self, &mut transaction, principal).await?;
            transaction.commit().await?;
            Ok(ledger)
        })
        .await
        .map_err(|_| StoreError::Unavailable("trial ledger deadline"))?
    }
}

pub(super) async fn capture(
    store: &PgJobStore,
    transaction: &mut Transaction<'_, Postgres>,
    principal: &Actor,
) -> StoreResult<TrialLedger> {
    Ok(capture_records(store, transaction, principal).await?.ledger)
}

pub(super) async fn capture_records(
    store: &PgJobStore,
    transaction: &mut Transaction<'_, Postgres>,
    principal: &Actor,
) -> StoreResult<TrialSnapshot> {
    // LEFT JOIN also exposes a missing index row; filtering through the index
    // would silently omit corrupt/unmigrated research jobs from the denominator.
    let rows = sqlx::query(
        "SELECT j.*, t.factor_spec_id AS trial_factor, t.run_id AS trial_run,
         t.specification_sha256 AS trial_spec FROM jobs j
         LEFT JOIN factor_trials t ON t.job_id = j.job_id
         WHERE j.kind IN ($1, $2)
         ORDER BY j.job_id LIMIT 4097",
    )
    .bind(JobKind::FactorEvaluation as i32)
    .bind(JobKind::Backtest as i32)
    .fetch_all(&mut **transaction)
    .await?;
    if rows.is_empty() || rows.len() > 4096 {
        return Err(StoreError::Invalid("complete trial ledger bounds"));
    }
    let mut entries = Vec::with_capacity(rows.len());
    let mut records = Vec::with_capacity(rows.len());
    let mut attempts = 0_u64;
    for row in rows {
        let record: JobRecord = record_from_row(&row)?;
        store
            .admission
            .authorize_job_command("loop.factors.read_trials", principal, &record)?;
        let job = record
            .specification
            .as_ref()
            .ok_or(StoreError::Corrupt("trial job"))?;
        let factor =
            super::library::trials::factor_id(job)?.ok_or(StoreError::Corrupt("trial kind"))?;
        let run = &job
            .run_id
            .as_ref()
            .ok_or(StoreError::Corrupt("trial run"))?
            .value;
        let digest = Sha256::digest(encode_message(job)?);
        if row.try_get::<Option<String>, _>("trial_factor")?.as_deref() != Some(factor)
            || row.try_get::<Option<String>, _>("trial_run")?.as_deref() != Some(run.as_str())
            || row.try_get::<Option<Vec<u8>>, _>("trial_spec")?.as_deref()
                != Some(digest.as_slice())
        {
            return Err(StoreError::Corrupt("global trial index binding"));
        }
        let counted = record.attempt.max(1);
        attempts += u64::from(counted);
        if attempts > 65_536 {
            return Err(StoreError::Invalid("global trial attempt bounds"));
        }
        entries.push(TrialEntry {
            job_id: job
                .job_id
                .as_ref()
                .ok_or(StoreError::Corrupt("trial ID"))?
                .value
                .clone(),
            run_id: run.clone(),
            factor_spec_id: factor.to_owned(),
            specification_sha256: format!("sha256:{digest:x}"),
            attempts: counted,
        });
        records.push(record);
    }
    Ok(TrialSnapshot {
        ledger: TrialLedger {
            schema: "loop.global-trials/v1".to_owned(),
            entries,
        },
        records,
    })
}
