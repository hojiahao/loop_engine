import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

import {
  type AuditActor,
  type AuditErrorCode,
  type AuditEvent,
  type AuditPayload,
  type AuditTarget,
  AuditValidationError,
  assertActorKind,
  assertAuditAction,
  assertAuditTargetKind,
  auditEventSha256,
  auditPayloadSha256,
  canonicalAuditEventBytes,
  canonicalizeAuditPayload,
  verifyAuditChain,
  verifyAuditEvent,
} from "../src/index.js";

interface Fixture {
  readonly schema: string;
  readonly accepted_chain: readonly AcceptedVector[];
  readonly action_values: readonly string[];
  readonly target_vectors: readonly { readonly kind: string; readonly value: string }[];
  readonly boundary_vectors: {
    readonly max_domain_id_bytes: string;
    readonly max_schema_text_bytes: string;
    readonly max_payload_bytes: string;
    readonly max_schema_version: string;
    readonly max_sequence: string;
  };
  readonly negative_vectors: readonly {
    readonly name: string;
    readonly mutation: string;
    readonly expected_code: AuditErrorCode;
  }[];
}

interface AcceptedVector {
  readonly name: string;
  readonly payload: {
    readonly schema_name: string;
    readonly schema_version: string;
    readonly canonical_utf8: string;
    readonly payload_sha256: string;
  };
  readonly event: {
    readonly audit_ledger_id: string;
    readonly sequence: string;
    readonly previous_event_sha256: string;
    readonly audit_event_id: string;
    readonly occurred_at: string;
    readonly correlation_id: string;
    readonly causation_id: string;
    readonly actor: {
      readonly actor_id: string;
      readonly kind: string;
      readonly display_name: string;
      readonly authenticated_subject: string;
    };
    readonly action: string;
    readonly target: { readonly kind: string; readonly value: string };
  };
  readonly canonical_event_utf8: string;
  readonly event_sha256: string;
}

interface ActionBindingFixture {
  readonly schema: string;
  readonly event_envelope: ActionEventEnvelope;
  readonly accepted: readonly ActionBindingVector[];
  readonly malformed_payloads: readonly {
    readonly name: string;
    readonly payload_schema: string;
    readonly canonical_payload: string;
    readonly expected_code: AuditErrorCode;
  }[];
}

interface ActionEventEnvelope {
  readonly audit_ledger_id: string;
  readonly sequence: string;
  readonly previous_event_sha256: string;
  readonly audit_event_id: string;
  readonly occurred_at: string;
  readonly correlation_id: string;
  readonly causation_id: string;
  readonly actor: {
    readonly actor_id: string;
    readonly kind: string;
    readonly display_name: string;
    readonly authenticated_subject: string;
  };
}

interface ActionBindingVector {
  readonly name: string;
  readonly action: string;
  readonly payload_schema: string;
  readonly canonical_payload: string;
  readonly target: { readonly kind: string; readonly value: string };
  readonly forbidden_target: { readonly kind: string; readonly value: string };
  readonly mismatched_target_value?: string;
  readonly invalid_target_values?: readonly string[];
  readonly payload_sha256: string;
  readonly canonical_event_utf8: string;
  readonly event_sha256: string;
}

const fixture = JSON.parse(
  readFileSync(
    new URL("../../../fixtures/contracts/audit/v1/canonical_chain_vectors.json", import.meta.url),
    "utf8",
  ),
) as Fixture;
const actionBindingFixture = JSON.parse(
  readFileSync(
    new URL("../../../fixtures/contracts/audit/v1/action_binding_vectors.json", import.meta.url),
    "utf8",
  ),
) as ActionBindingFixture;

const encoder = new TextEncoder();
const decoder = new TextDecoder();
const zeros = `sha256:${"0".repeat(64)}`;

