import base64
import json
from pathlib import Path
from typing import Any, cast

import pytest

from loop.v1 import common_pb2, job_pb2
from loop_protocol import (
    SERVICE_ERROR_TYPE_URL,
    RichStatusDetail,
    RuntimeValidationCode,
    RuntimeValidationError,
    validate_job_wire_dispatch_candidate,
    validate_operational_failure,
)

REPOSITORY_ROOT = Path(__file__).resolve().parents[3]
PROTOCOL_FIXTURES = REPOSITORY_ROOT / "fixtures" / "contracts" / "protocol" / "v1"


def _operational_fixture() -> dict[str, Any]:
    return cast(
        dict[str, Any],
        json.loads(
            (REPOSITORY_ROOT / "tests" / "contracts" / "operational_failure.json").read_text(
                encoding="ascii"
            )
        ),
    )


def _detail(fixture: dict[str, Any], type_url: str | None = None) -> RichStatusDetail:
    raw_detail = fixture["details"][0]
    return RichStatusDetail(
        type_url=type_url or raw_detail["type_url"],
        value=base64.b64decode(raw_detail["value_base64"], validate=True),
    )


def _assert_error_code(expected: RuntimeValidationCode, operation: Any) -> None:
    with pytest.raises(RuntimeValidationError) as caught:
        operation()
    assert caught.value.code is expected


def test_validates_shared_operational_failure_fixture() -> None:
    fixture = _operational_fixture()
    validated = validate_operational_failure(
        fixture["grpc_status_code"],
        base64.b64decode(fixture["response_body_base64"], validate=True),
        [_detail(fixture)],
    )
    assert validated.category == common_pb2.ERROR_CATEGORY_DEPENDENCY
    assert validated.code == fixture["expected"]["code"]
    assert validated.message == fixture["expected"]["message"]
    assert validated.retryable is fixture["expected"]["retryable"]


def test_operational_failure_type_and_transport_checks_fail_closed() -> None:
    fixture = _operational_fixture()
    for type_url in (
        "type.googleapis.com/loop.v1.FactorRejection",
        "type.googleapis.com/vendor.FutureError",
    ):
        _assert_error_code(
            RuntimeValidationCode.UNEXPECTED_DETAIL,
            lambda type_url=type_url: validate_operational_failure(
                fixture["grpc_status_code"], b"", [_detail(fixture, type_url)]
            ),
        )
    _assert_error_code(
        RuntimeValidationCode.INVALID_STATUS,
        lambda: validate_operational_failure(0, b"", [_detail(fixture)]),
    )
    _assert_error_code(
        RuntimeValidationCode.UNEXPECTED_RESPONSE_BODY,
        lambda: validate_operational_failure(
            fixture["grpc_status_code"], b"response", [_detail(fixture)]
        ),
    )
    assert _detail(fixture).type_url == SERVICE_ERROR_TYPE_URL


