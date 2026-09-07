import { readFileSync } from "node:fs";
import { create, fromBinary, toBinary } from "@bufbuild/protobuf";
import { TimestampSchema } from "@bufbuild/protobuf/wkt";
import { describe, expect, it } from "vitest";
import {
  ConsumeGrantAndEnqueueBacktestRequestSchema,
  ConsumeGrantAndEnqueueBacktestResponseSchema,
  GetHoldoutPeriodRequestSchema,
  HoldoutService,
  JobBatchHandleSchema,
} from "../src/generated/loop/holdout/v1/service_pb.js";
import {
  ArtifactRefSchema,
  ArtifactSchemaReferenceSchema,
} from "../src/generated/loop/v1/artifact_pb.js";
import {
  ArtifactIdSchema,
  HoldoutEvaluationPlanIdSchema,
  HoldoutGrantIdSchema,
  HoldoutPeriodIdSchema,
  JobBatchIdSchema,
  JobIdSchema,
  Sha256DigestSchema,
} from "../src/generated/loop/v1/common_pb.js";
import {
  FreezeManifestReferenceSchema,
  HoldoutEvaluationPlanReferenceSchema,
  HoldoutGrantReferenceSchema,
} from "../src/generated/loop/v1/holdout_pb.js";
import { HoldoutBacktestJobInputSchema } from "../src/generated/loop/v1/job_pb.js";
import * as holdoutWire from "../src/wire/holdout.js";

function digest(byte: number) {
  return create(Sha256DigestSchema, { value: new Uint8Array(32).fill(byte) });
}

const planId = `sha256:${"0a".repeat(32)}`;
const periodId = `sha256:${"09".repeat(32)}`;
const vectors = readFileSync(
  new URL("../../../tests/contracts/holdout_boundary_vectors.tsv", import.meta.url),
  "utf8",
);

function grantReference() {
  return create(HoldoutGrantReferenceSchema, {
    holdoutGrantId: create(HoldoutGrantIdSchema, { value: "grant.holdout.0001" }),
    holdoutPeriodId: create(HoldoutPeriodIdSchema, { value: periodId }),
    freezeManifestSha256: digest(6),
    holdoutEvaluationPlanId: create(HoldoutEvaluationPlanIdSchema, { value: planId }),
    evaluationPlanSha256: digest(7),
    evaluationPlanEntryCount: 2,
    canonicalPeriodSha256: digest(9),
  });
}

