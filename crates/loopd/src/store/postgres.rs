use std::collections::HashSet;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use chrono::{DateTime, SecondsFormat};
use loop_protocol::job::{validate_job_record, validate_job_specification};
use loop_protocol::wire::v1::{JobKind, JobRecord, JobSpecification, JobState, job_specification};
use prost::Message;
use prost_types::Timestamp;
use sha2::{Digest, Sha256};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgRow, PgSslMode};
use sqlx::{ConnectOptions, Connection, PgConnection, PgPool, Postgres, Row, Transaction};

use super::{
    AdmissionPolicy, BacktestPolicy, Clock, CommandResult, DenyBacktest, DenyHoldout,
    DenySubmission, HoldoutPolicy, JobRepository, StoreError, StoreResult, SubmitJob, SystemClock,
    audit, validate_id,
};

static MIGRATOR: LazyLock<sqlx::migrate::Migrator> = LazyLock::new(|| {
    let mut migrator = sqlx::migrate!("../../migrations/postgres");
    // Our schema-scoped lock covers both namespace creation and every migration.
    migrator.set_locking(false);
    migrator
});
const MAX_RECORD_BYTES: usize = 4 * 1024 * 1024;

/// PostgreSQL configuration. Credentials are private and never implement Debug.
pub struct StoreOptions {
    connect: PgConnectOptions,
    /// Trusted metadata namespace, never caller supplied.
    pub schema: String,
    /// Explicit deployment-only migration mode; requires schema-owner privileges.
    /// Runtime defaults to read/verify and never silently applies DDL.
    pub apply_migrations: bool,
    /// Immutable ledger identity verified on every open.
    pub audit_ledger_id: String,
    /// Server-owned time source, replaceable for deterministic testing.
    pub clock: Arc<dyn Clock>,
    /// Server-owned authorization and frozen-reference resolver.
    pub admission: Arc<dyn AdmissionPolicy>,
    /// Independent protected-store policy; ordinary job admission grants no access.
    pub holdout_policy: Arc<dyn HoldoutPolicy>,
    /// Independent result-manifest and current-context resolver; defaults to deny.
    pub backtest_policy: Arc<dyn BacktestPolicy>,
    /// Total migration deadline, including waiting for the database advisory lock.
    pub migration_lock_timeout: Duration,
}

impl StoreOptions {
    /// Parse a URL requiring TLS, with real time and default-deny command policy.
    /// Invalid URLs and SSL downgrade modes return redacted validation errors.
    pub fn new(url: &str) -> StoreResult<Self> {
        let parsed =
            url::Url::parse(url).map_err(|_| StoreError::Invalid("PostgreSQL connection URL"))?;
        let mut parameters = HashSet::new();
        if !matches!(parsed.scheme(), "postgres" | "postgresql")
            || parsed.host_str().is_none()
            || parsed.username().is_empty()
            || parsed.path().len() <= 1
            || parsed.fragment().is_some()
            || parsed.query_pairs().any(|(key, _)| {
                !matches!(
                    key.as_ref(),
                    "sslmode" | "sslrootcert" | "sslcert" | "sslkey"
                ) || !parameters.insert(key.into_owned())
            })
            || !parameters.contains("sslmode")
        {
            return Err(StoreError::Invalid("PostgreSQL connection URL"));
        }
        // SQLx logs unknown query parameters, so reject them before parsing secrets.
        let connect = PgConnectOptions::from_url(&parsed)
            .map_err(|_| StoreError::Invalid("PostgreSQL connection URL"))?;
        if !matches!(
            connect.get_ssl_mode(),
            PgSslMode::Require | PgSslMode::VerifyCa | PgSslMode::VerifyFull
        ) || connect.get_database().is_none_or(str::is_empty)
            || connect.get_socket().is_some()
            || connect.get_host().starts_with('/')
        {
            return Err(StoreError::Invalid("explicit database and required TLS"));
        }
        Ok(Self {
            connect: connect
                .application_name("loopd")
                .disable_statement_logging(),
            schema: "public".to_owned(),
            apply_migrations: false,
            audit_ledger_id: "ledger.loopd".to_owned(),
            clock: Arc::new(SystemClock),
            admission: Arc::new(DenySubmission),
            holdout_policy: Arc::new(DenyHoldout),
            backtest_policy: Arc::new(DenyBacktest),
            migration_lock_timeout: Duration::from_secs(10),
        })
    }
}

