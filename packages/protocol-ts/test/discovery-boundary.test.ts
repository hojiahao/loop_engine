import { create } from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";

import * as developmentData from "../src/generated/loop/v1/development_data_pb.js";
import * as discovery from "../src/wire/discovery.js";

describe("discovery public wire boundary", () => {
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