describe("frozen holdout evaluation boundary", () => {
  it("keeps the public role entrypoint free of caller-selected job contracts", () => {
    for (const forbiddenExport of [
      "BacktestSpecSchema",
      "BacktestJobInputSchema",
      "HoldoutBacktestJobInputSchema",
      "JobBudgetSchema",
      "JobSpecificationSchema",
    ]) {
      expect(Object.hasOwn(holdoutWire, forbiddenExport), forbiddenExport).toBe(false);
    }
    expect(Object.hasOwn(holdoutWire, "HoldoutService")).toBe(true);
    expect(Object.hasOwn(holdoutWire, "HoldoutGrantReferenceSchema")).toBe(true);
  });

  it("enforces the shared positive and negative surface vectors", () => {
    const surfaces = {
      consume_request: ConsumeGrantAndEnqueueBacktestRequestSchema.field,
      consume_response: ConsumeGrantAndEnqueueBacktestResponseSchema.field,
      freeze_manifest: FreezeManifestReferenceSchema.field,
      get_period_request: GetHoldoutPeriodRequestSchema.field,
      holdout_service: HoldoutService.method,
      internal_holdout_job: HoldoutBacktestJobInputSchema.field,
    };
    for (const line of vectors.split("\n")) {
      if (line.length === 0 || line.startsWith("#")) continue;
      const [name, surface, member, expected] = line.split("\t");
      if (!(surface in surfaces) || name === undefined || member === undefined) {
        throw new Error(`invalid shared holdout vector: ${line}`);
      }
      const localMember = surface === "holdout_service" ? lowerFirst(member) : snakeToCamel(member);
      expect(Object.hasOwn(surfaces[surface as keyof typeof surfaces], localMember), name).toBe(
        expected === "accept",
      );
    }
  });

  it("round-trips a freeze manifest that pins the complete plan artifact", () => {
    const plan = create(HoldoutEvaluationPlanReferenceSchema, {
      holdoutEvaluationPlanId: create(HoldoutEvaluationPlanIdSchema, { value: planId }),
      canonicalPlan: create(ArtifactRefSchema, {
        artifactId: create(ArtifactIdSchema, { value: `sha256:${"07".repeat(32)}` }),
        uri: `artifact://sha256/${"07".repeat(32)}`,
        sha256: digest(7),
        schema: create(ArtifactSchemaReferenceSchema, {
          name: "loop.holdout_evaluation_plan",
          version: 1,
          schemaSha256: digest(8),
        }),
        mediaType: "application/json",
        byteSize: 512n,
        createdAt: create(TimestampSchema, { seconds: 1n }),
      }),
      planSha256: digest(7),
      entryCount: 2,
      holdoutPeriodId: create(HoldoutPeriodIdSchema, { value: periodId }),
      canonicalPeriodSha256: digest(9),
    });
    const freeze = create(FreezeManifestReferenceSchema, { holdoutEvaluationPlan: plan });
    const decoded = fromBinary(
      FreezeManifestReferenceSchema,
      toBinary(FreezeManifestReferenceSchema, freeze),
    );

    expect(decoded.holdoutEvaluationPlan?.entryCount).toBe(2);
    expect(decoded.holdoutEvaluationPlan?.canonicalPlan?.uri).toBe(
      `artifact://sha256/${"07".repeat(32)}`,
    );
  });

  it("accepts only grant and revision inputs and returns a narrow batch handle", () => {
    const request = create(ConsumeGrantAndEnqueueBacktestRequestSchema, {
      grantReference: grantReference(),
      expectedGrantRevision: 3n,
      expectedPeriodRevision: 4n,
    });
    const decodedRequest = fromBinary(
      ConsumeGrantAndEnqueueBacktestRequestSchema,
      toBinary(ConsumeGrantAndEnqueueBacktestRequestSchema, request),
    );
    expect(decodedRequest.expectedGrantRevision).toBe(3n);
    expect(decodedRequest).not.toHaveProperty("frozenBacktestSpec");
    expect(decodedRequest).not.toHaveProperty("budget");

    const response = create(ConsumeGrantAndEnqueueBacktestResponseSchema, {
      consumedGrant: grantReference(),
      jobBatch: create(JobBatchHandleSchema, {
        jobBatchId: create(JobBatchIdSchema, { value: "batch.holdout.0001" }),
        holdoutGrantId: create(HoldoutGrantIdSchema, { value: "grant.holdout.0001" }),
        holdoutEvaluationPlanId: create(HoldoutEvaluationPlanIdSchema, { value: planId }),
        evaluationPlanSha256: digest(7),
        evaluationPlanEntryCount: 2,
        jobCount: 2,
        jobIds: [
          create(JobIdSchema, { value: "job.holdout.0001" }),
          create(JobIdSchema, { value: "job.holdout.0002" }),
        ],
        revision: 1n,
      }),
    });
    const decodedResponse = fromBinary(
      ConsumeGrantAndEnqueueBacktestResponseSchema,
      toBinary(ConsumeGrantAndEnqueueBacktestResponseSchema, response),
    );
    expect(decodedResponse.jobBatch?.jobIds).toHaveLength(2);
    expect(decodedResponse).not.toHaveProperty("job");
    expect(decodedResponse.jobBatch).not.toHaveProperty("specification");
  });

  it("keeps parsed specifications only on the internal durable job", () => {
    const internal = create(HoldoutBacktestJobInputSchema, {
      consumedGrant: grantReference(),
      consumedGrantRevision: 4n,
      jobBatchId: create(JobBatchIdSchema, { value: "batch.holdout.0001" }),
      holdoutEvaluationPlanId: create(HoldoutEvaluationPlanIdSchema, { value: planId }),
      evaluationPlanSha256: digest(7),
      evaluationPlanEntryIndex: 2,
    });
    const decoded = fromBinary(
      HoldoutBacktestJobInputSchema,
      toBinary(HoldoutBacktestJobInputSchema, internal),
    );

    expect(decoded.evaluationPlanEntryIndex).toBe(2);
    expect(decoded.holdoutEvaluationPlanId?.value).toBe(planId);
  });
});

function snakeToCamel(value: string): string {
  return value.replace(/_([a-z])/g, (_, letter: string) => letter.toUpperCase());
}

function lowerFirst(value: string): string {
  return `${value.slice(0, 1).toLowerCase()}${value.slice(1)}`;
}
