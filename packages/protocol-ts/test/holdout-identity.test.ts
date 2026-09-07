import { readFileSync } from "node:fs";
import { create } from "@bufbuild/protobuf";
import { TimestampSchema } from "@bufbuild/protobuf/wkt";
import { describe, expect, it } from "vitest";
import {
  ArtifactRefSchema,
  ArtifactSchemaReferenceSchema,
} from "../src/generated/loop/v1/artifact_pb.js";
import {
  ArtifactIdSchema,
  CivilDateSchema,
  HoldoutEvaluationPlanIdSchema,
  HoldoutPeriodIdSchema,
  Sha256DigestSchema,
  SnapshotIdSchema,
} from "../src/generated/loop/v1/common_pb.js";
import { SampleRole, SampleWindowSchema } from "../src/generated/loop/v1/data_pb.js";
import {
  HoldoutEvaluationPlanReferenceSchema,
  HoldoutPeriodSchema,
} from "../src/generated/loop/v1/holdout_pb.js";
import {
  type BacktestSpecArtifactValue,
  type CanonicalHoldoutEvaluationPlan,
  type CanonicalHoldoutPeriod,
  type EvaluationPlanReference,
  HoldoutValidationError,
  type PlanArtifactReference,
  parseCanonicalHoldoutEvaluationPlan,
  parseCanonicalHoldoutPeriod,
  validateHoldoutEvaluationPlanReference,
  validateWireHoldoutEvaluationPlanReference,
  validateWireHoldoutPeriod,
  verifyHoldoutPeriodIdentity,
} from "../src/holdout-identity.js";

interface GoldenFixture {
  readonly trusted_plan_schema_sha256: string;
  readonly trusted_backtest_schema_sha256: string;
  readonly backtest_artifacts: readonly { readonly sha256: string; readonly content: string }[];
  readonly periods: readonly {
    readonly name: string;
    readonly canonical_json: string;
    readonly canonical_sha256: string;
    readonly holdout_period_id: string;
  }[];
  readonly plans: readonly {
    readonly name: string;
    readonly period: string;
    readonly canonical_json: string;
    readonly plan_sha256: string;
    readonly holdout_evaluation_plan_id: string;
    readonly entry_count: number;
  }[];
}

interface MutablePlanJson {
  schema: string;
  holdout_period_id: string;
  canonical_period_sha256: string;
  entries: Array<{
    entry_index: string;
    factor_spec_id: string;
    backtest_spec_artifact: BacktestSpecArtifactValue & Record<string, unknown>;
    job_budget: {
      maximum_steps: string;
      maximum_input_tokens: string;
      maximum_output_tokens: string;
      maximum_cost: { amount: string; currency_code: string };
      maximum_wall_time_ns: string;
    } & Record<string, unknown>;
  }>;
  [key: string]: unknown;
}

const fixture = JSON.parse(
  readFileSync(
    new URL("../../../tests/contracts/holdout_identity_golden.json", import.meta.url),
    "utf8",
  ),
) as GoldenFixture;
const negativeVectors = readFileSync(
  new URL("../../../tests/contracts/holdout_identity_negative.tsv", import.meta.url),
  "utf8",
);
const encoder = new TextEncoder();
const trustedPlan = decodeDigest(fixture.trusted_plan_schema_sha256);
const trustedBacktest = decodeDigest(fixture.trusted_backtest_schema_sha256);
const resolved = new Map(
  fixture.backtest_artifacts.map((artifact) => [artifact.sha256, encoder.encode(artifact.content)]),
);

