"""Executable descriptor assertions for protocol privilege boundaries."""

from __future__ import annotations

import base64
import json
import sys
from pathlib import Path
from typing import Any

from google.protobuf.descriptor_pb2 import (
    DescriptorProto,
    FieldDescriptorProto,
    FileDescriptorProto,
    FileDescriptorSet,
)

from loop.v1.common_pb2 import ERROR_CATEGORY_DEPENDENCY, ErrorCategory, ServiceError

PROVIDER_FILE = "loop/provider/v1/service.proto"
DISCOVERY_FILE = "loop/discovery/v1/service.proto"
RESEARCH_FILE = "loop/research/v1/service.proto"
ARTIFACT_FILE = "loop/v1/artifact.proto"
COMMON_FILE = "loop/v1/common.proto"
DEVELOPMENT_DATA_FILE = "loop/v1/development_data.proto"
HOLDOUT_FILE = "loop/v1/holdout.proto"
HOLDOUT_SERVICE_FILE = "loop/holdout/v1/service.proto"
JOB_FILE = "loop/v1/job.proto"
PROVIDER_FILE_ALLOWLIST = {
    "google/protobuf/duration.proto",
    "google/protobuf/timestamp.proto",
    "loop/provider/v1/service.proto",
    "loop/v1/artifact.proto",
    "loop/v1/common.proto",
    "loop/v1/model.proto",
    "loop/v1/stream.proto",
}
PROVIDER_FORBIDDEN_TYPE_TOKENS = (
    "approval",
    "backtest",
    "factor",
    "grant",
    "holdout",
    "job",
    "research",
)
DISCOVERY_FILE_ALLOWLIST = {
    "google/protobuf/duration.proto",
    "google/protobuf/timestamp.proto",
    "loop/discovery/v1/service.proto",
    "loop/v1/artifact.proto",
    "loop/v1/common.proto",
    "loop/v1/development_data.proto",
    "loop/v1/model.proto",
}
DISCOVERY_FORBIDDEN_TYPE_TOKENS = (
    "approval",
    "backtestspec",
    "grant",
    "holdout",
    "joblease",
    "joboutcome",
    "jobrecord",
    "jobspecification",
)
RESEARCH_FILE_ALLOWLIST = {
    "google/protobuf/duration.proto",
    "google/protobuf/timestamp.proto",
    "loop/research/v1/service.proto",
    "loop/v1/common.proto",
    "loop/v1/development_data.proto",
    "loop/v1/factor.proto",
    "loop/v1/research_common.proto",
}
RESEARCH_FORBIDDEN_TYPE_TOKENS = (
    "approval",
    "backtestspec",
    "grant",
    "holdout",
    "joblease",
    "joboutcome",
    "jobrecord",
    "jobspecification",
    "samplerole",
    "samplewindow",
)


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("usage: check_protocol_boundaries.py <descriptor-set>")
    descriptor = FileDescriptorSet.FromString(Path(sys.argv[1]).read_bytes())
    files = {file.name: file for file in descriptor.file}

    assert_development_dataset_leaf(files)

    provider = require_file(files, PROVIDER_FILE)
    assert set(provider.dependency) == {
        "loop/v1/common.proto",
        "loop/v1/model.proto",
        "loop/v1/stream.proto",
    }
    assert all(
        "holdout" not in dependency and "research" not in dependency
        for dependency in provider.dependency
    )
    assert_service(
        provider,
        "ProviderService",
        {
            "InvokeModel": (
                ".loop.provider.v1.InvokeModelRequest",
                ".loop.provider.v1.InvokeModelResponse",
            ),
            "StreamModel": (
                ".loop.provider.v1.StreamModelRequest",
                ".loop.provider.v1.StreamModelResponse",
            ),
        },
    )
    assert dependency_closure(files, PROVIDER_FILE) == PROVIDER_FILE_ALLOWLIST
    assert_provider_message_graph(descriptor, provider)
    assert_provider_language_exports()
    provider_response = next(
        message for message in provider.message_type if message.name == "InvokeModelResponse"
    )
    assert all(
        field.type_name not in {".loop.v1.ServiceError", ".loop.v1.FactorRejection"}
        for field in provider_response.field
    )

    discovery = require_file(files, DISCOVERY_FILE)
    assert set(discovery.dependency) == {
        "google/protobuf/duration.proto",
        "google/protobuf/timestamp.proto",
        "loop/v1/common.proto",
        "loop/v1/development_data.proto",
        "loop/v1/model.proto",
    }
    assert_service(
        discovery,
        "DiscoveryService",
        {
            "StartDiscovery": (
                ".loop.discovery.v1.StartDiscoveryRequest",
                ".loop.discovery.v1.StartDiscoveryResponse",
            )
        },
    )
    assert dependency_closure(files, DISCOVERY_FILE) == DISCOVERY_FILE_ALLOWLIST
    assert "loop/v1/data.proto" not in dependency_closure(files, DISCOVERY_FILE)
    assert_discovery_message_graph(descriptor, discovery)
    assert_discovery_language_exports()

    research = require_file(files, RESEARCH_FILE)
    assert set(research.dependency) == {
        "google/protobuf/duration.proto",
        "google/protobuf/timestamp.proto",
        "loop/v1/common.proto",
        "loop/v1/development_data.proto",
        "loop/v1/factor.proto",
        "loop/v1/research_common.proto",
    }
    assert_service(
        research,
        "ResearchService",
        {
            "EnqueueFactorEvaluation": (
                ".loop.research.v1.EnqueueFactorEvaluationRequest",
                ".loop.research.v1.EnqueueFactorEvaluationResponse",
            ),
            "EnqueueBacktest": (
                ".loop.research.v1.EnqueueBacktestRequest",
                ".loop.research.v1.EnqueueBacktestResponse",
            ),
            "EnqueueReconciliation": (
                ".loop.research.v1.EnqueueReconciliationRequest",
                ".loop.research.v1.EnqueueReconciliationResponse",
            ),
        },
    )
    assert dependency_closure(files, RESEARCH_FILE) == RESEARCH_FILE_ALLOWLIST
    assert "loop/v1/data.proto" not in dependency_closure(files, RESEARCH_FILE)
    assert_research_message_graph(descriptor, research)
    assert_research_language_exports()
    assert_holdout_plan_boundary(descriptor, files)
    assert_holdout_language_exports()

    for service_file in (provider, discovery, research):
        surface = "\n".join(
            [
                *(service.name for service in service_file.service),
                *(method.name for service in service_file.service for method in service.method),
                *(
                    method.input_type
                    for service in service_file.service
                    for method in service.method
                ),
                *(
                    method.output_type
                    for service in service_file.service
                    for method in service.method
                ),
            ]
        ).lower()
        assert "holdout" not in surface
        assert "grant" not in surface
        assert "capability" not in surface

    artifact = require_file(files, ARTIFACT_FILE)
    reference = next(message for message in artifact.message_type if message.name == "ArtifactRef")
    field_names = {field.name for field in reference.field}
    forbidden_inline_names = {"bytes", "data", "payload", "content", "inline_bytes"}
    assert field_names.isdisjoint(forbidden_inline_names)
    assert all(field.type != FieldDescriptorProto.TYPE_BYTES for field in reference.field)

    verify_operational_failure_fixture(Path(__file__).with_name("operational_failure.json"))

    print(
        "Protocol service imports, discovery/research role reachability, method "
        "capabilities, frozen holdout-plan, inline-artifact, and operational-failure "
        "boundaries passed."
    )


