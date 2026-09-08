import { createHash } from "node:crypto";

export const MAX_AUDIT_PAYLOAD_BYTES = 256 * 1_024;
export const MAX_AUDIT_TEXT_BYTES = 4_096;
export const MAX_AUDIT_ID_BYTES = 128;

const MAX_HOLDOUT_APPROVAL_RECORDS = 8;

const PAYLOAD_DOMAIN = "loop.audit-payload/v1";
const EVENT_DOMAIN = "loop.audit-event/v1";
const EVENT_SCHEMA = "loop.audit-event/v1";
const ZERO_SHA256 = `sha256:${"0".repeat(64)}`;
const SHA256_PATTERN = /^sha256:[0-9a-f]{64}$/;
const SCHEMA_PATTERN = /^[a-z][a-z0-9_]*(?:\.[a-z][a-z0-9_]*)*$/;
const DOMAIN_ID_PATTERN = /^[A-Za-z0-9][A-Za-z0-9._:-]*$/;
const TIMESTAMP_PATTERN = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})\.(\d{9})Z$/;
const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true });

export type AuditErrorCode =
  | "invalid_schema"
  | "unsupported_schema"
  | "non_canonical_payload"
  | "payload_digest_mismatch"
  | "invalid_digest"
  | "invalid_identifier"
  | "invalid_text"
  | "invalid_timestamp"
  | "invalid_sequence"
  | "invalid_enum"
  | "invalid_target"
  | "action_payload_mismatch"
  | "action_target_mismatch"
  | "event_digest_mismatch"
  | "ledger_mismatch"
  | "chain_mismatch"
  | "duplicate_event_id"
  | "size_limit";

export class AuditValidationError extends Error {
  public override readonly name = "AuditValidationError";

  public constructor(
    public readonly code: AuditErrorCode,
    public readonly field: string,
    detail: string,
  ) {
    super(`${field} failed audit validation (${code}): ${detail}`);
  }
}

export type ActorKind = "human" | "service" | "agent" | "scheduler";

export type AuditAction =
  | "command_accepted"
  | "state_transitioned"
  | "factor_admitted"
  | "factor_rejected"
  | "override_authorized"
  | "readmission_requested"
  | "readmission_decided"
  | "holdout_grant_issued"
  | "holdout_grant_consumed"
  | "artifact_exported"
  | "holdout_approval_recorded";

export type AuditTargetKind =
  | "run_id"
  | "job_id"
  | "factor_spec_id"
  | "backtest_id"
  | "snapshot_id"
  | "holdout_grant_id"
  | "artifact_id"
  | "holdout_approval_record_id"
  | "holdout_period_id";

export interface AuditActor {
  readonly actorId: string;
  readonly kind: ActorKind;
  readonly displayName: string;
  readonly authenticatedSubject: string;
}

export interface AuditTarget {
  readonly kind: AuditTargetKind;
  readonly value: string;
}

export interface AuditPayload {
  readonly schemaName: string;
  readonly schemaVersion: number;
  readonly canonicalBytes: Uint8Array;
  readonly payloadSha256: string;
}

export interface AuditEvent {
  readonly auditLedgerId: string;
  readonly sequence: bigint;
  readonly previousEventSha256: string;
  readonly auditEventId: string;
  readonly occurredAt: string;
  readonly correlationId: string;
  readonly causationId: string;
  readonly actor: AuditActor;
  readonly action: AuditAction;
  readonly target: AuditTarget;
  readonly payload: AuditPayload;
  readonly eventSha256: string;
}

export function canonicalizeAuditPayload(
  schemaName: string,
  schemaVersion: number,
  submitted: Uint8Array | string,
): Readonly<AuditPayload> {
  validateSchemaName(schemaName);
  validateSchemaVersion(schemaVersion);
  const { bytes, text } = decodeCanonicalBytes(submitted);
  if (bytes.byteLength > MAX_AUDIT_PAYLOAD_BYTES) {
    fail("size_limit", "payload.canonical_bytes", "canonical payload exceeds 256 KiB");
  }

  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch (error) {
    fail(
      "non_canonical_payload",
      "payload.canonical_bytes",
      error instanceof Error ? error.message : "payload is not JSON",
    );
  }

  const { rewritten } = canonicalizeRegisteredPayload(schemaName, schemaVersion, raw);

  const rewrittenBytes = encoder.encode(rewritten);
  if (!equalBytes(bytes, rewrittenBytes)) {
    fail(
      "non_canonical_payload",
      "payload.canonical_bytes",
      "payload bytes differ from the registered dedicated writer",
    );
  }
  return Object.freeze({
    schemaName,
    schemaVersion,
    canonicalBytes: rewrittenBytes,
    payloadSha256: auditPayloadSha256(schemaName, schemaVersion, rewrittenBytes),
  });
}

