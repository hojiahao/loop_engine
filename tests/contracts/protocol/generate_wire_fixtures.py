"""Assemble versioned, non-sensitive cross-language Protobuf fixtures."""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import platform
from pathlib import Path

from google import protobuf
from google.protobuf.timestamp_pb2 import Timestamp

from loop.discovery.v1 import service_pb2 as discovery_pb2
from loop.provider.v1 import service_pb2 as provider_pb2
from loop.v1 import common_pb2, job_pb2, model_pb2

REPOSITORY_ROOT = Path(__file__).resolve().parents[3]
FIXTURE_DIRECTORY = REPOSITORY_ROOT / "fixtures" / "contracts" / "protocol" / "v1"
SCHEMA_DESCRIPTOR = FIXTURE_DIRECTORY / "schema.current.binpb"
PYTHON_FIXTURE = "protocol_info_v1.binpb"
RUST_FIXTURE = "protocol_info_v1.rust.binpb"
TYPESCRIPT_FIXTURE = "protocol_info_v1.typescript.binpb"
UNKNOWN_FIELD_FIXTURE = "protocol_info_v1_unknown_field.binpb"
UNKNOWN_ENUM_FIXTURE = "job_specification_v1_unknown_enum.binpb"
UNKNOWN_ONEOF_FIXTURE = "job_specification_v1_unknown_oneof.binpb"
OPERATIONAL_FAILURE_FIXTURE = "operational_failure.json"
LOOKUP_REQUEST_FIXTURE = "provider_lookup_v1.binpb"
LOOKUP_RESPONSE_FIXTURE = "provider_completed_v1.binpb"
DISCOVERY_REQUEST_FIXTURE = "discovery_execute_v1.binpb"
DISCOVERY_RESPONSE_FIXTURE = "discovery_completed_v1.binpb"
MANIFEST = "wire_fixtures.json"
EXPECTED_PYTHON_VERSION = (REPOSITORY_ROOT / ".python-version").read_text(encoding="ascii").strip()
EXPECTED_PROTOBUF_VERSION = "7.36.1"
UNKNOWN_FIELD_NUMBER = 536_870_911
UNKNOWN_FIELD_PAYLOAD = b"ignored-additive-field"
FUTURE_JOB_INPUT_FIELD_NUMBER = 16
UNKNOWN_JOB_KIND = 127


def _protocol_info() -> common_pb2.ProtocolInfo:
    return common_pb2.ProtocolInfo(
        supported_packages=[
            "loop.audit.v1",
            "loop.discovery.v1",
            "loop.holdout.v1",
            "loop.jobs.v1",
            "loop.protocol.v1",
            "loop.provider.v1",
            "loop.research.v1",
            "loop.runs.v1",
            "loop.v1",
        ],
        features=[
            "artifacts.by-reference.v1",
            "factors.canonical-json.v1",
        ],
        limits=common_pb2.ProtocolLimits(
            maximum_unary_bytes=4_194_304,
            maximum_stream_event_bytes=1_048_576,
            maximum_canonical_ast_bytes=262_144,
            maximum_ast_nodes=4_096,
            maximum_ast_depth=64,
            maximum_page_records=500,
            maximum_identity_bytes=128,
            maximum_artifact_uri_bytes=2_048,
        ),
        build_version="0.2.0-alpha.1+wire-fixture.1",
        build_sha256=common_pb2.Sha256Digest(value=bytes(range(32))),
    )


def _encode_varint(value: int) -> bytes:
    encoded = bytearray()
    while value > 0x7F:
        encoded.append((value & 0x7F) | 0x80)
        value >>= 7
    encoded.append(value)
    return bytes(encoded)


def _length_delimited_field(field_number: int, payload: bytes) -> bytes:
    field_key = (field_number << 3) | 2
    return _encode_varint(field_key) + _encode_varint(len(payload)) + payload


def _sha256(data: bytes) -> str:
    return f"sha256:{hashlib.sha256(data).hexdigest()}"


def _source_record(path: str) -> dict[str, str]:
    return {
        "path": path,
        "sha256": _sha256((REPOSITORY_ROOT / path).read_bytes()),
    }