describe("audit canonicalization v1", () => {
  it("matches the shared multi-event chain byte for byte", () => {
    expect(fixture.schema).toBe("loop.audit-conformance/v1");
    const events = chain();
    for (const [index, event] of events.entries()) {
      const vector = fixture.accepted_chain[index];
      expect(vector).toBeDefined();
      expect(decoder.decode(event.payload.canonicalBytes), vector?.name).toBe(
        vector?.payload.canonical_utf8,
      );
      expect(event.payload.payloadSha256, vector?.name).toBe(vector?.payload.payload_sha256);
      expect(decoder.decode(canonicalAuditEventBytes(event)), vector?.name).toBe(
        vector?.canonical_event_utf8,
      );
      expect(auditEventSha256(event), vector?.name).toBe(vector?.event_sha256);
    }
    expect(() => verifyAuditChain(events)).not.toThrow();
  });

  it("closes every action and typed target enum spelling", () => {
    for (const action of fixture.action_values) {
      expect(assertAuditAction(action)).toBe(action);
    }
    expectAuditError(() => assertAuditAction("unknown"), "invalid_enum");

    for (const target of fixture.target_vectors) {
      expect(assertAuditTargetKind(target.kind)).toBe(target.kind);
    }
    expectAuditError(() => assertAuditTargetKind("unknown"), "invalid_target");
  });

  it("binds all 11 actions to exact schemas, targets, and payload subjects before hashing", () => {
    expect(actionBindingFixture.schema).toBe("loop.audit-action-binding/v1");
    expect(actionBindingFixture.accepted).toHaveLength(12);
    const malformedNames = new Set(
      actionBindingFixture.malformed_payloads.map((vector) => vector.name),
    );
    for (const required of [
      "holdout_approval_records_unsorted",
      "holdout_approval_record_id_duplicate",
      "holdout_approval_record_digest_duplicate",
      "holdout_approval_actor_duplicate",
      "invalid_holdout_approval_record_digest",
      "invalid_holdout_approval_actor_id",
      "unknown_holdout_capability_class",
      "unknown_holdout_authorization_decision",
    ]) {
      expect(malformedNames.has(required), `missing ${required} vector`).toBe(true);
    }
    for (const [index, vector] of actionBindingFixture.accepted.entries()) {
      const event = actionEvent(actionBindingFixture.event_envelope, vector);
      expect(event.payload.payloadSha256, `${vector.name} payload digest`).toBe(
        vector.payload_sha256,
      );
      expect(decoder.decode(canonicalAuditEventBytes(event)), `${vector.name} event bytes`).toBe(
        vector.canonical_event_utf8,
      );
      expect(auditEventSha256(event), `${vector.name} event digest`).toBe(vector.event_sha256);
      const sealed = { ...event, eventSha256: vector.event_sha256 };
      expect(() => verifyAuditEvent(sealed), vector.name).not.toThrow();

      for (const value of vector.invalid_target_values ?? []) {
        expectAuditError(
          () => auditEventSha256({ ...event, target: { ...event.target, value } }),
          "invalid_target",
          `${vector.name} invalid target ${value}`,
        );
      }

      const wrongAction =
        actionBindingFixture.accepted[(index + 1) % actionBindingFixture.accepted.length];
      expect(wrongAction).toBeDefined();
      expectAuditError(
        () =>
          auditEventSha256({
            ...event,
            action: assertAuditAction(wrongAction?.action ?? ""),
            target: {
              kind: assertAuditTargetKind(wrongAction?.target.kind ?? ""),
              value: wrongAction?.target.value ?? "",
            },
          }),
        "action_payload_mismatch",
        `${vector.name} wrong action/schema`,
      );

      expectAuditError(
        () =>
          auditEventSha256({
            ...event,
            target: {
              kind: assertAuditTargetKind(vector.forbidden_target.kind),
              value: vector.forbidden_target.value,
            },
          }),
        "action_target_mismatch",
        `${vector.name} forbidden target`,
      );

      if (vector.mismatched_target_value !== undefined) {
        expectAuditError(
          () =>
            auditEventSha256({
              ...event,
              target: { ...event.target, value: vector.mismatched_target_value ?? "" },
            }),
          "action_target_mismatch",
          `${vector.name} mismatched subject`,
        );
      }
    }

    for (const vector of actionBindingFixture.malformed_payloads) {
      expectAuditError(
        () => canonicalizeAuditPayload(vector.payload_schema, 1, vector.canonical_payload),
        vector.expected_code,
        vector.name,
      );
    }
  });

  it("validates holdout grant authorization evidence and canonical approval ordering", () => {
    const first = approvalRecord("01", "1", "actor.approver-a");
    const second = approvalRecord("02", "2", "actor.approver-b");
    const maximum = Array.from({ length: 8 }, (_, index) =>
      approvalRecord(
        index.toString().padStart(2, "0"),
        (index + 1).toString(16),
        `actor.approver-${index.toString().padStart(2, "0")}`,
      ),
    );
    expect(() =>
      canonicalizeAuditPayload(
        "loop.audit.holdout_grant_issued",
        1,
        holdoutGrantIssuedPayload([first, second]),
      ),
    ).not.toThrow();
    expect(() =>
      canonicalizeAuditPayload(
        "loop.audit.holdout_grant_issued",
        1,
        holdoutGrantIssuedPayload(maximum),
      ),
    ).not.toThrow();

    for (const [name, records] of [
      ["empty", []],
      ["over limit", [...maximum, approvalRecord("08", "9", "actor.approver-08")]],
      ["unsorted", [second, first]],
      [
        "duplicate record ID",
        [first, { ...second, holdout_approval_record_id: first.holdout_approval_record_id }],
      ],
      [
        "duplicate record digest",
        [first, { ...second, approval_record_sha256: first.approval_record_sha256 }],
      ],
      [
        "duplicate actor ID",
        [first, { ...second, approved_by_actor_id: first.approved_by_actor_id }],
      ],
    ] satisfies readonly (readonly [string, readonly HoldoutApprovalRecordInput[]])[]) {
      expectAuditError(
        () =>
          canonicalizeAuditPayload(
            "loop.audit.holdout_grant_issued",
            1,
            holdoutGrantIssuedPayload(records),
          ),
        "non_canonical_payload",
        name,
      );
    }

    for (const [name, record] of [
      ["invalid approval record ID", { ...first, holdout_approval_record_id: "bad id" }],
      ["invalid approver actor ID", { ...first, approved_by_actor_id: "bad id" }],
    ] satisfies readonly (readonly [string, HoldoutApprovalRecordInput])[]) {
      expectAuditError(
        () =>
          canonicalizeAuditPayload(
            "loop.audit.holdout_grant_issued",
            1,
            holdoutGrantIssuedPayload([record]),
          ),
        "invalid_identifier",
        name,
      );
    }
    expectAuditError(
      () =>
        canonicalizeAuditPayload(
          "loop.audit.holdout_grant_issued",
          1,
          holdoutGrantIssuedPayload([{ ...first, approval_record_sha256: "sha256:ABC" }]),
        ),
      "invalid_digest",
      "invalid approval record digest",
    );

    expectAuditError(
      () =>
        canonicalizeAuditPayload(
          "loop.audit.holdout_grant_issued",
          1,
          holdoutGrantIssuedPayload([first], "privileged", "authorized"),
        ),
      "invalid_enum",
      "issued capability class",
    );
    expectAuditError(
      () =>
        canonicalizeAuditPayload(
          "loop.audit.holdout_grant_issued",
          1,
          holdoutGrantIssuedPayload([first], "holdout_evaluation", "denied"),
        ),
      "invalid_enum",
      "issued authorization decision",
    );

    expect(() =>
      canonicalizeAuditPayload(
        "loop.audit.holdout_grant_consumed",
        1,
        holdoutGrantConsumedPayload(),
      ),
    ).not.toThrow();
    expectAuditError(
      () =>
        canonicalizeAuditPayload(
          "loop.audit.holdout_grant_consumed",
          1,
          holdoutGrantConsumedPayload("privileged", "authorized"),
        ),
      "invalid_enum",
      "consumed capability class",
    );
    expectAuditError(
      () =>
        canonicalizeAuditPayload(
          "loop.audit.holdout_grant_consumed",
          1,
          holdoutGrantConsumedPayload("holdout_evaluation", "denied"),
        ),
      "invalid_enum",
      "consumed authorization decision",
    );
  });

  it("fails closed on hostile deep JSON without leaking a native exception", () => {
    const payload = `${"[".repeat(512)}{}${"]".repeat(512)}`;
    expectAuditError(
      () => canonicalizeAuditPayload("loop.audit.command_accepted", 1, payload),
      "non_canonical_payload",
    );
  });

  it("fails closed on every shared tamper, reorder, genesis, and cross-ledger vector", () => {
    for (const negative of fixture.negative_vectors) {
      let events = chain();
      const first = events[0];
      const second = events[1];
      expect(first).toBeDefined();
      expect(second).toBeDefined();
      switch (negative.mutation) {
        case "payload_tamper": {
          const tamperedPayload: AuditPayload = {
            ...first.payload,
            canonicalBytes: encoder.encode(
              decoder.decode(first.payload.canonicalBytes).replace("ok", "no"),
            ),
          };
          events = [{ ...first, payload: tamperedPayload }, second] as AuditEvent[];
          break;
        }
        case "event_tamper":
          events = [
            { ...first, actor: { ...first.actor, displayName: `${first.actor.displayName}!` } },
            second,
          ] as AuditEvent[];
          break;
        case "event_reorder":
          events = [second, first];
          break;
        case "previous_digest_tamper":
          events = [first, seal({ ...second, previousEventSha256: `sha256:${"1".repeat(64)}` })];
          break;
        case "cross_ledger_replay":
          events = [first, { ...second, auditLedgerId: "ledger.secondary" }];
          break;
        case "sequence_gap":
          events = [first, seal({ ...second, sequence: 3n })];
          break;
        case "invalid_genesis":
          events = [seal({ ...first, previousEventSha256: `sha256:${"2".repeat(64)}` }), second];
          break;
        case "invalid_timestamp":
          events = [{ ...first, occurredAt: "2026-09-05T06:30:00Z" }, second];
          break;
        case "duplicate_event_id":
          events = [first, seal({ ...second, auditEventId: first.auditEventId })];
          break;
        case "schema_mutation":
          events = [
            { ...first, payload: { ...first.payload, schemaName: "loop.audit.unknown" } },
            second,
          ];
          break;
        case "schema_version_mutation":
          events = [{ ...first, payload: { ...first.payload, schemaVersion: 2 } }, second];
          break;
        default:
          throw new Error(`unknown negative mutation ${negative.mutation}`);
      }
      expectAuditError(() => verifyAuditChain(events), negative.expected_code, negative.name);
    }
  });

  it("accepts the shared maxima and rejects one-byte overruns", () => {
    const boundaries = fixture.boundary_vectors;
    const maxIdBytes = Number(boundaries.max_domain_id_bytes);
    const maxTextBytes = Number(boundaries.max_schema_text_bytes);
    const maxPayloadBytes = Number(boundaries.max_payload_bytes);
    const maxSchemaVersion = Number(boundaries.max_schema_version);
    const maxSequence = BigInt(boundaries.max_sequence);
    const base = chain()[0];
    expect(base).toBeDefined();

    const boundaryEvent = seal({
      ...base,
      auditLedgerId: "a".repeat(maxIdBytes),
      sequence: maxSequence,
      actor: { ...base.actor, displayName: "x".repeat(maxTextBytes) },
    } as AuditEvent);
    expect(() => verifyAuditEvent(boundaryEvent)).not.toThrow();

    const summary = "x".repeat(maxTextBytes);
    const payload = `{"command":"research.run","request_id":"request.1","summary":"${summary}"}`;
    expect(() => canonicalizeAuditPayload("loop.audit.command_accepted", 1, payload)).not.toThrow();
    expectAuditError(
      () =>
        canonicalizeAuditPayload(
          "loop.audit.command_accepted",
          1,
          payload.replace(summary, "x".repeat(maxTextBytes + 1)),
        ),
      "invalid_text",
    );
    expectAuditError(
      () =>
        canonicalizeAuditPayload(
          "loop.audit.command_accepted",
          1,
          new Uint8Array(maxPayloadBytes + 1),
        ),
      "size_limit",
    );
    expect(
      auditPayloadSha256("loop.audit.command_accepted", maxSchemaVersion, new Uint8Array()),
    ).toMatch(/^sha256:[0-9a-f]{64}$/);
    expectAuditError(
      () =>
        auditEventSha256({
          ...base,
          auditLedgerId: "a".repeat(maxIdBytes + 1),
        } as AuditEvent),
      "invalid_identifier",
    );
  });

  it("rejects noncanonical payloads, unknown schemas, timestamps, and surrogates", () => {
    for (const value of [
      ' {"command":"research.run","request_id":"request.1","summary":"ok"}',
      '{"request_id":"request.1","command":"research.run","summary":"ok"}',
      '{"command":"research.run","request_id":"request.1","summary":"ok","extra":"x"}',
      '{"command":"research.run","command":"research.run","request_id":"request.1","summary":"ok"}',
    ]) {
      expectAuditError(
        () => canonicalizeAuditPayload("loop.audit.command_accepted", 1, value),
        "non_canonical_payload",
      );
    }
    expectAuditError(
      () => canonicalizeAuditPayload("loop.audit.unknown", 1, "{}"),
      "unsupported_schema",
    );

    const base = chain()[0];
    expect(base).toBeDefined();
    for (const occurredAt of [
      "2026-02-29T00:00:00.000000000Z",
      "2026-09-05T06:30:60.000000000Z",
      "2026-09-05T06:30:00.00000000Z",
      "2026-09-05T06:30:00.000000000+00:00",
    ]) {
      expectAuditError(
        () => auditEventSha256({ ...base, occurredAt } as AuditEvent),
        "invalid_timestamp",
      );
    }
    expectAuditError(
      () =>
        auditEventSha256({
          ...base,
          actor: { ...base.actor, displayName: "\ud800" },
        } as AuditEvent),
      "invalid_text",
    );
    expect(assertActorKind("scheduler")).toBe("scheduler");
    expectAuditError(() => assertActorKind("unknown"), "invalid_enum");
  });
});

