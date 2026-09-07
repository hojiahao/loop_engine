"""Canonical audit payloads and append-only event hash chains."""

from __future__ import annotations

import hashlib
import hmac
import json
import re
from dataclasses import dataclass
from enum import StrEnum
from typing import Any, Final, NoReturn

MAX_AUDIT_PAYLOAD_BYTES: Final = 256 * 1_024
MAX_AUDIT_TEXT_BYTES: Final = 4_096
MAX_AUDIT_ID_BYTES: Final = 128
MAX_HOLDOUT_APPROVAL_RECORDS: Final = 8

_PAYLOAD_DOMAIN: Final = b"loop.audit-payload/v1\x00"
_EVENT_DOMAIN: Final = b"loop.audit-event/v1\x00"
_EVENT_SCHEMA: Final = "loop.audit-event/v1"
_ZERO_SHA256: Final = "sha256:" + "0" * 64
_SHA256_RE = re.compile(r"^sha256:[0-9a-f]{64}$", re.ASCII)
_SCHEMA_RE = re.compile(r"^[a-z][a-z0-9_]*(?:\.[a-z][a-z0-9_]*)*$", re.ASCII)
_DOMAIN_ID_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._:-]*$", re.ASCII)
_TIMESTAMP_RE = re.compile(
    r"^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})\.(\d{9})Z$",
    re.ASCII,
)


class AuditErrorCode(StrEnum):
    INVALID_SCHEMA = "invalid_schema"
    UNSUPPORTED_SCHEMA = "unsupported_schema"
    NON_CANONICAL_PAYLOAD = "non_canonical_payload"
    PAYLOAD_DIGEST_MISMATCH = "payload_digest_mismatch"
    INVALID_DIGEST = "invalid_digest"
    INVALID_IDENTIFIER = "invalid_identifier"
    INVALID_TEXT = "invalid_text"
    INVALID_TIMESTAMP = "invalid_timestamp"
    INVALID_SEQUENCE = "invalid_sequence"
    INVALID_ENUM = "invalid_enum"
    INVALID_TARGET = "invalid_target"
    ACTION_PAYLOAD_MISMATCH = "action_payload_mismatch"
    ACTION_TARGET_MISMATCH = "action_target_mismatch"
    EVENT_DIGEST_MISMATCH = "event_digest_mismatch"
    LEDGER_MISMATCH = "ledger_mismatch"
    CHAIN_MISMATCH = "chain_mismatch"
    DUPLICATE_EVENT_ID = "duplicate_event_id"
    SIZE_LIMIT = "size_limit"


class AuditValidationError(ValueError):
    """An audit value did not satisfy the canonical v1 contract."""

    def __init__(self, code: AuditErrorCode, field: str, detail: str) -> None:
        self.code = code
        self.field = field
        super().__init__(f"{field} failed audit validation ({code.value}): {detail}")


class ActorKind(StrEnum):
    HUMAN = "human"
    SERVICE = "service"
    AGENT = "agent"
    SCHEDULER = "scheduler"


class AuditAction(StrEnum):
    COMMAND_ACCEPTED = "command_accepted"
    STATE_TRANSITIONED = "state_transitioned"
    FACTOR_ADMITTED = "factor_admitted"
    FACTOR_REJECTED = "factor_rejected"
    OVERRIDE_AUTHORIZED = "override_authorized"
    READMISSION_REQUESTED = "readmission_requested"
    READMISSION_DECIDED = "readmission_decided"
    HOLDOUT_GRANT_ISSUED = "holdout_grant_issued"
    HOLDOUT_GRANT_CONSUMED = "holdout_grant_consumed"
    ARTIFACT_EXPORTED = "artifact_exported"
    HOLDOUT_APPROVAL_RECORDED = "holdout_approval_recorded"


class AuditTargetKind(StrEnum):
    RUN_ID = "run_id"
    JOB_ID = "job_id"
    FACTOR_SPEC_ID = "factor_spec_id"
    BACKTEST_ID = "backtest_id"
    SNAPSHOT_ID = "snapshot_id"
    HOLDOUT_GRANT_ID = "holdout_grant_id"
    ARTIFACT_ID = "artifact_id"
    HOLDOUT_APPROVAL_RECORD_ID = "holdout_approval_record_id"


@dataclass(frozen=True, slots=True)
class AuditActor:
    actor_id: str
    kind: ActorKind
    display_name: str
    authenticated_subject: str


@dataclass(frozen=True, slots=True)
class AuditTarget:
    kind: AuditTargetKind
    value: str


@dataclass(frozen=True, slots=True)
class AuditPayload:
    schema_name: str
    schema_version: int
    canonical_bytes: bytes
    payload_sha256: str


@dataclass(frozen=True, slots=True)
class AuditEvent:
    audit_ledger_id: str
    sequence: int
    previous_event_sha256: str
    audit_event_id: str
    occurred_at: str
    correlation_id: str
    causation_id: str
    actor: AuditActor
    action: AuditAction
    target: AuditTarget
    payload: AuditPayload
    event_sha256: str


