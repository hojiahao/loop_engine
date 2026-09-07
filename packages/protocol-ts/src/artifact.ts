import type { ArtifactRef } from "./generated/loop/v1/artifact_pb.js";

export const MAX_ARTIFACT_URI_BYTES = 2_048;
const CONTENT_ADDRESS_PREFIX = "artifact://sha256/";
const CONTENT_ADDRESS_PATTERN = /^artifact:\/\/sha256\/[0-9a-f]{64}$/;
const SCHEMA_NAME_PATTERN = /^[a-z][a-z0-9_]*(?:\.[a-z][a-z0-9_]*)*$/;
const MIN_TIMESTAMP_SECONDS = -62_135_596_800n;
const MAX_TIMESTAMP_SECONDS = 253_402_300_799n;
const textEncoder = new TextEncoder();

export type ArtifactValidationCode =
  | "missing_field"
  | "invalid_digest"
  | "invalid_locator"
  | "uri_too_long"
  | "identity_mismatch"
  | "invalid_schema"
  | "invalid_media_type"
  | "invalid_timestamp";

export class ArtifactValidationError extends Error {
  public override readonly name = "ArtifactValidationError";

  public constructor(
    public readonly code: ArtifactValidationCode,
    public readonly field: string,
  ) {
    super(`${field} failed artifact validation (${code})`);
  }
}

export interface ValidatedArtifactRef {
  readonly artifactId: string;
  readonly uri: string;
  readonly sha256Hex: string;
  readonly schemaName: string;
  readonly schemaVersion: number;
  readonly schemaSha256Hex: string;
  readonly mediaType: string;
  readonly byteSize: bigint;
  readonly rowCount?: bigint;
  readonly createdAtSeconds: bigint;
  readonly createdAtNanos: number;
  readonly manifestSha256Hex?: string;
}

/** Validate a generated DTO before domain or storage code can observe it. */
export function validateArtifactRef(value: ArtifactRef): Readonly<ValidatedArtifactRef> {
  rejectInlineProperties(value);
  const digestHex = requireDigest(value.sha256, "sha256");

  if (textEncoder.encode(value.uri).byteLength > MAX_ARTIFACT_URI_BYTES) {
    fail("uri_too_long", "uri");
  }
  if (!CONTENT_ADDRESS_PATTERN.test(value.uri)) {
    fail("invalid_locator", "uri");
  }
  if (value.uri !== `${CONTENT_ADDRESS_PREFIX}${digestHex}`) {
    fail("identity_mismatch", "uri");
  }

  const artifactId = value.artifactId?.value;
  if (artifactId === undefined) {
    fail("missing_field", "artifact_id");
  }
  if (artifactId !== `sha256:${digestHex}`) {
    fail("identity_mismatch", "artifact_id");
  }

  const schema = value.schema;
  if (schema === undefined) {
    fail("missing_field", "schema");
  }
  if (
    schema.version < 1 ||
    !Number.isSafeInteger(schema.version) ||
    !SCHEMA_NAME_PATTERN.test(schema.name) ||
    textEncoder.encode(schema.name).byteLength > 128
  ) {
    fail("invalid_schema", "schema");
  }
  const schemaSha256Hex = requireDigest(schema.schemaSha256, "schema.schema_sha256");
  if (!isMediaType(value.mediaType)) {
    fail("invalid_media_type", "media_type");
  }
  const createdAt = requireTimestamp(value.createdAt, "created_at");
  const manifestSha256Hex =
    value.manifestSha256 === undefined
      ? undefined
      : requireDigest(value.manifestSha256, "manifest_sha256");

  return Object.freeze({
    artifactId,
    uri: value.uri,
    sha256Hex: digestHex,
    schemaName: schema.name,
    schemaVersion: schema.version,
    schemaSha256Hex,
    mediaType: value.mediaType,
    byteSize: value.byteSize,
    ...(value.rowCount === undefined ? {} : { rowCount: value.rowCount }),
    createdAtSeconds: createdAt.seconds,
    createdAtNanos: createdAt.nanos,
    ...(manifestSha256Hex === undefined ? {} : { manifestSha256Hex }),
  });
}

function requireTimestamp(
  timestamp: { readonly seconds: bigint; readonly nanos: number } | undefined,
  field: string,
): { readonly seconds: bigint; readonly nanos: number } {
  if (timestamp === undefined) {
    fail("missing_field", field);
  }
  if (
    timestamp.seconds < MIN_TIMESTAMP_SECONDS ||
    timestamp.seconds > MAX_TIMESTAMP_SECONDS ||
    !Number.isInteger(timestamp.nanos) ||
    timestamp.nanos < 0 ||
    timestamp.nanos >= 1_000_000_000
  ) {
    fail("invalid_timestamp", field);
  }
  return timestamp;
}

function rejectInlineProperties(value: ArtifactRef): void {
  const record = value as ArtifactRef & Record<string, unknown>;
  for (const field of ["bytes", "data", "payload", "content", "inlineBytes", "inline_bytes"]) {
    if (Object.hasOwn(record, field)) {
      fail("invalid_locator", field);
    }
  }
}

function requireDigest(digest: { readonly value: Uint8Array } | undefined, field: string): string {
  if (digest === undefined) {
    fail("missing_field", field);
  }
  if (!(digest.value instanceof Uint8Array) || digest.value.byteLength !== 32) {
    fail("invalid_digest", field);
  }
  return Array.from(digest.value, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

function isMediaType(value: string): boolean {
  const parts = value.split("/");
  return (
    parts.length === 2 &&
    parts.every((part) => part.length > 0) &&
    textEncoder.encode(value).byteLength <= 255 &&
    Array.from(value).every((character) => {
      const code = character.charCodeAt(0);
      return code > 32 && code < 127;
    })
  );
}

function fail(code: ArtifactValidationCode, field: string): never {
  throw new ArtifactValidationError(code, field);
}