def _expected_projection() -> dict[str, object]:
    message = _protocol_info()
    return {
        "supported_packages": list(message.supported_packages),
        "features": list(message.features),
        "limits": {
            "maximum_unary_bytes": 4_194_304,
            "maximum_stream_event_bytes": 1_048_576,
            "maximum_canonical_ast_bytes": 262_144,
            "maximum_ast_nodes": 4_096,
            "maximum_ast_depth": 64,
            "maximum_page_records": 500,
            "maximum_identity_bytes": 128,
            "maximum_artifact_uri_bytes": 2_048,
        },
        "build_version": "0.2.0-alpha.1+wire-fixture.1",
        "build_sha256": bytes(range(32)).hex(),
    }


def _operational_failure() -> bytes:
    service_error = common_pb2.ServiceError(
        category=common_pb2.ERROR_CATEGORY_DEPENDENCY,
        code="artifact_digest_mismatch",
        message="artifact verification failed",
        retryable=True,
    )
    document = {
        "contract": "loop.rpc-operational-failure/v1",
        "rpc": "/loop.provider.v1.ProviderService/InvokeModel",
        "grpc_status_code": 14,
        "grpc_status_name": "UNAVAILABLE",
        "response_body_base64": "",
        "details": [
            {
                "type_url": "type.googleapis.com/loop.v1.ServiceError",
                "value_base64": base64.b64encode(
                    service_error.SerializeToString(deterministic=True)
                ).decode("ascii"),
            }
        ],
        "expected": {
            "category": "ERROR_CATEGORY_DEPENDENCY",
            "code": service_error.code,
            "message": service_error.message,
            "retryable": service_error.retryable,
        },
    }
    return (json.dumps(document, indent=2, ensure_ascii=True) + "\n").encode("ascii")


def _lookup_fixtures() -> dict[str, bytes]:
    request = provider_pb2.LookupInvocationRequest(
        context=common_pb2.CommandContext(
            request_id=common_pb2.RequestId(value="lookup.1"),
            correlation_id=common_pb2.CorrelationId(value="correlation.1"),
            causation_id=common_pb2.CausationId(value="invoke.1"),
            idempotency_key=common_pb2.IdempotencyKey(value="lookup-key.1"),
            actor=common_pb2.Actor(
                actor_id=common_pb2.ActorId(value="actor.1"),
                kind=common_pb2.ACTOR_KIND_SERVICE,
                authenticated_subject="subject.1",
            ),
            requested_at=Timestamp(seconds=1_800_000_000),
        ),
        original_request_id=common_pb2.RequestId(value="invoke.1"),
        original_idempotency_key=common_pb2.IdempotencyKey(value="invoke-key.1"),
        request_sha256=common_pb2.Sha256Digest(value=bytes(range(32))),
    )
    response = provider_pb2.LookupInvocationResponse(
        state=provider_pb2.INVOCATION_STATE_COMPLETED,
        response=model_pb2.ModelResponse(
            request_id=common_pb2.RequestId(value="invoke.1"),
            resolution_id=common_pb2.ModelResolutionId(value="resolution.1"),
            content=[model_pb2.ContentBlock(text=model_pb2.TextContent(text="fixture result"))],
            finish_reason=model_pb2.MODEL_FINISH_REASON_STOP,
            usage=model_pb2.ModelUsage(input_tokens=11, output_tokens=3),
        ),
        reserved_cost=common_pb2.Money(
            amount=common_pb2.ExactDecimal(value="0.125"), currency_code="USD"
        ),
    )
    return {
        LOOKUP_REQUEST_FIXTURE: request.SerializeToString(deterministic=True),
        LOOKUP_RESPONSE_FIXTURE: response.SerializeToString(deterministic=True),
    }


