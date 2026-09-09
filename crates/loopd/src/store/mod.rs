//! Internal durable commands. Transport authorization is not delegated to DTOs.
#![deny(missing_docs)]

mod approval;
mod audit;
#[cfg(test)]
mod crash_tests;
mod grant;
mod holdout;
mod lifecycle;
mod postgres;
mod submission;

use std::future::Future;
use std::time::{SystemTime, UNIX_EPOCH};

use loop_protocol::wire::v1::{Actor, JobRecord, JobSpecification};
use thiserror::Error;

pub use approval::ApprovalResult;
pub use grant::{CloseGrant, GrantClosure, GrantResult, ResolvedFreeze};
pub use holdout::{
    DenyHoldout, HoldoutPolicy, HoldoutRepository, PeriodRegistration, RegisterPeriod,
};
pub use lifecycle::{JobMutation, RecoveryCommand};
pub use postgres::{PgJobStore, StoreOptions};
pub use submission::{RoleCommand, RoleJobHandle, RoleSubmissionResult, SubmissionMetadata};

/// Fail-closed command errors; none represent rejection of a research factor.
#[derive(Debug, Error)]
pub enum StoreError {
    /// The caller supplied an invalid command envelope.
    #[error("invalid storage command: {0}")]
    Invalid(&'static str),
    /// Server-owned reference or authority policy denied the command.
    #[error("storage command is not authorized")]
    AdmissionDenied,
    /// One principal reused a command key with different semantic content.
    #[error("idempotency key was already used for a different command")]
    IdempotencyConflict,
    /// Another command already registered this job identity.
    #[error("job identity already exists")]
    DuplicateJob,
    /// The canonical locked period already exists, possibly in a terminal state.
    #[error("holdout period already exists and cannot be registered again")]
    DuplicatePeriod,
    /// No persistent aggregate exists for the requested identity.
    #[error("persistent aggregate does not exist")]
    NotFound,
    /// Another accepted command advanced the aggregate.
    #[error("expected revision does not match persistent state")]
    RevisionConflict,
    /// The lifecycle does not permit this operation at the current state.
    #[error("command is not valid in the current aggregate state")]
    InvalidTransition,
    /// The worker no longer owns a valid lease.
    #[error("lease is absent, expired, or owned by another principal")]
    LeaseFenced,
    /// Stored envelopes, projections, or evidence do not match.
    #[error("persistent state is corrupt: {0}")]
    Corrupt(&'static str),
    /// Wall time predates a previously committed command.
    #[error("server clock moved backwards")]
    ClockRegression,
    /// An infrastructure resource was unavailable within its bounded wait.
    #[error("storage operation unavailable: {0}")]
    Unavailable(&'static str),
    /// The typed job contract rejected an input or outcome.
    #[error("job wire contract is invalid: {0}")]
    Job(#[from] loop_protocol::job::JobValidationError),
    /// Canonical holdout content or its typed identity is invalid.
    #[error("holdout contract is invalid: {0}")]
    Holdout(#[from] loop_core::holdout::HoldoutValidationError),
    /// Canonical audit verification failed.
    #[error("audit contract is invalid: {0}")]
    Audit(#[from] loop_core::audit::AuditError),
    /// PostgreSQL rejected a statement or could not acquire a connection.
    #[error("database operation failed: {0}")]
    Database(#[from] sqlx::Error),
    /// A migration failed or a stored migration checksum changed.
    #[error("database migration failed: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    /// A local filesystem operation failed.
    #[error("storage file operation failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Result returned by durable command operations.
pub type StoreResult<T> = Result<T, StoreError>;

/// Injectable server clock; sampled only after acquiring a write transaction.
pub trait Clock: Send + Sync {
    /// Return milliseconds since Unix epoch, or fail if time is unavailable.
    fn now_millis(&self) -> StoreResult<i64>;
}

/// Production wall-clock implementation; monotonicity is checked in the store.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_millis(&self) -> StoreResult<i64> {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| StoreError::ClockRegression)?;
        i64::try_from(elapsed.as_millis()).map_err(|_| StoreError::Invalid("clock range"))
    }
}

/// Server-owned preflight hook. Implementations must resolve references and
/// prove pinned protocol availability; wire-shape validation alone is not enough.
pub trait AdmissionPolicy: Send + Sync {
    /// Approve frozen references and protocol availability or deny admission.
    fn validate_submission(&self, specification: &JobSpecification) -> StoreResult<()>;

    /// Authorize a transport-authenticated principal for this operation and job.
    /// The default denies all lifecycle commands, including recovery.
    fn authorize_job_command(
        &self,
        _operation: &str,
        _actor: &Actor,
        _record: &JobRecord,
    ) -> StoreResult<()> {
        Err(StoreError::AdmissionDenied)
    }
}

/// Safe default until authenticated role handlers can resolve all references.
pub struct DenySubmission;

impl AdmissionPolicy for DenySubmission {
    fn validate_submission(&self, _: &JobSpecification) -> StoreResult<()> {
        Err(StoreError::AdmissionDenied)
    }
}

/// Trusted role-handler submission, not an authorization-bearing wire request.
#[derive(Clone, Debug)]
pub struct SubmitJob {
    /// Transport request identifier, excluded from semantic retry identity.
    pub request_id: String,
    /// Immutable, typed job input including frozen protocol and budget.
    pub specification: JobSpecification,
}

/// Original response to an accepted command, including durable retry results.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandResult {
    /// Record at the command's commit, which may be older than current state.
    pub job: JobRecord,
    /// True when no new mutation or event was committed. This is not a dispatch.
    pub replayed: bool,
}

/// Backend-independent command boundary; SQL and pool handles remain private.
pub trait JobRepository: Send + Sync {
    /// Validate and atomically queue a narrow role request. `principal` must be
    /// transport-authenticated by the caller; metadata is server-resolved. The
    /// admission policy must resolve references and pinned protocol availability.
    /// No holdout command is representable here. Replays retain the original
    /// receipt and protocol selection and do not authorize redispatch.
    /// Cancellation before commit rolls back; retry resolves uncertain commits.
    /// Returns validation, admission, identity, clock, or storage errors.
    fn submit_role(
        &self,
        principal: &Actor,
        command: RoleCommand,
        metadata: SubmissionMetadata,
    ) -> impl Future<Output = StoreResult<RoleSubmissionResult>> + Send;

    /// Queue one authorized job atomically with its receipt and audit event.
    /// Returns validation, admission, identity, clock, or storage errors.
    fn submit(&self, command: SubmitJob)
    -> impl Future<Output = StoreResult<CommandResult>> + Send;

    /// Read and verify a job; `None` means absent, corruption is an error.
    fn get(&self, job_id: &str) -> impl Future<Output = StoreResult<Option<JobRecord>>> + Send;

    /// Apply a revision-fenced command. `principal` must come from transport
    /// authentication, never from the request body. Unmatched actors are denied.
    fn mutate(
        &self,
        principal: &Actor,
        command: JobMutation,
    ) -> impl Future<Output = StoreResult<CommandResult>> + Send;
}

fn validate_id(value: &str) -> StoreResult<()> {
    if value.is_empty()
        || value.len() > 128
        || !value.as_bytes()[0].is_ascii_alphanumeric()
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_alphanumeric() && !b"._:-".contains(&byte))
    {
        return Err(StoreError::Invalid("identifier"));
    }
    Ok(())
}