describe("holdout canonical identity v1", () => {
  it("matches every shared period and plan golden byte for byte", () => {
    for (const periodFixture of fixture.periods) {
      const period = parseCanonicalHoldoutPeriod(periodFixture.canonical_json);
      expect(new TextDecoder().decode(period.canonicalBytes), periodFixture.name).toBe(
        periodFixture.canonical_json,
      );
      expect(encodeDigest(period.canonicalPeriodSha256), periodFixture.name).toBe(
        periodFixture.canonical_sha256,
      );
      expect(period.holdoutPeriodId, periodFixture.name).toBe(periodFixture.holdout_period_id);

      for (const planFixture of fixture.plans.filter(
        (candidate) => candidate.period === periodFixture.name,
      )) {
        const plan = parseCanonicalHoldoutEvaluationPlan(
          planFixture.canonical_json,
          period,
          trustedBacktest,
          resolved,
        );
        expect(new TextDecoder().decode(plan.canonicalBytes), planFixture.name).toBe(
          planFixture.canonical_json,
        );
        expect(encodeDigest(plan.planSha256), planFixture.name).toBe(planFixture.plan_sha256);
        expect(plan.holdoutEvaluationPlanId, planFixture.name).toBe(
          planFixture.holdout_evaluation_plan_id,
        );
        expect(
          validateHoldoutEvaluationPlanReference(
            domainReference(plan, period, planFixture.entry_count),
            plan.canonicalBytes,
            period,
            trustedPlan,
            trustedBacktest,
            resolved,
          ).holdoutEvaluationPlanId,
        ).toBe(planFixture.holdout_evaluation_plan_id);
      }
    }
  });

  it("validates generated period and plan references as exact canonical projections", () => {
    const periodFixture = requireItem(fixture.periods, 0);
    const planFixture = requireItem(fixture.plans, 0);
    const period = parseCanonicalHoldoutPeriod(periodFixture.canonical_json);
    const plan = parseCanonicalHoldoutEvaluationPlan(
      planFixture.canonical_json,
      period,
      trustedBacktest,
      resolved,
    );
    const periodJson = JSON.parse(periodFixture.canonical_json) as {
      sample: { role: string; start_inclusive: string; end_inclusive: string };
      snapshot_ids: string[];
      snapshot_manifest_sha256: string;
    };
    const wirePeriod = create(HoldoutPeriodSchema, {
      holdoutPeriodId: create(HoldoutPeriodIdSchema, { value: period.holdoutPeriodId }),
      sample: create(SampleWindowSchema, {
        role: SampleRole.FIRST_LOCKED_CONFIRMATION,
        startInclusive: civilDate(periodJson.sample.start_inclusive),
        endInclusive: civilDate(periodJson.sample.end_inclusive),
      }),
      snapshotIds: periodJson.snapshot_ids.map((value) => create(SnapshotIdSchema, { value })),
      snapshotManifestSha256: wireDigest(decodeDigest(periodJson.snapshot_manifest_sha256)),
      canonicalPeriodSha256: wireDigest(period.canonicalPeriodSha256),
    });
    expect(validateWireHoldoutPeriod(wirePeriod, period.canonicalBytes).holdoutPeriodId).toBe(
      period.holdoutPeriodId,
    );

    const reference = domainReference(plan, period, planFixture.entry_count);
    const wirePlan = create(HoldoutEvaluationPlanReferenceSchema, {
      holdoutEvaluationPlanId: create(HoldoutEvaluationPlanIdSchema, {
        value: reference.holdoutEvaluationPlanId,
      }),
      canonicalPlan: create(ArtifactRefSchema, {
        artifactId: create(ArtifactIdSchema, { value: reference.canonicalPlan.artifactId }),
        uri: reference.canonicalPlan.uri,
        sha256: wireDigest(reference.canonicalPlan.sha256),
        schema: create(ArtifactSchemaReferenceSchema, {
          name: reference.canonicalPlan.schemaName,
          version: reference.canonicalPlan.schemaVersion,
          schemaSha256: wireDigest(reference.canonicalPlan.schemaSha256),
        }),
        mediaType: reference.canonicalPlan.mediaType,
        byteSize: reference.canonicalPlan.byteSize,
        createdAt: create(TimestampSchema, { seconds: 1n }),
      }),
      planSha256: wireDigest(reference.planSha256),
      entryCount: reference.entryCount,
      holdoutPeriodId: create(HoldoutPeriodIdSchema, { value: reference.holdoutPeriodId }),
      canonicalPeriodSha256: wireDigest(reference.canonicalPeriodSha256),
    });
    expect(
      validateWireHoldoutEvaluationPlanReference(
        wirePlan,
        plan.canonicalBytes,
        period,
        trustedPlan,
        trustedBacktest,
        resolved,
      ).holdoutEvaluationPlanId,
    ).toBe(plan.holdoutEvaluationPlanId);

    if (wirePlan.canonicalPlan === undefined) throw new Error("wire plan artifact is required");
    wirePlan.canonicalPlan.createdAt = undefined;
    expect(() =>
      validateWireHoldoutEvaluationPlanReference(
        wirePlan,
        plan.canonicalBytes,
        period,
        trustedPlan,
        trustedBacktest,
        resolved,
      ),
    ).toThrow(HoldoutValidationError);
    wirePlan.canonicalPlan.createdAt = create(TimestampSchema, { seconds: 1n });
    wirePlan.canonicalPlan.rowCount = 1n;
    expect(() =>
      validateWireHoldoutEvaluationPlanReference(
        wirePlan,
        plan.canonicalBytes,
        period,
        trustedPlan,
        trustedBacktest,
        resolved,
      ),
    ).toThrow(HoldoutValidationError);
  });

  it("executes every shared negative vector and fails closed", () => {
    const periodFixture = requireItem(fixture.periods, 0);
    const planFixture = requireItem(fixture.plans, 0);
    const period = parseCanonicalHoldoutPeriod(periodFixture.canonical_json);
    const plan = parseCanonicalHoldoutEvaluationPlan(
      planFixture.canonical_json,
      period,
      trustedBacktest,
      resolved,
    );
    for (const line of negativeVectors.split("\n")) {
      if (line.length === 0 || line.startsWith("#")) continue;
      const [name, target, mutation, extra] = line.split("\t");
      if (
        name === undefined ||
        target === undefined ||
        mutation === undefined ||
        extra !== undefined
      ) {
        throw new Error(`invalid negative holdout vector: ${line}`);
      }
      expect(
        () =>
          executeNegative(
            target,
            mutation,
            periodFixture.canonical_json,
            planFixture.canonical_json,
            period,
            plan,
          ),
        name,
      ).toThrow(HoldoutValidationError);
    }
  });
});