function chain(): AuditEvent[] {
  return fixture.accepted_chain.map(toEvent);
}

function toEvent(vector: AcceptedVector): AuditEvent {
  const payload = canonicalizeAuditPayload(
    vector.payload.schema_name,
    Number(vector.payload.schema_version),
    vector.payload.canonical_utf8,
  );
  expect(payload.payloadSha256, vector.name).toBe(vector.payload.payload_sha256);
  const actor: AuditActor = {
    actorId: vector.event.actor.actor_id,
    kind: assertActorKind(vector.event.actor.kind),
    displayName: vector.event.actor.display_name,
    authenticatedSubject: vector.event.actor.authenticated_subject,
  };
  const target: AuditTarget = {
    kind: assertAuditTargetKind(vector.event.target.kind),
    value: vector.event.target.value,
  };
  return {
    auditLedgerId: vector.event.audit_ledger_id,
    sequence: BigInt(vector.event.sequence),
    previousEventSha256: vector.event.previous_event_sha256,
    auditEventId: vector.event.audit_event_id,
    occurredAt: vector.event.occurred_at,
    correlationId: vector.event.correlation_id,
    causationId: vector.event.causation_id,
    actor,
    action: assertAuditAction(vector.event.action),
    target,
    payload,
    eventSha256: vector.event_sha256,
  };
}

