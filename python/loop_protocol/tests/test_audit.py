from __future__ import annotations

import json
from dataclasses import replace
from pathlib import Path
from typing import Any

import pytest

from loop_protocol import (
    ActorKind,
    AuditAction,
    AuditActor,
    AuditErrorCode,
    AuditEvent,
    AuditTarget,
    AuditTargetKind,
    AuditValidationError,
    audit_event_sha256,
    audit_payload_sha256,
    canonical_audit_event_bytes,
    canonicalize_audit_payload,
    verify_audit_chain,
    verify_audit_event,
)

FIXTURE_PATH = (
    Path(__file__).resolve().parents[3]
    / "fixtures"
    / "contracts"
    / "audit"
    / "v1"
    / "canonical_chain_vectors.json"
)
ACTION_BINDING_FIXTURE_PATH = FIXTURE_PATH.with_name("action_binding_vectors.json")
ZERO_SHA256 = "sha256:" + "0" * 64


def test_shared_chain_vectors_are_byte_and_digest_exact() -> None:
    fixture = _fixture()
    assert fixture["schema"] == "loop.audit-conformance/v1"
    events = _chain(fixture)
    for event, vector in zip(events, fixture["accepted_chain"], strict=True):
        assert event.payload.canonical_bytes == vector["payload"]["canonical_utf8"].encode()
        assert event.payload.payload_sha256 == vector["payload"]["payload_sha256"]
        assert canonical_audit_event_bytes(event) == vector["canonical_event_utf8"].encode()
        assert audit_event_sha256(event) == vector["event_sha256"]
    verify_audit_chain(events)


def test_every_action_and_typed_target_enum_spelling_is_closed() -> None:
    fixture = _fixture()
    for value in fixture["action_values"]:
        assert AuditAction(value).value == value
    with pytest.raises(ValueError):
        AuditAction("unknown")

    for vector in fixture["target_vectors"]:
        assert AuditTargetKind(vector["kind"]).value == vector["kind"]
    with pytest.raises(ValueError):
        AuditTargetKind("unknown")


def test_shared_action_registry_binds_schema_target_and_subject_before_hashing() -> None:
    fixture = _action_binding_fixture()
    assert fixture["schema"] == "loop.audit-action-binding/v1"
    assert len(fixture["accepted"]) == 11
    malformed_names = {vector["name"] for vector in fixture["malformed_payloads"]}
    assert {
        "holdout_approval_records_unsorted",
        "holdout_approval_record_id_duplicate",
        "holdout_approval_record_digest_duplicate",
        "holdout_approval_actor_duplicate",
        "invalid_holdout_approval_record_digest",
        "invalid_holdout_approval_actor_id",
        "unknown_holdout_capability_class",
        "unknown_holdout_authorization_decision",
    } <= malformed_names
    for index, vector in enumerate(fixture["accepted"]):
        event = _action_event(fixture["event_envelope"], vector)
        assert event.payload.payload_sha256 == vector["payload_sha256"], vector["name"]
        assert canonical_audit_event_bytes(event) == vector["canonical_event_utf8"].encode(), (
            vector["name"]
        )
        assert audit_event_sha256(event) == vector["event_sha256"], vector["name"]
        verify_audit_event(replace(event, event_sha256=vector["event_sha256"]))

        wrong_action = fixture["accepted"][(index + 1) % len(fixture["accepted"])]
        wrong_schema = replace(
            event,
            action=AuditAction(wrong_action["action"]),
            target=AuditTarget(
                AuditTargetKind(wrong_action["target"]["kind"]),
                wrong_action["target"]["value"],
            ),
        )
        with pytest.raises(AuditValidationError) as captured:
            audit_event_sha256(wrong_schema)
        assert captured.value.code is AuditErrorCode.ACTION_PAYLOAD_MISMATCH, vector["name"]

        forbidden_target = replace(
            event,
            target=AuditTarget(
                AuditTargetKind(vector["forbidden_target"]["kind"]),
                vector["forbidden_target"]["value"],
            ),
        )
        with pytest.raises(AuditValidationError) as captured:
            audit_event_sha256(forbidden_target)
        assert captured.value.code is AuditErrorCode.ACTION_TARGET_MISMATCH, vector["name"]

        if "mismatched_target_value" in vector:
            mismatched_subject = replace(
                event,
                target=replace(event.target, value=vector["mismatched_target_value"]),
            )
            with pytest.raises(AuditValidationError) as captured:
                audit_event_sha256(mismatched_subject)
            assert captured.value.code is AuditErrorCode.ACTION_TARGET_MISMATCH, vector["name"]

    for vector in fixture["malformed_payloads"]:
        with pytest.raises(AuditValidationError) as captured:
            canonicalize_audit_payload(
                vector["payload_schema"], 1, vector["canonical_payload"].encode()
            )
        assert captured.value.code is AuditErrorCode(vector["expected_code"]), vector["name"]


