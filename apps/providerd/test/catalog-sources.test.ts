import { generateKeyPairSync, sign } from "node:crypto";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { catalog_signing, fetch_catalog, verify_catalog } from "../src/catalog-fetch.js";
import { build_catalog } from "../src/catalog-merge.js";
import { load_catalog, publish_catalog } from "../src/catalog-store.js";
import {
  type CatalogDocument,
  type CatalogSources,
  catalog_sources,
} from "../src/catalog-types.js";
import type { Deployment } from "../src/config.js";
import { open_journal } from "../src/journal.js";
import { CATALOG_PLUGIN, CATALOG_SECRETS, seed_sources } from "./catalog-fixture.js";
import { test_config } from "./fixture.js";
import { metadata_fixture } from "./metadata-fixture.js";

let directory: string;
let config: Deployment;
let sources: CatalogSources;
let fixture: Awaited<ReturnType<typeof metadata_fixture>>;
const keys = generateKeyPairSync("ed25519");
const remote = {
  source_id: "trusted-models",
  key_id: "catalog-key",
  url: "https://catalog.example/models.json",
};

beforeEach(async () => {
  directory = await mkdtemp(join(tmpdir(), "loop-catalog-sources-"));
  config = test_config(directory);
  config.catalog = {
    directory: join(directory, "catalog"),
    sources: join(directory, "sources.json"),
    maximum_generations: 64,
    trusted_keys: [
      {
        id: remote.key_id,
        public_key: keys.publicKey.export({ type: "spki", format: "pem" }).toString(),
      },
    ],
  };
  sources = seed_sources(config);
  await open_journal(config.catalog.directory);
  fixture = await metadata_fixture();
});
afterEach(async () => {
  await fixture?.close();
  if (directory) await rm(directory, { recursive: true, force: true });
});

function document(): CatalogDocument {
  return {
    schema: "loop.model-catalog/v1",
    source_id: remote.source_id,
    revision: 1,
    issued_at: new Date(Date.now() - 1000).toISOString(),
    expires_at: new Date(Date.now() + 60_000).toISOString(),
    entries: [{ route_id: "responses", input_usd: "2", availability: "active" }],
  };
}

function envelope(payload = document()) {
  return {
    schema: "loop.signed-catalog/v1",
    key_id: remote.key_id,
    payload,
    signature: sign(null, catalog_signing(payload), keys.privateKey).toString("base64"),
  };
}

async function build() {
  if (!config.catalog) throw new Error("missing_catalog");
  await writeFile(config.catalog.sources, JSON.stringify(sources), { mode: 0o600 });
  return build_catalog(
    config,
    CATALOG_PLUGIN,
    await load_catalog(config),
    CATALOG_SECRETS,
    new AbortController().signal,
    fixture.fetcher,
  );
}