export function auditPayloadSha256(
  schemaName: string,
  schemaVersion: number,
  canonicalPayloadBytes: Uint8Array,
): string {
  validateSchemaName(schemaName);
  validateSchemaVersion(schemaVersion);
  const hash = createHash("sha256");
  hash.update(PAYLOAD_DOMAIN, "ascii");
  hash.update(Uint8Array.of(0));
  hash.update(schemaName, "ascii");
  hash.update(Uint8Array.of(0));
  hash.update(schemaVersion.toString(), "ascii");
  hash.update(Uint8Array.of(0));
  hash.update(canonicalPayloadBytes);
  return `sha256:${hash.digest("hex")}`;
}

export function verifyAuditPayload(payload: AuditPayload): void {
  validateSha256(payload.payloadSha256, "payload.payload_sha256");
  const verified = canonicalizeAuditPayload(
    payload.schemaName,
    payload.schemaVersion,
    payload.canonicalBytes,
  );
  if (verified.payloadSha256 !== payload.payloadSha256) {
    fail(
      "payload_digest_mismatch",
      "payload.payload_sha256",
      "claimed payload digest does not match canonical payload bytes",
    );
  }
}

export function canonicalAuditEventBytes(event: AuditEvent): Uint8Array {
  verifyAuditPayload(event.payload);
  validateEvent(event);
  const canonical =
    `{"schema":"${EVENT_SCHEMA}","audit_ledger_id":${writeJsonString(event.auditLedgerId)},` +
    `"sequence":"${event.sequence.toString()}","previous_event_sha256":"${event.previousEventSha256}",` +
    `"audit_event_id":${writeJsonString(event.auditEventId)},"occurred_at":${writeJsonString(event.occurredAt)},` +
    `"correlation_id":${writeJsonString(event.correlationId)},"causation_id":${writeJsonString(event.causationId)},` +
    `"actor":{"actor_id":${writeJsonString(event.actor.actorId)},"kind":"${event.actor.kind}",` +
    `"display_name":${writeJsonString(event.actor.displayName)},` +
    `"authenticated_subject":${writeJsonString(event.actor.authenticatedSubject)}},` +
    `"action":"${event.action}","target":{"kind":"${event.target.kind}",` +
    `"value":${writeJsonString(event.target.value)}},"payload":{"schema_name":"${event.payload.schemaName}",` +
    `"schema_version":"${event.payload.schemaVersion.toString()}",` +
    `"payload_sha256":"${event.payload.payloadSha256}"}}`;
  return encoder.encode(canonical);
}

export function auditEventSha256(event: AuditEvent): string {
  const hash = createHash("sha256");
  hash.update(EVENT_DOMAIN, "ascii");
  hash.update(Uint8Array.of(0));
  hash.update(canonicalAuditEventBytes(event));
  return `sha256:${hash.digest("hex")}`;
}

export function verifyAuditEvent(event: AuditEvent): void {
  validateSha256(event.eventSha256, "event_sha256");
  const computed = auditEventSha256(event);
  if (computed !== event.eventSha256) {
    fail(
      "event_digest_mismatch",
      "event_sha256",
      "claimed event digest does not match canonical event bytes",
    );
  }
}

export function verifyAuditChain(events: readonly AuditEvent[]): void {
  if (events.length === 0) return;
  const ledgerId = events[0]?.auditLedgerId;
  let previous = ZERO_SHA256;
  let expectedSequence = 1n;
  const eventIds = new Set<string>();
  for (const event of events) {
    verifyAuditEvent(event);
    if (event.auditLedgerId !== ledgerId) {
      fail(
        "ledger_mismatch",
        "audit_ledger_id",
        "all events in one verified chain must use the same ledger ID",
      );
    }
    if (event.sequence !== expectedSequence) {
      fail(
        "invalid_sequence",
        "sequence",
        "audit sequence must begin at one and increase exactly by one",
      );
    }
    if (event.previousEventSha256 !== previous) {
      fail(
        "chain_mismatch",
        "previous_event_sha256",
        "event does not commit to the immediately preceding event digest",
      );
    }
    if (eventIds.has(event.auditEventId)) {
      fail("duplicate_event_id", "audit_event_id", "audit event IDs must be unique within a chain");
    }
    eventIds.add(event.auditEventId);
    previous = event.eventSha256;
    expectedSequence += 1n;
  }
}

export function assertActorKind(value: string): ActorKind {
  if (value === "human" || value === "service" || value === "agent" || value === "scheduler") {
    return value;
  }
  fail("invalid_enum", "actor.kind", "unknown actor kind");
}

export function assertAuditAction(value: string): AuditAction {
  if (
    value === "command_accepted" ||
    value === "state_transitioned" ||
    value === "factor_admitted" ||
    value === "factor_rejected" ||
    value === "override_authorized" ||
    value === "readmission_requested" ||
    value === "readmission_decided" ||
    value === "holdout_grant_issued" ||
    value === "holdout_grant_consumed" ||
    value === "artifact_exported" ||
    value === "holdout_approval_recorded"
  ) {
    return value;
  }
  fail("invalid_enum", "action", "unknown audit action");
}