def require_file(files: dict[str, FileDescriptorProto], name: str) -> FileDescriptorProto:
    try:
        return files[name]
    except KeyError as error:
        raise AssertionError(f"missing descriptor file: {name}") from error


def assert_service(
    file: FileDescriptorProto,
    name: str,
    expected_methods: dict[str, tuple[str, str]],
) -> None:
    services = [service for service in file.service if service.name == name]
    assert len(services) == 1
    actual = {method.name: (method.input_type, method.output_type) for method in services[0].method}
    assert actual == expected_methods


def dependency_closure(files: dict[str, FileDescriptorProto], root: str) -> set[str]:
    pending = [root]
    visited: set[str] = set()
    while pending:
        name = pending.pop()
        if name in visited:
            continue
        file = require_file(files, name)
        visited.add(name)
        pending.extend(file.dependency)
    return visited


def assert_development_dataset_leaf(files: dict[str, FileDescriptorProto]) -> None:
    leaf = require_file(files, DEVELOPMENT_DATA_FILE)
    assert set(leaf.dependency) == {COMMON_FILE}
    assert not leaf.enum_type
    assert not leaf.service
    assert not leaf.extension
    assert [message.name for message in leaf.message_type] == ["DevelopmentDatasetReference"]
    reference = leaf.message_type[0]
    assert {
        field.name: (field.number, field.type_name)
        for field in reference.field
    } == {
        "snapshot_ids": (1, ".loop.v1.SnapshotId"),
        "manifest_sha256": (2, ".loop.v1.Sha256Digest"),
    }


