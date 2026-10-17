import { readFileSync } from "node:fs";
import { equals, fromBinary, toBinary } from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";
import { JobSpecificationSchema } from "../src/generated/loop/v1/job_pb.js";
import { PerturbationWorkSchema } from "../src/generated/loop/v1/perturbation_pb.js";
import { validate_job_specification } from "../src/job.js";
import * as runs from "../src/wire/runs.js";

const fixture = JSON.parse(
  readFileSync(
    new URL("../../../fixtures/contracts/protocol/v1/adaptive_workflow_v3.json", import.meta.url),
    "utf8",
  ),
) as { view_wire_hex: string; job_wire_hex: string; observed_work_wire_hex: string };

function golden(): runs.RunView {
  return fromBinary(runs.RunViewSchema, Buffer.from(fixture.view_wire_hex, "hex"));
}
function job() {
  return fromBinary(JobSpecificationSchema, Buffer.from(fixture.job_wire_hex, "hex"));
}
function required<T>(value: T | undefined): T {
  if (value === undefined) throw new Error("missing fixture");
  return value;
}

describe("adaptive workflow", () => {
  it("preserves successful observations separately from failed and proposed IDs", () => {
    const work = fromBinary(
      PerturbationWorkSchema,
      Buffer.from(fixture.observed_work_wire_hex, "hex"),
    );
    expect(work.observedFactorIds).toHaveLength(2);
    expect(work.observedFactorIds).toEqual(
      work.candidates.map((candidate) => candidate.factorSpecId),
    );
    expect(work.failedFactorIds).toEqual([]);
    expect(required(work.state).proposedFactorIds).toEqual([]);
    expect(
      equals(
        PerturbationWorkSchema,
        fromBinary(PerturbationWorkSchema, toBinary(PerturbationWorkSchema, work)),
        work,
      ),
    ).toBe(true);
  });
  it("preserves the shared v3 projection and real perturbation envelope", () => {
    const view = golden();
    const input = job();
    runs.validate_view(view);
    validate_job_specification(input);
    expect(Buffer.from(toBinary(runs.RunViewSchema, view)).toString("hex")).toBe(
      fixture.view_wire_hex,
    );
    expect(Buffer.from(toBinary(JobSpecificationSchema, input)).toString("hex")).toBe(
      fixture.job_wire_hex,
    );
  });
  const changes: Record<string, (view: runs.RunView) => void> = {
    legacy: (view) => {
      view.adaptive = undefined;
    },
    policy: (view) => {
      required(view.adaptive).feedbackPolicy = undefined;
    },
    sources: (view) => {
      required(view.adaptive).sourceCount = 17;
    },
    future: (view) => {
      required(view.adaptive).sourceCount = 2;
    },
    feedback: (view) => {
      required(view.adaptive).feedbackSha256 = undefined;
    },
    context: (view) => {
      required(view.adaptive).modelContextSha256 = undefined;
    },
    family: (view) => {
      required(view.adaptive).familySha256 = undefined;
    },
    state: (view) => {
      required(view.adaptive).stateSha256 = undefined;
    },
    draws: (view) => {
      required(view.adaptive).randomDraws = 97n;
    },
    origin: (view) => {
      required(view.research).discoveryJobId = required(view.research).perturbationJobId;
    },
    missing: (view) => {
      required(view.research).perturbationJobId = undefined;
    },
    first: (view) => {
      required(view.research).round = 1;
    },
    stage: (view) => {
      required(view.research).stage = runs.RunStage.DISCOVERY;
    },
    exhausted: (view) => {
      view.status = runs.RunStatus.SEARCH_EXHAUSTED;
    },
  };
  for (const [name, change] of Object.entries(changes))
    it(`rejects ${name}`, () => {
      const view = golden();
      change(view);
      expect(() => runs.validate_view(view)).toThrow();
    });
  it("retains the real round count on exhaustion", () => {
    const view = golden();
    view.status = runs.RunStatus.SEARCH_EXHAUSTED;
    view.completedRounds = 2;
    const projection = required(view.research);
    projection.disposition = runs.RoundDisposition.EXHAUSTED;
    if (projection.currentChild.case !== "research") throw new Error("research handle");
    projection.currentChild.value.status = runs.ResearchJobStatus.SUCCEEDED;
    runs.validate_view(view);
    expect(view.completedRounds).toBeLessThan(view.maximumRounds);
    projection.currentChild.value.status = runs.ResearchJobStatus.FACTOR_REJECTED;
    expect(() => runs.validate_view(view)).toThrow();
  });
  it("rejects missing work bindings and model allowance", () => {
    for (const field of [
      "dataset",
      "feedbackPolicy",
      "parentBindingSha256",
      "familySha256",
      "sourceCodeSha256",
      "environmentSha256",
      "work",
      "budget",
    ] as const) {
      const input = job();
      if (input.input.case !== "perturbation") throw new Error("perturbation input");
      input.input.value[field] = undefined;
      expect(() => validate_job_specification(input)).toThrow();
    }
    const input = job();
    if (input.input.case !== "perturbation") throw new Error("perturbation input");
    required(input.input.value.budget).maximumInputTokens = 1n;
    expect(() => validate_job_specification(input)).toThrow();
  });
  it("keeps the published formerly unknown input denied", () => {
    const input = fromBinary(JobSpecificationSchema, Buffer.from("1805820100", "hex"));
    expect(input.input.case).toBe("perturbation");
    expect(() => validate_job_specification(input)).toThrow();
  });
});