export function assertAuditTargetKind(value: string): AuditTargetKind {
  if (
    value === "run_id" ||
    value === "job_id" ||
    value === "factor_spec_id" ||
    value === "backtest_id" ||
    value === "snapshot_id" ||
    value === "holdout_grant_id" ||
    value === "artifact_id" ||
    value === "holdout_approval_record_id" ||
    value === "holdout_period_id"
  ) {
    return value;
  }
  fail("invalid_target", "target.kind", "unknown audit target kind");
}

interface CanonicalPayloadProjection {
  readonly rewritten: string;
  readonly subject?: string;
}

interface HoldoutApprovalAuditRecord {
  readonly holdoutApprovalRecordId: string;
  readonly approvalRecordSha256: string;
  readonly approvedByActorId: string;
}

function canonicalizeRegisteredPayload(
  schemaName: string,
  schemaVersion: number,
  raw: unknown,
): CanonicalPayloadProjection {
  if (schemaVersion !== 1) {
    fail(
      "unsupported_schema",
      "payload.schema_name",
      "audit payload schema/version is not registered",
    );
  }
  if (schemaName === "loop.audit.command_accepted") {
    const payload = requireExactObject(raw, ["command", "request_id", "summary"]);
    const command = requireString(payload.command, "payload.command");
    const requestId = requireString(payload.request_id, "payload.request_id");
    const summary = requireString(payload.summary, "payload.summary");
    validateSchemaIdentifier(command, "payload.command");
    validateDomainId(requestId, "payload.request_id");
    validateText(summary, "payload.summary", true);
    return {
      rewritten: writePayloadFields([
        ["command", command],
        ["request_id", requestId],
        ["summary", summary],
      ]),
    };
  }
  if (schemaName === "loop.audit.state_transitioned") {
    const payload = requireExactObject(raw, ["from", "to", "reason"]);
    const from = requireString(payload.from, "payload.from");
    const to = requireString(payload.to, "payload.to");
    const reason = requireString(payload.reason, "payload.reason");
    validateSchemaIdentifier(from, "payload.from");
    validateSchemaIdentifier(to, "payload.to");
    validateText(reason, "payload.reason", true);
    return {
      rewritten: writePayloadFields([
        ["from", from],
        ["to", to],
        ["reason", reason],
      ]),
    };
  }
  if (schemaName === "loop.audit.factor_admitted") {
    const payload = requireExactObject(raw, ["factor_spec_id", "decision", "evidence_artifact_id"]);
    const factorSpecId = requireString(payload.factor_spec_id, "payload.factor_spec_id");
    const decision = requireString(payload.decision, "payload.decision");
    const evidenceArtifactId = requireString(
      payload.evidence_artifact_id,
      "payload.evidence_artifact_id",
    );
    validateSha256(factorSpecId, "payload.factor_spec_id");
    validateClosedEnum(decision, ["admitted"], "payload.decision");
    validateSha256(evidenceArtifactId, "payload.evidence_artifact_id");
    return {
      rewritten: writePayloadFields([
        ["factor_spec_id", factorSpecId],
        ["decision", decision],
        ["evidence_artifact_id", evidenceArtifactId],
      ]),
      subject: factorSpecId,
    };
  }
  if (schemaName === "loop.audit.factor_rejected") {
    const payload = requireExactObject(raw, [
      "factor_spec_id",
      "rejection_code",
      "reason",
      "evidence_artifact_id",
    ]);
    const factorSpecId = requireString(payload.factor_spec_id, "payload.factor_spec_id");
    const rejectionCode = requireString(payload.rejection_code, "payload.rejection_code");
    const reason = requireString(payload.reason, "payload.reason");
    const evidenceArtifactId = requireString(
      payload.evidence_artifact_id,
      "payload.evidence_artifact_id",
    );
    validateSha256(factorSpecId, "payload.factor_spec_id");
    validateClosedEnum(
      rejectionCode,
      [
        "duplicate",
        "previously_failed",
        "insufficient_coverage",
        "deterministic_filter",
        "performance",
        "correlation",
        "semantic_review",
        "policy",
      ],
      "payload.rejection_code",
    );
    validateText(reason, "payload.reason", true);
    validateSha256(evidenceArtifactId, "payload.evidence_artifact_id");
    return {
      rewritten: writePayloadFields([
        ["factor_spec_id", factorSpecId],
        ["rejection_code", rejectionCode],
        ["reason", reason],
        ["evidence_artifact_id", evidenceArtifactId],
      ]),
      subject: factorSpecId,
    };
  }
  if (schemaName === "loop.audit.override_authorized") {
    const payload = requireExactObject(raw, [
      "factor_spec_id",
      "override_kind",
      "authorized_by_actor_id",
      "reason",
      "approval_reference",
      "evidence_artifact_id",
    ]);
    const factorSpecId = requireString(payload.factor_spec_id, "payload.factor_spec_id");
    const overrideKind = requireString(payload.override_kind, "payload.override_kind");
    const authorizedByActorId = requireString(
      payload.authorized_by_actor_id,
      "payload.authorized_by_actor_id",
    );
    const reason = requireString(payload.reason, "payload.reason");
    const approvalReference = requireString(
      payload.approval_reference,
      "payload.approval_reference",
    );
    const evidenceArtifactId = requireString(
      payload.evidence_artifact_id,
      "payload.evidence_artifact_id",
    );
    validateSha256(factorSpecId, "payload.factor_spec_id");
    validateClosedEnum(
      overrideKind,
      ["force_admission", "readmission", "policy_exception"],
      "payload.override_kind",
    );
    validateDomainId(authorizedByActorId, "payload.authorized_by_actor_id");
    validateText(reason, "payload.reason", true);
    validateDomainId(approvalReference, "payload.approval_reference");
    validateSha256(evidenceArtifactId, "payload.evidence_artifact_id");
    return {
      rewritten: writePayloadFields([
        ["factor_spec_id", factorSpecId],
        ["override_kind", overrideKind],
        ["authorized_by_actor_id", authorizedByActorId],
        ["reason", reason],
        ["approval_reference", approvalReference],
        ["evidence_artifact_id", evidenceArtifactId],
      ]),
      subject: factorSpecId,
    };
  }
  if (schemaName === "loop.audit.readmission_requested") {
    const payload = requireExactObject(raw, [
      "factor_spec_id",
      "original_rejection_event_id",
      "requested_by_actor_id",
      "reason",
    ]);
    const factorSpecId = requireString(payload.factor_spec_id, "payload.factor_spec_id");
    const originalRejectionEventId = requireString(
      payload.original_rejection_event_id,
      "payload.original_rejection_event_id",
    );
    const requestedByActorId = requireString(
      payload.requested_by_actor_id,
      "payload.requested_by_actor_id",
    );
    const reason = requireString(payload.reason, "payload.reason");
    validateSha256(factorSpecId, "payload.factor_spec_id");
    validateDomainId(originalRejectionEventId, "payload.original_rejection_event_id");
    validateDomainId(requestedByActorId, "payload.requested_by_actor_id");
    validateText(reason, "payload.reason", true);
    return {
      rewritten: writePayloadFields([
        ["factor_spec_id", factorSpecId],
        ["original_rejection_event_id", originalRejectionEventId],
        ["requested_by_actor_id", requestedByActorId],
        ["reason", reason],
      ]),
      subject: factorSpecId,
    };
  }
  if (schemaName === "loop.audit.readmission_decided") {
    const payload = requireExactObject(raw, [
      "factor_spec_id",
      "original_rejection_event_id",
      "disposition",
      "decided_by_actor_id",
      "reason",
    ]);
    const factorSpecId = requireString(payload.factor_spec_id, "payload.factor_spec_id");
    const originalRejectionEventId = requireString(
      payload.original_rejection_event_id,
      "payload.original_rejection_event_id",
    );
    const disposition = requireString(payload.disposition, "payload.disposition");
    const decidedByActorId = requireString(
      payload.decided_by_actor_id,
      "payload.decided_by_actor_id",
    );
    const reason = requireString(payload.reason, "payload.reason");
    validateSha256(factorSpecId, "payload.factor_spec_id");
    validateDomainId(originalRejectionEventId, "payload.original_rejection_event_id");
    validateClosedEnum(disposition, ["admitted", "rejected", "quarantined"], "payload.disposition");
    validateDomainId(decidedByActorId, "payload.decided_by_actor_id");
    validateText(reason, "payload.reason", true);
    return {
      rewritten: writePayloadFields([
        ["factor_spec_id", factorSpecId],
        ["original_rejection_event_id", originalRejectionEventId],
        ["disposition", disposition],
        ["decided_by_actor_id", decidedByActorId],
        ["reason", reason],
      ]),
      subject: factorSpecId,
    };
  }
  if (schemaName === "loop.audit.holdout_grant_issued") {
    const payload = requireExactObject(raw, [
      "holdout_grant_id",
      "holdout_period_id",
      "freeze_manifest_sha256",
      "holdout_evaluation_plan_id",
      "approval_records",
      "capability_class",
      "authorization_decision",
    ]);
    const holdoutGrantId = requireString(payload.holdout_grant_id, "payload.holdout_grant_id");
    const holdoutPeriodId = requireString(payload.holdout_period_id, "payload.holdout_period_id");
    const freezeManifestSha256 = requireString(
      payload.freeze_manifest_sha256,
      "payload.freeze_manifest_sha256",
    );
    const holdoutEvaluationPlanId = requireString(
      payload.holdout_evaluation_plan_id,
      "payload.holdout_evaluation_plan_id",
    );
    const approvalRecords = requireHoldoutApprovalRecords(payload.approval_records);
    const capabilityClass = requireString(payload.capability_class, "payload.capability_class");
    const authorizationDecision = requireString(
      payload.authorization_decision,
      "payload.authorization_decision",
    );
    validateDomainId(holdoutGrantId, "payload.holdout_grant_id");
    validateSha256(holdoutPeriodId, "payload.holdout_period_id");
    validateSha256(freezeManifestSha256, "payload.freeze_manifest_sha256");
    validateSha256(holdoutEvaluationPlanId, "payload.holdout_evaluation_plan_id");
    validateClosedEnum(capabilityClass, ["holdout_evaluation"], "payload.capability_class");
    validateClosedEnum(authorizationDecision, ["authorized"], "payload.authorization_decision");
    return {
      rewritten: writeHoldoutGrantIssuedPayload(
        holdoutGrantId,
        holdoutPeriodId,
        freezeManifestSha256,
        holdoutEvaluationPlanId,
        approvalRecords,
        capabilityClass,
        authorizationDecision,
      ),
      subject: holdoutGrantId,
    };
  }
  if (schemaName === "loop.audit.holdout_grant_consumed") {
    const payload = requireExactObject(raw, [
      "holdout_grant_id",
      "holdout_period_id",
      "holdout_evaluation_plan_id",
      "job_batch_id",
      "capability_class",
      "authorization_decision",
    ]);
    const holdoutGrantId = requireString(payload.holdout_grant_id, "payload.holdout_grant_id");
    const holdoutPeriodId = requireString(payload.holdout_period_id, "payload.holdout_period_id");
    const holdoutEvaluationPlanId = requireString(
      payload.holdout_evaluation_plan_id,
      "payload.holdout_evaluation_plan_id",
    );
    const jobBatchId = requireString(payload.job_batch_id, "payload.job_batch_id");
    const capabilityClass = requireString(payload.capability_class, "payload.capability_class");
    const authorizationDecision = requireString(
      payload.authorization_decision,
      "payload.authorization_decision",
    );
    validateDomainId(holdoutGrantId, "payload.holdout_grant_id");
    validateSha256(holdoutPeriodId, "payload.holdout_period_id");
    validateSha256(holdoutEvaluationPlanId, "payload.holdout_evaluation_plan_id");
    validateDomainId(jobBatchId, "payload.job_batch_id");
    validateClosedEnum(capabilityClass, ["holdout_evaluation"], "payload.capability_class");
    validateClosedEnum(authorizationDecision, ["authorized"], "payload.authorization_decision");
    return {
      rewritten: writePayloadFields([
        ["holdout_grant_id", holdoutGrantId],
        ["holdout_period_id", holdoutPeriodId],
        ["holdout_evaluation_plan_id", holdoutEvaluationPlanId],
        ["job_batch_id", jobBatchId],
        ["capability_class", capabilityClass],
        ["authorization_decision", authorizationDecision],
      ]),
      subject: holdoutGrantId,
    };
  }
  if (schemaName === "loop.audit.artifact_exported") {
    const payload = requireExactObject(raw, [
      "artifact_id",
      "export_class",
      "policy_id",
      "destination_class",
    ]);
    const artifactId = requireString(payload.artifact_id, "payload.artifact_id");
    const exportClass = requireString(payload.export_class, "payload.export_class");
    const policyId = requireString(payload.policy_id, "payload.policy_id");
    const destinationClass = requireString(payload.destination_class, "payload.destination_class");
    validateSha256(artifactId, "payload.artifact_id");
    validateClosedEnum(
      exportClass,
      ["research_report", "audit_bundle", "data_snapshot", "factor_values"],
      "payload.export_class",
    );
    validateSha256(policyId, "payload.policy_id");
    validateClosedEnum(
      destinationClass,
      ["local_managed", "approved_object_store", "user_download"],
      "payload.destination_class",
    );
    return {
      rewritten: writePayloadFields([
        ["artifact_id", artifactId],
        ["export_class", exportClass],
        ["policy_id", policyId],
        ["destination_class", destinationClass],
      ]),
      subject: artifactId,
    };
  }
  if (schemaName === "loop.audit.holdout_approval_recorded") {
    const payload = requireExactObject(raw, [
      "holdout_approval_record_id",
      "holdout_period_id",
      "freeze_manifest_sha256",
      "approved_by_actor_id",
      "expires_at",
    ]);
    const holdoutApprovalRecordId = requireString(
      payload.holdout_approval_record_id,
      "payload.holdout_approval_record_id",
    );
    const holdoutPeriodId = requireString(payload.holdout_period_id, "payload.holdout_period_id");
    const freezeManifestSha256 = requireString(
      payload.freeze_manifest_sha256,
      "payload.freeze_manifest_sha256",
    );
    const approvedByActorId = requireString(
      payload.approved_by_actor_id,
      "payload.approved_by_actor_id",
    );
    const expiresAt = requireString(payload.expires_at, "payload.expires_at");
    validateDomainId(holdoutApprovalRecordId, "payload.holdout_approval_record_id");
    validateSha256(holdoutPeriodId, "payload.holdout_period_id");
    validateSha256(freezeManifestSha256, "payload.freeze_manifest_sha256");
    validateDomainId(approvedByActorId, "payload.approved_by_actor_id");
    validateTimestampField(expiresAt, "payload.expires_at");
    return {
      rewritten: writePayloadFields([
        ["holdout_approval_record_id", holdoutApprovalRecordId],
        ["holdout_period_id", holdoutPeriodId],
        ["freeze_manifest_sha256", freezeManifestSha256],
        ["approved_by_actor_id", approvedByActorId],
        ["expires_at", expiresAt],
      ]),
      subject: holdoutApprovalRecordId,
    };
  }
  fail(
    "unsupported_schema",
    "payload.schema_name",
    "audit payload schema/version is not registered",
  );
}