def test_holdout_grant_audit_rejects_more_than_eight_approval_records() -> None:
    approval_records = [
        {
            "holdout_approval_record_id": f"holdout_approval.{index:02d}",
            "approval_record_sha256": f"sha256:{index:064x}",
            "approved_by_actor_id": f"actor.approver{index:02d}",
        }
        for index in range(1, 10)
    ]
    payload = json.dumps(
        {
            "holdout_grant_id": "grant.01",
            "holdout_period_id": "sha256:" + "d" * 64,
            "freeze_manifest_sha256": "sha256:" + "e" * 64,
            "holdout_evaluation_plan_id": "sha256:" + "f" * 64,
            "approval_records": approval_records,
            "capability_class": "holdout_evaluation",
            "authorization_decision": "authorized",
        },
        separators=(",", ":"),
    ).encode()

    with pytest.raises(AuditValidationError) as captured:
        canonicalize_audit_payload("loop.audit.holdout_grant_issued", 1, payload)
    assert captured.value.code is AuditErrorCode.NON_CANONICAL_PAYLOAD


def test_hostile_deep_json_fails_with_stable_audit_error() -> None:
    payload = ("[" * 2_048 + "{}" + "]" * 2_048).encode()
    with pytest.raises(AuditValidationError) as captured:
        canonicalize_audit_payload("loop.audit.command_accepted", 1, payload)
    assert captured.value.code is AuditErrorCode.NON_CANONICAL_PAYLOAD


def test_shared_tamper_reorder_genesis_and_cross_ledger_vectors_fail_closed() -> None:
    fixture = _fixture()
    for negative in fixture["negative_vectors"]:
        events = _chain(fixture)
        match negative["mutation"]:
            case "payload_tamper":
                payload = replace(
                    events[0].payload,
                    canonical_bytes=events[0].payload.canonical_bytes.replace(b"ok", b"no"),
                )
                events[0] = replace(events[0], payload=payload)
            case "event_tamper":
                actor = replace(
                    events[0].actor,
                    display_name=events[0].actor.display_name + "!",
                )
                events[0] = replace(events[0], actor=actor)
            case "event_reorder":
                events.reverse()
            case "previous_digest_tamper":
                events[1] = _seal(replace(events[1], previous_event_sha256="sha256:" + "1" * 64))
            case "cross_ledger_replay":
                events[1] = replace(events[1], audit_ledger_id="ledger.secondary")
            case "sequence_gap":
                events[1] = _seal(replace(events[1], sequence=3))
            case "invalid_genesis":
                events[0] = _seal(replace(events[0], previous_event_sha256="sha256:" + "2" * 64))
            case "invalid_timestamp":
                events[0] = replace(events[0], occurred_at="2026-09-05T06:30:00Z")
            case "duplicate_event_id":
                events[1] = _seal(replace(events[1], audit_event_id=events[0].audit_event_id))
            case "schema_mutation":
                payload = replace(events[0].payload, schema_name="loop.audit.unknown")
                events[0] = replace(events[0], payload=payload)
            case "schema_version_mutation":
                payload = replace(events[0].payload, schema_version=2)
                events[0] = replace(events[0], payload=payload)
            case unknown:
                raise AssertionError(f"unknown negative mutation {unknown}")

        with pytest.raises(AuditValidationError) as captured:
            verify_audit_chain(events)
        assert captured.value.code.value == negative["expected_code"], negative["name"]


def test_shared_boundaries_are_accepted_and_overruns_are_rejected() -> None:
    boundaries = _fixture()["boundary_vectors"]
    max_id_bytes = int(boundaries["max_domain_id_bytes"])
    max_text_bytes = int(boundaries["max_schema_text_bytes"])
    max_payload_bytes = int(boundaries["max_payload_bytes"])
    max_schema_version = int(boundaries["max_schema_version"])
    max_sequence = int(boundaries["max_sequence"])
    base = _chain(_fixture())[0]

    event = _seal(
        replace(
            base,
            audit_ledger_id="a" * max_id_bytes,
            sequence=max_sequence,
            actor=replace(base.actor, display_name="x" * max_text_bytes),
        )
    )
    verify_audit_event(event)

    summary = "x" * max_text_bytes
    payload = (
        f'{{"command":"research.run","request_id":"request.1","summary":"{summary}"}}'
    ).encode()
    canonicalize_audit_payload("loop.audit.command_accepted", 1, payload)
    with pytest.raises(AuditValidationError) as captured:
        canonicalize_audit_payload(
            "loop.audit.command_accepted",
            1,
            payload.replace(summary.encode(), (summary + "x").encode()),
        )
    assert captured.value.code is AuditErrorCode.INVALID_TEXT
    with pytest.raises(AuditValidationError) as captured:
        canonicalize_audit_payload(
            "loop.audit.command_accepted",
            1,
            b"x" * (max_payload_bytes + 1),
        )
    assert captured.value.code is AuditErrorCode.SIZE_LIMIT
    assert audit_payload_sha256("loop.audit.command_accepted", max_schema_version, b"").startswith(
        "sha256:"
    )
    with pytest.raises(AuditValidationError) as captured:
        audit_event_sha256(replace(base, audit_ledger_id="a" * (max_id_bytes + 1)))
    assert captured.value.code is AuditErrorCode.INVALID_IDENTIFIER


