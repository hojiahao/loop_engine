import { describe, expect, it } from "vitest";

import { provider_health, providerHealthSchema } from "../src/health.js";

describe("provider health", () => {
  it("conforms to the bootstrap protocol", () => {
    const health = provider_health();
    expect(providerHealthSchema.safeParse(health).success).toBe(true);
    expect(health.status).toBe("ready");
  });
});
