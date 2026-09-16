import { createHash, timingSafeEqual } from "node:crypto";
import { validate_artifact_ref } from "./artifact.js";
import type { ArtifactRef } from "./generated/loop/v1/artifact_pb.js";
import type { Sha256Digest } from "./generated/loop/v1/common_pb.js";
import { SampleRole } from "./generated/loop/v1/data_pb.js";
import type {
  HoldoutPeriod as WireHoldoutPeriod,
  HoldoutEvaluationPlanReference as WirePlanReference,
} from "./generated/loop/v1/holdout_pb.js";

const PERIOD_SCHEMA = "loop.holdout-period/v1";
const PLAN_SCHEMA = "loop.holdout-evaluation-plan/v1";
const PERIOD_DOMAIN = "loop.holdout-period/v1";
const PLAN_DOMAIN = "loop.holdout-evaluation-plan/v1";
const BACKTEST_SCHEMA_NAME = "loop.backtest_spec";
const PLAN_ARTIFACT_SCHEMA_NAME = "loop.holdout_evaluation_plan";
const JSON_MEDIA_TYPE = "application/json";
const SHA256_PATTERN = /^sha256:[0-9a-f]{64}$/;
const UNSIGNED_PATTERN = /^(?:0|[1-9][0-9]*)$/;
const DATE_PATTERN = /^(\d{4})-(\d{2})-(\d{2})$/;
const CURRENCY_PATTERN = /^[A-Z]{3}$/;
const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true });

export const MAX_HOLDOUT_PERIOD_BYTES = 64 * 1_024;
export const MAX_HOLDOUT_PLAN_BYTES = 8 * 1_024 * 1_024;
export const MAX_HOLDOUT_SNAPSHOTS = 128;
export const MAX_HOLDOUT_PLAN_ENTRIES = 4_096;
export const MAX_BACKTEST_ARTIFACT_BYTES = 268_435_456n;
export const MAX_HOLDOUT_STEPS = 1_000_000n;
export const MAX_HOLDOUT_TOKENS = 1_000_000_000_000n;
export const MAX_HOLDOUT_WALL_TIME_NS = 604_800_000_000_000n;

export type HoldoutValidationCode =
  | "invalid_json"
  | "non_canonical"
  | "size_limit"
  | "invalid_schema"
  | "invalid_role"
  | "invalid_date"
  | "invalid_window"
  | "invalid_digest"
  | "invalid_snapshots"
  | "period_mismatch"
  | "invalid_entries"
  | "duplicate_identity"
  | "invalid_artifact"
  | "schema_mismatch"
  | "invalid_budget"
  | "unresolved_artifact"
  | "reference_mismatch";

export class HoldoutValidationError extends Error {
  public override readonly name = "HoldoutValidationError";

  public constructor(
    public readonly code: HoldoutValidationCode,
    public readonly field: string,
  ) {
    super(`${field} failed holdout validation (${code})`);
  }
}

export type LockedSampleRole = "first_locked_confirmation" | "second_locked_historical_holdout";

export interface HoldoutSampleWindow {
  readonly role: LockedSampleRole;
  readonly start_inclusive: string;
  readonly end_inclusive: string;
}

export interface HoldoutPeriodValue {
  readonly sample: HoldoutSampleWindow;
  readonly snapshot_ids: readonly string[];
  readonly snapshot_manifest_sha256: string;
}

export interface CanonicalHoldoutPeriod {
  readonly value: HoldoutPeriodValue;
  readonly canonicalBytes: Uint8Array;
  readonly canonicalPeriodSha256: Uint8Array;
  readonly holdoutPeriodId: string;
}

export interface BacktestSpecArtifactValue {
  readonly artifact_id: string;
  readonly uri: string;
  readonly sha256: string;
  readonly schema_name: string;
  readonly schema_version: string;
  readonly schema_sha256: string;
  readonly media_type: string;
  readonly byte_size: string;
}

export interface HoldoutMoneyBudget {
  readonly amount: string;
  readonly currency_code: string;
}

export interface HoldoutJobBudget {
  readonly maximum_steps: string;
  readonly maximum_input_tokens: string;
  readonly maximum_output_tokens: string;
  readonly maximum_cost: HoldoutMoneyBudget;
  readonly maximum_wall_time_ns: string;
}

