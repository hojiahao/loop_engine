//! Fail-closed conversion of security-sensitive wire DTOs into domain values.

use std::fmt;

use prost::Message;

use crate::job::{JobValidationCode, validate_job_specification, validate_service_error};
use crate::wire::v1::{JobKind, JobSpecification, ServiceError};

pub const SERVICE_ERROR_TYPE_URL: &str = "type.googleapis.com/loop.v1.ServiceError";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeValidationCode {
    InvalidStatus,
    UnexpectedResponseBody,
    UnexpectedDetail,
    UnsupportedEnum,
    UnsupportedOneof,
    UnsupportedKind,
    IncompatibleVariant,
    InvalidSpecification,
    MalformedDetail,
    InvalidServiceError,
}

impl RuntimeValidationCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidStatus => "invalid_status",
            Self::UnexpectedResponseBody => "unexpected_response_body",
            Self::UnexpectedDetail => "unexpected_detail",
            Self::UnsupportedEnum => "unsupported_enum",
            Self::UnsupportedOneof => "unsupported_oneof",
            Self::UnsupportedKind => "unsupported_kind",
            Self::IncompatibleVariant => "incompatible_variant",
            Self::InvalidSpecification => "invalid_specification",
            Self::MalformedDetail => "malformed_detail",
            Self::InvalidServiceError => "invalid_service_error",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeValidationError {
    pub code: RuntimeValidationCode,
}

impl RuntimeValidationError {
    const fn new(code: RuntimeValidationCode) -> Self {
        Self { code }
    }
}

impl fmt::Display for RuntimeValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code.as_str())
    }
}

impl std::error::Error for RuntimeValidationError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RichStatusDetail<'a> {
    pub type_url: &'a str,
    pub value: &'a [u8],
}

/// Validate a non-OK gRPC response before constructing a retryable failure.
///
/// Only the canonical gRPC status range and the exact ServiceError type URL are
/// accepted. Protobuf payloads are not self-describing, so decoding before the
/// type URL check could reinterpret a factor rejection as infrastructure data.
pub fn validate_operational_failure(
    grpc_status_code: i32,
    response_body: &[u8],
    details: &[RichStatusDetail<'_>],
) -> Result<ServiceError, RuntimeValidationError> {
    if !(1..=16).contains(&grpc_status_code) {
        return Err(RuntimeValidationError::new(
            RuntimeValidationCode::InvalidStatus,
        ));
    }
    if !response_body.is_empty() {
        return Err(RuntimeValidationError::new(
            RuntimeValidationCode::UnexpectedResponseBody,
        ));
    }
    let [detail] = details else {
        return Err(RuntimeValidationError::new(
            RuntimeValidationCode::UnexpectedDetail,
        ));
    };
    if detail.type_url != SERVICE_ERROR_TYPE_URL {
        return Err(RuntimeValidationError::new(
            RuntimeValidationCode::UnexpectedDetail,
        ));
    }

    let service_error = ServiceError::decode(detail.value)
        .map_err(|_| RuntimeValidationError::new(RuntimeValidationCode::MalformedDetail))?;
    validate_service_error(&service_error).map_err(|error| {
        if error.code == JobValidationCode::UnknownEnum {
            RuntimeValidationError::new(RuntimeValidationCode::UnsupportedEnum)
        } else {
            RuntimeValidationError::new(RuntimeValidationCode::InvalidServiceError)
        }
    })?;
    Ok(service_error)
}

/// Validate a job's wire envelope as a candidate for later dispatch.
///
/// Protobuf preserves unknown enum numbers but drops unknown oneof alternatives
/// from the generated projection. Passing this necessary check does not select
/// or authorize a handler. FactorSpec registry binding, holdout plan-entry and
/// Phase 7 owning-BacktestSpec parsing, artifact availability, server-owned
/// dataset snapshot/capability resolution, and runtime authorization remain
/// mandatory external gates.
pub fn validate_job_wire_dispatch_candidate(
    specification: &JobSpecification,
    enabled_job_kinds: &[JobKind],
) -> Result<JobKind, RuntimeValidationError> {
    let kind = validate_job_specification(specification)
        .map_err(|error| match error.code {
            JobValidationCode::UnknownEnum => {
                RuntimeValidationError::new(RuntimeValidationCode::UnsupportedEnum)
            }
            JobValidationCode::MissingField if error.field == "specification.input" => {
                RuntimeValidationError::new(RuntimeValidationCode::UnsupportedOneof)
            }
            JobValidationCode::KindInputMismatch => {
                RuntimeValidationError::new(RuntimeValidationCode::IncompatibleVariant)
            }
            _ => RuntimeValidationError::new(RuntimeValidationCode::InvalidSpecification),
        })?
        .kind;
    if !enabled_job_kinds.contains(&kind) {
        return Err(RuntimeValidationError::new(
            RuntimeValidationCode::UnsupportedKind,
        ));
    }
    Ok(kind)
}