def canonicalize_audit_payload(
    schema_name: str,
    schema_version: int,
    submitted_bytes: bytes,
) -> AuditPayload:
    """Validate one registered payload schema and require exact writer bytes."""

    _validate_schema_identifier(schema_name, "payload.schema_name")
    _validate_schema_version(schema_version)
    if len(submitted_bytes) > MAX_AUDIT_PAYLOAD_BYTES:
        _fail(AuditErrorCode.SIZE_LIMIT, "payload.canonical_bytes", "payload exceeds 256 KiB")
    raw = _parse_json(submitted_bytes)

    if schema_name == "loop.audit.command_accepted" and schema_version == 1:
        values = _require_exact_object(raw, ("command", "request_id", "summary"))
        command = _require_string(values["command"], "payload.command")
        request_id = _require_string(values["request_id"], "payload.request_id")
        summary = _require_string(values["summary"], "payload.summary")
        _validate_schema_identifier(command, "payload.command")
        _validate_domain_id(request_id, "payload.request_id")
        _validate_text(summary, "payload.summary", require_nonempty=True)
        rewritten = (
            f'{{"command":{_write_json_string(command)},'
            f'"request_id":{_write_json_string(request_id)},'
            f'"summary":{_write_json_string(summary)}}}'
        ).encode()
    elif schema_name == "loop.audit.state_transitioned" and schema_version == 1:
        values = _require_exact_object(raw, ("from", "to", "reason"))
        from_state = _require_string(values["from"], "payload.from")
        to_state = _require_string(values["to"], "payload.to")
        reason = _require_string(values["reason"], "payload.reason")
        _validate_schema_identifier(from_state, "payload.from")
        _validate_schema_identifier(to_state, "payload.to")
        _validate_text(reason, "payload.reason", require_nonempty=True)
        rewritten = (
            f'{{"from":{_write_json_string(from_state)},'
            f'"to":{_write_json_string(to_state)},'
            f'"reason":{_write_json_string(reason)}}}'
        ).encode()
    elif schema_name == "loop.audit.factor_admitted" and schema_version == 1:
        values = _require_exact_object(raw, ("factor_spec_id", "decision", "evidence_artifact_id"))
        factor_spec_id = _require_string(values["factor_spec_id"], "payload.factor_spec_id")
        decision = _require_string(values["decision"], "payload.decision")
        evidence_artifact_id = _require_string(
            values["evidence_artifact_id"], "payload.evidence_artifact_id"
        )
        _validate_sha256(factor_spec_id, "payload.factor_spec_id")
        _validate_closed_enum(decision, ("admitted",), "payload.decision")
        _validate_sha256(evidence_artifact_id, "payload.evidence_artifact_id")
        rewritten = _write_payload_fields(
            (
                ("factor_spec_id", factor_spec_id),
                ("decision", decision),
                ("evidence_artifact_id", evidence_artifact_id),
            )
        )
    elif schema_name == "loop.audit.factor_rejected" and schema_version == 1:
        values = _require_exact_object(
            raw,
            ("factor_spec_id", "rejection_code", "reason", "evidence_artifact_id"),
        )
        factor_spec_id = _require_string(values["factor_spec_id"], "payload.factor_spec_id")
        rejection_code = _require_string(values["rejection_code"], "payload.rejection_code")
        reason = _require_string(values["reason"], "payload.reason")
        evidence_artifact_id = _require_string(
            values["evidence_artifact_id"], "payload.evidence_artifact_id"
        )
        _validate_sha256(factor_spec_id, "payload.factor_spec_id")
        _validate_closed_enum(
            rejection_code,
            (
                "duplicate",
                "previously_failed",
                "insufficient_coverage",
                "deterministic_filter",
                "performance",
                "correlation",
                "semantic_review",
                "policy",
            ),
            "payload.rejection_code",
        )
        _validate_text(reason, "payload.reason", require_nonempty=True)
        _validate_sha256(evidence_artifact_id, "payload.evidence_artifact_id")
        rewritten = _write_payload_fields(
            (
                ("factor_spec_id", factor_spec_id),
                ("rejection_code", rejection_code),
                ("reason", reason),
                ("evidence_artifact_id", evidence_artifact_id),
            )
        )
    elif schema_name == "loop.audit.override_authorized" and schema_version == 1:
        values = _require_exact_object(
            raw,
            (
                "factor_spec_id",
                "override_kind",
                "authorized_by_actor_id",
                "reason",
                "approval_reference",
                "evidence_artifact_id",
            ),
        )
        factor_spec_id = _require_string(values["factor_spec_id"], "payload.factor_spec_id")
        override_kind = _require_string(values["override_kind"], "payload.override_kind")
        authorized_by_actor_id = _require_string(
            values["authorized_by_actor_id"], "payload.authorized_by_actor_id"
        )
        reason = _require_string(values["reason"], "payload.reason")
        approval_reference = _require_string(
            values["approval_reference"], "payload.approval_reference"
        )
        evidence_artifact_id = _require_string(
            values["evidence_artifact_id"], "payload.evidence_artifact_id"
        )
        _validate_sha256(factor_spec_id, "payload.factor_spec_id")
        _validate_closed_enum(
            override_kind,
            ("force_admission", "readmission", "policy_exception"),
            "payload.override_kind",
        )
        _validate_domain_id(authorized_by_actor_id, "payload.authorized_by_actor_id")
        _validate_text(reason, "payload.reason", require_nonempty=True)
        _validate_domain_id(approval_reference, "payload.approval_reference")
        _validate_sha256(evidence_artifact_id, "payload.evidence_artifact_id")
        rewritten = _write_payload_fields(
            (
                ("factor_spec_id", factor_spec_id),
                ("override_kind", override_kind),
                ("authorized_by_actor_id", authorized_by_actor_id),
                ("reason", reason),
                ("approval_reference", approval_reference),
                ("evidence_artifact_id", evidence_artifact_id),
            )
        )
    elif schema_name == "loop.audit.readmission_requested" and schema_version == 1:
        values = _require_exact_object(
            raw,
            (
                "factor_spec_id",
                "original_rejection_event_id",
                "requested_by_actor_id",
                "reason",
            ),
        )
        factor_spec_id = _require_string(values["factor_spec_id"], "payload.factor_spec_id")
        original_rejection_event_id = _require_string(
            values["original_rejection_event_id"], "payload.original_rejection_event_id"
        )
        requested_by_actor_id = _require_string(
            values["requested_by_actor_id"], "payload.requested_by_actor_id"
        )
        reason = _require_string(values["reason"], "payload.reason")
        _validate_sha256(factor_spec_id, "payload.factor_spec_id")
        _validate_domain_id(original_rejection_event_id, "payload.original_rejection_event_id")
        _validate_domain_id(requested_by_actor_id, "payload.requested_by_actor_id")
        _validate_text(reason, "payload.reason", require_nonempty=True)
        rewritten = _write_payload_fields(
            (
                ("factor_spec_id", factor_spec_id),
                ("original_rejection_event_id", original_rejection_event_id),
                ("requested_by_actor_id", requested_by_actor_id),
                ("reason", reason),
            )
        )
    elif schema_name == "loop.audit.readmission_decided" and schema_version == 1:
        values = _require_exact_object(
            raw,
            (
                "factor_spec_id",
                "original_rejection_event_id",
                "disposition",
                "decided_by_actor_id",
                "reason",
            ),
        )
        factor_spec_id = _require_string(values["factor_spec_id"], "payload.factor_spec_id")
        original_rejection_event_id = _require_string(
            values["original_rejection_event_id"], "payload.original_rejection_event_id"
        )
        disposition = _require_string(values["disposition"], "payload.disposition")
        decided_by_actor_id = _require_string(
            values["decided_by_actor_id"], "payload.decided_by_actor_id"
        )
        reason = _require_string(values["reason"], "payload.reason")
        _validate_sha256(factor_spec_id, "payload.factor_spec_id")
        _validate_domain_id(original_rejection_event_id, "payload.original_rejection_event_id")
        _validate_closed_enum(
            disposition,
            ("admitted", "rejected", "quarantined"),
            "payload.disposition",
        )
        _validate_domain_id(decided_by_actor_id, "payload.decided_by_actor_id")
        _validate_text(reason, "payload.reason", require_nonempty=True)
        rewritten = _write_payload_fields(
            (
                ("factor_spec_id", factor_spec_id),
                ("original_rejection_event_id", original_rejection_event_id),
                ("disposition", disposition),
                ("decided_by_actor_id", decided_by_actor_id),
                ("reason", reason),
            )
        )
    elif schema_name == "loop.audit.holdout_grant_issued" and schema_version == 1:
        values = _require_exact_object(
            raw,
            (
                "holdout_grant_id",
                "holdout_period_id",
                "freeze_manifest_sha256",
                "holdout_evaluation_plan_id",
                "approval_records",
                "capability_class",
                "authorization_decision",
            ),
        )
        holdout_grant_id = _require_string(values["holdout_grant_id"], "payload.holdout_grant_id")
        holdout_period_id = _require_string(
            values["holdout_period_id"], "payload.holdout_period_id"
        )
        freeze_manifest_sha256 = _require_string(
            values["freeze_manifest_sha256"], "payload.freeze_manifest_sha256"
        )
        holdout_evaluation_plan_id = _require_string(
            values["holdout_evaluation_plan_id"], "payload.holdout_evaluation_plan_id"
        )
        approval_records = _require_holdout_approval_records(values["approval_records"])
        capability_class = _require_string(values["capability_class"], "payload.capability_class")
        authorization_decision = _require_string(
            values["authorization_decision"], "payload.authorization_decision"
        )
        _validate_domain_id(holdout_grant_id, "payload.holdout_grant_id")
        _validate_sha256(holdout_period_id, "payload.holdout_period_id")
        _validate_sha256(freeze_manifest_sha256, "payload.freeze_manifest_sha256")
        _validate_sha256(holdout_evaluation_plan_id, "payload.holdout_evaluation_plan_id")
        _validate_closed_enum(capability_class, ("holdout_evaluation",), "payload.capability_class")
        _validate_closed_enum(
            authorization_decision, ("authorized",), "payload.authorization_decision"
        )
        rewritten = _write_holdout_grant_issued_payload(
            holdout_grant_id,
            holdout_period_id,
            freeze_manifest_sha256,
            holdout_evaluation_plan_id,
            approval_records,
            capability_class,
            authorization_decision,
        )
    elif schema_name == "loop.audit.holdout_grant_consumed" and schema_version == 1:
        values = _require_exact_object(
            raw,
            (
                "holdout_grant_id",
                "holdout_period_id",
                "holdout_evaluation_plan_id",
                "job_batch_id",
                "capability_class",
                "authorization_decision",
            ),
        )
        holdout_grant_id = _require_string(values["holdout_grant_id"], "payload.holdout_grant_id")
        holdout_period_id = _require_string(
            values["holdout_period_id"], "payload.holdout_period_id"
        )
        holdout_evaluation_plan_id = _require_string(
            values["holdout_evaluation_plan_id"], "payload.holdout_evaluation_plan_id"
        )
        job_batch_id = _require_string(values["job_batch_id"], "payload.job_batch_id")
        capability_class = _require_string(values["capability_class"], "payload.capability_class")
        authorization_decision = _require_string(
            values["authorization_decision"], "payload.authorization_decision"
        )
        _validate_domain_id(holdout_grant_id, "payload.holdout_grant_id")
        _validate_sha256(holdout_period_id, "payload.holdout_period_id")
        _validate_sha256(holdout_evaluation_plan_id, "payload.holdout_evaluation_plan_id")
        _validate_domain_id(job_batch_id, "payload.job_batch_id")
        _validate_closed_enum(capability_class, ("holdout_evaluation",), "payload.capability_class")
        _validate_closed_enum(
            authorization_decision, ("authorized",), "payload.authorization_decision"
        )
        rewritten = _write_payload_fields(
            (
                ("holdout_grant_id", holdout_grant_id),
                ("holdout_period_id", holdout_period_id),
                ("holdout_evaluation_plan_id", holdout_evaluation_plan_id),
                ("job_batch_id", job_batch_id),
                ("capability_class", capability_class),
                ("authorization_decision", authorization_decision),
            )
        )
    elif schema_name == "loop.audit.artifact_exported" and schema_version == 1:
        values = _require_exact_object(
            raw, ("artifact_id", "export_class", "policy_id", "destination_class")
        )
        artifact_id = _require_string(values["artifact_id"], "payload.artifact_id")
        export_class = _require_string(values["export_class"], "payload.export_class")
        policy_id = _require_string(values["policy_id"], "payload.policy_id")
        destination_class = _require_string(
            values["destination_class"], "payload.destination_class"
        )
        _validate_sha256(artifact_id, "payload.artifact_id")
        _validate_closed_enum(
            export_class,
            ("research_report", "audit_bundle", "data_snapshot", "factor_values"),
            "payload.export_class",
        )
        _validate_sha256(policy_id, "payload.policy_id")
        _validate_closed_enum(
            destination_class,
            ("local_managed", "approved_object_store", "user_download"),
            "payload.destination_class",
        )
        rewritten = _write_payload_fields(
            (
                ("artifact_id", artifact_id),
                ("export_class", export_class),
                ("policy_id", policy_id),
                ("destination_class", destination_class),
            )
        )
    elif schema_name == "loop.audit.holdout_approval_recorded" and schema_version == 1:
        values = _require_exact_object(
            raw,
            (
                "holdout_approval_record_id",
                "holdout_period_id",
                "freeze_manifest_sha256",
                "approved_by_actor_id",
                "expires_at",
            ),
        )
        holdout_approval_record_id = _require_string(
            values["holdout_approval_record_id"], "payload.holdout_approval_record_id"
        )
        holdout_period_id = _require_string(
            values["holdout_period_id"], "payload.holdout_period_id"
        )
        freeze_manifest_sha256 = _require_string(
            values["freeze_manifest_sha256"], "payload.freeze_manifest_sha256"
        )
        approved_by_actor_id = _require_string(
            values["approved_by_actor_id"], "payload.approved_by_actor_id"
        )
        expires_at = _require_string(values["expires_at"], "payload.expires_at")
        _validate_domain_id(holdout_approval_record_id, "payload.holdout_approval_record_id")
        _validate_sha256(holdout_period_id, "payload.holdout_period_id")
        _validate_sha256(freeze_manifest_sha256, "payload.freeze_manifest_sha256")
        _validate_domain_id(approved_by_actor_id, "payload.approved_by_actor_id")
        _validate_timestamp_field(expires_at, "payload.expires_at")
        rewritten = _write_payload_fields(
            (
                ("holdout_approval_record_id", holdout_approval_record_id),
                ("holdout_period_id", holdout_period_id),
                ("freeze_manifest_sha256", freeze_manifest_sha256),
                ("approved_by_actor_id", approved_by_actor_id),
                ("expires_at", expires_at),
            )
        )
    else:
        _fail(
            AuditErrorCode.UNSUPPORTED_SCHEMA,
            "payload.schema_name",
            "audit payload schema/version is not registered",
        )

    if rewritten != submitted_bytes:
        _fail(
            AuditErrorCode.NON_CANONICAL_PAYLOAD,
            "payload.canonical_bytes",
            "payload bytes differ from the registered dedicated writer",
        )
    return AuditPayload(
        schema_name=schema_name,
        schema_version=schema_version,
        canonical_bytes=rewritten,
        payload_sha256=audit_payload_sha256(schema_name, schema_version, rewritten),
    )


