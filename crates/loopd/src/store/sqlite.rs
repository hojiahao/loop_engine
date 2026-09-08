use std::fs::{OpenOptions, TryLockError};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, SecondsFormat};
use loop_protocol::job::{validate_job_record, validate_job_specification};
use loop_protocol::wire::v1::{JobKind, JobRecord, JobSpecification, JobState, job_specification};
use prost::Message;
use prost_types::Timestamp;
use sha2::{Digest, Sha256};
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow, SqliteSynchronous,
};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};

use super::{
    AdmissionPolicy, Clock, CommandResult, DenySubmission, JobRepository, StoreError, StoreResult,
    SubmitJob, SystemClock, audit, validate_id,
};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations/sqlite");
const MAX_RECORD_BYTES: usize = 4 * 1024 * 1024;

/// Local-disk database configuration. Network filesystems are unsupported.
pub struct StoreOptions {
    /// Persistent SQLite file; in-memory databases are rejected.
    pub path: PathBuf,
    /// Immutable ledger identity verified on every open.
    pub audit_ledger_id: String,
    /// Server-owned time source, replaceable for deterministic testing.
    pub clock: Arc<dyn Clock>,
    /// Server-owned authorization and frozen-reference resolver.
    pub admission: Arc<dyn AdmissionPolicy>,
    /// Maximum time spent waiting for another process's startup migration.
    pub migration_lock_timeout: Duration,
}

impl StoreOptions {
    /// Configure a local file with real time and default-deny command policy.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            audit_ledger_id: "ledger.loopd".to_owned(),
            clock: Arc::new(SystemClock),
            admission: Arc::new(DenySubmission),
            migration_lock_timeout: Duration::from_secs(10),
        }
    }
}

/// Transactional job repository with private connection pooling and WAL storage.
/// Clones share a pool; separately opened instances coordinate through SQLite.
#[derive(Clone)]
pub struct SqliteJobStore {
    pub(super) pool: SqlitePool,
    pub(super) ledger_id: String,
    pub(super) clock: Arc<dyn Clock>,
    pub(super) admission: Arc<dyn AdmissionPolicy>,
}

impl SqliteJobStore {
    /// Open and migrate a local database with FULL synchronization and foreign keys.
    ///
    /// # Errors
    /// Rejects invalid paths, migration contention past the timeout, changed
    /// migration checksums, ledger mismatches, and filesystem/database failures.
    pub async fn open(options: StoreOptions) -> StoreResult<Self> {
        validate_id(&options.audit_ledger_id)?;
        let name = options
            .path
            .file_name()
            .ok_or(StoreError::Invalid("database path"))?;
        if name == ":memory:" {
            return Err(StoreError::Invalid("persistent file required"));
        }
        let absolute = if options.path.is_absolute() {
            options.path.clone()
        } else {
            std::env::current_dir()?.join(&options.path)
        };
        let parent = absolute
            .parent()
            .ok_or(StoreError::Invalid("database parent"))?;
        std::fs::create_dir_all(parent)?;
        let path = parent.canonicalize()?.join(name);
        let migration_path =
            path.with_file_name(format!("{}.migrate.lock", name.to_string_lossy()));
        let migration_lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(migration_path)?;
        let deadline = tokio::time::Instant::now()
            .checked_add(options.migration_lock_timeout)
            .ok_or(StoreError::Invalid("migration lock timeout"))?;
        loop {
            match migration_lock.try_lock() {
                Ok(()) => break,
                Err(TryLockError::Error(error)) => return Err(error.into()),
                Err(TryLockError::WouldBlock) => {
                    if tokio::time::Instant::now() >= deadline {
                        return Err(StoreError::Unavailable("migration lock timeout"));
                    }
                    // Cancellation drops this file; no blocking lock worker survives.
                    tokio::time::sleep_until(
                        deadline.min(tokio::time::Instant::now() + Duration::from_millis(10)),
                    )
                    .await;
                }
            }
        }

        let connect = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5))
            .pragma("trusted_schema", "OFF");
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(connect)
            .await?;
        MIGRATOR.run(&pool).await?;
        let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::query(
            "INSERT INTO store_metadata (singleton, audit_ledger_id, last_observed_at_ms)
             VALUES (1, ?, 0) ON CONFLICT(singleton) DO NOTHING",
        )
        .bind(&options.audit_ledger_id)
        .execute(&mut *transaction)
        .await?;
        let ledger: String =
            sqlx::query_scalar("SELECT audit_ledger_id FROM store_metadata WHERE singleton = 1")
                .fetch_one(&mut *transaction)
                .await?;
        if ledger != options.audit_ledger_id {
            return Err(StoreError::Corrupt("audit ledger identity"));
        }
        transaction.commit().await?;
        drop(migration_lock);
        Ok(Self {
            pool,
            ledger_id: ledger,
            clock: options.clock,
            admission: options.admission,
        })
    }

    /// Stop new operations and drain connections shared by this pool's clones.
    pub async fn close(&self) {
        self.pool.close().await;
    }

    /// Check SQLite durability settings and `quick_check`.
    ///
    /// # Errors
    /// Returns corruption for unsafe settings or failed integrity checks.
    pub async fn verify_configuration(&self) -> StoreResult<()> {
        let mut connection = self.pool.acquire().await?;
        let journal: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&mut *connection)
            .await?;
        let synchronous: i32 = sqlx::query_scalar("PRAGMA synchronous")
            .fetch_one(&mut *connection)
            .await?;
        let foreign_keys: i32 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&mut *connection)
            .await?;
        let check: String = sqlx::query_scalar("PRAGMA quick_check")
            .fetch_one(&mut *connection)
            .await?;
        if journal != "wal" || synchronous != 2 || foreign_keys != 1 || check != "ok" {
            return Err(StoreError::Corrupt("SQLite durability configuration"));
        }
        Ok(())
    }

    /// Read a verified page after a sequence cursor, with a maximum of 500 events.
    /// Start at zero to verify the chain from genesis; pages must remain linked.
    ///
    /// # Errors
    /// Rejects invalid/absent cursors, oversized pages, and corrupt events/chains.
    pub async fn audit_events(
        &self,
        after: u64,
        limit: u32,
    ) -> StoreResult<Vec<loop_core::audit::AuditEvent>> {
        audit::read_page(&self.pool, &self.ledger_id, after, limit).await
    }

    pub(super) async fn observe_clock(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
    ) -> StoreResult<i64> {
        let now = self.clock.now_millis()?;
        audit_timestamp(now)?;
        let previous: i64 = sqlx::query_scalar(
            "SELECT last_observed_at_ms FROM store_metadata WHERE singleton = 1",
        )
        .fetch_one(&mut **transaction)
        .await?;
        if now < previous {
            return Err(StoreError::ClockRegression);
        }
        Ok(now)
    }
}

