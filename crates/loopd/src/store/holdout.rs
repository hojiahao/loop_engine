use std::future::Future;

use loop_core::audit::{
    AuditAction, AuditTarget, AuditTargetKind, Sha256Digest, state_transition_payload,
};
use loop_core::holdout::CanonicalHoldoutPeriod;
use loop_protocol::holdout::validate_holdout_period;
use loop_protocol::wire::v1::{
    Actor, CommandContext, HoldoutPeriod, HoldoutPeriodRecord, HoldoutPeriodState,
};
use prost::Message;
use sha2::{Digest, Sha256};
use sqlx::postgres::PgRow;
use sqlx::{Postgres, Row, Transaction};

use super::lifecycle::validate_context;
use super::postgres::{audit_timestamp, encode_message, timestamp_millis, verified_blob};
use super::{PgJobStore, StoreError, StoreResult, audit, validate_id};

const REGISTER: &str = "loop.holdout.register-period";
const READ: &str = "loop.holdout.read-period";

/// Server-owned authorization and immutable snapshot resolver for protected state.
/// Hooks must be bounded, side-effect-free checks against already resolved state;
/// network or artifact retrieval must complete before entering the repository.
pub trait HoldoutPolicy: Send + Sync {
    /// Authorize the authenticated principal for one operation and exact period.
    /// A matching actor label or an ordinary job policy is not sufficient authority.
    fn authorize_period(
        &self,
        operation: &str,
        principal: &Actor,
        period_id: &str,
    ) -> StoreResult<()>;

    /// Verify immutable manifest membership, coverage, and locked sample roles
    /// against server-owned references. Canonical syntax alone is not this proof.
    fn validate_registration(&self, period: &CanonicalHoldoutPeriod) -> StoreResult<()>;
}

/// Default policy until protected authorization and snapshot registries exist.
pub struct DenyHoldout;

impl HoldoutPolicy for DenyHoldout {
    fn authorize_period(&self, _: &str, _: &Actor, _: &str) -> StoreResult<()> {
        Err(StoreError::AdmissionDenied)
    }

    fn validate_registration(&self, _: &CanonicalHoldoutPeriod) -> StoreResult<()> {
        Err(StoreError::AdmissionDenied)
    }
}

/// Trusted internal registration input, not a remotely exposed enqueue command.
/// Canonical metadata contains only bounded immutable references, never datasets.
#[derive(Clone, PartialEq, Message)]
pub struct RegisterPeriod {
    /// Request metadata checked against the separately authenticated principal.
    #[prost(message, optional, tag = "1")]
    pub context: Option<CommandContext>,
    /// Exact wire projection of the independently resolved canonical period.
    #[prost(message, optional, tag = "2")]
    pub period: Option<HoldoutPeriod>,
    /// Server-resolved closed canonical document, reparsed at the store boundary.
    #[prost(bytes = "vec", tag = "3")]
    pub canonical_bytes: Vec<u8>,
}

impl RegisterPeriod {
    fn normalized(&self) -> Self {
        let mut normalized = self.clone();
        if let Some(context) = &mut normalized.context {
            context.request_id = None;
            context.requested_at = None;
        }
        normalized
    }
}

/// Immutable result at initial registration, including idempotent retries.
#[derive(Clone, Debug, PartialEq)]
pub struct PeriodRegistration {
    /// Original sealed revision, even if a later command advanced the aggregate.
    pub record: HoldoutPeriodRecord,
    /// True when no state or audit event was added. This never unlocks a period.
    pub replayed: bool,
}