/// Transactional PostgreSQL repository. A locked ledger row serializes each
/// state/receipt/audit commit across independent connections and OS processes.
#[derive(Clone)]
pub struct PgJobStore {
    pub(super) pool: PgPool,
    pub(super) ledger_id: String,
    pub(super) clock: Arc<dyn Clock>,
    pub(super) admission: Arc<dyn AdmissionPolicy>,
    pub(super) holdout_policy: Arc<dyn HoldoutPolicy>,
    pub(super) backtest_policy: Arc<dyn BacktestPolicy>,
}

impl PgJobStore {
    /// Connect over TLS, optionally migrate with deployment authority, and verify.
    ///
    /// # Errors
    /// Rejects invalid settings, bounded migration contention, changed checksums,
    /// unsafe sessions, unresolved migration namespaces, and storage failures.
    /// Cancellation closes the dedicated migration connection; advisory locks
    /// never return to the runtime pool.
    pub async fn open(options: StoreOptions) -> StoreResult<Self> {
        validate_id(&options.audit_ledger_id)?;
        if options.schema.is_empty()
            || options.schema.len() > 63
            || !options
                .schema
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            || options.schema.as_bytes()[0].is_ascii_digit()
            || options.migration_lock_timeout.is_zero()
            || options.migration_lock_timeout > Duration::from_secs(60)
        {
            return Err(StoreError::Invalid(
                "database namespace or migration timeout",
            ));
        }
        let search_path = format!("\"{}\",pg_catalog", options.schema);
        let connect = options.connect.options([
            ("timezone", "UTC"),
            ("statement_timeout", "30000"),
            ("lock_timeout", "5000"),
            ("idle_in_transaction_session_timeout", "15000"),
            ("synchronous_commit", "on"),
        ]);
        if options.apply_migrations {
            // Resolve the target only after its creation under the namespace lock.
            let migration_connect = connect.clone().options([("search_path", "pg_catalog")]);
            let mut connection = tokio::time::timeout(
                Duration::from_secs(10),
                PgConnection::connect_with(&migration_connect),
            )
            .await
            .map_err(|_| StoreError::Unavailable("migration connection timeout"))??;
            let migration = async {
                sqlx::query("SELECT pg_advisory_lock(hashtextextended($1, 0))")
                    .bind(format!("loop.migrations.{}", options.schema))
                    .execute(&mut connection)
                    .await?;
                sqlx::query(&format!(
                    "CREATE SCHEMA IF NOT EXISTS \"{}\"",
                    options.schema
                ))
                .execute(&mut connection)
                .await?;
                sqlx::query("SELECT pg_catalog.set_config('search_path', $1, false)")
                    .bind(&search_path)
                    .execute(&mut connection)
                    .await?;
                let resolved: Option<String> =
                    sqlx::query_scalar("SELECT pg_catalog.current_schema()::text")
                        .fetch_one(&mut connection)
                        .await?;
                if resolved.as_deref() != Some(options.schema.as_str()) {
                    return Err(StoreError::Corrupt("migration namespace resolution"));
                }
                MIGRATOR.run(&mut connection).await?;
                sqlx::query(
                    "INSERT INTO store_metadata (singleton, audit_ledger_id, last_observed_at_ms)
                     VALUES (1, $1, 0) ON CONFLICT (singleton) DO NOTHING",
                )
                .bind(&options.audit_ledger_id)
                .execute(&mut connection)
                .await?;
                Ok::<_, StoreError>(())
            };
            let result = tokio::time::timeout(options.migration_lock_timeout, migration).await;
            let closed = tokio::time::timeout(Duration::from_secs(2), connection.close()).await;
            result.map_err(|_| StoreError::Unavailable("migration timeout"))??;
            closed.map_err(|_| StoreError::Unavailable("migration close timeout"))??;
        }
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(10))
            .after_connect(|connection, _| Box::pin(verify_session(connection)))
            .connect_with(connect.options([("search_path", search_path.as_str())]))
            .await?;
        verify_migrations(&pool).await?;
        let ledger: String =
            sqlx::query_scalar("SELECT audit_ledger_id FROM store_metadata WHERE singleton = 1")
                .fetch_one(&pool)
                .await?;
        if ledger != options.audit_ledger_id {
            return Err(StoreError::Corrupt("audit ledger identity"));
        }
        Ok(Self {
            pool,
            ledger_id: ledger,
            clock: options.clock,
            admission: options.admission,
            holdout_policy: options.holdout_policy,
            backtest_policy: options.backtest_policy,
        })
    }

    /// Stop new operations and drain connections shared by this pool's clones.
    pub async fn close(&self) {
        self.pool.close().await;
    }

    /// Verify TLS/session settings, migration checksums, and the ledger identity.
    ///
    /// # Errors
    /// Returns errors for unsafe sessions, missing schemas, and corrupt identity.
    pub async fn verify_configuration(&self) -> StoreResult<()> {
        let mut connection = self.pool.acquire().await?;
        verify_session(&mut connection).await?;
        let ledger: String =
            sqlx::query_scalar("SELECT audit_ledger_id FROM store_metadata WHERE singleton = 1")
                .fetch_one(&mut *connection)
                .await?;
        if ledger != self.ledger_id {
            return Err(StoreError::Corrupt("audit ledger identity"));
        }
        drop(connection);
        verify_migrations(&self.pool).await?;
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
        transaction: &mut Transaction<'_, Postgres>,
    ) -> StoreResult<i64> {
        let previous: i64 = sqlx::query_scalar(
            "SELECT last_observed_at_ms FROM store_metadata WHERE singleton = 1 FOR UPDATE",
        )
        .fetch_one(&mut **transaction)
        .await?;
        let now = self.clock.now_millis()?;
        audit_timestamp(now)?;
        if now < previous {
            return Err(StoreError::ClockRegression);
        }
        Ok(now)
    }
}