impl JobRepository for SqliteJobStore {
    async fn submit_role(
        &self,
        principal: &loop_protocol::wire::v1::Actor,
        command: super::RoleCommand,
        metadata: super::SubmissionMetadata,
    ) -> StoreResult<super::RoleSubmissionResult> {
        super::submission::submit(self, principal, command, metadata).await
    }

    async fn submit(&self, command: SubmitJob) -> StoreResult<CommandResult> {
        validate_id(&command.request_id)?;
        let specification = &command.specification;
        let shape = validate_job_specification(specification)?;
        if shape.kind == JobKind::HoldoutBacktest {
            return Err(StoreError::AdmissionDenied);
        }
        self.admission.validate_submission(specification)?;
        if !specification
            .protocol_selection
            .as_ref()
            .expect("validated selection")
            .enabled_features
            .iter()
            .any(|value| value == "jobs.prelease-terminal.v1")
        {
            return Err(StoreError::AdmissionDenied);
        }
        let request_blob = encode_message(specification)?;
        let job_id = &specification
            .job_id
            .as_ref()
            .expect("validated job id")
            .value;
        let actor_id = &specification
            .submitted_by
            .as_ref()
            .expect("validated actor")
            .actor_id
            .as_ref()
            .expect("validated actor id")
            .value;
        let key = &specification
            .idempotency_key
            .as_ref()
            .expect("validated key")
            .value;

        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let now = self.observe_clock(&mut transaction).await?;
        let receipt = sqlx::query(
            "SELECT * FROM command_receipts
             WHERE actor_id = ? AND operation = 'loop.jobs.submit' AND idempotency_key = ?",
        )
        .bind(actor_id)
        .bind(key)
        .fetch_optional(&mut *transaction)
        .await?;
        if let Some(receipt) = receipt {
            let old = verified_blob(&receipt, "request_blob", "request_sha256")?;
            let old = JobSpecification::decode(old.as_slice())
                .map_err(|_| StoreError::Corrupt("command request envelope"))?;
            if old != *specification {
                return Err(StoreError::IdempotencyConflict);
            }
            let response = verified_blob(&receipt, "response_blob", "response_sha256")?;
            let job = decode_record(&response)?;
            if job.specification.as_ref() != Some(specification) {
                return Err(StoreError::Corrupt("command receipt binding"));
            }
            transaction.commit().await?;
            return Ok(CommandResult {
                job,
                replayed: true,
            });
        }

        let record = insert_job(&mut transaction, specification, now).await?;
        let response_blob = encode_message(&record)?;
        let response_digest = Sha256::digest(&response_blob).to_vec();
        audit::append_submission(&mut transaction, &self.ledger_id, &command, now).await?;
        sqlx::query(
            "INSERT INTO command_receipts (actor_id, operation, idempotency_key,
                request_id, job_id, request_blob, request_sha256, response_blob,
                response_sha256, committed_at_ms)
             VALUES (?, 'loop.jobs.submit', ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(actor_id)
        .bind(key)
        .bind(&command.request_id)
        .bind(job_id)
        .bind(&request_blob)
        .bind(Sha256::digest(&request_blob).to_vec())
        .bind(&response_blob)
        .bind(&response_digest)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        sqlx::query("UPDATE store_metadata SET last_observed_at_ms = ? WHERE singleton = 1")
            .bind(now)
            .execute(&mut *transaction)
            .await?;
        #[cfg(test)]
        super::crash_tests::fault_point("before_commit").await;
        transaction.commit().await?;
        #[cfg(test)]
        super::crash_tests::fault_point("after_commit").await;
        Ok(CommandResult {
            job: record,
            replayed: false,
        })
    }

    async fn get(&self, job_id: &str) -> StoreResult<Option<JobRecord>> {
        validate_id(job_id)?;
        sqlx::query("SELECT * FROM jobs WHERE job_id = ?")
            .bind(job_id)
            .fetch_optional(&self.pool)
            .await?
            .as_ref()
            .map(record_from_row)
            .transpose()
    }

    async fn mutate(
        &self,
        principal: &loop_protocol::wire::v1::Actor,
        command: super::JobMutation,
    ) -> StoreResult<CommandResult> {
        super::lifecycle::mutate(self, principal, command).await
    }
}

/// Insert the queued projection; the caller owns authorization, receipt, and audit.
pub(super) async fn insert_job(
    transaction: &mut Transaction<'_, Sqlite>,
    specification: &JobSpecification,
    now: i64,
) -> StoreResult<JobRecord> {
    validate_job_specification(specification)?;
    let job_id = &specification
        .job_id
        .as_ref()
        .expect("validated job id")
        .value;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM jobs WHERE job_id = ?)")
        .bind(job_id)
        .fetch_one(&mut **transaction)
        .await?;
    if exists {
        return Err(StoreError::DuplicateJob);
    }
    let submitted_at = timestamp_millis(
        specification.submitted_at.as_ref().expect("validated time"),
        true,
    )?;
    let deadline = deadline_millis(specification)?;
    if submitted_at > now || deadline <= now {
        return Err(StoreError::Invalid("submission time or elapsed budget"));
    }
    let record = JobRecord {
        specification: Some(specification.clone()),
        state: JobState::Queued as i32,
        revision: 1,
        attempt: 0,
        active_lease: None,
        outcome: None,
        updated_at: Some(timestamp(now)),
    };
    validate_job_record(&record)?;
    let response_blob = encode_message(&record)?;
    let response_digest = Sha256::digest(&response_blob).to_vec();
    sqlx::query(
        "INSERT INTO jobs (job_id, run_id, kind, state, revision, attempt,
            submitted_at_ms, updated_at_ms, deadline_ms, record_blob, record_sha256)
         VALUES (?, ?, ?, ?, 1, 0, ?, ?, ?, ?, ?)",
    )
    .bind(job_id)
    .bind(
        &specification
            .run_id
            .as_ref()
            .expect("validated run id")
            .value,
    )
    .bind(specification.kind)
    .bind(record.state)
    .bind(submitted_at)
    .bind(now)
    .bind(deadline)
    .bind(&response_blob)
    .bind(&response_digest)
    .execute(&mut **transaction)
    .await?;
    Ok(record)
}