def test_payload_registry_rejects_noncanonical_and_unknown_documents() -> None:
    for value in (
        b' {"command":"research.run","request_id":"request.1","summary":"ok"}',
        b'{"request_id":"request.1","command":"research.run","summary":"ok"}',
        b'{"command":"research.run","request_id":"request.1","summary":"ok","extra":"x"}',
        b'{"command":"research.run","command":"research.run","request_id":"request.1","summary":"ok"}',
    ):
        with pytest.raises(AuditValidationError) as captured:
            canonicalize_audit_payload("loop.audit.command_accepted", 1, value)
        assert captured.value.code is AuditErrorCode.NON_CANONICAL_PAYLOAD

    with pytest.raises(AuditValidationError) as captured:
        canonicalize_audit_payload("loop.audit.unknown", 1, b"{}")
    assert captured.value.code is AuditErrorCode.UNSUPPORTED_SCHEMA


@pytest.mark.parametrize(
    "occurred_at",
    [
        "2026-02-29T00:00:00.000000000Z",
        "2026-09-05T06:30:60.000000000Z",
        "2026-09-05T06:30:00.00000000Z",
        "2026-09-05T06:30:00.000000000+00:00",
        "0000-01-01T00:00:00.000000000Z",
    ],
)
def test_timestamp_is_exact_and_round_trip_safe(occurred_at: str) -> None:
    event = replace(_chain(_fixture())[0], occurred_at=occurred_at)
    with pytest.raises(AuditValidationError) as captured:
        audit_event_sha256(event)
    assert captured.value.code is AuditErrorCode.INVALID_TIMESTAMP


def test_unpaired_surrogate_is_rejected() -> None:
    event = _chain(_fixture())[0]
    event = replace(event, actor=replace(event.actor, display_name="\ud800"))
    with pytest.raises(AuditValidationError) as captured:
        audit_event_sha256(event)
    assert captured.value.code is AuditErrorCode.INVALID_TEXT


def _fixture() -> dict[str, Any]:
    with FIXTURE_PATH.open(encoding="utf-8") as stream:
        return json.load(stream)  # type: ignore[no-any-return]


def _action_binding_fixture() -> dict[str, Any]:
    with ACTION_BINDING_FIXTURE_PATH.open(encoding="utf-8") as stream:
        return json.load(stream)  # type: ignore[no-any-return]


def _chain(fixture: dict[str, Any]) -> list[AuditEvent]:
    return [_event(vector) for vector in fixture["accepted_chain"]]


def _event(vector: dict[str, Any]) -> AuditEvent:
    payload_vector = vector["payload"]
    payload = canonicalize_audit_payload(
        payload_vector["schema_name"],
        int(payload_vector["schema_version"]),
        payload_vector["canonical_utf8"].encode(),
    )
    assert payload.payload_sha256 == payload_vector["payload_sha256"]
    raw = vector["event"]
    actor = raw["actor"]
    target = raw["target"]
    return AuditEvent(
        audit_ledger_id=raw["audit_ledger_id"],
        sequence=int(raw["sequence"]),
        previous_event_sha256=raw["previous_event_sha256"],
        audit_event_id=raw["audit_event_id"],
        occurred_at=raw["occurred_at"],
        correlation_id=raw["correlation_id"],
        causation_id=raw["causation_id"],
        actor=AuditActor(
            actor_id=actor["actor_id"],
            kind=ActorKind(actor["kind"]),
            display_name=actor["display_name"],
            authenticated_subject=actor["authenticated_subject"],
        ),
        action=AuditAction(raw["action"]),
        target=AuditTarget(AuditTargetKind(target["kind"]), target["value"]),
        payload=payload,
        event_sha256=vector["event_sha256"],
    )


def _action_event(envelope: dict[str, Any], vector: dict[str, Any]) -> AuditEvent:
    actor = envelope["actor"]
    return AuditEvent(
        audit_ledger_id=envelope["audit_ledger_id"],
        sequence=int(envelope["sequence"]),
        previous_event_sha256=envelope["previous_event_sha256"],
        audit_event_id=envelope["audit_event_id"],
        occurred_at=envelope["occurred_at"],
        correlation_id=envelope["correlation_id"],
        causation_id=envelope["causation_id"],
        actor=AuditActor(
            actor_id=actor["actor_id"],
            kind=ActorKind(actor["kind"]),
            display_name=actor["display_name"],
            authenticated_subject=actor["authenticated_subject"],
        ),
        action=AuditAction(vector["action"]),
        target=AuditTarget(AuditTargetKind(vector["target"]["kind"]), vector["target"]["value"]),
        payload=canonicalize_audit_payload(
            vector["payload_schema"], 1, vector["canonical_payload"].encode()
        ),
        event_sha256=ZERO_SHA256,
    )


def _seal(event: AuditEvent) -> AuditEvent:
    candidate = replace(event, event_sha256=ZERO_SHA256)
    return replace(candidate, event_sha256=audit_event_sha256(candidate))