function executeNegative(
  target: string,
  mutation: string,
  periodSource: string,
  planSource: string,
  period: CanonicalHoldoutPeriod,
  plan: CanonicalHoldoutEvaluationPlan,
): void {
  if (target === "period") {
    parseCanonicalHoldoutPeriod(mutatePeriod(periodSource, mutation));
    return;
  }
  if (target === "period_reference") {
    verifyHoldoutPeriodIdentity(
      periodSource,
      mutation === "period_id_mismatch" ? digestText(238) : period.holdoutPeriodId,
      mutation === "period_digest_mismatch"
        ? new Uint8Array(32).fill(238)
        : period.canonicalPeriodSha256,
    );
    return;
  }
  if (target === "plan") {
    const mutationResult = mutatePlan(planSource, mutation);
    parseCanonicalHoldoutEvaluationPlan(
      mutationResult.source,
      period,
      mutationResult.trustedBacktest ?? trustedBacktest,
      mutationResult.resolved ?? resolved,
    );
    return;
  }
  if (target === "plan_reference") {
    const reference = mutateReference(
      domainReference(plan, period, plan.value.entries.length),
      mutation,
    );
    validateHoldoutEvaluationPlanReference(
      reference,
      plan.canonicalBytes,
      period,
      mutation === "plan_schema_digest_mismatch" ? new Uint8Array(32).fill(239) : trustedPlan,
      trustedBacktest,
      resolved,
    );
    return;
  }
  throw new Error(`unimplemented negative target ${target}`);
}

function mutatePeriod(source: string, mutation: string): string {
  const parsed = JSON.parse(source) as Record<string, unknown>;
  switch (mutation) {
    case "unknown_field":
      return source.replace("{", '{"unknown":"x",');
    case "duplicate_schema":
      return source.replace(
        '"schema":"loop.holdout-period/v1"',
        '"schema":"loop.holdout-period/v1","schema":"loop.holdout-period/v1"',
      );
    case "reorder_top_level":
      return JSON.stringify({
        sample: parsed.sample,
        schema: parsed.schema,
        snapshot_ids: parsed.snapshot_ids,
        snapshot_manifest_sha256: parsed.snapshot_manifest_sha256,
      });
    case "leading_whitespace":
      return ` ${source}`;
    case "wrong_schema":
      return source.replace("loop.holdout-period/v1", "loop.holdout-period/v2");
    case "forbidden_role":
      return source.replace("first_locked_confirmation", "development_validation");
    case "invalid_date":
      return source.replace("2024-12-31", "2024-02-30");
    case "reversed_window":
      return source.replace("2021-01-01", "2025-01-01");
    case "empty_snapshots":
      return source.replace(/"snapshot_ids":\[[^\]]+\]/, '"snapshot_ids":[]');
    case "unsorted_snapshots": {
      const snapshots = parsed.snapshot_ids as string[];
      return source.replace(JSON.stringify(snapshots), JSON.stringify([...snapshots].reverse()));
    }
    case "duplicate_snapshots": {
      const snapshots = parsed.snapshot_ids as string[];
      return source.replace(
        JSON.stringify(snapshots),
        JSON.stringify([snapshots[0], snapshots[0]]),
      );
    }
    case "bad_snapshot_digest":
      return source.replace(
        (parsed.snapshot_ids as string[])[0] ?? "missing",
        `sha256:${"A".repeat(64)}`,
      );
    case "bad_manifest_digest":
      return source.replace(String(parsed.snapshot_manifest_sha256), `sha256:${"g".repeat(64)}`);
    case "number_date":
      return source.replace('"start_inclusive":"2021-01-01"', '"start_inclusive":20210101');
    case "deep_nesting":
      return `${"[".repeat(40)}0${"]".repeat(40)}`;
    default:
      throw new Error(`unimplemented period mutation ${mutation}`);
  }
}