export interface HoldoutEvaluationPlanEntry {
  readonly entry_index: string;
  readonly factor_spec_id: string;
  readonly backtest_spec_artifact: BacktestSpecArtifactValue;
  readonly job_budget: HoldoutJobBudget;
}

export interface HoldoutEvaluationPlanValue {
  readonly holdout_period_id: string;
  readonly canonical_period_sha256: string;
  readonly entries: readonly HoldoutEvaluationPlanEntry[];
}

export interface CanonicalHoldoutEvaluationPlan {
  readonly value: HoldoutEvaluationPlanValue;
  readonly canonicalBytes: Uint8Array;
  readonly planSha256: Uint8Array;
  readonly holdoutEvaluationPlanId: string;
}

export interface PlanArtifactReference {
  readonly artifactId: string;
  readonly uri: string;
  readonly sha256: Uint8Array;
  readonly schemaName: string;
  readonly schemaVersion: number;
  readonly schemaSha256: Uint8Array;
  readonly mediaType: string;
  readonly byteSize: bigint;
  readonly hasRowCount: boolean;
  readonly hasManifestSha256: boolean;
}

export interface EvaluationPlanReference {
  readonly holdoutEvaluationPlanId: string;
  readonly canonicalPlan: PlanArtifactReference;
  readonly planSha256: Uint8Array;
  readonly entryCount: number;
  readonly holdoutPeriodId: string;
  readonly canonicalPeriodSha256: Uint8Array;
}

type JsonObject = Record<string, unknown>;

export function canonicalize_holdout_period(value: HoldoutPeriodValue): CanonicalHoldoutPeriod {
  validate_period(value);
  const canonicalBytes = encoder.encode(write_period(value));
  if (canonicalBytes.byteLength > MAX_HOLDOUT_PERIOD_BYTES) fail("size_limit", "period");
  const digest = domain_hash(PERIOD_DOMAIN, canonicalBytes);
  return Object.freeze({
    value: freeze_period(value),
    canonicalBytes,
    canonicalPeriodSha256: digest,
    holdoutPeriodId: encode_digest(digest),
  });
}

export function parse_holdout_period(input: Uint8Array | string): CanonicalHoldoutPeriod {
  const { bytes, value } = parse_json(input, MAX_HOLDOUT_PERIOD_BYTES, "period");
  const raw = require_object(value, "period");
  require_exact_keys(
    raw,
    ["schema", "sample", "snapshot_ids", "snapshot_manifest_sha256"],
    "period",
  );
  if (require_string(raw.schema, "schema") !== PERIOD_SCHEMA) fail("invalid_schema", "schema");
  const sample = require_object(raw.sample, "sample");
  require_exact_keys(sample, ["role", "start_inclusive", "end_inclusive"], "sample");
  const snapshots = require_array(raw.snapshot_ids, "snapshot_ids").map((item, index) =>
    require_string(item, `snapshot_ids[${index}]`),
  );
  const parsed = canonicalize_holdout_period({
    sample: {
      role: parse_role(require_string(sample.role, "sample.role")),
      start_inclusive: require_string(sample.start_inclusive, "sample.start_inclusive"),
      end_inclusive: require_string(sample.end_inclusive, "sample.end_inclusive"),
    },
    snapshot_ids: snapshots,
    snapshot_manifest_sha256: require_string(
      raw.snapshot_manifest_sha256,
      "snapshot_manifest_sha256",
    ),
  });
  if (!equal_bytes(bytes, parsed.canonicalBytes)) fail("non_canonical", "period");
  return parsed;
}

export function verify_period_identity(
  input: Uint8Array | string,
  holdoutPeriodId: string,
  canonicalPeriodSha256: Uint8Array,
): CanonicalHoldoutPeriod {
  const parsed = parse_holdout_period(input);
  if (
    parsed.holdoutPeriodId !== holdoutPeriodId ||
    !equal_digest(parsed.canonicalPeriodSha256, canonicalPeriodSha256)
  ) {
    fail("period_mismatch", "holdout_period_id");
  }
  return parsed;
}