pub(super) fn record_from_row(row: &SqliteRow) -> StoreResult<JobRecord> {
    let blob = verified_blob(row, "record_blob", "record_sha256")?;
    let record = decode_record(&blob)?;
    let specification = record
        .specification
        .as_ref()
        .expect("validated specification");
    let lease_id = record
        .active_lease
        .as_ref()
        .and_then(|lease| lease.lease_id.as_ref())
        .map(|id| id.value.as_str());
    let lease_owner = record
        .active_lease
        .as_ref()
        .and_then(|lease| lease.owner.as_ref())
        .and_then(|actor| actor.actor_id.as_ref())
        .map(|id| id.value.as_str());
    let lease_expiry = record
        .active_lease
        .as_ref()
        .and_then(|lease| lease.expires_at.as_ref())
        .map(|value| timestamp_millis(value, false))
        .transpose()?;
    if row.try_get::<String, _>("job_id")?
        != specification.job_id.as_ref().expect("validated id").value
        || row.try_get::<String, _>("run_id")?
            != specification.run_id.as_ref().expect("validated run").value
        || row.try_get::<i32, _>("kind")? != specification.kind
        || row.try_get::<i32, _>("state")? != record.state
        || row.try_get::<i64, _>("revision")?
            != i64::try_from(record.revision).map_err(|_| StoreError::Corrupt("revision range"))?
        || row.try_get::<i64, _>("attempt")? != i64::from(record.attempt)
        || row.try_get::<i64, _>("submitted_at_ms")?
            != timestamp_millis(
                specification.submitted_at.as_ref().expect("validated time"),
                true,
            )?
        || row.try_get::<i64, _>("updated_at_ms")?
            != timestamp_millis(record.updated_at.as_ref().expect("validated time"), false)?
        || row.try_get::<i64, _>("deadline_ms")? != deadline_millis(specification)?
        || row.try_get::<Option<&str>, _>("lease_id")? != lease_id
        || row.try_get::<Option<&str>, _>("lease_owner_id")? != lease_owner
        || row.try_get::<Option<i64>, _>("lease_expires_at_ms")? != lease_expiry
    {
        return Err(StoreError::Corrupt(
            "job projection disagrees with envelope",
        ));
    }
    Ok(record)
}