function mutatePlan(
  source: string,
  mutation: string,
): { source: string; trustedBacktest?: Uint8Array; resolved?: ReadonlyMap<string, Uint8Array> } {
  const parsed = JSON.parse(source) as MutablePlanJson;
  const first = requireItem(parsed.entries, 0);
  const second = requireItem(parsed.entries, 1);
  const artifact = first.backtest_spec_artifact;
  const budget = first.job_budget;
  switch (mutation) {
    case "unknown_field":
      parsed.unknown = "x";
      break;
    case "duplicate_schema":
      return {
        source: source.replace(
          `"schema":"${parsed.schema}"`,
          `"schema":"${parsed.schema}","schema":"${parsed.schema}"`,
        ),
      };
    case "reorder_top_level":
      return {
        source: JSON.stringify({
          holdout_period_id: parsed.holdout_period_id,
          schema: parsed.schema,
          canonical_period_sha256: parsed.canonical_period_sha256,
          entries: parsed.entries,
        }),
      };
    case "leading_whitespace":
      return { source: ` ${source}` };
    case "wrong_schema":
      parsed.schema = "loop.holdout-evaluation-plan/v2";
      break;
    case "period_id_mismatch":
      parsed.holdout_period_id = digestText(225);
      break;
    case "period_digest_mismatch":
      parsed.canonical_period_sha256 = digestText(226);
      break;
    case "empty_entries":
      parsed.entries = [];
      break;
    case "noncontiguous_entries":
      second.entry_index = "3";
      break;
    case "duplicate_factor":
      second.factor_spec_id = first.factor_spec_id;
      break;
    case "duplicate_backtest":
      second.backtest_spec_artifact = { ...first.backtest_spec_artifact };
      break;
    case "inline_backtest":
      artifact.inline = { schema: "forbidden" };
      break;
    case "bad_locator":
      artifact.uri = `https://user:secret@example.invalid/${artifact.sha256}`;
      break;
    case "wrong_artifact_schema":
      artifact.schema_name = "loop.other_spec";
      break;
    case "wrong_artifact_version":
      artifact.schema_version = "2";
      break;
    case "wrong_backtest_schema_digest":
      artifact.schema_sha256 = digestText(227);
      break;
    case "wrong_media_type":
      artifact.media_type = "application/octet-stream";
      break;
    case "zero_artifact_size":
      artifact.byte_size = "0";
      break;
    case "oversized_artifact":
      artifact.byte_size = "268435457";
      break;
    case "non_normalized_artifact_size":
      artifact.byte_size = "050";
      break;
    case "zero_steps":
      budget.maximum_steps = "0";
      break;
    case "non_normalized_steps":
      budget.maximum_steps = "040";
      break;
    case "token_overflow":
      budget.maximum_input_tokens = "1000000000001";
      break;
    case "negative_token":
      budget.maximum_output_tokens = "-1";
      break;
    case "cost_overflow":
      budget.maximum_cost.amount = "1000000.1";
      break;
    case "cost_precision":
      budget.maximum_cost.amount = "1234567890123456789";
      break;
    case "cost_scale":
      budget.maximum_cost.amount = "0.1234567891";
      break;
    case "lowercase_currency":
      budget.maximum_cost.currency_code = "usd";
      break;
    case "zero_wall_time":
      budget.maximum_wall_time_ns = "0";
      break;
    case "wall_time_overflow":
      budget.maximum_wall_time_ns = "604800000000001";
      break;
    case "unresolved_artifact": {
      const reduced = new Map(resolved);
      reduced.delete(artifact.sha256);
      return { source, resolved: reduced };
    }
    case "artifact_content_mismatch": {
      const changed = new Map(resolved);
      changed.set(artifact.sha256, encoder.encode("different bytes of exactly no relevance"));
      return { source, resolved: changed };
    }
    case "deep_nesting":
      return { source: `${"[".repeat(40)}0${"]".repeat(40)}` };
    default:
      throw new Error(`unimplemented plan mutation ${mutation}`);
  }
  return { source: JSON.stringify(parsed) };
}