export function canonicalize_holdout_plan(
  value: HoldoutEvaluationPlanValue,
  expectedPeriod: CanonicalHoldoutPeriod,
  trustedBacktestSchemaSha256: Uint8Array,
  resolvedBacktestArtifacts: ReadonlyMap<string, Uint8Array>,
): CanonicalHoldoutEvaluationPlan {
  require_raw_digest(trustedBacktestSchemaSha256, "trusted_backtest_schema_sha256");
  validate_plan(value, expectedPeriod, trustedBacktestSchemaSha256, resolvedBacktestArtifacts);
  const canonicalBytes = encoder.encode(write_plan(value));
  if (canonicalBytes.byteLength > MAX_HOLDOUT_PLAN_BYTES) fail("size_limit", "plan");
  return Object.freeze({
    value: freeze_plan(value),
    canonicalBytes,
    planSha256: raw_hash(canonicalBytes),
    holdoutEvaluationPlanId: encode_digest(domain_hash(PLAN_DOMAIN, canonicalBytes)),
  });
}

export function parse_holdout_plan(
  input: Uint8Array | string,
  expectedPeriod: CanonicalHoldoutPeriod,
  trustedBacktestSchemaSha256: Uint8Array,
  resolvedBacktestArtifacts: ReadonlyMap<string, Uint8Array>,
): CanonicalHoldoutEvaluationPlan {
  const { bytes, value } = parse_json(input, MAX_HOLDOUT_PLAN_BYTES, "plan");
  const raw = require_object(value, "plan");
  require_exact_keys(
    raw,
    ["schema", "holdout_period_id", "canonical_period_sha256", "entries"],
    "plan",
  );
  if (require_string(raw.schema, "schema") !== PLAN_SCHEMA) fail("invalid_schema", "schema");
  const entries = require_array(raw.entries, "entries").map((entry, index) =>
    decode_plan_entry(entry, index),
  );
  const parsed = canonicalize_holdout_plan(
    {
      holdout_period_id: require_string(raw.holdout_period_id, "holdout_period_id"),
      canonical_period_sha256: require_string(
        raw.canonical_period_sha256,
        "canonical_period_sha256",
      ),
      entries,
    },
    expectedPeriod,
    trustedBacktestSchemaSha256,
    resolvedBacktestArtifacts,
  );
  if (!equal_bytes(bytes, parsed.canonicalBytes)) fail("non_canonical", "plan");
  return parsed;
}

export function validate_plan_reference(
  reference: EvaluationPlanReference,
  canonicalPlanBytes: Uint8Array,
  expectedPeriod: CanonicalHoldoutPeriod,
  trustedPlanSchemaSha256: Uint8Array,
  trustedBacktestSchemaSha256: Uint8Array,
  resolvedBacktestArtifacts: ReadonlyMap<string, Uint8Array>,
): CanonicalHoldoutEvaluationPlan {
  require_raw_digest(trustedPlanSchemaSha256, "trusted_plan_schema_sha256");
  const parsed = parse_holdout_plan(
    canonicalPlanBytes,
    expectedPeriod,
    trustedBacktestSchemaSha256,
    resolvedBacktestArtifacts,
  );
  const rawId = encode_digest(parsed.planSha256);
  const artifact = reference.canonicalPlan;
  if (
    artifact.artifactId !== rawId ||
    artifact.uri !== `artifact://sha256/${rawId.slice(7)}` ||
    !equal_digest(artifact.sha256, parsed.planSha256) ||
    !equal_digest(reference.planSha256, parsed.planSha256) ||
    artifact.byteSize !== BigInt(canonicalPlanBytes.byteLength) ||
    artifact.hasRowCount ||
    artifact.hasManifestSha256
  ) {
    fail("reference_mismatch", "canonical_plan");
  }
  if (
    artifact.schemaName !== PLAN_ARTIFACT_SCHEMA_NAME ||
    artifact.schemaVersion !== 1 ||
    artifact.mediaType !== JSON_MEDIA_TYPE ||
    !equal_digest(artifact.schemaSha256, trustedPlanSchemaSha256)
  ) {
    fail("schema_mismatch", "canonical_plan.schema");
  }
  if (
    reference.holdoutEvaluationPlanId !== parsed.holdoutEvaluationPlanId ||
    reference.entryCount !== parsed.value.entries.length ||
    reference.holdoutPeriodId !== expectedPeriod.holdoutPeriodId ||
    !equal_digest(reference.canonicalPeriodSha256, expectedPeriod.canonicalPeriodSha256)
  ) {
    fail("reference_mismatch", "plan_reference");
  }
  return parsed;
}