function validateEvent(event: AuditEvent): void {
  validateDomainId(event.auditLedgerId, "audit_ledger_id");
  if (event.sequence <= 0n || event.sequence > 18_446_744_073_709_551_615n) {
    fail("invalid_sequence", "sequence", "event sequence must be a positive uint64");
  }
  validateSha256(event.previousEventSha256, "previous_event_sha256");
  validateDomainId(event.auditEventId, "audit_event_id");
  validateTimestamp(event.occurredAt);
  validateDomainId(event.correlationId, "correlation_id");
  validateDomainId(event.causationId, "causation_id");
  validateDomainId(event.actor.actorId, "actor.actor_id");
  assertActorKind(event.actor.kind);
  validateText(event.actor.displayName, "actor.display_name", false);
  validateText(event.actor.authenticatedSubject, "actor.authenticated_subject", true);
  assertAuditAction(event.action);
  assertAuditTargetKind(event.target.kind);
  if (
    event.target.kind === "factor_spec_id" ||
    event.target.kind === "artifact_id" ||
    event.target.kind === "holdout_period_id"
  ) {
    if (!SHA256_PATTERN.test(event.target.value)) {
      fail(
        "invalid_target",
        "target.value",
        "content-addressed target requires a full SHA-256 identity",
      );
    }
  } else {
    try {
      validateDomainId(event.target.value, "target.value");
    } catch {
      fail("invalid_target", "target.value", "target value does not satisfy its typed ID encoding");
    }
  }
  validateActionBinding(event);
}