def _discovery_fixtures() -> dict[str, bytes]:
    canonical = b'{"node":"field","field":"market.close"}'
    request = discovery_pb2.ExecuteDiscoveryRequest(
        context=common_pb2.CommandContext(
            request_id=common_pb2.RequestId(value="execute.1"),
            correlation_id=common_pb2.CorrelationId(value="correlation.1"),
            idempotency_key=common_pb2.IdempotencyKey(value="execute-key.1"),
            actor=common_pb2.Actor(
                actor_id=common_pb2.ActorId(value="researcher.1"),
                kind=common_pb2.ACTOR_KIND_HUMAN,
            ),
            requested_at=Timestamp(seconds=1_788_192_000),
        ),
        job_id=common_pb2.JobId(value="discovery.1"),
        expected_revision=3,
    )
    response = discovery_pb2.ExecuteDiscoveryResponse(
        step=discovery_pb2.DiscoveryStepView(
            job=discovery_pb2.DiscoveryJobHandle(
                job_id=common_pb2.JobId(value="discovery.1"),
                status=discovery_pb2.DISCOVERY_JOB_STATUS_SUCCEEDED,
                revision=5,
                submitted_at=Timestamp(seconds=1_788_192_000),
                updated_at=Timestamp(seconds=1_788_192_010),
            ),
            state=discovery_pb2.DISCOVERY_STEP_STATE_COMPLETED,
            candidate=discovery_pb2.DiscoveryCandidate(
                expression_id=common_pb2.FactorExpressionId(
                    value=_sha256(b"loop.factor-ast/v1\0" + canonical)
                ),
                canonicalization_profile="loop.factor-ast/v1",
                canonical_json=canonical,
            ),
            reserved_cost=common_pb2.Money(
                amount=common_pb2.ExactDecimal(value="0.125"), currency_code="USD"
            ),
            reserved_input_tokens=4096,
            reserved_output_tokens=1024,
            plan_verified=True,
        )
    )
    return {
        DISCOVERY_REQUEST_FIXTURE: request.SerializeToString(deterministic=True),
        DISCOVERY_RESPONSE_FIXTURE: response.SerializeToString(deterministic=True),
    }


def _manifest(
    protocol_fixtures: dict[str, bytes],
    unknown_wire: bytes,
    unknown_enum_wire: bytes,
    unknown_oneof_wire: bytes,
    operational_failure: bytes,
    lookup_fixtures: dict[str, bytes],
    discovery_fixtures: dict[str, bytes],
) -> bytes:
    document = {
        "fixture_schema": "loop.contract-fixture-manifest/v2",
        "message_type": "loop.v1.ProtocolInfo",
        "schema_descriptor": {
            "path": "schema.current.binpb",
            "sha256": _sha256(SCHEMA_DESCRIPTOR.read_bytes()),
        },
        "expected_domain_projection": _expected_projection(),
        "producers": [
            {
                "language": "python",
                "toolchain": [
                    f"python=={EXPECTED_PYTHON_VERSION}",
                    f"protobuf=={EXPECTED_PROTOBUF_VERSION}",
                ],
                "source": _source_record("tests/contracts/protocol/generate_wire_fixtures.py"),
                "path": PYTHON_FIXTURE,
                "wire_sha256": _sha256(protocol_fixtures[PYTHON_FIXTURE]),
            },
            {
                "language": "rust",
                "toolchain": ["rustc==1.93.1", "prost==0.14.4"],
                "source": _source_record("crates/loop-protocol/src/bin/generate_wire_fixture.rs"),
                "path": RUST_FIXTURE,
                "wire_sha256": _sha256(protocol_fixtures[RUST_FIXTURE]),
            },
            {
                "language": "typescript",
                "toolchain": [
                    "node==24.17.0",
                    "typescript==7.0.2",
                    "@bufbuild/protobuf==2.14.1",
                ],
                "source": _source_record("packages/protocol-ts/src/bin/generate-wire-fixture.ts"),
                "path": TYPESCRIPT_FIXTURE,
                "wire_sha256": _sha256(protocol_fixtures[TYPESCRIPT_FIXTURE]),
            },
        ],
        "compatibility_fixtures": [
            {
                "path": UNKNOWN_FIELD_FIXTURE,
                "wire_sha256": _sha256(unknown_wire),
                "contains_injected_unknown_field": True,
                "unknown_field": {
                    "field_number": UNKNOWN_FIELD_NUMBER,
                    "wire_type": "length-delimited",
                    "payload_utf8": UNKNOWN_FIELD_PAYLOAD.decode("ascii"),
                },
                "lossless_original_byte_forwarding_required": False,
            }
        ],
        "semantic_negative_fixtures": [
            {
                "path": UNKNOWN_ENUM_FIXTURE,
                "message_type": "loop.v1.JobSpecification",
                "wire_sha256": _sha256(unknown_enum_wire),
                "unknown_enum_numeric": UNKNOWN_JOB_KIND,
                "expected_error": "unsupported_enum",
            },
            {
                "path": UNKNOWN_ONEOF_FIXTURE,
                "message_type": "loop.v1.JobSpecification",
                "wire_sha256": _sha256(unknown_oneof_wire),
                "future_oneof_field_number": FUTURE_JOB_INPUT_FIELD_NUMBER,
                "expected_error": "unsupported_oneof",
            },
        ],
        "auxiliary_contracts": [
            {
                "path": "tests/contracts/operational_failure.json",
                "sha256": _sha256(operational_failure),
                "contract": "loop.rpc-operational-failure/v1",
            }
        ],
        "provider_recovery_fixtures": [
            {
                "path": name,
                "wire_sha256": _sha256(content),
                "producer_language": "python",
                "capability_identity": "provider.invocation-lookup.v1",
            }
            for name, content in lookup_fixtures.items()
        ],
        "discovery_step_fixtures": [
            {
                "path": name,
                "wire_sha256": _sha256(content),
                "producer_language": "python",
                "capability_identity": "discovery.model-step.v1",
            }
            for name, content in discovery_fixtures.items()
        ],
    }
    return (json.dumps(document, indent=2, ensure_ascii=True) + "\n").encode("ascii")