/// Backend-independent protected period commands. SQL handles remain private.
pub trait HoldoutRepository: Send + Sync {
    /// Register a validated period at sealed revision one with an immutable
    /// receipt and audit event in one transaction. Principal comes from transport
    /// authentication; the independent holdout policy must resolve all references.
    /// Identical retries return the original receipt; another key cannot reset or
    /// replace the period. Cancellation before commit rolls back; an uncertain
    /// commit is resolved by retry. No approval, grant, or data access is issued.
    /// Returns validation, authority, duplicate/conflict, clock, or storage errors.
    fn register_period(
        &self,
        principal: &Actor,
        command: RegisterPeriod,
    ) -> impl Future<Output = StoreResult<PeriodRegistration>> + Send;

    /// Read and verify one period after independent protected-store authorization.
    /// Returns `None` only for an authorized absent identity. Malformed IDs,
    /// unauthorized principals, and corrupt records fail closed. Cancellation
    /// drops the read without changing state; the response conveys no capability.
    fn get_period(
        &self,
        principal: &Actor,
        period_id: &str,
    ) -> impl Future<Output = StoreResult<Option<HoldoutPeriodRecord>>> + Send;
}

impl HoldoutRepository for PgJobStore {
    async fn register_period(
        &self,
        principal: &Actor,
        command: RegisterPeriod,
    ) -> StoreResult<PeriodRegistration> {
        register(self, principal, command).await
    }

    async fn get_period(
        &self,
        principal: &Actor,
        period_id: &str,
    ) -> StoreResult<Option<HoldoutPeriodRecord>> {
        Sha256Digest::parse(period_id).map_err(|_| StoreError::Invalid("period identity"))?;
        self.holdout_policy
            .authorize_period(READ, principal, period_id)?;
        sqlx::query("SELECT * FROM holdout_periods WHERE period_id = $1")
            .bind(period_id)
            .fetch_optional(&self.pool)
            .await?
            .as_ref()
            .map(record_from_row)
            .transpose()
    }
}

