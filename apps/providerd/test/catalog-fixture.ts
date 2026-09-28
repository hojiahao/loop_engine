import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import { build_catalog } from "../src/catalog-merge.js";
import { load_catalog, publish_catalog } from "../src/catalog-store.js";
import type { CatalogSources } from "../src/catalog-types.js";
import type { Deployment } from "../src/config.js";
import { open_journal } from "../src/journal.js";
import { TEST_SECRET } from "./fixture.js";
import { rich_fixture } from "./rich-fixture.js";

export const CATALOG_PLUGIN = new Uint8Array(32).fill(1);
export const CATALOG_SECRETS = { LOOP_LLM_TEST: TEST_SECRET };

export function seed_sources(config: Deployment): CatalogSources {
  return {
    schema: "loop.catalog-sources/v1",
    source_id: "local-administrator",
    revision: 1,
    issued_at: new Date(Date.now() - 1000).toISOString(),
    expires_at: new Date(Date.now() + 86_400_000).toISOString(),
    discovery: [],
    remotes: [],
    overrides: config.models.map((model) => ({ route_id: model.id, availability: "active" })),
  };
}

export async function catalog_fixture(directory: string) {
  const fixture = await rich_fixture(directory, (config) => {
    config.catalog = {
      directory: join(directory, "catalog"),
      sources: join(directory, "sources.json"),
      maximum_generations: 64,
      trusted_keys: [],
    };
  });
  const options = fixture.config.catalog;
  if (!options) throw new Error("missing_catalog");
  await open_journal(options.directory);
  const sources = seed_sources(fixture.config);
  await writeFile(options.sources, JSON.stringify(sources), { mode: 0o600 });
  const generation = await build_catalog(
    fixture.config,
    CATALOG_PLUGIN,
    [],
    CATALOG_SECRETS,
    new AbortController().signal,
    fixture.fetcher,
  );
  await publish_catalog(fixture.config, generation);
  await fixture.host.reload_catalog();
  return { ...fixture, sources };
}

export async function update_catalog(
  fixture: Awaited<ReturnType<typeof catalog_fixture>>,
  change: (sources: CatalogSources) => void,
) {
  const options = fixture.config.catalog;
  if (!options) throw new Error("missing_catalog");
  const sources = structuredClone(fixture.sources);
  sources.revision++;
  sources.issued_at = new Date().toISOString();
  change(sources);
  await writeFile(options.sources, JSON.stringify(sources), { mode: 0o600 });
  const history = await load_catalog(fixture.config);
  const generation = await build_catalog(
    fixture.config,
    CATALOG_PLUGIN,
    history,
    CATALOG_SECRETS,
    new AbortController().signal,
    fixture.fetcher,
  );
  await publish_catalog(fixture.config, generation);
  fixture.sources = sources;
  return generation;
}