def audit_payload_sha256(
    schema_name: str,
    schema_version: int,
    canonical_payload_bytes: bytes,
) -> str:
    _validate_schema_identifier(schema_name, "payload.schema_name")
    _validate_schema_version(schema_version)
    digest = hashlib.sha256(
        _PAYLOAD_DOMAIN
        + schema_name.encode("ascii")
        + b"\x00"
        + str(schema_version).encode("ascii")
        + b"\x00"
        + canonical_payload_bytes
    ).hexdigest()
    return f"sha256:{digest}"


def verify_audit_payload(payload: AuditPayload) -> None:
    _validate_sha256(payload.payload_sha256, "payload.payload_sha256")
    verified = canonicalize_audit_payload(
        payload.schema_name,
        payload.schema_version,
        payload.canonical_bytes,
    )
    if not hmac.compare_digest(verified.payload_sha256, payload.payload_sha256):
        _fail(
            AuditErrorCode.PAYLOAD_DIGEST_MISMATCH,
            "payload.payload_sha256",
            "claimed payload digest does not match canonical payload bytes",
        )


def canonical_audit_event_bytes(event: AuditEvent) -> bytes:
    verify_audit_payload(event.payload)
    _validate_event(event)
    canonical = (
        f'{{"schema":"{_EVENT_SCHEMA}",'
        f'"audit_ledger_id":{_write_json_string(event.audit_ledger_id)},'
        f'"sequence":"{event.sequence}",'
        f'"previous_event_sha256":"{event.previous_event_sha256}",'
        f'"audit_event_id":{_write_json_string(event.audit_event_id)},'
        f'"occurred_at":{_write_json_string(event.occurred_at)},'
        f'"correlation_id":{_write_json_string(event.correlation_id)},'
        f'"causation_id":{_write_json_string(event.causation_id)},'
        f'"actor":{{"actor_id":{_write_json_string(event.actor.actor_id)},'
        f'"kind":"{event.actor.kind.value}",'
        f'"display_name":{_write_json_string(event.actor.display_name)},'
        f'"authenticated_subject":{_write_json_string(event.actor.authenticated_subject)}}},'
        f'"action":"{event.action.value}",'
        f'"target":{{"kind":"{event.target.kind.value}",'
        f'"value":{_write_json_string(event.target.value)}}},'
        f'"payload":{{"schema_name":"{event.payload.schema_name}",'
        f'"schema_version":"{event.payload.schema_version}",'
        f'"payload_sha256":"{event.payload.payload_sha256}"}}}}'
    )
    return canonical.encode("utf-8")