describe("signed metadata and deterministic precedence", () => {
  it("verifies the pinned Ed25519 key over canonical payload bytes", () => {
    if (!config.catalog) throw new Error("missing_catalog");
    const options = config.catalog;
    const signed = envelope();
    expect(verify_catalog(signed, remote, config.catalog, Date.now())).toEqual(signed.payload);
    signed.payload.entries[0] = { route_id: "responses", input_usd: "999" };
    expect(() => verify_catalog(signed, remote, options, Date.now())).toThrow(
      "provider_catalog_signature",
    );
  });

  it.each(["key", "source", "signature", "expired", "future"])("rejects %s forgery", (kind) => {
    if (!config.catalog) throw new Error("missing_catalog");
    const options = config.catalog;
    const payload = document();
    if (kind === "expired") payload.expires_at = "2020-01-01T00:00:00Z";
    if (kind === "future") payload.issued_at = "2099-01-01T00:00:00Z";
    const signed = envelope(payload);
    if (kind === "key") signed.key_id = "untrusted";
    if (kind === "source") signed.payload.source_id = "different-source";
    if (kind === "signature") signed.signature = Buffer.alloc(64).toString("base64");
    expect(() => verify_catalog(signed, remote, options, Date.now())).toThrow();
  });

  it("fetches signed catalogs without forwarding supplier credentials", async () => {
    if (!config.catalog) throw new Error("missing_catalog");
    fixture.state.pages = [envelope()];
    const result = await fetch_catalog(
      remote,
      config.catalog,
      new AbortController().signal,
      fixture.fetcher,
    );
    expect(result.source_id).toBe(remote.source_id);
    expect(fixture.requests[0]?.headers.authorization).toBeUndefined();
    expect(fixture.requests[0]?.headers["x-api-key"]).toBeUndefined();
    expect(fixture.requests[0]?.headers["x-goog-api-key"]).toBeUndefined();
  });

  it.each([
    "http://127.0.0.1/catalog",
    "https://user:secret@catalog.example/file",
    "https://catalog.example/file?key=secret",
    "https://catalog.example/%2e%2e/file",
  ])("denies unsafe remote destination %s", async (url) => {
    if (!config.catalog) throw new Error("missing_catalog");
    await expect(
      fetch_catalog(
        { ...remote, url },
        config.catalog,
        new AbortController().signal,
        fixture.fetcher,
      ),
    ).rejects.toThrow("provider_catalog_destination");
    expect(fixture.requests).toHaveLength(0);
  });

  it("merges discovery then signed metadata then local overrides with provenance", async () => {
    sources.discovery = ["responses"];
    sources.remotes = [remote];
    sources.overrides[0] = { route_id: "responses", input_usd: "3", availability: "active" };
    fixture.state.pages = [{ data: [{ id: config.models[0]?.model }] }, envelope()];
    const generation = await build();
    expect(generation.models[0]?.input_usd).toBe("3");
    expect(generation.statuses[0]?.verification).toBe("contract_verified");
    expect(generation.sources.map((source) => source.kind)).toEqual([
      "builtin",
      "discovery",
      "remote",
      "local",
    ]);
    await publish_catalog(config, generation);
    expect(await load_catalog(config)).toEqual([generation]);
  });

  it("keeps unresolved availability unknown when no trusted layer declares it", async () => {
    sources.overrides = [];
    const generation = await build();
    expect(generation.statuses.every((status) => status.availability === "unknown")).toBe(true);
  });

  it("marks an absent listed model unavailable without changing its ID", async () => {
    sources.discovery = ["responses"];
    sources.overrides = [];
    fixture.state.pages = [{ data: [] }];
    const generation = await build();
    expect(generation.statuses[0]?.availability).toBe("unavailable");
    expect(generation.models[0]?.model).toBe(config.models[0]?.model);
  });

  it("narrows reported capacities without enabling unknown features", async () => {
    sources.discovery = ["claude"];
    sources.overrides = [];
    fixture.state.pages = [
      {
        data: [
          {
            id: config.models[2]?.model,
            max_input_tokens: 2048,
            max_tokens: 128,
            capabilities: { image_input: { supported: true } },
          },
        ],
        has_more: false,
      },
    ];
    const generation = await build();
    expect(generation.models[2]).toMatchObject({
      context_tokens: 2048,
      output_tokens: 128,
      features: { vision: false },
    });
  });

  it.each(["endpoint", "secret_env", "plugin", "principals", "live_receipt"])(
    "does not let signed metadata change %s authority",
    async (field) => {
      sources.remotes = [remote];
      const payload = document();
      Object.assign(payload.entries[0] ?? {}, { [field]: "injected" });
      fixture.state.pages = [envelope(payload)];
      await expect(build()).rejects.toThrow();
      expect(await load_catalog(config)).toEqual([]);
    },
  );

  it("does not let local source metadata add an endpoint", () => {
    expect(() =>
      catalog_sources.parse({
        ...sources,
        overrides: [{ route_id: "responses", base_url: "https://evil.example" }],
      }),
    ).toThrow();
  });

  it("rejects duplicate source entries", async () => {
    sources.overrides.push({ route_id: "responses" });
    await expect(build()).rejects.toThrow("provider_catalog_duplicate");
  });

  it.each(["rollback", "conflict"])(
    "rejects source revision %s without replacing history",
    async (kind) => {
      const first = await build();
      await publish_catalog(config, first);
      sources.overrides[0] = { route_id: "responses", input_usd: "2", availability: "active" };
      // First advance the watermark to revision two.
      sources.revision = 2;
      await publish_catalog(config, await build());
      sources.revision = kind === "rollback" ? 1 : 2;
      sources.overrides[0] = { route_id: "responses", input_usd: "3", availability: "active" };
      await expect(publish_catalog(config, await build())).rejects.toThrow(
        "provider_catalog_corrupt",
      );
      expect(await load_catalog(config)).toHaveLength(2);
    },
  );

  it("rejects retirement reactivation in a successor generation", async () => {
    sources.overrides[0] = { route_id: "responses", availability: "retired" };
    await publish_catalog(config, await build());
    sources.revision++;
    sources.overrides[0] = { route_id: "responses", availability: "active" };
    await expect(publish_catalog(config, await build())).rejects.toThrow(
      "provider_catalog_retired",
    );
  });

  it("rejects an aborted refresh even with only local sources", async () => {
    if (!config.catalog) throw new Error("missing_catalog");
    await writeFile(config.catalog.sources, JSON.stringify(sources), { mode: 0o600 });
    await expect(
      build_catalog(
        config,
        CATALOG_PLUGIN,
        [],
        CATALOG_SECRETS,
        AbortSignal.abort(),
        fixture.fetcher,
      ),
    ).rejects.toThrow();
    expect(await load_catalog(config)).toEqual([]);
  });
});