function validateActionBinding(event: AuditEvent): void {
  const binding: Readonly<{
    schemaName: string;
    allowedTargets: readonly AuditTargetKind[];
  }> = (() => {
    switch (event.action) {
      case "command_accepted":
        return {
          schemaName: "loop.audit.command_accepted",
          allowedTargets: [
            "run_id",
            "job_id",
            "factor_spec_id",
            "backtest_id",
            "snapshot_id",
            "artifact_id",
          ],
        };
      case "state_transitioned":
        return {
          schemaName: "loop.audit.state_transitioned",
          allowedTargets: ["run_id", "job_id", "backtest_id", "snapshot_id", "holdout_period_id"],
        };
      case "factor_admitted":
        return { schemaName: "loop.audit.factor_admitted", allowedTargets: ["factor_spec_id"] };
      case "factor_rejected":
        return { schemaName: "loop.audit.factor_rejected", allowedTargets: ["factor_spec_id"] };
      case "override_authorized":
        return { schemaName: "loop.audit.override_authorized", allowedTargets: ["factor_spec_id"] };
      case "readmission_requested":
        return {
          schemaName: "loop.audit.readmission_requested",
          allowedTargets: ["factor_spec_id"],
        };
      case "readmission_decided":
        return { schemaName: "loop.audit.readmission_decided", allowedTargets: ["factor_spec_id"] };
      case "holdout_grant_issued":
        return {
          schemaName: "loop.audit.holdout_grant_issued",
          allowedTargets: ["holdout_grant_id"],
        };
      case "holdout_grant_consumed":
        return {
          schemaName: "loop.audit.holdout_grant_consumed",
          allowedTargets: ["holdout_grant_id"],
        };
      case "artifact_exported":
        return { schemaName: "loop.audit.artifact_exported", allowedTargets: ["artifact_id"] };
      case "holdout_approval_recorded":
        return {
          schemaName: "loop.audit.holdout_approval_recorded",
          allowedTargets: ["holdout_approval_record_id"],
        };
    }
  })();
  if (event.payload.schemaName !== binding.schemaName || event.payload.schemaVersion !== 1) {
    fail(
      "action_payload_mismatch",
      "action",
      "audit action is not bound to the payload schema/version",
    );
  }
  if (!binding.allowedTargets.includes(event.target.kind)) {
    fail("action_target_mismatch", "target.kind", "audit action does not permit this target kind");
  }

  let raw: unknown;
  try {
    raw = JSON.parse(decoder.decode(event.payload.canonicalBytes));
  } catch (error) {
    fail(
      "non_canonical_payload",
      "payload.canonical_bytes",
      error instanceof Error ? error.message : "payload is not JSON",
    );
  }
  const { subject } = canonicalizeRegisteredPayload(
    event.payload.schemaName,
    event.payload.schemaVersion,
    raw,
  );
  if (subject !== undefined && subject !== event.target.value) {
    fail(
      "action_target_mismatch",
      "target.value",
      "subject-bearing payload identity does not match the typed target value",
    );
  }
}