def audit_event_sha256(event: AuditEvent) -> str:
    digest = hashlib.sha256(_EVENT_DOMAIN + canonical_audit_event_bytes(event)).hexdigest()
    return f"sha256:{digest}"


def verify_audit_event(event: AuditEvent) -> None:
    _validate_sha256(event.event_sha256, "event_sha256")
    computed = audit_event_sha256(event)
    if not hmac.compare_digest(computed, event.event_sha256):
        _fail(
            AuditErrorCode.EVENT_DIGEST_MISMATCH,
            "event_sha256",
            "claimed event digest does not match canonical event bytes",
        )


def verify_audit_chain(events: tuple[AuditEvent, ...] | list[AuditEvent]) -> None:
    if not events:
        return
    ledger_id = events[0].audit_ledger_id
    previous = _ZERO_SHA256
    expected_sequence = 1
    event_ids: set[str] = set()
    for event in events:
        verify_audit_event(event)
        if event.audit_ledger_id != ledger_id:
            _fail(
                AuditErrorCode.LEDGER_MISMATCH,
                "audit_ledger_id",
                "all events in one verified chain must use the same ledger ID",
            )
        if event.sequence != expected_sequence:
            _fail(
                AuditErrorCode.INVALID_SEQUENCE,
                "sequence",
                "audit sequence must begin at one and increase exactly by one",
            )
        if event.previous_event_sha256 != previous:
            _fail(
                AuditErrorCode.CHAIN_MISMATCH,
                "previous_event_sha256",
                "event does not commit to the immediately preceding event digest",
            )
        if event.audit_event_id in event_ids:
            _fail(
                AuditErrorCode.DUPLICATE_EVENT_ID,
                "audit_event_id",
                "audit event IDs must be unique within a chain",
            )
        event_ids.add(event.audit_event_id)
        previous = event.event_sha256
        expected_sequence += 1