def assert_discovery_message_graph(
    descriptor: FileDescriptorSet, discovery_file: FileDescriptorProto
) -> None:
    messages: dict[str, Any] = {}
    for file in descriptor.file:
        for message in file.message_type:
            messages[f".{file.package}.{message.name}"] = message

    service = next(
        service for service in discovery_file.service if service.name == "DiscoveryService"
    )
    pending = [
        type_name
        for method in service.method
        for type_name in (method.input_type, method.output_type)
    ]
    visited: set[str] = set()
    field_types: set[str] = set()
    while pending:
        type_name = pending.pop()
        lowered = type_name.lower()
        assert not any(token in lowered for token in DISCOVERY_FORBIDDEN_TYPE_TOKENS), type_name
        if type_name in visited:
            continue
        visited.add(type_name)
        message = messages.get(type_name)
        if message is None:
            continue
        for field in message.field:
            if field.type_name:
                field_types.add(field.type_name)
                pending.append(field.type_name)

    assert ".loop.discovery.v1.DiscoveryJobInput" in visited
    assert ".loop.discovery.v1.DiscoveryJobBudget" in visited
    assert ".loop.discovery.v1.DiscoveryJobHandle" in visited
    assert ".loop.v1.JobId" in visited
    assert ".loop.v1.JobRecord" not in visited
    assert ".loop.v1.JobSpecification" not in visited
    assert ".loop.v1.HoldoutBacktestJobInput" not in visited
    assert all(
        not any(token in field_type.lower() for token in DISCOVERY_FORBIDDEN_TYPE_TOKENS)
        for field_type in field_types
    )


def assert_provider_message_graph(
    descriptor: FileDescriptorSet, provider_file: FileDescriptorProto
) -> None:
    messages = descriptor_messages(descriptor)
    service = next(
        service for service in provider_file.service if service.name == "ProviderService"
    )
    reachable = reachable_message_types(
        messages,
        [
            type_name
            for method in service.method
            for type_name in (method.input_type, method.output_type)
        ],
    )
    for type_name in reachable:
        assert not any(token in type_name.lower() for token in PROVIDER_FORBIDDEN_TYPE_TOKENS), (
            type_name
        )

    assert ".loop.v1.ModelInvocation" in reachable
    assert ".loop.v1.ModelResponse" in reachable
    assert ".loop.v1.ModelStreamEvent" in reachable
    assert ".loop.v1.JobRecord" not in reachable
    assert ".loop.v1.JobSpecification" not in reachable
    assert ".loop.v1.HoldoutGrantReference" not in reachable