function actionEvent(envelope: ActionEventEnvelope, vector: ActionBindingVector): AuditEvent {
  return {
    auditLedgerId: envelope.audit_ledger_id,
    sequence: BigInt(envelope.sequence),
    previousEventSha256: envelope.previous_event_sha256,
    auditEventId: envelope.audit_event_id,
    occurredAt: envelope.occurred_at,
    correlationId: envelope.correlation_id,
    causationId: envelope.causation_id,
    actor: {
      actorId: envelope.actor.actor_id,
      kind: assertActorKind(envelope.actor.kind),
      displayName: envelope.actor.display_name,
      authenticatedSubject: envelope.actor.authenticated_subject,
    },
    action: assertAuditAction(vector.action),
    target: {
      kind: assertAuditTargetKind(vector.target.kind),
      value: vector.target.value,
    },
    payload: canonicalizeAuditPayload(vector.payload_schema, 1, vector.canonical_payload),
    eventSha256: zeros,
  };
}

function seal(event: Omit<AuditEvent, "eventSha256"> | AuditEvent): AuditEvent {
  const candidate = { ...event, eventSha256: zeros } as AuditEvent;
  return { ...candidate, eventSha256: auditEventSha256(candidate) };
}

interface HoldoutApprovalRecordInput {
  readonly holdout_approval_record_id: string;
  readonly approval_record_sha256: string;
  readonly approved_by_actor_id: string;
}