def _validate_event(event: AuditEvent) -> None:
    _validate_domain_id(event.audit_ledger_id, "audit_ledger_id")
    if (
        not isinstance(event.sequence, int)
        or isinstance(event.sequence, bool)
        or not 1 <= event.sequence < 2**64
    ):
        _fail(AuditErrorCode.INVALID_SEQUENCE, "sequence", "sequence must be a positive uint64")
    _validate_sha256(event.previous_event_sha256, "previous_event_sha256")
    _validate_domain_id(event.audit_event_id, "audit_event_id")
    _validate_timestamp(event.occurred_at)
    _validate_domain_id(event.correlation_id, "correlation_id")
    _validate_domain_id(event.causation_id, "causation_id")
    _validate_domain_id(event.actor.actor_id, "actor.actor_id")
    if not isinstance(event.actor.kind, ActorKind):
        _fail(AuditErrorCode.INVALID_ENUM, "actor.kind", "unknown actor kind")
    _validate_text(event.actor.display_name, "actor.display_name", require_nonempty=False)
    _validate_text(
        event.actor.authenticated_subject,
        "actor.authenticated_subject",
        require_nonempty=True,
    )
    if not isinstance(event.action, AuditAction):
        _fail(AuditErrorCode.INVALID_ENUM, "action", "unknown audit action")
    if not isinstance(event.target.kind, AuditTargetKind):
        _fail(AuditErrorCode.INVALID_TARGET, "target.kind", "unknown audit target kind")
    if event.target.kind in (AuditTargetKind.FACTOR_SPEC_ID, AuditTargetKind.ARTIFACT_ID):
        if not _SHA256_RE.fullmatch(event.target.value):
            _fail(
                AuditErrorCode.INVALID_TARGET,
                "target.value",
                "content-addressed target requires a full SHA-256 identity",
            )
    else:
        try:
            _validate_domain_id(event.target.value, "target.value")
        except AuditValidationError:
            _fail(
                AuditErrorCode.INVALID_TARGET,
                "target.value",
                "target value does not satisfy its typed ID encoding",
            )
    _validate_action_binding(event)


