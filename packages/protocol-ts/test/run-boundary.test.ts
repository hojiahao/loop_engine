import { readFileSync } from "node:fs";
import { fromBinary, toBinary } from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";

import * as runs from "../src/wire/runs.js";

const fixture = JSON.parse(
  readFileSync(
    new URL("../../../fixtures/contracts/protocol/v1/run_view_v1.json", import.meta.url),
    "utf8",
  ),
) as { wire_hex: string; revision: string; reserved_input_tokens: string; reserved_cost: string };
const wire = Buffer.from(fixture.wire_hex, "hex");

function golden(): runs.RunView {
  return fromBinary(runs.RunViewSchema, wire);
}

function required<T>(value: T | undefined): T {
  if (value === undefined) throw new Error("missing golden field");
  return value;
}

describe("operator run projection", () => {
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
    ]);
    expect(runs.StepRunRequestSchema.fields.map((field) => field.name)).toEqual([
      "context",
      "run_id",
      "expected_revision",
    ]);
  });
});