def assert_provider_language_exports() -> None:
    repository = Path(__file__).parents[2]
    typescript_entrypoint = repository / "packages/protocol-ts/src/wire/provider.ts"
    typescript_source = typescript_entrypoint.read_text(encoding="utf-8").lower()
    assert 'export * from "../generated/loop/v1/common_pb.js"' not in typescript_source
    for token in (
        "backtest",
        "factor",
        "holdout",
        "job_pb",
        "jobid",
        "research",
    ):
        assert token not in typescript_source

    generated_typescript = (
        (repository / "packages/protocol-ts/src/generated/loop/provider/v1/service_pb.ts")
        .read_text(encoding="utf-8")
        .lower()
    )
    for token in ("job_pb", "holdout_pb", "research_pb", "backtest_pb", "factor_pb"):
        assert token not in generated_typescript

    generated_python = (
        (repository / "python/loop_protocol/src/loop/provider/v1/service_pb2.py")
        .read_text(encoding="utf-8")
        .lower()
    )
    for token in ("job_pb2", "holdout_pb2", "research_pb2", "backtest_pb2", "factor_pb2"):
        assert token not in generated_python


def assert_discovery_language_exports() -> None:
    repository = Path(__file__).parents[2]
    typescript_entrypoint = repository / "packages/protocol-ts/src/wire/discovery.ts"
    typescript_source = typescript_entrypoint.read_text(encoding="utf-8")
    assert "job_pb" not in typescript_source
    assert "holdout" not in typescript_source.lower()
    assert 'from "../generated/loop/v1/data_pb.js"' not in typescript_source
    assert 'from "../generated/loop/v1/development_data_pb.js"' in typescript_source

    generated_typescript = (
        repository / "packages/protocol-ts/src/generated/loop/discovery/v1/service_pb.ts"
    ).read_text(encoding="utf-8")
    assert "job_pb" not in generated_typescript
    assert "holdout_pb" not in generated_typescript
    assert 'from "../../v1/data_pb.js"' not in generated_typescript
    assert 'from "../../v1/development_data_pb.js"' in generated_typescript

    generated_python = (
        repository / "python/loop_protocol/src/loop/discovery/v1/service_pb2.py"
    ).read_text(encoding="utf-8")
    assert "job_pb2" not in generated_python
    assert "holdout_pb2" not in generated_python
    assert "loop_dot_v1_dot_data__pb2" not in generated_python
    assert "loop_dot_v1_dot_development__data__pb2" in generated_python


def assert_research_message_graph(
    descriptor: FileDescriptorSet, research_file: FileDescriptorProto
) -> None:
    messages = descriptor_messages(descriptor)
    service = next(
        service for service in research_file.service if service.name == "ResearchService"
    )
    reachable = reachable_message_types(
        messages,
        [
            type_name
            for method in service.method
            for type_name in (method.input_type, method.output_type)
        ],
    )
    for type_name in reachable:
        assert not any(token in type_name.lower() for token in RESEARCH_FORBIDDEN_TYPE_TOKENS), (
            type_name
        )

    for required in (
        ".loop.research.v1.FactorEvaluationInput",
        ".loop.research.v1.BacktestInput",
        ".loop.research.v1.ReconciliationInput",
        ".loop.research.v1.ResearchJobBudget",
        ".loop.research.v1.ResearchJobHandle",
        ".loop.v1.DevelopmentDatasetReference",
        ".loop.v1.FactorSpec",
        ".loop.v1.ResearchProvenanceFingerprint",
        ".loop.v1.ReturnDefinition",
    ):
        assert required in reachable, required

    response_reachable = reachable_message_types(
        messages, [method.output_type for method in service.method]
    )
    assert response_reachable == {
        ".google.protobuf.Timestamp",
        ".loop.research.v1.EnqueueBacktestResponse",
        ".loop.research.v1.EnqueueFactorEvaluationResponse",
        ".loop.research.v1.EnqueueReconciliationResponse",
        ".loop.research.v1.ResearchJobHandle",
        ".loop.research.v1.ResearchJobStatus",
        ".loop.v1.JobId",
    }


