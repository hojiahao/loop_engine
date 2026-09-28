import { createHash } from "node:crypto";
import { mkdtemp, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { catalog_digest, load_catalog, publish_catalog } from "../src/catalog-store.js";
import { ProviderHost } from "../src/host.js";
import {
  CATALOG_PLUGIN,
  CATALOG_SECRETS,
  catalog_fixture,
  update_catalog,
} from "./catalog-fixture.js";
import { test_request } from "./fixture.js";

let directory: string;
let fixture: Awaited<ReturnType<typeof catalog_fixture>>;
beforeEach(async () => {
  directory = await mkdtemp(join(tmpdir(), "loop-provider-catalog-"));
  fixture = await catalog_fixture(directory);
});
afterEach(async () => {
  vi.restoreAllMocks();
  await fixture?.close();
  if (directory) await rm(directory, { recursive: true, force: true });
});

async function invoke(command: ReturnType<typeof test_request>) {
  return (await fixture.client().invokeModel(command, { timeoutMs: 4500 })).response;
}

describe("persistent model catalog activation", () => {
  it("binds every resolved model to the published generation", async () => {
    const history = await load_catalog(fixture.config);
    const generation = history[0];
    if (!generation) throw new Error("missing_generation");
    const digest = catalog_digest(generation);
    for (const model of fixture.host.models)
      expect(Buffer.from(model.snapshot.catalogSha256?.value ?? []).toString("hex")).toBe(digest);
    expect(
      fixture.host.catalog_status.models.every(
        (model) => model?.verification === "contract_verified",
      ),
    ).toBe(true);
    expect(fixture.host.catalog_status.generation).toEqual({ revision: 1, sha256: digest });
  });

  it("keeps old prices while a new resolution uses updated prices", async () => {
    const prior = test_request(fixture.host);
    await update_catalog(fixture, (sources) => {
      const model = sources.overrides[0];
      if (!model) throw new Error("missing_model");
      model.input_usd = "10";
    });
    await fixture.host.reload_catalog();
    expect((await invoke(prior))?.resolutionId).toEqual(prior.invocation?.model?.resolutionId);
    const next = test_request(fixture.host);
    expect(next.invocation?.model?.resolutionId).not.toEqual(prior.invocation?.model?.resolutionId);
    await expect(invoke(next)).rejects.toThrow("provider_budget_denied");
  });

  it("restores retained resolutions after a Host restart", async () => {
    const prior = test_request(fixture.host);
    await update_catalog(fixture, (sources) => {
      const model = sources.overrides[0];
      if (!model) throw new Error("missing_model");
      model.input_usd = "2";
    });
    const restarted = new ProviderHost(
      fixture.config,
      CATALOG_PLUGIN,
      CATALOG_SECRETS,
      fixture.fetcher,
    );
    expect(restarted.models).toHaveLength(0);
    await restarted.reload_catalog();
    const response = await restarted.invoke(prior, fixture.principal, new AbortController().signal);
    expect(response.resolutionId).toEqual(prior.invocation?.model?.resolutionId);
    expect(restarted.models[0]?.snapshot.pricing?.inputPerMillionTokens?.amount?.value).toBe("2");
  });

  it("changes an alias target without changing the model used by a retained pin", async () => {
    const prior = test_request(fixture.host);
    const seed = fixture.config.models[0];
    if (!seed) throw new Error("missing_model");
    await update_catalog(fixture, (sources) => {
      sources.overrides[0] = {
        route_id: seed.id,
        model: "responses-fixture-20261001",
        context_tokens: seed.context_tokens,
        output_tokens: seed.output_tokens,
        input_usd: seed.input_usd,
        output_usd: seed.output_usd,
        cached_usd: seed.cached_usd,
        features: { ...seed.features },
        reasoning: seed.reasoning,
        availability: "active",
      };
    });
    await fixture.host.reload_catalog();
    const next = test_request(fixture.host);
    await invoke(prior);
    await invoke(next);
    expect(
      fixture.requests
        .filter((request) => request.path.endsWith("/responses"))
        .map((request) => request.body.model),
    ).toEqual([seed.model, "responses-fixture-20261001"]);
  });

  it("refuses activation after clock regression", async () => {
    const active = fixture.host.catalog_status.generation;
    vi.spyOn(Date, "now").mockReturnValue(Date.now() - 60_000);
    await expect(fixture.host.reload_catalog()).rejects.toThrow("provider_catalog_unavailable");
    expect(fixture.host.catalog_status.generation).toEqual(active);
  });

  it("preserves an in-flight call when a new generation activates", async () => {
    const prior = test_request(fixture.host);
    await update_catalog(fixture, (sources) => {
      const model = sources.overrides[0];
      if (!model) throw new Error("missing_model");
      model.input_usd = "10";
    });
    fixture.state.delay = 80;
    let reload: Promise<void> | undefined;
    fixture.state.on_request = () => {
      reload ??= fixture.host.reload_catalog();
    };
    const response = await invoke(prior);
    await reload;
    expect(response?.resolutionId).toEqual(prior.invocation?.model?.resolutionId);
    expect(fixture.host.catalog_status.generation?.revision).toBe(2);
    await expect(invoke(test_request(fixture.host))).rejects.toThrow("provider_budget_denied");
  });

  it("retains the last validated state after corrupt disk data", async () => {
    const before = fixture.host.catalog_status.generation;
    await writeFile(join(directory, "catalog", "0001.result"), "corrupt", { mode: 0o600 });
    await expect(fixture.host.reload_catalog()).rejects.toThrow();
    expect(fixture.host.catalog_status.generation).toEqual(before);
    expect((await invoke(test_request(fixture.host)))?.content).not.toHaveLength(0);
  });

  it.each(["unknown", "unavailable", "retired"] as const)(
    "denies %s models before dispatch",
    async (availability) => {
      const prior = test_request(fixture.host);
      await update_catalog(fixture, (sources) => {
        const model = sources.overrides[0];
        if (!model) throw new Error("missing_model");
        model.availability = availability;
      });
      await fixture.host.reload_catalog();
      await expect(invoke(prior)).rejects.toThrow("provider_model_unavailable");
      expect(fixture.requests).toHaveLength(0);
    },
  );

  it("cannot reactivate a retired model through a later override", async () => {
    await update_catalog(fixture, (sources) => {
      const model = sources.overrides[0];
      if (!model) throw new Error("missing_model");
      model.availability = "retired";
    });
    await expect(
      update_catalog(fixture, (sources) => {
        const model = sources.overrides[0];
        if (!model) throw new Error("missing_model");
        model.availability = "active";
      }),
    ).rejects.toThrow("provider_catalog_retired");
    expect(await load_catalog(fixture.config)).toHaveLength(2);
  });

  it("requires a complete profile when changing the selected model", async () => {
    await expect(
      update_catalog(fixture, (sources) => {
        const model = sources.overrides[0];
        if (!model) throw new Error("missing_model");
        model.model = "unreviewed-model";
      }),
    ).rejects.toThrow("provider_catalog_profile_required");
    expect(await load_catalog(fixture.config)).toHaveLength(1);
  });

  it("rejects an expired resolution even when its bytes remain intact", async () => {
    const command = test_request(fixture.host);
    const history = await load_catalog(fixture.config);
    const record = history[0];
    if (!record) throw new Error("missing_record");
    const now = Date.parse(record.expires_at) + 1;
    // Context freshness should not hide the catalog-expiry check.
    if (!command.context?.requestedAt) throw new Error("missing_context");
    command.context.requestedAt.seconds = BigInt(Math.floor(now / 1000));
    command.context.requestedAt.nanos = (now % 1000) * 1_000_000;
    vi.spyOn(Date, "now").mockReturnValue(now);
    await expect(
      fixture.host.invoke(command, fixture.principal, new AbortController().signal),
    ).rejects.toThrow("provider_model_unavailable");
    expect(fixture.requests).toHaveLength(0);
  });

  it("requires a matching successful invocation before live verification", async () => {
    const command = test_request(fixture.host);
    await invoke(command);
    const receipt = createHash("sha256")
      .update(fixture.principal.actor_id)
      .update("\0")
      .update(command.context?.idempotencyKey?.value ?? "")
      .digest("hex");
    await update_catalog(fixture, (sources) => {
      const model = sources.overrides[0];
      if (!model) throw new Error("missing_model");
      model.live_receipt = receipt;
    });
    await fixture.host.reload_catalog();
    expect(fixture.host.catalog_status.models[0]?.verification).toBe("live_verified");
    expect(fixture.host.catalog_status.models[0]?.live_receipt).toBe(receipt);
    expect(fixture.host.catalog_status.models[1]?.verification).toBe("contract_verified");
  });

  it("cannot use one model's invocation to verify another", async () => {
    const command = test_request(fixture.host);
    await invoke(command);
    const receipt = createHash("sha256")
      .update(fixture.principal.actor_id)
      .update("\0")
      .update(command.context?.idempotencyKey?.value ?? "")
      .digest("hex");
    await expect(
      update_catalog(fixture, (sources) => {
        const model = sources.overrides[1];
        if (!model) throw new Error("missing_model");
        model.live_receipt = receipt;
      }),
    ).rejects.toThrow("provider_catalog_live_receipt");
    expect(await load_catalog(fixture.config)).toHaveLength(1);
  });

  it("does not let an invalid successor corrupt the published chain", async () => {
    const history = await load_catalog(fixture.config);
    const prior = history[0];
    if (!prior) throw new Error("missing_record");
    const record = structuredClone(prior);
    record.revision = 2;
    record.previous_sha256 = catalog_digest(prior);
    record.statuses = [];
    await expect(publish_catalog(fixture.config, record)).rejects.toThrow();
    expect(await readdir(join(directory, "catalog"))).toEqual(["0001.result"]);
  });
});
