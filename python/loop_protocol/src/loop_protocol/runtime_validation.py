"""Fail-closed conversion of security-sensitive wire DTOs into domain values."""

from __future__ import annotations

from collections.abc import Set
from dataclasses import dataclass
from enum import StrEnum

from google.protobuf.message import DecodeError  # type: ignore[import-untyped]

from loop.v1 import common_pb2, job_pb2

from .job import (
    JobValidationCode,
    JobValidationError,
    validate_job_specification,
    validate_service_error,
)

SERVICE_ERROR_TYPE_URL = "type.googleapis.com/loop.v1.ServiceError"


class RuntimeValidationCode(StrEnum):
    INVALID_STATUS = "invalid_status"
    UNEXPECTED_RESPONSE_BODY = "unexpected_response_body"
    UNEXPECTED_DETAIL = "unexpected_detail"
    UNSUPPORTED_ENUM = "unsupported_enum"
    UNSUPPORTED_ONEOF = "unsupported_oneof"
    UNSUPPORTED_KIND = "unsupported_kind"
    INCOMPATIBLE_VARIANT = "incompatible_variant"
    INVALID_SPECIFICATION = "invalid_specification"
    MALFORMED_DETAIL = "malformed_detail"
    INVALID_SERVICE_ERROR = "invalid_service_error"


class RuntimeValidationError(ValueError):
    def __init__(self, code: RuntimeValidationCode) -> None:
        super().__init__(code.value)
        self.code = code


@dataclass(frozen=True, slots=True)
class RichStatusDetail:
    type_url: str
    value: bytes


def validate_operational_failure(
    grpc_status_code: int,
    response_body: bytes,
    details: tuple[RichStatusDetail, ...] | list[RichStatusDetail],
) -> common_pb2.ServiceError:
    """Validate a non-OK response before constructing a retryable failure."""

    if type(grpc_status_code) is not int or not 1 <= grpc_status_code <= 16:
        raise RuntimeValidationError(RuntimeValidationCode.INVALID_STATUS)
    if response_body:
        raise RuntimeValidationError(RuntimeValidationCode.UNEXPECTED_RESPONSE_BODY)
    if len(details) != 1 or details[0].type_url != SERVICE_ERROR_TYPE_URL:
        raise RuntimeValidationError(RuntimeValidationCode.UNEXPECTED_DETAIL)

    service_error = common_pb2.ServiceError()
    try:
        service_error.ParseFromString(details[0].value)
    except DecodeError as error:
        raise RuntimeValidationError(RuntimeValidationCode.MALFORMED_DETAIL) from error
    try:
        validate_service_error(service_error)
    except JobValidationError as error:
        if error.code is JobValidationCode.UNKNOWN_ENUM:
            raise RuntimeValidationError(RuntimeValidationCode.UNSUPPORTED_ENUM) from error
        raise RuntimeValidationError(RuntimeValidationCode.INVALID_SERVICE_ERROR) from error
    return service_error


def validate_job_wire_dispatch_candidate(
    specification: job_pb2.JobSpecification, enabled_job_kinds: Set[int]
) -> int:
    """Validate only a necessary wire-level dispatch candidate.

    A returned kind does not select or authorize a handler. FactorSpec registry
    binding, holdout plan-entry and Phase 7 owning-BacktestSpec parsing, artifact
    availability, server-owned dataset snapshot/capability resolution, and
    runtime authorization remain mandatory external gates.
    """

    try:
        kind = validate_job_specification(specification).kind
    except JobValidationError as error:
        if error.code is JobValidationCode.UNKNOWN_ENUM:
            raise RuntimeValidationError(RuntimeValidationCode.UNSUPPORTED_ENUM) from error
        if error.code is JobValidationCode.MISSING_FIELD and error.field == "specification.input":
            raise RuntimeValidationError(RuntimeValidationCode.UNSUPPORTED_ONEOF) from error
        if error.code is JobValidationCode.KIND_INPUT_MISMATCH:
            raise RuntimeValidationError(RuntimeValidationCode.INCOMPATIBLE_VARIANT) from error
        raise RuntimeValidationError(RuntimeValidationCode.INVALID_SPECIFICATION) from error
    if kind not in enabled_job_kinds:
        raise RuntimeValidationError(RuntimeValidationCode.UNSUPPORTED_KIND)
    return kind