export function validate_wire_period(
  wire: WireHoldoutPeriod,
  canonicalPeriodBytes: Uint8Array,
): CanonicalHoldoutPeriod {
  const periodId = wire.holdoutPeriodId?.value;
  if (periodId === undefined) fail("reference_mismatch", "holdout_period_id");
  const digest = require_wire_digest(wire.canonicalPeriodSha256, "canonical_period_sha256");
  const parsed = verify_period_identity(canonicalPeriodBytes, periodId, digest);
  const sample = wire.sample;
  if (sample === undefined) fail("reference_mismatch", "sample");
  const role =
    sample.role === SampleRole.FIRST_LOCKED_CONFIRMATION
      ? "first_locked_confirmation"
      : sample.role === SampleRole.SECOND_LOCKED_HISTORICAL_HOLDOUT
        ? "second_locked_historical_holdout"
        : undefined;
  if (role === undefined) fail("reference_mismatch", "sample.role");
  const start = format_wire_date(sample.startInclusive, "sample.start_inclusive");
  const end = format_wire_date(sample.endInclusive, "sample.end_inclusive");
  const snapshots = wire.snapshotIds.map((value) => value.value);
  const manifest = require_wire_digest(wire.snapshotManifestSha256, "snapshot_manifest_sha256");
  if (
    parsed.value.sample.role !== role ||
    parsed.value.sample.start_inclusive !== start ||
    parsed.value.sample.end_inclusive !== end ||
    snapshots.length !== parsed.value.snapshot_ids.length ||
    snapshots.some((value, index) => value !== parsed.value.snapshot_ids[index]) ||
    parsed.value.snapshot_manifest_sha256 !== encode_digest(manifest)
  ) {
    fail("reference_mismatch", "holdout_period");
  }
  return parsed;
}

export function validate_wire_plan(
  wire: WirePlanReference,
  canonicalPlanBytes: Uint8Array,
  expectedPeriod: CanonicalHoldoutPeriod,
  trustedPlanSchemaSha256: Uint8Array,
  trustedBacktestSchemaSha256: Uint8Array,
  resolvedBacktestArtifacts: ReadonlyMap<string, Uint8Array>,
): CanonicalHoldoutEvaluationPlan {
  const wireArtifact = wire.canonicalPlan;
  if (wireArtifact === undefined) fail("reference_mismatch", "canonical_plan");
  const artifact = validate_plan_artifact(wireArtifact);
  const planId = wire.holdoutEvaluationPlanId?.value;
  const periodId = wire.holdoutPeriodId?.value;
  if (planId === undefined) fail("reference_mismatch", "holdout_evaluation_plan_id");
  if (periodId === undefined) fail("reference_mismatch", "holdout_period_id");
  return validate_plan_reference(
    {
      holdoutEvaluationPlanId: planId,
      canonicalPlan: artifact,
      planSha256: require_wire_digest(wire.planSha256, "plan_sha256"),
      entryCount: wire.entryCount,
      holdoutPeriodId: periodId,
      canonicalPeriodSha256: require_wire_digest(
        wire.canonicalPeriodSha256,
        "canonical_period_sha256",
      ),
    },
    canonicalPlanBytes,
    expectedPeriod,
    trustedPlanSchemaSha256,
    trustedBacktestSchemaSha256,
    resolvedBacktestArtifacts,
  );
}

function validate_plan_artifact(value: ArtifactRef): PlanArtifactReference {
  let artifact: ReturnType<typeof validate_artifact_ref>;
  try {
    artifact = validate_artifact_ref(value);
  } catch {
    fail("reference_mismatch", "canonical_plan");
  }
  return {
    artifactId: artifact.artifactId,
    uri: artifact.uri,
    sha256: decode_digest_hex(artifact.sha256Hex),
    schemaName: artifact.schemaName,
    schemaVersion: artifact.schemaVersion,
    schemaSha256: decode_digest_hex(artifact.schemaSha256Hex),
    mediaType: artifact.mediaType,
    byteSize: artifact.byteSize,
    hasRowCount: artifact.rowCount !== undefined,
    hasManifestSha256: artifact.manifestSha256Hex !== undefined,
  };
}

