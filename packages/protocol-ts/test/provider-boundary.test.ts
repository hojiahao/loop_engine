import { describe, expect, it } from "vitest";

import * as provider from "../src/wire/provider.js";

describe("provider public wire boundary", () => {
  it("exports model transport types without research, job, or holdout symbols", () => {
    expect(provider).toHaveProperty("ProviderService");
    expect(provider).toHaveProperty("ModelInvocationSchema");
    expect(provider).toHaveProperty("ModelStreamEventSchema");

    for (const symbol of Object.keys(provider)) {
      expect(symbol).not.toMatch(/Approval|Backtest|Factor|Holdout|Job|Research/);
    }
  });
});