function approvalRecord(
  suffix: string,
  digestDigit: string,
  actorId: string,
): HoldoutApprovalRecordInput {
  return {
    holdout_approval_record_id: `holdout_approval.${suffix}`,
    approval_record_sha256: `sha256:${digestDigit.repeat(64)}`,
    approved_by_actor_id: actorId,
  };
}

function holdoutGrantIssuedPayload(
  approvalRecords: readonly HoldoutApprovalRecordInput[],
  capabilityClass = "holdout_evaluation",
  authorizationDecision = "authorized",
): string {
  return JSON.stringify({
    holdout_grant_id: "grant.01",
    holdout_period_id: `sha256:${"d".repeat(64)}`,
    freeze_manifest_sha256: `sha256:${"e".repeat(64)}`,
    holdout_evaluation_plan_id: `sha256:${"f".repeat(64)}`,
    approval_records: approvalRecords,
    capability_class: capabilityClass,
    authorization_decision: authorizationDecision,
  });
}

function holdoutGrantConsumedPayload(
  capabilityClass = "holdout_evaluation",
  authorizationDecision = "authorized",
): string {
  return JSON.stringify({
    holdout_grant_id: "grant.01",
    holdout_period_id: `sha256:${"d".repeat(64)}`,
    holdout_evaluation_plan_id: `sha256:${"f".repeat(64)}`,
    job_batch_id: "batch.01",
    capability_class: capabilityClass,
    authorization_decision: authorizationDecision,
  });
}

function expectAuditError(action: () => unknown, code: AuditErrorCode, name = code): void {
  try {
    action();
  } catch (error) {
    expect(error, name).toBeInstanceOf(AuditValidationError);
    expect((error as AuditValidationError).code, name).toBe(code);
    return;
  }
  throw new Error(`${name}: expected AuditValidationError(${code})`);
}