function validate_period(value: HoldoutPeriodValue): void {
  const start = parse_date(value.sample.start_inclusive);
  const end = parse_date(value.sample.end_inclusive);
  parse_role(value.sample.role);
  if (start > end) fail("invalid_window", "sample");
  if (value.snapshot_ids.length < 1 || value.snapshot_ids.length > MAX_HOLDOUT_SNAPSHOTS) {
    fail("invalid_snapshots", "snapshot_ids");
  }
  let previous: string | undefined;
  for (const snapshot of value.snapshot_ids) {
    require_digest_text(snapshot, "snapshot_ids");
    if (previous !== undefined && previous >= snapshot) fail("invalid_snapshots", "snapshot_ids");
    previous = snapshot;
  }
  require_digest_text(value.snapshot_manifest_sha256, "snapshot_manifest_sha256");
}

function validate_plan(
  value: HoldoutEvaluationPlanValue,
  expectedPeriod: CanonicalHoldoutPeriod,
  trustedBacktestSchemaSha256: Uint8Array,
  resolved: ReadonlyMap<string, Uint8Array>,
): void {
  if (
    value.holdout_period_id !== expectedPeriod.holdoutPeriodId ||
    value.canonical_period_sha256 !== expectedPeriod.holdoutPeriodId
  ) {
    fail("period_mismatch", "holdout_period_id");
  }
  if (value.entries.length < 1 || value.entries.length > MAX_HOLDOUT_PLAN_ENTRIES) {
    fail("invalid_entries", "entries");
  }
  const factors = new Set<string>();
  const artifacts = new Set<string>();
  value.entries.forEach((entry, index) => {
    if (
      parse_unsigned(entry.entry_index, 1n, BigInt(MAX_HOLDOUT_PLAN_ENTRIES), "entry_index") !==
      BigInt(index + 1)
    ) {
      fail("invalid_entries", "entry_index");
    }
    require_digest_text(entry.factor_spec_id, "factor_spec_id");
    if (factors.has(entry.factor_spec_id)) fail("duplicate_identity", "factor_spec_id");
    factors.add(entry.factor_spec_id);
    validate_backtest_artifact(entry.backtest_spec_artifact, trustedBacktestSchemaSha256, resolved);
    if (artifacts.has(entry.backtest_spec_artifact.artifact_id)) {
      fail("duplicate_identity", "backtest_spec_artifact.artifact_id");
    }
    artifacts.add(entry.backtest_spec_artifact.artifact_id);
    validate_budget(entry.job_budget);
  });
}

function validate_backtest_artifact(
  artifact: BacktestSpecArtifactValue,
  trustedSchemaSha256: Uint8Array,
  resolved: ReadonlyMap<string, Uint8Array>,
): void {
  const digest = require_digest_text(artifact.sha256, "backtest_spec_artifact.sha256");
  if (
    artifact.artifact_id !== artifact.sha256 ||
    artifact.uri !== `artifact://sha256/${artifact.sha256.slice(7)}`
  ) {
    fail("invalid_artifact", "backtest_spec_artifact");
  }
  if (
    artifact.schema_name !== BACKTEST_SCHEMA_NAME ||
    artifact.schema_version !== "1" ||
    artifact.media_type !== JSON_MEDIA_TYPE ||
    !equal_digest(
      require_digest_text(artifact.schema_sha256, "backtest_spec_artifact.schema_sha256"),
      trustedSchemaSha256,
    )
  ) {
    fail("schema_mismatch", "backtest_spec_artifact.schema");
  }
  const size = parse_unsigned(
    artifact.byte_size,
    1n,
    MAX_BACKTEST_ARTIFACT_BYTES,
    "backtest_spec_artifact.byte_size",
  );
  const content = resolved.get(artifact.sha256);
  if (content === undefined) fail("unresolved_artifact", "backtest_spec_artifact");
  if (BigInt(content.byteLength) !== size || !equal_digest(raw_hash(content), digest)) {
    fail("invalid_artifact", "backtest_spec_artifact");
  }
}

