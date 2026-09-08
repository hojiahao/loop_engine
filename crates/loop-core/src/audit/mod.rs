//! Canonical, append-only audit identities.
//!
//! Audit payloads and events deliberately use dedicated canonical writers.
//! Protobuf messages are transport DTOs and are never hash inputs.

mod canonical;

use std::fmt;

use thiserror::Error;

pub use canonical::{
    audit_event_sha256, audit_payload_sha256, canonical_audit_event_bytes,
    canonicalize_audit_payload, state_transition_payload, verify_audit_chain, verify_audit_event,
    verify_audit_payload,
};

pub const MAX_AUDIT_PAYLOAD_BYTES: usize = 256 * 1_024;
pub const MAX_AUDIT_TEXT_BYTES: usize = 4_096;
pub const MAX_AUDIT_ID_BYTES: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sha256Digest([u8; 32]);

impl Sha256Digest {
    pub const ZERO: Self = Self([0; 32]);

    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn parse(value: &str) -> Result<Self, AuditError> {
        let Some(hex) = value.strip_prefix("sha256:") else {
            return Err(AuditError::new(
                AuditErrorCode::InvalidDigest,
                "sha256",
                "digest must use the sha256: prefix",
            ));
        };
        if hex.len() != 64 {
            return Err(AuditError::new(
                AuditErrorCode::InvalidDigest,
                "sha256",
                "digest must contain exactly 64 lowercase hexadecimal characters",
            ));
        }
        let mut bytes = [0_u8; 32];
        for (index, pair) in hex.as_bytes().chunks_exact(2).enumerate() {
            bytes[index] = (decode_hex(pair[0])? << 4) | decode_hex(pair[1])?;
        }
        Ok(Self(bytes))
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("sha256:")?;
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

fn decode_hex(byte: u8) -> Result<u8, AuditError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(AuditError::new(
            AuditErrorCode::InvalidDigest,
            "sha256",
            "digest must contain lowercase hexadecimal characters only",
        )),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorKind {
    Human,
    Service,
    Agent,
    Scheduler,
}

impl ActorKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Service => "service",
            Self::Agent => "agent",
            Self::Scheduler => "scheduler",
        }
    }
}

impl TryFrom<&str> for ActorKind {
    type Error = AuditError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "human" => Ok(Self::Human),
            "service" => Ok(Self::Service),
            "agent" => Ok(Self::Agent),
            "scheduler" => Ok(Self::Scheduler),
            _ => Err(AuditError::new(
                AuditErrorCode::InvalidEnum,
                "actor.kind",
                "unknown actor kind",
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditAction {
    CommandAccepted,
    StateTransitioned,
    FactorAdmitted,
    FactorRejected,
    OverrideAuthorized,
    ReadmissionRequested,
    ReadmissionDecided,
    HoldoutGrantIssued,
    HoldoutGrantConsumed,
    ArtifactExported,
    HoldoutApprovalRecorded,
}

impl AuditAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CommandAccepted => "command_accepted",
            Self::StateTransitioned => "state_transitioned",
            Self::FactorAdmitted => "factor_admitted",
            Self::FactorRejected => "factor_rejected",
            Self::OverrideAuthorized => "override_authorized",
            Self::ReadmissionRequested => "readmission_requested",
            Self::ReadmissionDecided => "readmission_decided",
            Self::HoldoutGrantIssued => "holdout_grant_issued",
            Self::HoldoutGrantConsumed => "holdout_grant_consumed",
            Self::ArtifactExported => "artifact_exported",
            Self::HoldoutApprovalRecorded => "holdout_approval_recorded",
        }
    }
}