def _validate_action_binding(event: AuditEvent) -> None:
    bindings: dict[AuditAction, tuple[str, tuple[AuditTargetKind, ...]]] = {
        AuditAction.COMMAND_ACCEPTED: (
            "loop.audit.command_accepted",
            (
                AuditTargetKind.RUN_ID,
                AuditTargetKind.JOB_ID,
                AuditTargetKind.FACTOR_SPEC_ID,
                AuditTargetKind.BACKTEST_ID,
                AuditTargetKind.SNAPSHOT_ID,
                AuditTargetKind.ARTIFACT_ID,
            ),
        ),
        AuditAction.STATE_TRANSITIONED: (
            "loop.audit.state_transitioned",
            (
                AuditTargetKind.RUN_ID,
                AuditTargetKind.JOB_ID,
                AuditTargetKind.BACKTEST_ID,
                AuditTargetKind.SNAPSHOT_ID,
            ),
        ),
        AuditAction.FACTOR_ADMITTED: (
            "loop.audit.factor_admitted",
            (AuditTargetKind.FACTOR_SPEC_ID,),
        ),
        AuditAction.FACTOR_REJECTED: (
            "loop.audit.factor_rejected",
            (AuditTargetKind.FACTOR_SPEC_ID,),
        ),
        AuditAction.OVERRIDE_AUTHORIZED: (
            "loop.audit.override_authorized",
            (AuditTargetKind.FACTOR_SPEC_ID,),
        ),
        AuditAction.READMISSION_REQUESTED: (
            "loop.audit.readmission_requested",
            (AuditTargetKind.FACTOR_SPEC_ID,),
        ),
        AuditAction.READMISSION_DECIDED: (
            "loop.audit.readmission_decided",
            (AuditTargetKind.FACTOR_SPEC_ID,),
        ),
        AuditAction.HOLDOUT_GRANT_ISSUED: (
            "loop.audit.holdout_grant_issued",
            (AuditTargetKind.HOLDOUT_GRANT_ID,),
        ),
        AuditAction.HOLDOUT_GRANT_CONSUMED: (
            "loop.audit.holdout_grant_consumed",
            (AuditTargetKind.HOLDOUT_GRANT_ID,),
        ),
        AuditAction.ARTIFACT_EXPORTED: (
            "loop.audit.artifact_exported",
            (AuditTargetKind.ARTIFACT_ID,),
        ),
        AuditAction.HOLDOUT_APPROVAL_RECORDED: (
            "loop.audit.holdout_approval_recorded",
            (AuditTargetKind.HOLDOUT_APPROVAL_RECORD_ID,),
        ),
    }
    expected_schema, allowed_targets = bindings[event.action]
    if event.payload.schema_name != expected_schema or event.payload.schema_version != 1:
        _fail(
            AuditErrorCode.ACTION_PAYLOAD_MISMATCH,
            "action",
            "audit action is not bound to the payload schema/version",
        )
    if event.target.kind not in allowed_targets:
        _fail(
            AuditErrorCode.ACTION_TARGET_MISMATCH,
            "target.kind",
            "audit action does not permit this target kind",
        )

    if event.action in (AuditAction.COMMAND_ACCEPTED, AuditAction.STATE_TRANSITIONED):
        return
    raw = _parse_json(event.payload.canonical_bytes)
    if not isinstance(raw, _ObjectPairs) or not raw:
        _fail(
            AuditErrorCode.NON_CANONICAL_PAYLOAD,
            "payload.canonical_bytes",
            "subject-bearing audit payload must be a non-empty object",
        )
    subject = _require_string(raw[0][1], f"payload.{raw[0][0]}")
    if not hmac.compare_digest(subject, event.target.value):
        _fail(
            AuditErrorCode.ACTION_TARGET_MISMATCH,
            "target.value",
            "subject-bearing payload identity does not match the typed target value",
        )