def assert_research_language_exports() -> None:
    repository = Path(__file__).parents[2]
    typescript_entrypoint = repository / "packages/protocol-ts/src/wire/research.ts"
    typescript_source = typescript_entrypoint.read_text(encoding="utf-8").lower()
    assert "export *" not in typescript_source
    for forbidden_import in (
        "backtest_pb",
        'from "../generated/loop/v1/data_pb.js"',
        "holdout_pb",
        "job_pb",
    ):
        assert forbidden_import not in typescript_source
    assert 'from "../generated/loop/v1/development_data_pb.js"' in typescript_source
    for forbidden_export in (
        "backtestspec",
        "holdoutbacktestjobinput",
        "holdoutgrant",
        "joblease",
        "joboutcome",
        "jobrecord",
        "jobspecification",
        "samplerole",
        "samplewindow",
    ):
        assert forbidden_export not in typescript_source

    generated_typescript = (
        repository / "packages/protocol-ts/src/generated/loop/research/v1/service_pb.ts"
    ).read_text(encoding="utf-8").lower()
    generated_python = (
        repository / "python/loop_protocol/src/loop/research/v1/service_pb2.py"
    ).read_text(encoding="utf-8").lower()
    for forbidden_dependency in (
        'from "../../v1/backtest_pb.js"',
        'from "../../v1/data_pb.js"',
        'from "../../v1/holdout_pb.js"',
        'from "../../v1/job_pb.js"',
    ):
        assert forbidden_dependency not in generated_typescript
    assert 'from "../../v1/development_data_pb.js"' in generated_typescript
    for forbidden_dependency in (
        "loop_dot_v1_dot_backtest__pb2",
        "loop_dot_v1_dot_data__pb2",
        "loop_dot_v1_dot_holdout__pb2",
        "loop_dot_v1_dot_job__pb2",
    ):
        assert forbidden_dependency not in generated_python
    assert "loop_dot_v1_dot_development__data__pb2" in generated_python

    generated_rust = (
        repository / "crates/loop-protocol/src/generated/r#loop.research.v1.rs"
    ).read_text(encoding="utf-8")
    generated_rust_code = "\n".join(
        line for line in generated_rust.splitlines() if not line.lstrip().startswith("///")
    )
    for forbidden_type in (
        "BacktestSpec",
        "HoldoutBacktestJobInput",
        "HoldoutGrantReference",
        "JobLease",
        "JobOutcome",
        "JobRecord",
        "JobSpecification",
        "SampleRole",
        "SampleWindow",
    ):
        assert forbidden_type not in generated_rust_code


def assert_holdout_language_exports() -> None:
    repository = Path(__file__).parents[2]
    typescript_entrypoint = repository / "packages/protocol-ts/src/wire/holdout.ts"
    typescript_source = typescript_entrypoint.read_text(encoding="utf-8").lower()
    for forbidden_import in (
        "backtest_pb",
        "job_pb",
        "holdout-identity",
    ):
        assert forbidden_import not in typescript_source
    for forbidden_export in (
        "backtestspec",
        "jobspecification",
        "jobbudget",
        "jobinput",
    ):
        assert forbidden_export not in typescript_source

    generated_rust = (
        (repository / "crates/loop-protocol/src/generated/r#loop.holdout.v1.rs")
        .read_text(encoding="utf-8")
        .lower()
    )
    generated_python = (
        (repository / "python/loop_protocol/src/loop/holdout/v1/service_pb2.py")
        .read_text(encoding="utf-8")
        .lower()
    )
    for forbidden_token in (
        "backtestspec",
        "holdoutbacktestjobinput",
        "jobbudget",
        "jobrecord",
        "jobspecification",
    ):
        assert forbidden_token not in generated_rust
        assert forbidden_token not in generated_python


