import { readFileSync } from "node:fs";
import { create, fromBinary, toBinary } from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";

import * as runs from "../src/wire/runs.js";

const fixture = JSON.parse(
  readFileSync(
    new URL("../../../fixtures/contracts/protocol/v1/run_view_v1.json", import.meta.url),
    "utf8",
  ),
) as { wire_hex: string; revision: string; reserved_input_tokens: string; reserved_cost: string };
const wire = Buffer.from(fixture.wire_hex, "hex");
const numerical_wire = Buffer.from(
  (
    JSON.parse(
      readFileSync(
        new URL("../../../fixtures/contracts/protocol/v1/run_view_v2.json", import.meta.url),
        "utf8",
      ),
    ) as { wire_hex: string }
  ).wire_hex,
  "hex",
);

function golden(): runs.RunView {
  return fromBinary(runs.RunViewSchema, wire);
}

function numerical(): runs.RunView {
  return fromBinary(runs.RunViewSchema, numerical_wire);
}

function required<T>(value: T | undefined): T {
  if (value === undefined) throw new Error("missing golden field");
  return value;
}

describe("operator run projection", () => {
  it("retains terminal pre-screen evidence without exposing candidate inputs", () => {
    const view = numerical();
    view.status = runs.RunStatus.COMPLETED;
    view.completedRounds = view.maximumRounds;
    const projection = required(view.research);
    projection.stage = runs.RunStage.DISCOVERY;
    const child = required(golden().currentJob);
    child.status = runs.DiscoveryJobStatus.SUCCEEDED;
    projection.currentChild = { case: "discovery", value: child };
    projection.evaluationJobId = undefined;
    for (const disposition of [
      runs.RoundDisposition.DUPLICATE,
      runs.RoundDisposition.PREVIOUSLY_REJECTED,
    ]) {
      projection.disposition = disposition;
      runs.validate_view(view);
    }
  });
  it("keeps legacy version absent from the wire", () => {
    expect(toBinary(runs.StartRunRequestSchema, create(runs.StartRunRequestSchema))).toEqual(
      new Uint8Array(),
    );
    expect(
      toBinary(
        runs.StartRunRequestSchema,
        create(runs.StartRunRequestSchema, { workflowVersion: 2 }),
      ),
    ).toEqual(new Uint8Array([24, 2]));
  });
  it("preserves the numerical workflow golden and rejects unknown versions", () => {
    const view = numerical();
    runs.validate_view(view);
    expect(Buffer.from(toBinary(runs.RunViewSchema, view))).toEqual(numerical_wire);
    runs.validate_workflow(0);
    runs.validate_workflow(2);
    runs.validate_workflow(3);
    for (const value of [1, 4, -1, 0.5, Number.NaN])
      expect(() => runs.validate_workflow(value)).toThrow();
  });
  it("accepts completed coverage and portfolio evidence without implying admission", () => {
    const view = numerical();
    view.status = runs.RunStatus.COMPLETED;
    view.completedRounds = view.maximumRounds;
    const projection = required(view.research);
    projection.disposition = runs.RoundDisposition.INSUFFICIENT_COVERAGE;
    if (projection.currentChild.case !== "research") throw new Error("fixture research handle");
    projection.currentChild.value.status = runs.ResearchJobStatus.SUCCEEDED;
    runs.validate_view(view);
    projection.stage = runs.RunStage.PORTFOLIO;
    projection.disposition = runs.RoundDisposition.EVALUATED;
    required(projection.currentChild.value.jobId).value = "job.portfolio.2";
    runs.validate_view(view);
    projection.disposition = runs.RoundDisposition.UNSPECIFIED;
    expect(() => runs.validate_view(view)).toThrow();
  });
  const numerical_changes: Record<string, (view: runs.RunView) => void> = {
    ambiguous_handle: (view) => {
      view.currentJob = golden().currentJob;
    },
    revision_bound: (view) => {
      view.revision = 8n;
    },
    zero_round: (view) => {
      required(view.research).round = 0;
    },
    stale_round: (view) => {
      required(view.research).round = 1;
    },
    unknown_stage: (view) => {
      required(view.research).stage = 127;
    },
    unspecified_stage: (view) => {
      required(view.research).stage = 0;
    },
    wrong_stage: (view) => {
      required(view.research).stage = runs.RunStage.DISCOVERY;
    },
    missing_digest: (view) => {
      required(view.research).bindingSha256 = undefined;
    },
    short_digest: (view) => {
      required(required(view.research).bindingSha256).value = new Uint8Array(31);
    },
    missing_handle: (view) => {
      required(view.research).currentChild = { case: undefined };
    },
    missing_evaluation: (view) => {
      required(view.research).evaluationJobId = undefined;
    },
    missing_discovery: (view) => {
      required(view.research).discoveryJobId = undefined;
    },
    missing_factor: (view) => {
      required(view.research).expressionId = undefined;
    },
    malformed_factor: (view) => {
      required(required(view.research).factorSpecId).value += "\n";
    },
    unknown_disposition: (view) => {
      required(view.research).disposition = 127;
    },
    wrong_disposition: (view) => {
      required(view.research).disposition = runs.RoundDisposition.EVALUATED;
    },
    active_disposition: (view) => {
      required(view.research).disposition = runs.RoundDisposition.INSUFFICIENT_COVERAGE;
    },
    reused_source: (view) => {
      required(view.research).discoveryJobId = required(view.research).evaluationJobId;
    },
  };
  it.each(Object.entries(numerical_changes))("rejects numerical %s", (_, mutate) => {
    const view = numerical();
    mutate(view);
    expect(() => runs.validate_view(view)).toThrow();
  });
  it("preserves uint64 values and exact USD across languages", () => {
    const view = golden();
    runs.validate_view(view);
    expect(view.revision.toString()).toBe(fixture.revision);
    expect(view.reservedInputTokens.toString()).toBe(fixture.reserved_input_tokens);
    expect(view.reservedCost?.amount?.value).toBe(fixture.reserved_cost);
    expect(Buffer.from(toBinary(runs.RunViewSchema, view))).toEqual(wire);
  });

  it("observes a completed child before durable parent advancement", () => {
    const view = golden();
    required(view.currentJob).status = runs.DiscoveryJobStatus.SUCCEEDED;
    required(required(view.currentJob).updatedAt).seconds += 2n;
    view.planVerified = false;
    expect(() => runs.validate_view(view)).not.toThrow();
  });

  const changes: Record<string, (view: runs.RunView) => void> = {
    newline_identity: (view) => {
      required(view.runId).value = "run\n";
    },
    newline_cost: (view) => {
      required(required(view.reservedCost).amount).value = "1\n";
    },
    zero_input: (view) => {
      view.reservedInputTokens = 0n;
    },
    zero_output: (view) => {
      view.reservedOutputTokens = 0n;
    },
    zero_input_budget: (view) => {
      required(view.budget).maximumInputTokens = 0n;
    },
    zero_output_budget: (view) => {
      required(view.budget).maximumOutputTokens = 0n;
    },
    zero_cost: (view) => {
      required(required(view.reservedCost).amount).value = "0";
    },
    unknown_status: (view) => {
      view.status = 127;
    },
    unspecified_status: (view) => {
      view.status = 0;
    },
    zero_revision: (view) => {
      view.revision = 0n;
    },
    huge_revision: (view) => {
      view.revision = 18_446_744_073_709_551_615n;
    },
    missing_budget: (view) => {
      view.budget = undefined;
    },
    missing_child: (view) => {
      view.currentJob = undefined;
    },
    round_limit: (view) => {
      view.maximumRounds = 65;
    },
    excess_rounds: (view) => {
      view.completedRounds = 3;
    },
    active_full: (view) => {
      view.completedRounds = 2;
    },
    incomplete_rounds: (view) => {
      view.status = runs.RunStatus.COMPLETED;
    },
    excess_steps: (view) => {
      view.reservedSteps = 9n;
    },
    excess_input: (view) => {
      view.reservedInputTokens = 9_007_199_254_740_994n;
    },
    excess_output: (view) => {
      view.reservedOutputTokens = 2049n;
    },
    wrong_currency: (view) => {
      required(view.reservedCost).currencyCode = "EUR";
    },
    excess_cost: (view) => {
      required(required(view.reservedCost).amount).value = "0.6";
    },
    trailing_zero: (view) => {
      required(required(view.reservedCost).amount).value = "0.1250";
    },
    cost_precision: (view) => {
      required(required(view.reservedCost).amount).value = "0.0000000001";
    },
    negative_cost: (view) => {
      required(required(view.reservedCost).amount).value = "-0.1";
    },
    submillisecond_wall: (view) => {
      required(required(view.budget).maximumWallTime).nanos = 1;
    },
    changed_deadline: (view) => {
      required(view.deadline).seconds += 1n;
    },
    clock_regression: (view) => {
      required(view.updatedAt).seconds -= 2n;
    },
    missing_timestamp: (view) => {
      view.submittedAt = undefined;
    },
    child_status: (view) => {
      required(view.currentJob).status = 127;
    },
    child_revision: (view) => {
      required(view.currentJob).revision = 0n;
    },
    child_timestamp: (view) => {
      required(required(view.currentJob).submittedAt).seconds -= 2n;
    },
    invalid_identity: (view) => {
      required(view.runId).value = "run bad";
    },
  };
  it.each(Object.entries(changes))("rejects %s", (_, mutate) => {
    const view = golden();
    mutate(view);
    expect(() => runs.validate_view(view)).toThrow();
  });

  it("exposes no internal execution input or raw candidate", () => {
    for (const key of Object.keys(runs)) {
      expect(key).not.toMatch(
        /RunSpecification|DiscoveryJobInput|DiscoveryCandidate|Holdout|Grant|JobRecord/,
      );
    }
    expect(runs.RunService.methods.map((method) => method.name)).toEqual([
      "StartRun",
      "StepRun",
      "GetRun",
    ]);
    expect(runs.StartRunRequestSchema.fields.map((field) => field.name)).toEqual([
      "context",
      "plan",
      "workflow_version",
    ]);
    expect(runs.StepRunRequestSchema.fields.map((field) => field.name)).toEqual([
      "context",
      "run_id",
      "expected_revision",
      "workflow_version",
    ]);
  });
});