function validate_budget(value: HoldoutJobBudget): void {
  parse_unsigned(value.maximum_steps, 1n, MAX_HOLDOUT_STEPS, "maximum_steps");
  parse_unsigned(value.maximum_input_tokens, 0n, MAX_HOLDOUT_TOKENS, "maximum_input_tokens");
  parse_unsigned(value.maximum_output_tokens, 0n, MAX_HOLDOUT_TOKENS, "maximum_output_tokens");
  parse_unsigned(value.maximum_wall_time_ns, 1n, MAX_HOLDOUT_WALL_TIME_NS, "maximum_wall_time_ns");
  validate_cost(value.maximum_cost);
}

function validate_cost(value: HoldoutMoneyBudget): void {
  if (!CURRENCY_PATTERN.test(value.currency_code))
    fail("invalid_budget", "maximum_cost.currency_code");
  const match = /^(0|[1-9][0-9]*)(?:\.([0-9]*[1-9]))?$/.exec(value.amount);
  if (match === null) fail("invalid_budget", "maximum_cost.amount");
  const integer = match[1] ?? "";
  const fraction = match[2];
  if (fraction !== undefined && fraction.length > 9) fail("invalid_budget", "maximum_cost.amount");
  const significant =
    integer === "0"
      ? Math.max((fraction ?? "").replace(/^0+/, "").length, 1)
      : integer.length + (fraction?.length ?? 0);
  if (
    significant > 18 ||
    BigInt(integer) > 1_000_000n ||
    (integer === "1000000" && fraction !== undefined)
  ) {
    fail("invalid_budget", "maximum_cost.amount");
  }
}

function decode_plan_entry(value: unknown, index: number): HoldoutEvaluationPlanEntry {
  const path = `entries[${index}]`;
  const raw = require_object(value, path);
  require_exact_keys(
    raw,
    ["entry_index", "factor_spec_id", "backtest_spec_artifact", "job_budget"],
    path,
  );
  const artifact = require_object(raw.backtest_spec_artifact, `${path}.backtest_spec_artifact`);
  require_exact_keys(
    artifact,
    [
      "artifact_id",
      "uri",
      "sha256",
      "schema_name",
      "schema_version",
      "schema_sha256",
      "media_type",
      "byte_size",
    ],
    `${path}.backtest_spec_artifact`,
  );
  const budget = require_object(raw.job_budget, `${path}.job_budget`);
  require_exact_keys(
    budget,
    [
      "maximum_steps",
      "maximum_input_tokens",
      "maximum_output_tokens",
      "maximum_cost",
      "maximum_wall_time_ns",
    ],
    `${path}.job_budget`,
  );
  const cost = require_object(budget.maximum_cost, `${path}.job_budget.maximum_cost`);
  require_exact_keys(cost, ["amount", "currency_code"], `${path}.job_budget.maximum_cost`);
  return {
    entry_index: require_string(raw.entry_index, `${path}.entry_index`),
    factor_spec_id: require_string(raw.factor_spec_id, `${path}.factor_spec_id`),
    backtest_spec_artifact: {
      artifact_id: require_string(
        artifact.artifact_id,
        `${path}.backtest_spec_artifact.artifact_id`,
      ),
      uri: require_string(artifact.uri, `${path}.backtest_spec_artifact.uri`),
      sha256: require_string(artifact.sha256, `${path}.backtest_spec_artifact.sha256`),
      schema_name: require_string(
        artifact.schema_name,
        `${path}.backtest_spec_artifact.schema_name`,
      ),
      schema_version: require_string(
        artifact.schema_version,
        `${path}.backtest_spec_artifact.schema_version`,
      ),
      schema_sha256: require_string(
        artifact.schema_sha256,
        `${path}.backtest_spec_artifact.schema_sha256`,
      ),
      media_type: require_string(artifact.media_type, `${path}.backtest_spec_artifact.media_type`),
      byte_size: require_string(artifact.byte_size, `${path}.backtest_spec_artifact.byte_size`),
    },
    job_budget: {
      maximum_steps: require_string(budget.maximum_steps, `${path}.job_budget.maximum_steps`),
      maximum_input_tokens: require_string(
        budget.maximum_input_tokens,
        `${path}.job_budget.maximum_input_tokens`,
      ),
      maximum_output_tokens: require_string(
        budget.maximum_output_tokens,
        `${path}.job_budget.maximum_output_tokens`,
      ),
      maximum_cost: {
        amount: require_string(cost.amount, `${path}.job_budget.maximum_cost.amount`),
        currency_code: require_string(
          cost.currency_code,
          `${path}.job_budget.maximum_cost.currency_code`,
        ),
      },
      maximum_wall_time_ns: require_string(
        budget.maximum_wall_time_ns,
        `${path}.job_budget.maximum_wall_time_ns`,
      ),
    },
  };
}