def assert_holdout_plan_boundary(
    descriptor: FileDescriptorSet, files: dict[str, FileDescriptorProto]
) -> None:
    service_file = require_file(files, HOLDOUT_SERVICE_FILE)
    core_file = require_file(files, HOLDOUT_FILE)
    job_file = require_file(files, JOB_FILE)
    common_file = require_file(files, COMMON_FILE)

    assert set(service_file.dependency) == {
        "google/protobuf/timestamp.proto",
        "loop/v1/artifact.proto",
        "loop/v1/common.proto",
        "loop/v1/holdout.proto",
    }
    closure = dependency_closure(files, HOLDOUT_SERVICE_FILE)
    for forbidden_file in (
        "loop/v1/backtest.proto",
        "loop/v1/factor.proto",
        "loop/v1/job.proto",
    ):
        assert forbidden_file not in closure, forbidden_file

    messages = descriptor_messages(descriptor)
    assert_service(
        service_file,
        "HoldoutService",
        {
            "RecordHoldoutApproval": (
                ".loop.holdout.v1.RecordHoldoutApprovalRequest",
                ".loop.holdout.v1.RecordHoldoutApprovalResponse",
            ),
            "RequestHoldoutGrant": (
                ".loop.holdout.v1.RequestHoldoutGrantRequest",
                ".loop.holdout.v1.RequestHoldoutGrantResponse",
            ),
            "ConsumeGrantAndEnqueueBacktest": (
                ".loop.holdout.v1.ConsumeGrantAndEnqueueBacktestRequest",
                ".loop.holdout.v1.ConsumeGrantAndEnqueueBacktestResponse",
            ),
            "GetHoldoutPeriod": (
                ".loop.holdout.v1.GetHoldoutPeriodRequest",
                ".loop.holdout.v1.GetHoldoutPeriodResponse",
            ),
            "GetHoldoutApprovalRecord": (
                ".loop.holdout.v1.GetHoldoutApprovalRecordRequest",
                ".loop.holdout.v1.GetHoldoutApprovalRecordResponse",
            ),
            "GetHoldoutGrant": (
                ".loop.holdout.v1.GetHoldoutGrantRequest",
                ".loop.holdout.v1.GetHoldoutGrantResponse",
            ),
        },
    )
    service = next(
        candidate for candidate in service_file.service if candidate.name == "HoldoutService"
    )
    assert not any(
        token in method.name.lower()
        for method in service.method
        for token in ("createperiod", "registerperiod", "upsertperiod")
    )

    forbidden_rpc_types = {
        ".loop.v1.BacktestSpec",
        ".loop.v1.FactorSpec",
        ".loop.v1.HoldoutBacktestJobInput",
        ".loop.v1.JobBudget",
        ".loop.v1.JobRecord",
        ".loop.v1.JobSpecification",
    }
    reachable = reachable_message_types(
        messages,
        [
            type_name
            for method in service.method
            for type_name in (method.input_type, method.output_type)
        ],
    )
    assert forbidden_rpc_types.isdisjoint(reachable), forbidden_rpc_types & reachable
    request_reachable = reachable_message_types(
        messages, [method.input_type for method in service.method]
    )
    assert ".loop.v1.HoldoutPeriod" not in request_reachable

    consume_request = require_message(service_file, "ConsumeGrantAndEnqueueBacktestRequest")
    assert_message_fields(
        consume_request,
        {
            "context": (1, ".loop.v1.CommandContext"),
            "grant_reference": (2, ".loop.v1.HoldoutGrantReference"),
            "expected_grant_revision": (3, ""),
            "expected_period_revision": (6, ""),
        },
    )
    assert_reserved(consume_request, {4, 5}, {"frozen_backtest_spec", "budget"})

    batch = require_message(service_file, "JobBatchHandle")
    assert_message_fields(
        batch,
        {
            "job_batch_id": (1, ".loop.v1.JobBatchId"),
            "holdout_grant_id": (2, ".loop.v1.HoldoutGrantId"),
            "holdout_evaluation_plan_id": (3, ".loop.v1.HoldoutEvaluationPlanId"),
            "evaluation_plan_sha256": (4, ".loop.v1.Sha256Digest"),
            "evaluation_plan_entry_count": (5, ""),
            "job_count": (6, ""),
            "job_ids": (7, ".loop.v1.JobId"),
            "revision": (8, ""),
            "created_at": (9, ".google.protobuf.Timestamp"),
        },
    )
    consume_response = require_message(service_file, "ConsumeGrantAndEnqueueBacktestResponse")
    assert_message_fields(
        consume_response,
        {
            "consumed_grant": (1, ".loop.v1.HoldoutGrantReference"),
            "job_batch": (2, ".loop.holdout.v1.JobBatchHandle"),
            "period_record": (3, ".loop.v1.HoldoutPeriodRecord"),
        },
    )

    plan = require_message(core_file, "HoldoutEvaluationPlanReference")
    assert_message_fields(
        plan,
        {
            "holdout_evaluation_plan_id": (1, ".loop.v1.HoldoutEvaluationPlanId"),
            "canonical_plan": (2, ".loop.v1.ArtifactRef"),
            "plan_sha256": (3, ".loop.v1.Sha256Digest"),
            "entry_count": (4, ""),
            "holdout_period_id": (5, ".loop.v1.HoldoutPeriodId"),
            "canonical_period_sha256": (6, ".loop.v1.Sha256Digest"),
        },
    )
    freeze = require_message(core_file, "FreezeManifestReference")
    assert field_signature(freeze)["holdout_evaluation_plan"] == (
        9,
        ".loop.v1.HoldoutEvaluationPlanReference",
    )

    period = require_message(core_file, "HoldoutPeriod")
    assert_message_fields(
        period,
        {
            "holdout_period_id": (1, ".loop.v1.HoldoutPeriodId"),
            "sample": (2, ".loop.v1.SampleWindow"),
            "snapshot_ids": (3, ".loop.v1.SnapshotId"),
            "snapshot_manifest_sha256": (4, ".loop.v1.Sha256Digest"),
            "canonical_period_sha256": (5, ".loop.v1.Sha256Digest"),
        },
    )

    assert_plan_binding(require_message(core_file, "HoldoutApprovalRecord"), 10)
    assert_plan_binding(require_message(core_file, "HoldoutApprovalRecordReference"), 8)
    assert_plan_binding(require_message(core_file, "HoldoutGrantReference"), 6)
    grant_record = require_message(core_file, "HoldoutGrantRecord")
    assert_plan_binding(grant_record, 7)
    record_approval = require_message(service_file, "RecordHoldoutApprovalRequest")
    assert_plan_binding(record_approval, 7, canonical_period_field_number=6)

    grant_request = require_message(service_file, "RequestHoldoutGrantRequest")
    grant_fields = field_signature(grant_request)
    assert grant_fields["freeze_manifest"] == (3, ".loop.v1.FreezeManifestReference")
    assert grant_fields["approval_record_ids"] == (
        4,
        ".loop.v1.HoldoutApprovalRecordId",
    )
    assert grant_fields["holdout_period_id"] == (6, ".loop.v1.HoldoutPeriodId")
    assert grant_fields["canonical_period_sha256"] == (8, ".loop.v1.Sha256Digest")

    internal_job = require_message(job_file, "HoldoutBacktestJobInput")
    assert_message_fields(
        internal_job,
        {
            "consumed_grant": (1, ".loop.v1.HoldoutGrantReference"),
            "consumed_grant_revision": (2, ""),
            "frozen_backtest_spec": (3, ".loop.v1.BacktestSpec"),
            "budget": (4, ".loop.v1.JobBudget"),
            "job_batch_id": (5, ".loop.v1.JobBatchId"),
            "holdout_evaluation_plan_id": (6, ".loop.v1.HoldoutEvaluationPlanId"),
            "evaluation_plan_sha256": (7, ".loop.v1.Sha256Digest"),
            "evaluation_plan_entry_index": (8, ""),
        },
    )

    for request_type in (method.input_type for method in service.method):
        request = messages[request_type]
        fields = field_signature(request)
        if "holdout_period_id" in fields:
            assert fields["canonical_period_sha256"][1] == ".loop.v1.Sha256Digest"

    for name in ("HoldoutEvaluationPlanId", "HoldoutPeriodId", "JobBatchId"):
        identifier = require_message(common_file, name)
        assert_message_fields(identifier, {"value": (1, "")})