function validateSchemaName(value: string): void {
  validateSchemaIdentifier(value, "payload.schema_name");
}

function validateSchemaIdentifier(value: string, field: string): void {
  if (!SCHEMA_PATTERN.test(value) || encoder.encode(value).byteLength > MAX_AUDIT_ID_BYTES) {
    fail("invalid_schema", field, "value must be a dot-qualified lowercase ASCII identifier");
  }
}

function validateSchemaVersion(value: number): void {
  if (!Number.isInteger(value) || value < 1 || value > 4_294_967_295) {
    fail("invalid_schema", "payload.schema_version", "schema version must be a positive uint32");
  }
}

function validateDomainId(value: string, field: string): void {
  if (!DOMAIN_ID_PATTERN.test(value) || encoder.encode(value).byteLength > MAX_AUDIT_ID_BYTES) {
    fail(
      "invalid_identifier",
      field,
      "identifier must be 1..=128 bytes in the canonical ASCII domain alphabet",
    );
  }
}

function validateText(value: string, field: string, requireNonempty: boolean): void {
  validateUnicodeScalar(value, field);
  if (
    (requireNonempty && value.length === 0) ||
    encoder.encode(value).byteLength > MAX_AUDIT_TEXT_BYTES
  ) {
    fail("invalid_text", field, "text is empty or exceeds the audit text byte limit");
  }
}