function parse_json(
  input: Uint8Array | string,
  maximumBytes: number,
  field: string,
): { readonly bytes: Uint8Array; readonly value: unknown } {
  const bytes = typeof input === "string" ? encoder.encode(input) : new Uint8Array(input);
  validate_json_envelope(bytes, maximumBytes);
  let text: string;
  try {
    text = decoder.decode(bytes);
  } catch {
    fail("invalid_json", field);
  }
  try {
    return { bytes, value: JSON.parse(text) as unknown };
  } catch {
    fail("invalid_json", field);
  }
}

function validate_json_envelope(bytes: Uint8Array, maximumBytes: number): void {
  if (bytes.byteLength < 1 || bytes.byteLength > maximumBytes) fail("size_limit", "json");
  let depth = 0;
  let inString = false;
  let escaped = false;
  for (const byte of bytes) {
    if (inString) {
      if (escaped) escaped = false;
      else if (byte === 0x5c) escaped = true;
      else if (byte === 0x22) inString = false;
      continue;
    }
    if (byte === 0x22) inString = true;
    else if (byte === 0x7b || byte === 0x5b) {
      depth += 1;
      if (depth > 32) fail("size_limit", "json.depth");
    } else if (byte === 0x7d || byte === 0x5d) {
      depth = Math.max(0, depth - 1);
    }
  }
}

function require_object(value: unknown, field: string): JsonObject {
  if (typeof value !== "object" || value === null || Array.isArray(value))
    fail("invalid_json", field);
  const prototype = Object.getPrototypeOf(value);
  if (prototype !== Object.prototype && prototype !== null) fail("invalid_json", field);
  return value as JsonObject;
}

function require_array(value: unknown, field: string): unknown[] {
  if (!Array.isArray(value)) fail("invalid_json", field);
  return value;
}

function require_exact_keys(value: JsonObject, expected: readonly string[], field: string): void {
  const actual = Object.keys(value);
  if (actual.length !== expected.length || actual.some((key, index) => key !== expected[index])) {
    fail("non_canonical", field);
  }
}

function require_string(value: unknown, field: string): string {
  if (typeof value !== "string") fail("invalid_json", field);
  return value;
}

function parse_role(value: string): LockedSampleRole {
  if (value === "first_locked_confirmation" || value === "second_locked_historical_holdout")
    return value;
  fail("invalid_role", "sample.role");
}