def descriptor_messages(descriptor: FileDescriptorSet) -> dict[str, DescriptorProto]:
    return {
        f".{file.package}.{message.name}": message
        for file in descriptor.file
        for message in file.message_type
    }


def reachable_message_types(messages: dict[str, DescriptorProto], roots: list[str]) -> set[str]:
    pending = roots[:]
    visited: set[str] = set()
    while pending:
        type_name = pending.pop()
        if type_name in visited:
            continue
        visited.add(type_name)
        message = messages.get(type_name)
        if message is not None:
            pending.extend(field.type_name for field in message.field if field.type_name)
    return visited


def require_message(file: FileDescriptorProto, name: str) -> DescriptorProto:
    matches = [message for message in file.message_type if message.name == name]
    assert len(matches) == 1, f"expected one {name} message in {file.name}"
    return matches[0]


def field_signature(message: DescriptorProto) -> dict[str, tuple[int, str]]:
    return {field.name: (field.number, field.type_name) for field in message.field}


def assert_message_fields(message: DescriptorProto, expected: dict[str, tuple[int, str]]) -> None:
    assert field_signature(message) == expected, message.name


def assert_reserved(message: DescriptorProto, numbers: set[int], names: set[str]) -> None:
    reserved_numbers = {
        number
        for reserved_range in message.reserved_range
        for number in range(reserved_range.start, reserved_range.end)
    }
    assert numbers <= reserved_numbers
    assert names <= set(message.reserved_name)