def _expected_files(output_directory: Path) -> dict[str, bytes]:
    if platform.python_version() != EXPECTED_PYTHON_VERSION:
        raise RuntimeError(
            f"fixture producer requires python=={EXPECTED_PYTHON_VERSION}; "
            f"found {platform.python_version()}"
        )
    if protobuf.__version__ != EXPECTED_PROTOBUF_VERSION:
        raise RuntimeError(
            f"fixture producer requires protobuf=={EXPECTED_PROTOBUF_VERSION}; "
            f"found {protobuf.__version__}"
        )

    protocol_fixtures = {
        PYTHON_FIXTURE: _protocol_info().SerializeToString(deterministic=True),
        RUST_FIXTURE: (output_directory / RUST_FIXTURE).read_bytes(),
        TYPESCRIPT_FIXTURE: (output_directory / TYPESCRIPT_FIXTURE).read_bytes(),
    }
    unknown_wire = protocol_fixtures[PYTHON_FIXTURE] + _length_delimited_field(
        UNKNOWN_FIELD_NUMBER, UNKNOWN_FIELD_PAYLOAD
    )
    unknown_enum_wire = job_pb2.JobSpecification(
        kind=UNKNOWN_JOB_KIND,
        discovery=job_pb2.DiscoveryJobInput(maximum_candidates=1),
    ).SerializeToString(deterministic=True)
    unknown_oneof_wire = job_pb2.JobSpecification(kind=job_pb2.JOB_KIND_REPORT).SerializeToString(
        deterministic=True
    ) + _length_delimited_field(FUTURE_JOB_INPUT_FIELD_NUMBER, b"")
    operational_failure = _operational_failure()
    lookup_fixtures = _lookup_fixtures()
    discovery_fixtures = _discovery_fixtures()
    expected = {
        **protocol_fixtures,
        **lookup_fixtures,
        **discovery_fixtures,
        UNKNOWN_FIELD_FIXTURE: unknown_wire,
        UNKNOWN_ENUM_FIXTURE: unknown_enum_wire,
        UNKNOWN_ONEOF_FIXTURE: unknown_oneof_wire,
        OPERATIONAL_FAILURE_FIXTURE: operational_failure,
    }
    expected[MANIFEST] = _manifest(
        protocol_fixtures,
        unknown_wire,
        unknown_enum_wire,
        unknown_oneof_wire,
        operational_failure,
        lookup_fixtures,
        discovery_fixtures,
    )
    return expected


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True)
    arguments = parser.parse_args()
    arguments.output_dir.mkdir(parents=True, exist_ok=True)
    for name, content in _expected_files(arguments.output_dir).items():
        (arguments.output_dir / name).write_bytes(content)


if __name__ == "__main__":
    main()
