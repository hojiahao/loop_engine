import { mkdtemp, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { create } from "@bufbuild/protobuf";
import { ModelUsageSchema } from "@loop-engine/protocol/provider";
import { afterEach, describe, expect, it, vi } from "vitest";
import { decimal_units, reserve_cost, usage_cost } from "../src/pricing.js";
import { RequestLimits } from "../src/request-limits.js";
import { test_config, test_request } from "./fixture.js";
import { rich_fixture } from "./rich-fixture.js";

let directory: string;
let fixture: Awaited<ReturnType<typeof rich_fixture>> | undefined;
afterEach(async () => {
  vi.restoreAllMocks();
  await fixture?.close();
  fixture = undefined;
  if (directory) await rm(directory, { recursive: true, force: true });
});

describe("bounded Provider platform admission", () => {
  it.each(["requests", "tokens", "maximum_usd"] as const)(
    "bounds %s before journal claim or outbound work",
    async (field) => {
      directory = await mkdtemp(join(tmpdir(), "loop-provider-platform-"));
      fixture = await rich_fixture(directory, (config) => {
        config.policy.rate = { window_ms: 1000, requests: 10, tokens: 10_000, maximum_usd: "1" };
        if (field === "requests") config.policy.rate.requests = 1;
        if (field === "tokens") config.policy.rate.tokens = 192;
        if (field === "maximum_usd") config.policy.rate.maximum_usd = "0.000256";
      });
      const first = test_request(fixture.host);
      const next = test_request(fixture.host);
      const signal = new AbortController().signal;
      let clock = 1000;
      vi.spyOn(performance, "now").mockImplementation(() => clock);
      await fixture.host.invoke(first, fixture.principal, signal);
      await expect(fixture.host.invoke(next, fixture.principal, signal)).rejects.toThrow(
        "provider_rate_capacity",
      );
      expect(fixture.requests).toHaveLength(2);
      expect(await readdir(fixture.config.journal)).toHaveLength(2);
      clock = 2000;
      await fixture.host.invoke(next, fixture.principal, signal);
      expect(fixture.requests).toHaveLength(4);
    },
  );

  it("uses a sliding window and exact boundary without accumulating denied entries", () => {
    const limits = new RequestLimits(
      { window_ms: 1000, requests: 2, tokens: 20, maximum_usd: "1" },
      100n,
    );
    limits.reserve(10n, 50n, 100);
    limits.reserve(10n, 50n, 200);
    for (let index = 0; index < 20; index++)
      expect(() => limits.reserve(1n, 1n, 1099)).toThrow("provider_rate_capacity");
    limits.reserve(10n, 50n, 1100);
    expect(() => limits.reserve(1n, 1n, 1100)).toThrow("provider_rate_capacity");
    limits.reserve(10n, 50n, 1200);
  });

  it("fails closed on monotonic clock regression", () => {
    const limits = new RequestLimits(
      { window_ms: 1000, requests: 2, tokens: 10, maximum_usd: "1" },
      100n,
    );
    limits.reserve(1n, 1n, 1000);
    expect(() => limits.reserve(1n, 1n, 999)).toThrow("provider_rate_clock");
  });

  it("does not overbook simultaneous callers", async () => {
    directory = await mkdtemp(join(tmpdir(), "loop-provider-platform-"));
    fixture = await rich_fixture(directory, (config) => {
      config.policy.rate.requests = 1;
    });
    const current = fixture;
    const results = await Promise.allSettled(
      Array.from({ length: 4 }, () =>
        current.host.invoke(
          test_request(current.host),
          current.principal,
          new AbortController().signal,
        ),
      ),
    );
    expect(results.filter((result) => result.status === "fulfilled")).toHaveLength(1);
    expect(
      results.filter((result) => result.status === "rejected").map((result) => result.reason.code),
    ).toEqual(Array(3).fill("provider_rate_capacity"));
    expect(current.requests).toHaveLength(2);
  });

  it("retains conservative reservations after upstream failure without automatic retry", async () => {
    directory = await mkdtemp(join(tmpdir(), "loop-provider-platform-"));
    fixture = await rich_fixture(directory, (config) => {
      config.policy.rate.requests = 1;
    });
    fixture.state.status = 429;
    await expect(
      fixture.host.invoke(
        test_request(fixture.host),
        fixture.principal,
        new AbortController().signal,
      ),
    ).rejects.toThrow("provider_rate_limited");
    await expect(
      fixture.host.invoke(
        test_request(fixture.host),
        fixture.principal,
        new AbortController().signal,
      ),
    ).rejects.toThrow("provider_rate_capacity");
    expect(fixture.requests).toHaveLength(1);
  });
});

describe("pinned token-price arithmetic", () => {
  it("prices cache reads, writes and reasoning-inclusive output exactly", () => {
    const model = test_config("/unused").models[2];
    if (!model) throw new Error("missing_model");
    Object.assign(model, {
      input_usd: "3",
      output_usd: "15",
      cached_usd: "0.3",
      cache_creation_usd: "3.75",
    });
    const usage = create(ModelUsageSchema, {
      inputTokens: 1000n,
      cachedInputTokens: 300n,
      cacheCreationInputTokens: 200n,
      outputTokens: 100n,
      reasoningTokens: 40n,
    });
    // 500*3 + 300*0.3 + 200*3.75 + 100*15 = 3840 microdollars.
    expect(usage_cost(model, usage)).toBe(3_840_000n);
    expect(reserve_cost(model, 1000n, 100n)).toBe(5_250_000n);
    expect(usage.chargedCost).toBeUndefined();
  });

  it("rounds up sub-nanodollar estimates without floating-point loss", () => {
    const model = test_config("/unused").models[0];
    if (!model) throw new Error("missing_model");
    model.input_usd = "0.000000001";
    expect(decimal_units(model.input_usd)).toBe(1n);
    expect(usage_cost(model, create(ModelUsageSchema, { inputTokens: 1n }))).toBe(1n);
  });

  it("rejects inconsistent cache counts instead of producing negative cost", () => {
    const model = test_config("/unused").models[0];
    if (!model) throw new Error("missing_model");
    expect(() =>
      usage_cost(model, create(ModelUsageSchema, { inputTokens: 1n, cachedInputTokens: 2n })),
    ).toThrow("invalid_priced_usage");
  });
});
