import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { create, fromBinary, toBinary } from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";

import * as developmentData from "../src/generated/loop/v1/development_data_pb.js";
import * as discovery from "../src/wire/discovery.js";

function fixture(name: string): Uint8Array {
  return new Uint8Array(
    readFileSync(new URL(`../../../fixtures/contracts/protocol/v1/${name}`, import.meta.url)),
  );
}

describe("discovery public wire boundary", () => {
  it("binds execution to a job revision without caller model or prompt overrides", () => {
    const request = fromBinary(
      discovery.ExecuteDiscoveryRequestSchema,
      fixture("discovery_execute_v1.binpb"),
    );
    expect(request.jobId?.value).toBe("discovery.1");
    expect(request.expectedRevision).toBe(3n);
    expect(request.context?.requestId?.value).toBe("execute.1");
    expect(
      fromBinary(
        discovery.ExecuteDiscoveryRequestSchema,
        toBinary(discovery.ExecuteDiscoveryRequestSchema, request),
      ),
    ).toEqual(request);
    expect(discovery.DiscoveryService.method.executeDiscovery.input).toBe(
      discovery.ExecuteDiscoveryRequestSchema,
    );
    expect(discovery.DiscoveryService.method.getDiscovery.input).toBe(
      discovery.GetDiscoveryRequestSchema,
    );
  });

  it("preserves canonical candidate identity and conservative reservations", () => {
    const response = fromBinary(
      discovery.ExecuteDiscoveryResponseSchema,
      fixture("discovery_completed_v1.binpb"),
    );
    const step = response.step;
    expect(step?.state).toBe(discovery.DiscoveryStepState.COMPLETED);
    expect(step?.job?.status).toBe(discovery.DiscoveryJobStatus.SUCCEEDED);
    expect(step?.job?.revision).toBe(5n);
    expect(step?.reservedInputTokens).toBe(4096n);
    expect(step?.reservedOutputTokens).toBe(1024n);
    expect(step?.reservedCost?.amount?.value).toBe("0.125");
    expect(step?.reservedCost?.currencyCode).toBe("USD");
    const candidate = step?.candidate;
    expect(candidate?.canonicalizationProfile).toBe("loop.factor-ast/v1");
    expect(new TextDecoder().decode(candidate?.canonicalJson)).toBe(
      '{"node":"field","field":"market.close"}',
    );
    const expected = createHash("sha256")
      .update("loop.factor-ast/v1\0")
      .update(candidate?.canonicalJson ?? new Uint8Array())
      .digest("hex");
    expect(candidate?.expressionId?.value).toBe(`sha256:${expected}`);
    expect(
      fromBinary(
        discovery.ExecuteDiscoveryResponseSchema,
        toBinary(discovery.ExecuteDiscoveryResponseSchema, response),
      ),
    ).toEqual(response);
  });

  it("does not coerce an unknown step state to completion", () => {
    const step = fromBinary(discovery.DiscoveryStepViewSchema, Uint8Array.of(16, 127));
    expect(step.state).toBe(127);
    expect(
      Object.values(discovery.DiscoveryStepStateSchema.values).map((state) => state.number),
    ).not.toContain(step.state);
    expect(step.candidate).toBeUndefined();
    expect(step.reservedCost).toBeUndefined();
  });

  it("depends on a leaf module that cannot expose locked data types", () => {
    expect(Object.keys(developmentData).sort()).toEqual(
      ["DevelopmentDatasetReferenceSchema", "file_loop_v1_development_data"].sort(),
    );
    for (const forbiddenExport of [
      "SampleRole",
      "SampleRoleSchema",
      "SampleWindowSchema",
      "DataSnapshotSchema",
    ]) {
      expect(Object.hasOwn(developmentData, forbiddenExport), forbiddenExport).toBe(false);
    }
  });

  it("exports a narrow handle without generic job or holdout symbols", () => {
    for (const symbol of Object.keys(discovery)) {
      expect(symbol).not.toMatch(/Holdout|Grant|Approval|JobRecord|JobSpecification|BacktestSpec/);
    }

    expect(discovery).not.toHaveProperty("JobRecordSchema");
    expect(discovery).not.toHaveProperty("JobSpecificationSchema");
    expect(discovery).not.toHaveProperty("HoldoutBacktestJobInputSchema");
    expect(discovery).toHaveProperty("DevelopmentDatasetReferenceSchema");
  });

  it("round-trips only the safe discovery job projection", () => {
    const response = create(discovery.StartDiscoveryResponseSchema, {
      job: create(discovery.DiscoveryJobHandleSchema, {
        jobId: create(discovery.JobIdSchema, { value: "job.discovery.0001" }),
        status: discovery.DiscoveryJobStatus.RUNNING,
        revision: 7n,
      }),
    });

    expect(response.job).toMatchObject({
      jobId: { value: "job.discovery.0001" },
      status: discovery.DiscoveryJobStatus.RUNNING,
      revision: 7n,
    });
    expect(response.job).not.toHaveProperty("specification");
    expect(response.job).not.toHaveProperty("outcome");
    expect(response.job).not.toHaveProperty("activeLease");
  });
});