class _ObjectPairs(list[tuple[str, Any]]):
    pass


def _parse_json(submitted_bytes: bytes) -> Any:
    if submitted_bytes.startswith(b"\xef\xbb\xbf"):
        _fail(
            AuditErrorCode.NON_CANONICAL_PAYLOAD,
            "payload.canonical_bytes",
            "UTF-8 BOM is forbidden",
        )
    try:
        text = submitted_bytes.decode("utf-8")
        return json.loads(
            text,
            object_pairs_hook=_ObjectPairs,
            parse_int=_reject_json_number,
            parse_float=_reject_json_number,
            parse_constant=_reject_json_constant,
        )
    except (
        UnicodeDecodeError,
        json.JSONDecodeError,
        AuditValidationError,
        RecursionError,
        MemoryError,
    ) as error:
        _fail(
            AuditErrorCode.NON_CANONICAL_PAYLOAD,
            "payload.canonical_bytes",
            f"payload is not the registered closed JSON shape: {error}",
        )


def _require_exact_object(raw: Any, keys: tuple[str, ...]) -> dict[str, Any]:
    if not isinstance(raw, _ObjectPairs) or tuple(key for key, _ in raw) != keys:
        _fail(
            AuditErrorCode.NON_CANONICAL_PAYLOAD,
            "payload.canonical_bytes",
            "payload has unknown, missing, duplicate, or out-of-order fields",
        )
    return dict(raw)


def _require_holdout_approval_records(raw: Any) -> tuple[tuple[str, str, str], ...]:
    if not isinstance(raw, list) or isinstance(raw, _ObjectPairs):
        _fail(
            AuditErrorCode.NON_CANONICAL_PAYLOAD,
            "payload.approval_records",
            "approval_records must be an array",
        )
    if not 1 <= len(raw) <= MAX_HOLDOUT_APPROVAL_RECORDS:
        _fail(
            AuditErrorCode.NON_CANONICAL_PAYLOAD,
            "payload.approval_records",
            "approval_records must contain 1..=8 entries",
        )

    records: list[tuple[str, str, str]] = []
    record_ids: set[str] = set()
    record_digests: set[str] = set()
    actor_ids: set[str] = set()
    previous_actor_id: str | None = None
    for index, item in enumerate(raw):
        field = f"payload.approval_records[{index}]"
        values = _require_exact_object(
            item,
            (
                "holdout_approval_record_id",
                "approval_record_sha256",
                "approved_by_actor_id",
            ),
        )
        record_id = _require_string(
            values["holdout_approval_record_id"], f"{field}.holdout_approval_record_id"
        )
        record_sha256 = _require_string(
            values["approval_record_sha256"], f"{field}.approval_record_sha256"
        )
        actor_id = _require_string(values["approved_by_actor_id"], f"{field}.approved_by_actor_id")
        _validate_domain_id(record_id, f"{field}.holdout_approval_record_id")
        _validate_sha256(record_sha256, f"{field}.approval_record_sha256")
        _validate_domain_id(actor_id, f"{field}.approved_by_actor_id")
        if previous_actor_id is not None and actor_id <= previous_actor_id:
            _fail(
                AuditErrorCode.NON_CANONICAL_PAYLOAD,
                "payload.approval_records",
                "approval_records must be in strictly increasing actor-ID order",
            )
        if record_id in record_ids or record_sha256 in record_digests or actor_id in actor_ids:
            _fail(
                AuditErrorCode.NON_CANONICAL_PAYLOAD,
                "payload.approval_records",
                "approval record IDs, digests, and actor IDs must each be unique",
            )
        records.append((record_id, record_sha256, actor_id))
        record_ids.add(record_id)
        record_digests.add(record_sha256)
        actor_ids.add(actor_id)
        previous_actor_id = actor_id
    return tuple(records)


def _require_string(value: Any, field: str) -> str:
    if not isinstance(value, str):
        _fail(AuditErrorCode.NON_CANONICAL_PAYLOAD, field, "payload field must be a string")
    _validate_unicode_scalar(value, field)
    return value


def _validate_schema_identifier(value: str, field: str) -> None:
    if not isinstance(value, str) or not _SCHEMA_RE.fullmatch(value) or len(value.encode()) > 128:
        _fail(
            AuditErrorCode.INVALID_SCHEMA,
            field,
            "value must be a dot-qualified lowercase ASCII identifier",
        )


def _validate_schema_version(value: int) -> None:
    if not isinstance(value, int) or isinstance(value, bool) or not 1 <= value < 2**32:
        _fail(
            AuditErrorCode.INVALID_SCHEMA,
            "payload.schema_version",
            "schema version must be a positive uint32",
        )


def _validate_domain_id(value: str, field: str) -> None:
    if (
        not isinstance(value, str)
        or not _DOMAIN_ID_RE.fullmatch(value)
        or len(value.encode("ascii")) > MAX_AUDIT_ID_BYTES
    ):
        _fail(
            AuditErrorCode.INVALID_IDENTIFIER,
            field,
            "identifier must be 1..=128 bytes in the canonical ASCII domain alphabet",
        )


