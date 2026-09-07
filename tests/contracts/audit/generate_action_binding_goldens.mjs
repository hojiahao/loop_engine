import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";

const fixtureUrl = new URL(
  "../../../fixtures/contracts/audit/v1/action_binding_vectors.json",
  import.meta.url,
);
const mode = process.argv[2] ?? "--check";
if (mode !== "--check" && mode !== "--write") {
  throw new Error("usage: generate_action_binding_goldens.mjs [--check|--write]");
}

const current = readFileSync(fixtureUrl, "utf8");
const fixture = JSON.parse(current);
const version = "1";

function sha256(parts) {
  const hash = createHash("sha256");
  for (const part of parts) hash.update(part);
  return `sha256:${hash.digest("hex")}`;
}

function canonicalEvent(envelope, vector, payloadSha256) {
  return (
    `{"schema":"loop.audit-event/v1","audit_ledger_id":${JSON.stringify(envelope.audit_ledger_id)},` +
    `"sequence":${JSON.stringify(envelope.sequence)},` +
    `"previous_event_sha256":${JSON.stringify(envelope.previous_event_sha256)},` +
    `"audit_event_id":${JSON.stringify(envelope.audit_event_id)},` +
    `"occurred_at":${JSON.stringify(envelope.occurred_at)},` +
    `"correlation_id":${JSON.stringify(envelope.correlation_id)},` +
    `"causation_id":${JSON.stringify(envelope.causation_id)},` +
    `"actor":{"actor_id":${JSON.stringify(envelope.actor.actor_id)},` +
    `"kind":${JSON.stringify(envelope.actor.kind)},` +
    `"display_name":${JSON.stringify(envelope.actor.display_name)},` +
    `"authenticated_subject":${JSON.stringify(envelope.actor.authenticated_subject)}},` +
    `"action":${JSON.stringify(vector.action)},` +
    `"target":{"kind":${JSON.stringify(vector.target.kind)},` +
    `"value":${JSON.stringify(vector.target.value)}},` +
    `"payload":{"schema_name":${JSON.stringify(vector.payload_schema)},` +
    `"schema_version":"1","payload_sha256":${JSON.stringify(payloadSha256)}}}`
  );
}

const accepted = fixture.accepted.map((vector) => {
  const parsedPayload = JSON.parse(vector.canonical_payload);
  if (JSON.stringify(parsedPayload) !== vector.canonical_payload) {
    throw new Error(`${vector.name}: canonical_payload is not compact canonical JSON`);
  }
  const payloadSha256 = sha256([
    Buffer.from("loop.audit-payload/v1\0", "ascii"),
    Buffer.from(vector.payload_schema, "ascii"),
    Buffer.from([0]),
    Buffer.from(version, "ascii"),
    Buffer.from([0]),
    Buffer.from(vector.canonical_payload, "utf8"),
  ]);
  const canonicalEventUtf8 = canonicalEvent(fixture.event_envelope, vector, payloadSha256);
  const eventSha256 = sha256([
    Buffer.from("loop.audit-event/v1\0", "ascii"),
    Buffer.from(canonicalEventUtf8, "utf8"),
  ]);
  return {
    ...vector,
    payload_sha256: payloadSha256,
    canonical_event_utf8: canonicalEventUtf8,
    event_sha256: eventSha256,
  };
});

const rendered = `${JSON.stringify({ ...fixture, accepted }, null, 2)}\n`;
if (mode === "--write") {
  writeFileSync(fixtureUrl, rendered);
} else if (current !== rendered) {
  throw new Error("audit action-binding golden fixture is stale; run with --write");
}