async fn register(
    store: &PgJobStore,
    principal: &Actor,
    command: RegisterPeriod,
) -> StoreResult<PeriodRegistration> {
    let context = validate_context(command.context.as_ref(), principal)?;
    let period = command
        .period
        .as_ref()
        .ok_or(StoreError::Invalid("holdout period"))?;
    let canonical = validate_holdout_period(period, &command.canonical_bytes)?;
    let period_id = &canonical.holdout_period_id;
    store
        .holdout_policy
        .authorize_period(REGISTER, principal, period_id)?;
    let normalized = command.normalized();
    let request_blob = encode_message(&normalized)?;
    let actor_id = &principal.actor_id.as_ref().expect("validated actor").value;
    let key = &context
        .idempotency_key
        .as_ref()
        .expect("validated key")
        .value;

    let mut transaction = store.pool.begin().await?;
    let now = store.observe_clock(&mut transaction).await?;
    if timestamp_millis(context.requested_at.as_ref().expect("validated time"), true)? > now {
        return Err(StoreError::Invalid("future command time"));
    }
    store
        .holdout_policy
        .authorize_period(REGISTER, principal, period_id)?;
    store.holdout_policy.validate_registration(&canonical)?;
    let receipt = sqlx::query(
        "SELECT * FROM holdout_command_receipts
         WHERE actor_id = $1 AND operation = $2 AND idempotency_key = $3",
    )
    .bind(actor_id)
    .bind(REGISTER)
    .bind(key)
    .fetch_optional(&mut *transaction)
    .await?;
    if let Some(receipt) = receipt {
        let previous = verified_blob(&receipt, "request_blob", "request_sha256")?;
        let previous = RegisterPeriod::decode(previous.as_slice())
            .map_err(|_| StoreError::Corrupt("period receipt request"))?;
        if previous != normalized {
            return Err(StoreError::IdempotencyConflict);
        }
        let response = verified_blob(&receipt, "response_blob", "response_sha256")?;
        let record = decode_record(&response, &command.canonical_bytes)?;
        let committed_at: i64 = receipt.try_get("committed_at_ms")?;
        audit_timestamp(committed_at).map_err(|_| StoreError::Corrupt("period receipt time"))?;
        if record.period.as_ref() != Some(period)
            || record.state != HoldoutPeriodState::Sealed as i32
            || receipt.try_get::<String, _>("period_id")? != *period_id
            || committed_at > now
        {
            return Err(StoreError::Corrupt("period receipt binding"));
        }
        let row = sqlx::query("SELECT * FROM holdout_periods WHERE period_id = $1")
            .bind(period_id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or(StoreError::Corrupt("period receipt target absent"))?;
        if record_from_row(&row)?.period != record.period {
            return Err(StoreError::Corrupt("period receipt target changed"));
        }
        transaction.commit().await?;
        return Ok(PeriodRegistration {
            record,
            replayed: true,
        });
    }

    if let Some(row) = sqlx::query("SELECT * FROM holdout_periods WHERE period_id = $1")
        .bind(period_id)
        .fetch_optional(&mut *transaction)
        .await?
    {
        record_from_row(&row)?;
        return Err(StoreError::DuplicatePeriod);
    }
    let record = HoldoutPeriodRecord {
        period: Some(period.clone()),
        state: HoldoutPeriodState::Sealed as i32,
        revision: 1,
        ..Default::default()
    };
    let response_blob = encode_message(&record)?;
    sqlx::query(
        "INSERT INTO holdout_periods
         (period_id, canonical_sha256, canonical_blob, state, revision, record_blob, record_sha256)
         VALUES ($1, $2, $3, 1, 1, $4, $5)",
    )
    .bind(period_id)
    .bind(canonical.canonical_period_sha256.as_slice())
    .bind(&command.canonical_bytes)
    .bind(&response_blob)
    .bind(Sha256::digest(&response_blob).as_slice())
    .execute(&mut *transaction)
    .await?;
    audit::append(
        &mut transaction,
        &store.ledger_id,
        now,
        audit::EventInput {
            actor: principal,
            correlation_id: &context
                .correlation_id
                .as_ref()
                .expect("validated correlation")
                .value,
            causation_id: &context
                .causation_id
                .as_ref()
                .expect("validated causation")
                .value,
            action: AuditAction::StateTransitioned,
            target: AuditTarget {
                kind: AuditTargetKind::HoldoutPeriodId,
                value: period_id.clone(),
            },
            payload: state_transition_payload(
                "holdout.unregistered",
                "holdout.sealed",
                "locked period registered",
            )?,
        },
    )
    .await?;
    save_receipt(
        &mut transaction,
        context,
        period_id,
        &request_blob,
        &response_blob,
        now,
    )
    .await?;
    #[cfg(test)]
    super::crash_tests::fault_point("period_before_commit").await;
    transaction.commit().await?;
    #[cfg(test)]
    super::crash_tests::fault_point("period_after_commit").await;
    Ok(PeriodRegistration {
        record,
        replayed: false,
    })
}

async fn save_receipt(
    transaction: &mut Transaction<'_, Postgres>,
    context: &CommandContext,
    period_id: &str,
    request: &[u8],
    response: &[u8],
    now: i64,
) -> StoreResult<()> {
    sqlx::query(
        "INSERT INTO holdout_command_receipts
         (actor_id, operation, idempotency_key, request_id, period_id, request_blob,
          request_sha256, response_blob, response_sha256, committed_at_ms)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(
        &context
            .actor
            .as_ref()
            .expect("validated actor")
            .actor_id
            .as_ref()
            .expect("validated id")
            .value,
    )
    .bind(REGISTER)
    .bind(
        &context
            .idempotency_key
            .as_ref()
            .expect("validated key")
            .value,
    )
    .bind(
        &context
            .request_id
            .as_ref()
            .expect("validated request")
            .value,
    )
    .bind(period_id)
    .bind(request)
    .bind(Sha256::digest(request).as_slice())
    .bind(response)
    .bind(Sha256::digest(response).as_slice())
    .bind(now)
    .execute(&mut **transaction)
    .await?;
    sqlx::query("UPDATE store_metadata SET last_observed_at_ms = $1 WHERE singleton = 1")
        .bind(now)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

fn decode_record(bytes: &[u8], canonical_bytes: &[u8]) -> StoreResult<HoldoutPeriodRecord> {
    let record = HoldoutPeriodRecord::decode(bytes)
        .map_err(|_| StoreError::Corrupt("holdout period envelope"))?;
    validate_holdout_period(
        record
            .period
            .as_ref()
            .ok_or(StoreError::Corrupt("period absent"))?,
        canonical_bytes,
    )
    .map_err(|_| StoreError::Corrupt("period canonical identity"))?;
    let valid_state = match HoldoutPeriodState::try_from(record.state) {
        Ok(HoldoutPeriodState::Sealed) => {
            record.revision == 1
                && record.issued_grant_id.is_none()
                && record.grant_issued_at.is_none()
                && record.terminal_at.is_none()
        }
        Ok(HoldoutPeriodState::GrantIssued) => {
            record.revision == 2
                && record.issued_grant_id.is_some()
                && record.grant_issued_at.is_some()
                && record.terminal_at.is_none()
        }
        Ok(HoldoutPeriodState::Consumed | HoldoutPeriodState::Closed) => {
            record.revision == 3
                && record.issued_grant_id.is_some()
                && record.grant_issued_at.is_some()
                && record.terminal_at.is_some()
        }
        _ => false,
    };
    if !valid_state {
        return Err(StoreError::Corrupt("period lifecycle"));
    }
    if let Some(grant_id) = &record.issued_grant_id {
        validate_id(&grant_id.value).map_err(|_| StoreError::Corrupt("period grant id"))?;
    }
    let issued = record_time(record.grant_issued_at.as_ref())?;
    let terminal = record_time(record.terminal_at.as_ref())?;
    if matches!((issued, terminal), (Some(start), Some(end)) if end < start) {
        return Err(StoreError::Corrupt("period time order"));
    }
    Ok(record)
}

fn record_time(value: Option<&prost_types::Timestamp>) -> StoreResult<Option<i64>> {
    value
        .map(|value| {
            if value.nanos % 1_000_000 != 0 {
                return Err(StoreError::Corrupt("period timestamp precision"));
            }
            let millis = timestamp_millis(value, false)
                .map_err(|_| StoreError::Corrupt("period timestamp"))?;
            audit_timestamp(millis).map_err(|_| StoreError::Corrupt("period timestamp range"))?;
            Ok(millis)
        })
        .transpose()
}

fn record_from_row(row: &PgRow) -> StoreResult<HoldoutPeriodRecord> {
    let bytes = verified_blob(row, "record_blob", "record_sha256")?;
    let canonical: Vec<u8> = row.try_get("canonical_blob")?;
    let record = decode_record(&bytes, &canonical)?;
    let period = record.period.as_ref().expect("validated period");
    if row.try_get::<String, _>("period_id")?
        != period
            .holdout_period_id
            .as_ref()
            .expect("validated id")
            .value
        || row.try_get::<Vec<u8>, _>("canonical_sha256")?
            != period
                .canonical_period_sha256
                .as_ref()
                .expect("validated digest")
                .value
        || row.try_get::<i32, _>("state")? != record.state
        || row.try_get::<i64, _>("revision")? != record.revision as i64
        || row.try_get::<Option<String>, _>("issued_grant_id")?
            != record.issued_grant_id.as_ref().map(|id| id.value.clone())
        || row.try_get::<Option<i64>, _>("grant_issued_at_ms")?
            != record_time(record.grant_issued_at.as_ref())?
        || row.try_get::<Option<i64>, _>("terminal_at_ms")?
            != record_time(record.terminal_at.as_ref())?
    {
        return Err(StoreError::Corrupt(
            "period projection disagrees with envelope",
        ));
    }
    Ok(record)
}