function parse_date(value: string): number {
  const match = DATE_PATTERN.exec(value);
  if (match === null) fail("invalid_date", "sample.date");
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  if (year < 1900 || year > 9999 || month < 1 || month > 12) fail("invalid_date", "sample.date");
  const leap = year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
  const days = [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
  if (day < 1 || day > (days[month - 1] ?? 0)) fail("invalid_date", "sample.date");
  return year * 10_000 + month * 100 + day;
}

function parse_unsigned(value: string, minimum: bigint, maximum: bigint, field: string): bigint {
  if (!UNSIGNED_PATTERN.test(value)) fail("invalid_budget", field);
  const parsed = BigInt(value);
  if (parsed < minimum || parsed > maximum) fail("invalid_budget", field);
  return parsed;
}

function require_digest_text(value: string, field: string): Uint8Array {
  if (!SHA256_PATTERN.test(value)) fail("invalid_digest", field);
  return decode_digest_hex(value.slice(7));
}

function require_raw_digest(value: Uint8Array, field: string): void {
  if (!(value instanceof Uint8Array) || value.byteLength !== 32) fail("invalid_digest", field);
}

function require_wire_digest(value: Sha256Digest | undefined, field: string): Uint8Array {
  if (value === undefined) fail("reference_mismatch", field);
  require_raw_digest(value.value, field);
  return new Uint8Array(value.value);
}

function format_wire_date(
  value: { readonly year: number; readonly month: number; readonly day: number } | undefined,
  field: string,
): string {
  if (value === undefined) fail("reference_mismatch", field);
  if (![value.year, value.month, value.day].every(Number.isSafeInteger))
    fail("reference_mismatch", field);
  return `${String(value.year).padStart(4, "0")}-${String(value.month).padStart(2, "0")}-${String(value.day).padStart(2, "0")}`;
}

function write_period(value: HoldoutPeriodValue): string {
  return `{"schema":"${PERIOD_SCHEMA}","sample":{"role":"${value.sample.role}","start_inclusive":"${value.sample.start_inclusive}","end_inclusive":"${value.sample.end_inclusive}"},"snapshot_ids":[${value.snapshot_ids.map(quote).join(",")}],"snapshot_manifest_sha256":"${value.snapshot_manifest_sha256}"}`;
}

function write_plan(value: HoldoutEvaluationPlanValue): string {
  const entries = value.entries.map((entry) => {
    const artifact = entry.backtest_spec_artifact;
    const budget = entry.job_budget;
    return `{"entry_index":"${entry.entry_index}","factor_spec_id":"${entry.factor_spec_id}","backtest_spec_artifact":{"artifact_id":"${artifact.artifact_id}","uri":"${artifact.uri}","sha256":"${artifact.sha256}","schema_name":"${artifact.schema_name}","schema_version":"${artifact.schema_version}","schema_sha256":"${artifact.schema_sha256}","media_type":"${artifact.media_type}","byte_size":"${artifact.byte_size}"},"job_budget":{"maximum_steps":"${budget.maximum_steps}","maximum_input_tokens":"${budget.maximum_input_tokens}","maximum_output_tokens":"${budget.maximum_output_tokens}","maximum_cost":{"amount":"${budget.maximum_cost.amount}","currency_code":"${budget.maximum_cost.currency_code}"},"maximum_wall_time_ns":"${budget.maximum_wall_time_ns}"}}`;
  });
  return `{"schema":"${PLAN_SCHEMA}","holdout_period_id":"${value.holdout_period_id}","canonical_period_sha256":"${value.canonical_period_sha256}","entries":[${entries.join(",")}]}`;
}

function quote(value: string): string {
  return `"${value}"`;
}

function freeze_period(value: HoldoutPeriodValue): HoldoutPeriodValue {
  return Object.freeze({
    sample: Object.freeze({ ...value.sample }),
    snapshot_ids: Object.freeze([...value.snapshot_ids]),
    snapshot_manifest_sha256: value.snapshot_manifest_sha256,
  });
}

function freeze_plan(value: HoldoutEvaluationPlanValue): HoldoutEvaluationPlanValue {
  return Object.freeze({
    holdout_period_id: value.holdout_period_id,
    canonical_period_sha256: value.canonical_period_sha256,
    entries: Object.freeze(
      value.entries.map((entry) =>
        Object.freeze({
          entry_index: entry.entry_index,
          factor_spec_id: entry.factor_spec_id,
          backtest_spec_artifact: Object.freeze({ ...entry.backtest_spec_artifact }),
          job_budget: Object.freeze({
            ...entry.job_budget,
            maximum_cost: Object.freeze({ ...entry.job_budget.maximum_cost }),
          }),
        }),
      ),
    ),
  });
}

function raw_hash(bytes: Uint8Array): Uint8Array {
  return new Uint8Array(createHash("sha256").update(bytes).digest());
}

function domain_hash(domain: string, bytes: Uint8Array): Uint8Array {
  return new Uint8Array(
    createHash("sha256").update(domain, "ascii").update(Uint8Array.of(0)).update(bytes).digest(),
  );
}

function encode_digest(value: Uint8Array): string {
  require_raw_digest(value, "digest");
  return `sha256:${Buffer.from(value).toString("hex")}`;
}

function decode_digest_hex(value: string): Uint8Array {
  return new Uint8Array(Buffer.from(value, "hex"));
}

function equal_digest(left: Uint8Array, right: Uint8Array): boolean {
  return left.byteLength === 32 && right.byteLength === 32 && timingSafeEqual(left, right);
}

function equal_bytes(left: Uint8Array, right: Uint8Array): boolean {
  return left.byteLength === right.byteLength && timingSafeEqual(left, right);
}

function fail(code: HoldoutValidationCode, field: string): never {
  throw new HoldoutValidationError(code, field);
}