def assert_plan_binding(
    message: DescriptorProto,
    first_field_number: int,
    *,
    canonical_period_field_number: int | None = None,
) -> None:
    fields = field_signature(message)
    assert fields["holdout_evaluation_plan_id"] == (
        first_field_number,
        ".loop.v1.HoldoutEvaluationPlanId",
    )
    assert fields["evaluation_plan_sha256"] == (
        first_field_number + 1,
        ".loop.v1.Sha256Digest",
    )
    assert fields["evaluation_plan_entry_count"] == (first_field_number + 2, "")
    assert fields["canonical_period_sha256"] == (
        canonical_period_field_number or first_field_number + 3,
        ".loop.v1.Sha256Digest",
    )


def verify_operational_failure_fixture(path: Path) -> None:
    fixture = json.loads(path.read_text(encoding="ascii"))
    assert fixture["contract"] == "loop.rpc-operational-failure/v1"
    assert fixture["rpc"] == "/loop.provider.v1.ProviderService/InvokeModel"
    assert fixture["grpc_status_code"] == 14
    assert fixture["grpc_status_name"] == "UNAVAILABLE"
    assert fixture["response_body_base64"] == ""

    service_error = decode_service_error_detail(fixture)
    expected = fixture["expected"]
    assert service_error.category == ERROR_CATEGORY_DEPENDENCY
    assert service_error.category == ErrorCategory.Value(expected["category"])
    assert service_error.code == expected["code"]
    assert service_error.message == expected["message"]
    assert service_error.retryable is expected["retryable"]

    # A Protobuf payload has no self-describing message type. The rich-status
    # type URL is therefore validated before decoding. A forged rejection type
    # is rejected instead of interpreting operational bytes under that schema.
    forged: dict[str, Any] = {**fixture, "details": [dict(fixture["details"][0])]}
    forged["details"][0]["type_url"] = "type.googleapis.com/loop.v1.FactorRejection"
    try:
        decode_service_error_detail(forged)
    except AssertionError:
        pass
    else:
        raise AssertionError("an operational failure was accepted as FactorRejection")


def decode_service_error_detail(fixture: dict[str, Any]) -> ServiceError:
    assert fixture["grpc_status_code"] != 0, "operational failure must use non-OK gRPC status"
    assert fixture["response_body_base64"] == "", "non-OK RPC must not return an app response"
    details = fixture["details"]
    assert isinstance(details, list) and len(details) == 1
    detail = details[0]
    assert detail["type_url"] == "type.googleapis.com/loop.v1.ServiceError"
    assert detail["type_url"] != "type.googleapis.com/loop.v1.FactorRejection"
    return ServiceError.FromString(base64.b64decode(detail["value_base64"], validate=True))


if __name__ == "__main__":
    main()