impl TryFrom<&str> for AuditAction {
    type Error = AuditError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "command_accepted" => Ok(Self::CommandAccepted),
            "state_transitioned" => Ok(Self::StateTransitioned),
            "factor_admitted" => Ok(Self::FactorAdmitted),
            "factor_rejected" => Ok(Self::FactorRejected),
            "override_authorized" => Ok(Self::OverrideAuthorized),
            "readmission_requested" => Ok(Self::ReadmissionRequested),
            "readmission_decided" => Ok(Self::ReadmissionDecided),
            "holdout_grant_issued" => Ok(Self::HoldoutGrantIssued),
            "holdout_grant_consumed" => Ok(Self::HoldoutGrantConsumed),
            "artifact_exported" => Ok(Self::ArtifactExported),
            "holdout_approval_recorded" => Ok(Self::HoldoutApprovalRecorded),
            _ => Err(AuditError::new(
                AuditErrorCode::InvalidEnum,
                "action",
                "unknown audit action",
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditTargetKind {
    RunId,
    JobId,
    FactorSpecId,
    BacktestId,
    SnapshotId,
    HoldoutGrantId,
    ArtifactId,
    HoldoutApprovalRecordId,
}

impl AuditTargetKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RunId => "run_id",
            Self::JobId => "job_id",
            Self::FactorSpecId => "factor_spec_id",
            Self::BacktestId => "backtest_id",
            Self::SnapshotId => "snapshot_id",
            Self::HoldoutGrantId => "holdout_grant_id",
            Self::ArtifactId => "artifact_id",
            Self::HoldoutApprovalRecordId => "holdout_approval_record_id",
        }
    }
}