def test_operational_failure_rejects_unbounded_or_malformed_service_error_fields() -> None:
    baseline = common_pb2.ServiceError(
        category=common_pb2.ERROR_CATEGORY_DEPENDENCY,
        code="artifact_digest_mismatch",
        message="artifact verification failed",
        retryable=True,
    )
    valid = common_pb2.ServiceError()
    valid.CopyFrom(baseline)
    valid.details.add(
        field_path="$.artifact.sha256",
        code="digest_mismatch",
        message="declared and computed digests differ",
    )
    _validate_service_error_bytes(valid)

    unknown_category = common_pb2.ServiceError()
    unknown_category.CopyFrom(baseline)
    unknown_category.category = cast(Any, 999)
    _assert_error_code(
        RuntimeValidationCode.UNSUPPORTED_ENUM,
        lambda: _validate_service_error_bytes(unknown_category),
    )

    invalid: list[common_pb2.ServiceError] = []
    for field, value in (
        ("code", "Invalid Code"),
        ("code", "a" * 129),
        ("message", "é" * 1_025),
        ("message", "forged\nrecord"),
    ):
        error = common_pb2.ServiceError()
        error.CopyFrom(baseline)
        setattr(error, field, value)
        invalid.append(error)
    for detail in (
        common_pb2.ErrorDetail(),
        common_pb2.ErrorDetail(
            field_path="$.field\tname",
            code="invalid_field",
            message="invalid field",
        ),
        common_pb2.ErrorDetail(
            field_path="x" * 513,
            code="invalid_field",
            message="invalid field",
        ),
        common_pb2.ErrorDetail(
            field_path="$.field",
            code="INVALID",
            message="invalid code",
        ),
        common_pb2.ErrorDetail(
            field_path="$.field",
            code="invalid_field",
            message=" ",
        ),
    ):
        error = common_pb2.ServiceError()
        error.CopyFrom(baseline)
        error.details.append(detail)
        invalid.append(error)

    for service_error in invalid:
        _assert_error_code(
            RuntimeValidationCode.INVALID_SERVICE_ERROR,
            lambda service_error=service_error: _validate_service_error_bytes(service_error),
        )


def _validate_service_error_bytes(service_error: common_pb2.ServiceError) -> None:
    validate_operational_failure(
        14,
        b"",
        [
            RichStatusDetail(
                type_url=SERVICE_ERROR_TYPE_URL,
                value=service_error.SerializeToString(),
            )
        ],
    )


def test_unknown_job_enum_and_oneof_fixtures_fail_closed() -> None:
    unknown_enum = job_pb2.JobSpecification.FromString(
        (PROTOCOL_FIXTURES / "job_specification_v1_unknown_enum.binpb").read_bytes()
    )
    assert unknown_enum.kind == 127
    assert unknown_enum.WhichOneof("input") == "discovery"
    _assert_error_code(
        RuntimeValidationCode.UNSUPPORTED_ENUM,
        lambda: validate_job_wire_dispatch_candidate(unknown_enum, {job_pb2.JOB_KIND_DISCOVERY}),
    )

    unknown_oneof = job_pb2.JobSpecification.FromString(
        (PROTOCOL_FIXTURES / "job_specification_v1_unknown_oneof.binpb").read_bytes()
    )
    assert unknown_oneof.kind == job_pb2.JOB_KIND_REPORT
    assert unknown_oneof.WhichOneof("input") is None
    _assert_error_code(
        RuntimeValidationCode.UNSUPPORTED_ONEOF,
        lambda: validate_job_wire_dispatch_candidate(unknown_oneof, {job_pb2.JOB_KIND_REPORT}),
    )


def test_known_job_dispatch_requires_a_complete_envelope_and_matching_input() -> None:
    valid = job_pb2.JobSpecification(
        kind=job_pb2.JOB_KIND_DISCOVERY,
        discovery=job_pb2.DiscoveryJobInput(),
    )
    _assert_error_code(
        RuntimeValidationCode.INVALID_SPECIFICATION,
        lambda: validate_job_wire_dispatch_candidate(valid, {job_pb2.JOB_KIND_DISCOVERY}),
    )

    mismatched = job_pb2.JobSpecification(
        kind=job_pb2.JOB_KIND_DISCOVERY,
        artifact=job_pb2.ArtifactJobInput(),
    )
    _assert_error_code(
        RuntimeValidationCode.INCOMPATIBLE_VARIANT,
        lambda: validate_job_wire_dispatch_candidate(mismatched, {job_pb2.JOB_KIND_DISCOVERY}),
    )

    disabled = job_pb2.JobSpecification(
        kind=job_pb2.JOB_KIND_PROSPECTIVE_OBSERVATION,
        artifact=job_pb2.ArtifactJobInput(),
    )
    _assert_error_code(
        RuntimeValidationCode.INVALID_SPECIFICATION,
        lambda: validate_job_wire_dispatch_candidate(disabled, {job_pb2.JOB_KIND_DISCOVERY}),
    )
