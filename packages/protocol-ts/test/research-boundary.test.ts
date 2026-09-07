import { readFileSync } from "node:fs";
import { create, fromBinary, toBinary } from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";

import * as research from "../src/wire/research.js";

const vectors = readFileSync(
  new URL("../../../tests/contracts/research_boundary_vectors.tsv", import.meta.url),
  "utf8",
);

describe("research public wire boundary", () => {
  it("exports only development inputs and a narrow job projection", () => {
    expect(Object.keys(research).sort()).toEqual(
      [
        "ActorIdSchema",
        "ActorKind",
        "ActorKindSchema",
        "ActorSchema",
        "BacktestIdSchema",
        "BacktestInputSchema",
        "CausationIdSchema",
        "CommandContextSchema",
        "CorrelationIdSchema",
        "DevelopmentDatasetReferenceSchema",
        "EnqueueBacktestRequestSchema",
        "EnqueueBacktestResponseSchema",
        "EnqueueFactorEvaluationRequestSchema",
        "EnqueueFactorEvaluationResponseSchema",
        "EnqueueReconciliationRequestSchema",
        "EnqueueReconciliationResponseSchema",
        "ExactDecimalSchema",
        "FactorEvaluationInputSchema",
        "FactorSpecIdSchema",
        "FactorSpecSchema",
        "IdempotencyKeySchema",
        "JobIdSchema",
        "MoneySchema",
        "PolicyIdSchema",
        "PolicyReferenceSchema",
        "ReconciliationInputSchema",
        "RequestIdSchema",
        "ResearchJobBudgetSchema",
        "ResearchJobHandleSchema",
        "ResearchJobStatus",
        "ResearchJobStatusSchema",
        "ResearchProvenanceFingerprintSchema",
        "ResearchService",
        "ReturnDefinition",
        "ReturnDefinitionSchema",
        "Sha256DigestSchema",
        "SnapshotIdSchema",
      ].sort(),
    );
    for (const forbiddenExport of [
      "BacktestSpecSchema",
      "HoldoutBacktestJobInputSchema",
      "HoldoutGrantReferenceSchema",
      "JobBudgetSchema",
      "JobLeaseSchema",
      "JobOutcomeSchema",
      "JobRecordSchema",
      "JobSpecificationSchema",
      "SampleRoleSchema",
      "SampleWindowSchema",
    ]) {
      expect(Object.hasOwn(research, forbiddenExport), forbiddenExport).toBe(false);
    }

    expect(Object.hasOwn(research, "ResearchService")).toBe(true);
    expect(Object.hasOwn(research, "ResearchJobHandleSchema")).toBe(true);
    expect(Object.hasOwn(research, "DevelopmentDatasetReferenceSchema")).toBe(true);
    expect(Object.keys(research.DevelopmentDatasetReferenceSchema.field).sort()).toEqual([
      "manifestSha256",
      "snapshotIds",
    ]);
  });

  it("enforces shared positive and negative role-surface vectors", () => {
    const surfaces = {
      backtest_input: research.BacktestInputSchema.field,
      factor_input: research.FactorEvaluationInputSchema.field,
      job_handle: research.ResearchJobHandleSchema.field,
      reconciliation_input: research.ReconciliationInputSchema.field,
    };
    for (const line of vectors.split("\n")) {
      if (line.length === 0 || line.startsWith("#")) continue;
      const [name, surface, member, expected] = line.split("\t");
      if (!(surface in surfaces) || name === undefined || member === undefined) {
        throw new Error(`invalid shared research vector: ${line}`);
      }
      expect(
        Object.hasOwn(surfaces[surface as keyof typeof surfaces], snakeToCamel(member)),
        name,
      ).toBe(expected === "accept");
    }
  });

  it("round-trips the response without specification, lease, or outcome", () => {
    const response = create(research.EnqueueBacktestResponseSchema, {
      job: create(research.ResearchJobHandleSchema, {
        jobId: create(research.JobIdSchema, { value: "job.research.0001" }),
        status: research.ResearchJobStatus.RUNNING,
        revision: 7n,
      }),
    });
    const decoded = fromBinary(
      research.EnqueueBacktestResponseSchema,
      toBinary(research.EnqueueBacktestResponseSchema, response),
    );

    expect(decoded.job).toMatchObject({
      jobId: { value: "job.research.0001" },
      revision: 7n,
      status: research.ResearchJobStatus.RUNNING,
    });
    expect(decoded.job).not.toHaveProperty("specification");
    expect(decoded.job).not.toHaveProperty("activeLease");
    expect(decoded.job).not.toHaveProperty("outcome");
  });
});

function snakeToCamel(value: string): string {
  return value.replace(/_([a-z])/g, (_, letter: string) => letter.toUpperCase());
}