function validateUnicodeScalar(value: string, field: string): void {
  for (let index = 0; index < value.length; index += 1) {
    const unit = value.charCodeAt(index);
    if (unit >= 0xd800 && unit <= 0xdbff) {
      const next = value.charCodeAt(index + 1);
      if (!(next >= 0xdc00 && next <= 0xdfff)) {
        fail("invalid_text", field, "text contains an unpaired UTF-16 surrogate");
      }
      index += 1;
    } else if (unit >= 0xdc00 && unit <= 0xdfff) {
      fail("invalid_text", field, "text contains an unpaired UTF-16 surrogate");
    }
  }
}

function validateTimestamp(value: string): void {
  const match = TIMESTAMP_PATTERN.exec(value);
  if (match === null) invalidTimestamp();
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  const hour = Number(match[4]);
  const minute = Number(match[5]);
  const second = Number(match[6]);
  if (
    year < 1 ||
    month < 1 ||
    month > 12 ||
    day < 1 ||
    day > daysInMonth(year, month) ||
    hour > 23 ||
    minute > 59 ||
    second > 59
  ) {
    invalidTimestamp();
  }
}

function invalidTimestamp(): never {
  fail(
    "invalid_timestamp",
    "occurred_at",
    "timestamp must be a valid UTC instant with exactly nine fractional digits",
  );
}

function daysInMonth(year: number, month: number): number {
  if (month === 2) {
    return year % 400 === 0 || (year % 4 === 0 && year % 100 !== 0) ? 29 : 28;
  }
  return [4, 6, 9, 11].includes(month) ? 30 : 31;
}

function decodeCanonicalBytes(input: Uint8Array | string): {
  readonly bytes: Uint8Array;
  readonly text: string;
} {
  const bytes = typeof input === "string" ? encoder.encode(input) : new Uint8Array(input);
  if (bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf) {
    fail("non_canonical_payload", "payload.canonical_bytes", "UTF-8 BOM is forbidden");
  }
  try {
    return { bytes, text: decoder.decode(bytes) };
  } catch (error) {
    fail(
      "non_canonical_payload",
      "payload.canonical_bytes",
      error instanceof Error ? error.message : "payload is not valid UTF-8",
    );
  }
}

function requireExactObject(raw: unknown, keys: readonly string[]): Record<string, unknown> {
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) {
    fail("non_canonical_payload", "payload.canonical_bytes", "payload must be an object");
  }
  const actual = Object.keys(raw);
  if (actual.length !== keys.length || actual.some((key, index) => key !== keys[index])) {
    fail(
      "non_canonical_payload",
      "payload.canonical_bytes",
      "payload has unknown, missing, duplicate, or out-of-order fields",
    );
  }
  return raw as Record<string, unknown>;
}

function requireString(value: unknown, field: string): string {
  if (typeof value !== "string") {
    fail("non_canonical_payload", field, "payload field must be a string");
  }
  validateUnicodeScalar(value, field);
  return value;
}