pub(super) fn encode_message(message: &impl Message) -> StoreResult<Vec<u8>> {
    if message.encoded_len() > MAX_RECORD_BYTES {
        return Err(StoreError::Invalid("envelope size"));
    }
    Ok(message.encode_to_vec())
}

pub(super) fn decode_record(bytes: &[u8]) -> StoreResult<JobRecord> {
    let record = JobRecord::decode(bytes).map_err(|_| StoreError::Corrupt("job envelope"))?;
    validate_job_record(&record).map_err(|_| StoreError::Corrupt("job contract"))?;
    Ok(record)
}

pub(super) fn verified_blob(
    row: &SqliteRow,
    blob_column: &str,
    digest_column: &str,
) -> StoreResult<Vec<u8>> {
    let blob: Vec<u8> = row.try_get(blob_column)?;
    let expected: Vec<u8> = row.try_get(digest_column)?;
    if blob.len() > MAX_RECORD_BYTES || Sha256::digest(&blob).as_slice() != expected {
        return Err(StoreError::Corrupt("envelope checksum"));
    }
    Ok(blob)
}

pub(super) fn budget(
    specification: &JobSpecification,
) -> StoreResult<&loop_protocol::wire::v1::JobBudget> {
    match specification
        .input
        .as_ref()
        .ok_or(StoreError::Invalid("input"))?
    {
        job_specification::Input::Discovery(value) => value.budget.as_ref(),
        job_specification::Input::FactorEvaluation(value) => value.budget.as_ref(),
        job_specification::Input::Backtest(value) => value.budget.as_ref(),
        job_specification::Input::Reconciliation(value) => value.budget.as_ref(),
        job_specification::Input::HoldoutBacktest(value) => value.budget.as_ref(),
        job_specification::Input::Artifact(value) => value.budget.as_ref(),
    }
    .ok_or(StoreError::Invalid("budget"))
}

pub(super) fn deadline_millis(specification: &JobSpecification) -> StoreResult<i64> {
    let budget = budget(specification)?;
    let duration = budget
        .maximum_wall_time
        .as_ref()
        .ok_or(StoreError::Invalid("wall time"))?;
    let submitted = specification
        .submitted_at
        .as_ref()
        .ok_or(StoreError::Invalid("submitted time"))?;
    let nanos = (i128::from(submitted.seconds) + i128::from(duration.seconds)) * 1_000_000_000
        + i128::from(submitted.nanos)
        + i128::from(duration.nanos);
    i64::try_from(nanos / 1_000_000).map_err(|_| StoreError::Invalid("deadline range"))
}

pub(super) fn timestamp_millis(value: &Timestamp, round_up: bool) -> StoreResult<i64> {
    if value.seconds < 0 || !(0..1_000_000_000).contains(&value.nanos) {
        return Err(StoreError::Invalid("timestamp range"));
    }
    let adjustment = if round_up { 999_999 } else { 0 };
    value
        .seconds
        .checked_mul(1_000)
        .and_then(|millis| millis.checked_add((i64::from(value.nanos) + adjustment) / 1_000_000))
        .ok_or(StoreError::Invalid("timestamp range"))
}

pub(super) fn timestamp(millis: i64) -> Timestamp {
    Timestamp {
        seconds: millis.div_euclid(1_000),
        nanos: (millis.rem_euclid(1_000) * 1_000_000) as i32,
    }
}

pub(super) fn audit_timestamp(millis: i64) -> StoreResult<String> {
    if !(0..=253_402_300_799_999).contains(&millis) {
        return Err(StoreError::Invalid("server timestamp range"));
    }
    DateTime::from_timestamp_millis(millis)
        .map(|value| value.to_rfc3339_opts(SecondsFormat::Nanos, true))
        .ok_or(StoreError::Invalid("audit timestamp"))
}