async fn verify_session(connection: &mut PgConnection) -> Result<(), sqlx::Error> {
    let valid: bool = sqlx::query_scalar(
        "SELECT COALESCE((SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()), false)
         AND current_setting('synchronous_commit') = 'on'
         AND current_setting('fsync') = 'on'
         AND current_setting('full_page_writes') = 'on'
         AND current_setting('transaction_isolation') = 'read committed'
         AND current_setting('statement_timeout') = '30s'
         AND current_setting('lock_timeout') = '5s'
         AND current_setting('idle_in_transaction_session_timeout') = '15s'
         AND current_setting('TimeZone') = 'UTC'",
    )
    .fetch_one(connection)
    .await?;
    if !valid {
        return Err(sqlx::Error::Protocol(
            "unsafe PostgreSQL session".to_owned(),
        ));
    }
    Ok(())
}

async fn verify_migrations(pool: &PgPool) -> StoreResult<()> {
    let rows = sqlx::query(
        "SELECT version, checksum, success FROM _sqlx_migrations ORDER BY version LIMIT 1025",
    )
    .fetch_all(pool)
    .await?;
    if rows.len() != MIGRATOR.iter().count() {
        return Err(StoreError::Corrupt("database migration count"));
    }
    for (row, migration) in rows.iter().zip(MIGRATOR.iter()) {
        if row.try_get::<i64, _>("version")? != migration.version
            || row.try_get::<Vec<u8>, _>("checksum")? != migration.checksum.as_ref()
            || !row.try_get::<bool, _>("success")?
        {
            return Err(StoreError::Corrupt("database migration checksum"));
        }
    }
    Ok(())
}

impl JobRepository for PgJobStore {
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

        let mut transaction = self.pool.begin().await?;
        let now = self.observe_clock(&mut transaction).await?;
        let receipt = sqlx::query(
            "SELECT * FROM command_receipts
             WHERE actor_id = $1 AND operation = 'loop.jobs.submit' AND idempotency_key = $2",
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
             VALUES ($1, 'loop.jobs.submit', $2, $3, $4, $5, $6, $7, $8, $9)",
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
        sqlx::query("UPDATE store_metadata SET last_observed_at_ms = $1 WHERE singleton = 1")
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
        sqlx::query("SELECT * FROM jobs WHERE job_id = $1")
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
    transaction: &mut Transaction<'_, Postgres>,
    specification: &JobSpecification,
    now: i64,
) -> StoreResult<JobRecord> {
    validate_job_specification(specification)?;
    let job_id = &specification
        .job_id
        .as_ref()
        .expect("validated job id")
        .value;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM jobs WHERE job_id = $1)")
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
         VALUES ($1, $2, $3, $4, 1, 0, $5, $6, $7, $8, $9)",
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

pub(super) fn record_from_row(row: &PgRow) -> StoreResult<JobRecord> {
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
    row: &PgRow,
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