function requireHoldoutApprovalRecords(value: unknown): readonly HoldoutApprovalAuditRecord[] {
  if (!Array.isArray(value)) {
    fail("non_canonical_payload", "payload.approval_records", "field must be an array");
  }
  if (value.length < 1 || value.length > MAX_HOLDOUT_APPROVAL_RECORDS) {
    fail(
      "non_canonical_payload",
      "payload.approval_records",
      "approval records must contain 1..=8 entries",
    );
  }

  const recordIds = new Set<string>();
  const recordDigests = new Set<string>();
  const actorIds = new Set<string>();
  const records: HoldoutApprovalAuditRecord[] = [];
  let previousActorId: string | undefined;

  for (const [index, raw] of value.entries()) {
    const prefix = `payload.approval_records[${index}]`;
    const record = requireExactObject(raw, [
      "holdout_approval_record_id",
      "approval_record_sha256",
      "approved_by_actor_id",
    ]);
    const holdoutApprovalRecordId = requireString(
      record.holdout_approval_record_id,
      `${prefix}.holdout_approval_record_id`,
    );
    const approvalRecordSha256 = requireString(
      record.approval_record_sha256,
      `${prefix}.approval_record_sha256`,
    );
    const approvedByActorId = requireString(
      record.approved_by_actor_id,
      `${prefix}.approved_by_actor_id`,
    );
    validateDomainId(holdoutApprovalRecordId, `${prefix}.holdout_approval_record_id`);
    validateSha256(approvalRecordSha256, `${prefix}.approval_record_sha256`);
    validateDomainId(approvedByActorId, `${prefix}.approved_by_actor_id`);

    if (
      recordIds.has(holdoutApprovalRecordId) ||
      recordDigests.has(approvalRecordSha256) ||
      actorIds.has(approvedByActorId)
    ) {
      fail(
        "non_canonical_payload",
        "payload.approval_records",
        "approval record IDs, digests, and actor IDs must each be unique",
      );
    }
    if (previousActorId !== undefined && approvedByActorId <= previousActorId) {
      fail(
        "non_canonical_payload",
        "payload.approval_records",
        "approval records must be in strict ASCII order by approved_by_actor_id",
      );
    }

    recordIds.add(holdoutApprovalRecordId);
    recordDigests.add(approvalRecordSha256);
    actorIds.add(approvedByActorId);
    previousActorId = approvedByActorId;
    records.push({ holdoutApprovalRecordId, approvalRecordSha256, approvedByActorId });
  }
  return records;
}

function validateSha256(value: string, field: string): void {
  if (!SHA256_PATTERN.test(value)) {
    fail("invalid_digest", field, "digest must use sha256: and 64 lowercase hexadecimal digits");
  }
}

function validateClosedEnum(value: string, allowed: readonly string[], field: string): void {
  if (!allowed.includes(value)) {
    fail("invalid_enum", field, "value is not registered in the closed audit payload enum");
  }
}

function validateTimestampField(value: string, field: string): void {
  try {
    validateTimestamp(value);
  } catch {
    fail(
      "invalid_timestamp",
      field,
      "timestamp must be a valid UTC instant with exactly nine fractional digits",
    );
  }
}

function writePayloadFields(fields: readonly (readonly [string, string])[]): string {
  return `{${fields.map(([name, value]) => `"${name}":${writeJsonString(value)}`).join(",")}}`;
}

function writeHoldoutGrantIssuedPayload(
  holdoutGrantId: string,
  holdoutPeriodId: string,
  freezeManifestSha256: string,
  holdoutEvaluationPlanId: string,
  approvalRecords: readonly HoldoutApprovalAuditRecord[],
  capabilityClass: string,
  authorizationDecision: string,
): string {
  const records = approvalRecords
    .map((record) =>
      writePayloadFields([
        ["holdout_approval_record_id", record.holdoutApprovalRecordId],
        ["approval_record_sha256", record.approvalRecordSha256],
        ["approved_by_actor_id", record.approvedByActorId],
      ]),
    )
    .join(",");
  return (
    `{"holdout_grant_id":${writeJsonString(holdoutGrantId)},` +
    `"holdout_period_id":${writeJsonString(holdoutPeriodId)},` +
    `"freeze_manifest_sha256":${writeJsonString(freezeManifestSha256)},` +
    `"holdout_evaluation_plan_id":${writeJsonString(holdoutEvaluationPlanId)},` +
    `"approval_records":[${records}],` +
    `"capability_class":${writeJsonString(capabilityClass)},` +
    `"authorization_decision":${writeJsonString(authorizationDecision)}}`
  );
}

function writeJsonString(value: string): string {
  validateUnicodeScalar(value, "json_string");
  let output = '"';
  for (const character of value) {
    switch (character) {
      case '"':
        output += '\\"';
        break;
      case "\\":
        output += "\\\\";
        break;
      case "\b":
        output += "\\b";
        break;
      case "\t":
        output += "\\t";
        break;
      case "\n":
        output += "\\n";
        break;
      case "\f":
        output += "\\f";
        break;
      case "\r":
        output += "\\r";
        break;
      default: {
        const codePoint = character.codePointAt(0);
        if (codePoint !== undefined && codePoint <= 0x1f) {
          output += `\\u00${codePoint.toString(16).padStart(2, "0")}`;
        } else {
          output += character;
        }
      }
    }
  }
  return `${output}"`;
}

function equalBytes(left: Uint8Array, right: Uint8Array): boolean {
  if (left.byteLength !== right.byteLength) return false;
  return left.every((byte, index) => byte === right[index]);
}

function fail(code: AuditErrorCode, field: string, detail: string): never {
  throw new AuditValidationError(code, field, detail);
}