def _validate_text(value: str, field: str, *, require_nonempty: bool) -> None:
    if not isinstance(value, str):
        _fail(AuditErrorCode.INVALID_TEXT, field, "text must be a string")
    _validate_unicode_scalar(value, field)
    if (require_nonempty and not value) or len(value.encode("utf-8")) > MAX_AUDIT_TEXT_BYTES:
        _fail(
            AuditErrorCode.INVALID_TEXT,
            field,
            "text is empty or exceeds the audit text byte limit",
        )


def _validate_unicode_scalar(value: str, field: str) -> None:
    if any(0xD800 <= ord(character) <= 0xDFFF for character in value):
        _fail(AuditErrorCode.INVALID_TEXT, field, "text contains an unpaired surrogate")


def _validate_timestamp(value: str) -> None:
    match = _TIMESTAMP_RE.fullmatch(value) if isinstance(value, str) else None
    if match is None:
        _invalid_timestamp()
    year, month, day, hour, minute, second, _nanoseconds = map(int, match.groups())
    if (
        year < 1
        or not 1 <= month <= 12
        or not 1 <= day <= _days_in_month(year, month)
        or hour > 23
        or minute > 59
        or second > 59
    ):
        _invalid_timestamp()


def _invalid_timestamp() -> NoReturn:
    _fail(
        AuditErrorCode.INVALID_TIMESTAMP,
        "occurred_at",
        "timestamp must be a valid UTC instant with exactly nine fractional digits",
    )


def _days_in_month(year: int, month: int) -> int:
    if month == 2:
        return 29 if year % 400 == 0 or (year % 4 == 0 and year % 100 != 0) else 28
    return 30 if month in (4, 6, 9, 11) else 31


def _validate_sha256(value: str, field: str) -> None:
    if not isinstance(value, str) or not _SHA256_RE.fullmatch(value):
        _fail(
            AuditErrorCode.INVALID_DIGEST,
            field,
            "digest must use sha256: and 64 lowercase hexadecimal digits",
        )


def _validate_closed_enum(value: str, allowed: tuple[str, ...], field: str) -> None:
    if value not in allowed:
        _fail(
            AuditErrorCode.INVALID_ENUM,
            field,
            "value is not registered in the closed audit payload enum",
        )


def _validate_timestamp_field(value: str, field: str) -> None:
    try:
        _validate_timestamp(value)
    except AuditValidationError:
        _fail(
            AuditErrorCode.INVALID_TIMESTAMP,
            field,
            "timestamp must be a valid UTC instant with exactly nine fractional digits",
        )


def _write_payload_fields(fields: tuple[tuple[str, str], ...]) -> bytes:
    return (
        "{" + ",".join(f'"{name}":{_write_json_string(value)}' for name, value in fields) + "}"
    ).encode("utf-8")


def _write_holdout_grant_issued_payload(
    holdout_grant_id: str,
    holdout_period_id: str,
    freeze_manifest_sha256: str,
    holdout_evaluation_plan_id: str,
    approval_records: tuple[tuple[str, str, str], ...],
    capability_class: str,
    authorization_decision: str,
) -> bytes:
    records = ",".join(
        "{"
        f'"holdout_approval_record_id":{_write_json_string(record_id)},'
        f'"approval_record_sha256":{_write_json_string(record_sha256)},'
        f'"approved_by_actor_id":{_write_json_string(actor_id)}'
        "}"
        for record_id, record_sha256, actor_id in approval_records
    )
    return (
        "{"
        f'"holdout_grant_id":{_write_json_string(holdout_grant_id)},'
        f'"holdout_period_id":{_write_json_string(holdout_period_id)},'
        f'"freeze_manifest_sha256":{_write_json_string(freeze_manifest_sha256)},'
        f'"holdout_evaluation_plan_id":{_write_json_string(holdout_evaluation_plan_id)},'
        f'"approval_records":[{records}],'
        f'"capability_class":{_write_json_string(capability_class)},'
        f'"authorization_decision":{_write_json_string(authorization_decision)}'
        "}"
    ).encode()


def _write_json_string(value: str) -> str:
    _validate_unicode_scalar(value, "json_string")
    output = ['"']
    escapes = {
        '"': '\\"',
        "\\": "\\\\",
        "\b": "\\b",
        "\t": "\\t",
        "\n": "\\n",
        "\f": "\\f",
        "\r": "\\r",
    }
    for character in value:
        if character in escapes:
            output.append(escapes[character])
        elif ord(character) <= 0x1F:
            output.append(f"\\u00{ord(character):02x}")
        else:
            output.append(character)
    output.append('"')
    return "".join(output)


def _reject_json_number(value: str) -> NoReturn:
    _fail(
        AuditErrorCode.NON_CANONICAL_PAYLOAD,
        "payload.canonical_bytes",
        f"JSON number token is forbidden: {value}",
    )


def _reject_json_constant(value: str) -> NoReturn:
    _fail(
        AuditErrorCode.NON_CANONICAL_PAYLOAD,
        "payload.canonical_bytes",
        f"JSON constant is forbidden: {value}",
    )


def _fail(code: AuditErrorCode, field: str, detail: str) -> NoReturn:
    raise AuditValidationError(code, field, detail)