impl TryFrom<&str> for AuditTargetKind {
    type Error = AuditError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "run_id" => Ok(Self::RunId),
            "job_id" => Ok(Self::JobId),
            "factor_spec_id" => Ok(Self::FactorSpecId),
            "backtest_id" => Ok(Self::BacktestId),
            "snapshot_id" => Ok(Self::SnapshotId),
            "holdout_grant_id" => Ok(Self::HoldoutGrantId),
            "artifact_id" => Ok(Self::ArtifactId),
            "holdout_approval_record_id" => Ok(Self::HoldoutApprovalRecordId),
            _ => Err(AuditError::new(
                AuditErrorCode::InvalidTarget,
                "target.kind",
                "unknown audit target kind",
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditActor {
    pub actor_id: String,
    pub kind: ActorKind,
    pub display_name: String,
    pub authenticated_subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditTarget {
    pub kind: AuditTargetKind,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditPayload {
    pub schema_name: String,
    pub schema_version: u32,
    pub canonical_bytes: Vec<u8>,
    pub payload_sha256: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEvent {
    pub audit_ledger_id: String,
    pub sequence: u64,
    pub previous_event_sha256: Sha256Digest,
    pub audit_event_id: String,
    pub occurred_at: String,
    pub correlation_id: String,
    pub causation_id: String,
    pub actor: AuditActor,
    pub action: AuditAction,
    pub target: AuditTarget,
    pub payload: AuditPayload,
    pub event_sha256: Sha256Digest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditErrorCode {
    InvalidSchema,
    UnsupportedSchema,
    NonCanonicalPayload,
    PayloadDigestMismatch,
    InvalidDigest,
    InvalidIdentifier,
    InvalidText,
    InvalidTimestamp,
    InvalidSequence,
    InvalidEnum,
    InvalidTarget,
    ActionPayloadMismatch,
    ActionTargetMismatch,
    EventDigestMismatch,
    LedgerMismatch,
    ChainMismatch,
    DuplicateEventId,
    SizeLimit,
}

impl AuditErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidSchema => "invalid_schema",
            Self::UnsupportedSchema => "unsupported_schema",
            Self::NonCanonicalPayload => "non_canonical_payload",
            Self::PayloadDigestMismatch => "payload_digest_mismatch",
            Self::InvalidDigest => "invalid_digest",
            Self::InvalidIdentifier => "invalid_identifier",
            Self::InvalidText => "invalid_text",
            Self::InvalidTimestamp => "invalid_timestamp",
            Self::InvalidSequence => "invalid_sequence",
            Self::InvalidEnum => "invalid_enum",
            Self::InvalidTarget => "invalid_target",
            Self::ActionPayloadMismatch => "action_payload_mismatch",
            Self::ActionTargetMismatch => "action_target_mismatch",
            Self::EventDigestMismatch => "event_digest_mismatch",
            Self::LedgerMismatch => "ledger_mismatch",
            Self::ChainMismatch => "chain_mismatch",
            Self::DuplicateEventId => "duplicate_event_id",
            Self::SizeLimit => "size_limit",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{field} failed audit validation ({code:?}): {detail}")]
pub struct AuditError {
    code: AuditErrorCode,
    field: &'static str,
    detail: String,
}

impl AuditError {
    pub(crate) fn new(
        code: AuditErrorCode,
        field: &'static str,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            code,
            field,
            detail: detail.into(),
        }
    }

    pub const fn code(&self) -> AuditErrorCode {
        self.code
    }

    pub const fn field(&self) -> &'static str {
        self.field
    }
}

pub(crate) fn validate_domain_id(value: &str, field: &'static str) -> Result<(), AuditError> {
    let bytes = value.as_bytes();
    if bytes.is_empty()
        || bytes.len() > MAX_AUDIT_ID_BYTES
        || !bytes[0].is_ascii_alphanumeric()
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
    {
        return Err(AuditError::new(
            AuditErrorCode::InvalidIdentifier,
            field,
            "identifier must be 1..=128 bytes in the canonical ASCII domain alphabet",
        ));
    }
    Ok(())
}

pub(crate) fn validate_schema_name(value: &str) -> Result<(), AuditError> {
    if value.is_empty()
        || value.len() > MAX_AUDIT_ID_BYTES
        || !value.split('.').all(|segment| {
            let mut bytes = segment.bytes();
            bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
                && bytes
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        })
    {
        return Err(AuditError::new(
            AuditErrorCode::InvalidSchema,
            "payload.schema_name",
            "schema name must be a dot-qualified lowercase ASCII identifier",
        ));
    }
    Ok(())
}

pub(crate) fn validate_text(
    value: &str,
    field: &'static str,
    require_nonempty: bool,
) -> Result<(), AuditError> {
    if (require_nonempty && value.is_empty()) || value.len() > MAX_AUDIT_TEXT_BYTES {
        return Err(AuditError::new(
            AuditErrorCode::InvalidText,
            field,
            "text is empty or exceeds the audit text byte limit",
        ));
    }
    Ok(())
}

pub(crate) fn validate_timestamp(value: &str) -> Result<(), AuditError> {
    let bytes = value.as_bytes();
    let separators_are_valid = bytes.len() == 30
        && bytes.get(4) == Some(&b'-')
        && bytes.get(7) == Some(&b'-')
        && bytes.get(10) == Some(&b'T')
        && bytes.get(13) == Some(&b':')
        && bytes.get(16) == Some(&b':')
        && bytes.get(19) == Some(&b'.')
        && bytes.get(29) == Some(&b'Z');
    if !separators_are_valid
        || bytes.iter().enumerate().any(|(index, byte)| {
            !matches!(index, 4 | 7 | 10 | 13 | 16 | 19 | 29) && !byte.is_ascii_digit()
        })
    {
        return Err(invalid_timestamp());
    }

    let year = parse_component(bytes, 0, 4)?;
    let month = parse_component(bytes, 5, 2)?;
    let day = parse_component(bytes, 8, 2)?;
    let hour = parse_component(bytes, 11, 2)?;
    let minute = parse_component(bytes, 14, 2)?;
    let second = parse_component(bytes, 17, 2)?;
    if year == 0
        || month == 0
        || month > 12
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return Err(invalid_timestamp());
    }
    Ok(())
}

fn parse_component(bytes: &[u8], start: usize, width: usize) -> Result<u32, AuditError> {
    std::str::from_utf8(&bytes[start..start + width])
        .ok()
        .and_then(|value| value.parse().ok())
        .ok_or_else(invalid_timestamp)
}

const fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(400) || (year.is_multiple_of(4) && !year.is_multiple_of(100)) => {
            29
        }
        2 => 28,
        _ => 0,
    }
}

fn invalid_timestamp() -> AuditError {
    AuditError::new(
        AuditErrorCode::InvalidTimestamp,
        "occurred_at",
        "timestamp must be a valid UTC instant with exactly nine fractional digits",
    )
}

pub(crate) fn validate_target(target: &AuditTarget) -> Result<(), AuditError> {
    match target.kind {
        AuditTargetKind::FactorSpecId | AuditTargetKind::ArtifactId => {
            Sha256Digest::parse(&target.value).map(|_| ()).map_err(|_| {
                AuditError::new(
                    AuditErrorCode::InvalidTarget,
                    "target.value",
                    "content-addressed target requires a full SHA-256 identity",
                )
            })
        }
        _ => validate_domain_id(&target.value, "target.value").map_err(|_| {
            AuditError::new(
                AuditErrorCode::InvalidTarget,
                "target.value",
                "target value does not satisfy its typed ID encoding",
            )
        }),
    }
}

#[cfg(test)]
mod tests;
