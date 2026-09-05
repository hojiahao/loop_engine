import { describe, expect, it } from "vitest";

import { providerHealth, providerHealthSchema } from "../src/health.js";

describe("provider health", () => {
  it("conforms to the bootstrap protocol", () => {
    const health = providerHealth();
    expect(providerHealthSchema.safeParse(health).success).toBe(true);
    expect(health.status).toBe("ready");
  });
});