function mutateReference(
  reference: EvaluationPlanReference,
  mutation: string,
): EvaluationPlanReference {
  let canonicalPlan: PlanArtifactReference = { ...reference.canonicalPlan };
  const changed: EvaluationPlanReference = { ...reference, canonicalPlan };
  switch (mutation) {
    case "plan_sha256_mismatch":
      return { ...changed, planSha256: new Uint8Array(32).fill(230) };
    case "plan_id_mismatch":
      return { ...changed, holdoutEvaluationPlanId: digestText(231) };
    case "entry_count_mismatch":
      return { ...changed, entryCount: changed.entryCount + 1 };
    case "period_id_mismatch":
      return { ...changed, holdoutPeriodId: digestText(232) };
    case "period_digest_mismatch":
      return { ...changed, canonicalPeriodSha256: new Uint8Array(32).fill(233) };
    case "plan_artifact_schema_mismatch":
      canonicalPlan = { ...canonicalPlan, schemaName: "loop.other_plan" };
      break;
    case "plan_schema_digest_mismatch":
      canonicalPlan = { ...canonicalPlan, schemaSha256: new Uint8Array(32).fill(234) };
      break;
    case "plan_artifact_size_mismatch":
      canonicalPlan = { ...canonicalPlan, byteSize: canonicalPlan.byteSize + 1n };
      break;
    case "plan_artifact_locator_mismatch":
      canonicalPlan = { ...canonicalPlan, uri: `artifact://sha256/${"00".repeat(32)}` };
      break;
    default:
      throw new Error(`unimplemented plan reference mutation ${mutation}`);
  }
  return { ...changed, canonicalPlan };
}

function domainReference(
  plan: CanonicalHoldoutEvaluationPlan,
  period: CanonicalHoldoutPeriod,
  entryCount: number,
): EvaluationPlanReference {
  const raw = encodeDigest(plan.planSha256);
  return {
    holdoutEvaluationPlanId: plan.holdoutEvaluationPlanId,
    canonicalPlan: {
      artifactId: raw,
      uri: `artifact://sha256/${raw.slice(7)}`,
      sha256: plan.planSha256,
      schemaName: "loop.holdout_evaluation_plan",
      schemaVersion: 1,
      schemaSha256: trustedPlan,
      mediaType: "application/json",
      byteSize: BigInt(plan.canonicalBytes.byteLength),
      hasRowCount: false,
      hasManifestSha256: false,
    },
    planSha256: plan.planSha256,
    entryCount,
    holdoutPeriodId: period.holdoutPeriodId,
    canonicalPeriodSha256: period.canonicalPeriodSha256,
  };
}

function civilDate(value: string) {
  const [year, month, day] = value.split("-").map(Number);
  return create(CivilDateSchema, { year, month, day });
}

function wireDigest(value: Uint8Array) {
  return create(Sha256DigestSchema, { value });
}

function decodeDigest(value: string): Uint8Array {
  return new Uint8Array(Buffer.from(value.slice(7), "hex"));
}

function encodeDigest(value: Uint8Array): string {
  return `sha256:${Buffer.from(value).toString("hex")}`;
}

function digestText(byte: number): string {
  return `sha256:${byte.toString(16).padStart(2, "0").repeat(32)}`;
}

function requireItem<T>(values: readonly T[], index: number): T {
  const value = values[index];
  if (value === undefined) throw new Error(`missing fixture item ${index}`);
  return value;
}
