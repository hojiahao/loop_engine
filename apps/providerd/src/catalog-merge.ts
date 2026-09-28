import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { fromBinary } from "@bufbuild/protobuf";
import { ModelFinishReason, ModelResponseSchema } from "@loop-engine/protocol/provider";
import { z } from "zod";
import { check_validity, fetch_catalog } from "./catalog-fetch.js";
import {
  type CatalogGeneration,
  catalog_bytes,
  catalog_digest,
  deployment_digest,
  profile_digest,
} from "./catalog-store.js";
import {
  type CatalogStatus,
  catalog_sources,
  type ModelPatch,
  type SourceReceipt,
} from "./catalog-types.js";
import {
  type Deployment,
  type ModelRoute,
  model_schema,
  model_snapshot,
  read_private,
  validate_deployment,
} from "./config.js";
import { ProviderError } from "./errors.js";
import { digest_json, hex_digest } from "./identity.js";
import { json_digest, parse_json } from "./json.js";
import { type DiscoveredModel, discover_models } from "./model-discovery.js";

interface MergedEntry {
  route: ModelRoute;
  availability: CatalogStatus["availability"];
}

/** The source tree versions implemented codecs; it does not invent live models. */
async function builtin_receipt(plugin: Uint8Array): Promise<SourceReceipt> {
  const bytes = await readFile(new URL("../../../catalog/providers.v1.json", import.meta.url));
  const builtin = z
    .strictObject({
      schema: z.literal("loop.provider-profiles/v1"),
      revision: z.number().int().positive(),
      issued_at: z.iso.datetime(),
      profile: z.literal("native-content.1"),
      groups: z
        .array(
          z.strictObject({
            plugins: z.array(model_schema.shape.plugin).min(1),
            contract: z.string().regex(/^test\/[a-z-]+\.test\.ts$/),
          }),
        )
        .min(1)
        .max(32),
    })
    .parse(parse_json(bytes));
  const plugins = builtin.groups.flatMap((group) => group.plugins);
  if (
    new Set(plugins).size !== plugins.length ||
    [...plugins].sort().join("\0") !== [...model_schema.shape.plugin.options].sort().join("\0")
  )
    throw new ProviderError("provider_catalog_builtin");
  return {
    source_id: `builtin-${hex_digest(plugin)}`,
    revision: builtin.revision,
    issued_at: builtin.issued_at,
    expires_at: "9999-12-31T23:59:59Z",
    sha256: hex_digest(json_digest(bytes)),
    kind: "builtin",
  };
}

function merge_patch(entries: Map<string, MergedEntry>, patch: ModelPatch): void {
  const selected = entries.get(patch.route_id);
  if (!selected) throw new ProviderError("provider_catalog_route");
  if (patch.model !== undefined && patch.model !== selected.route.model) {
    if (
      patch.context_tokens === undefined ||
      patch.output_tokens === undefined ||
      patch.input_usd === undefined ||
      patch.output_usd === undefined ||
      patch.cached_usd === undefined ||
      patch.reasoning === undefined ||
      !patch.features ||
      Object.keys(selected.route.features).some(
        (key) => !Object.hasOwn(patch.features ?? {}, key),
      ) ||
      (selected.route.input_token_limit !== undefined && patch.input_token_limit === undefined)
    )
      throw new ProviderError("provider_catalog_profile_required");
    selected.availability = "unknown";
  }
  const { route_id: _route, availability, features, ...fields } = patch;
  selected.route = {
    ...selected.route,
    ...fields,
    features: { ...selected.route.features, ...features },
  };
  if (patch.reasoning !== undefined && patch.reasoning !== "enabled")
    delete selected.route.thinking_tokens;
  if (availability !== undefined) selected.availability = availability;
}

function merge_discovery(selected: MergedEntry, discovered: DiscoveredModel | undefined): void {
  if (!discovered) {
    selected.availability = "unavailable";
    return;
  }
  selected.availability = discovered.availability;
  const model = selected.route;
  // Listing only narrows an approved seed; it never silently enables a feature.
  if (discovered.input_tokens !== undefined) {
    model.context_tokens = Math.min(model.context_tokens, discovered.input_tokens);
    if (model.input_token_limit !== undefined)
      model.input_token_limit = Math.min(model.input_token_limit, discovered.input_tokens);
  }
  if (discovered.output_tokens !== undefined)
    model.output_tokens = Math.min(model.output_tokens, discovered.output_tokens);
  for (const feature of ["vision", "documents", "structured_output"] as const)
    if (discovered[feature] === false) model.features[feature] = false;
  if (discovered.reasoning === false && model.reasoning !== "off") {
    model.reasoning = "off";
    delete model.thinking_tokens;
  }
}

function unique_patches(patches: readonly ModelPatch[]): void {
  if (new Set(patches.map((entry) => entry.route_id)).size !== patches.length)
    throw new ProviderError("provider_catalog_duplicate");
}

async function verify_live(
  config: Deployment,
  history: readonly CatalogGeneration[],
  route: ModelRoute,
  plugin: Uint8Array,
  receipt: string,
): Promise<void> {
  const stored = z
    .strictObject({
      bytes: z.string(),
      sha256: z.string().regex(/^[a-f0-9]{64}$/),
    })
    .parse(parse_json(await read_private(join(config.journal, `${receipt}.result`)), 1_048_576));
  const bytes = Buffer.from(stored.bytes, "base64");
  if (
    bytes.length > 524_288 ||
    bytes.toString("base64") !== stored.bytes ||
    hex_digest(json_digest(bytes)) !== stored.sha256
  )
    throw new ProviderError("provider_catalog_live_receipt");
  const response = fromBinary(ModelResponseSchema, bytes);
  if (
    !response.usage ||
    !response.requestId?.value ||
    !response.content.length ||
    ![ModelFinishReason.STOP, ModelFinishReason.TOOL_CALL].includes(response.finishReason)
  )
    throw new ProviderError("provider_catalog_live_receipt");
  for (const generation of history) {
    if (
      generation.plugin_sha256 !== hex_digest(plugin) ||
      generation.deployment_sha256 !== deployment_digest(config)
    )
      continue;
    const previous = generation.models.find(
      (model) => model.id === route.id && profile_digest(model) === profile_digest(route),
    );
    if (!previous) continue;
    const snapshot = model_snapshot(
      { ...config, models: generation.models, resolved_at: generation.resolved_at },
      previous,
      plugin,
      Buffer.from(catalog_digest(generation), "hex"),
    );
    if (response.resolutionId?.value === snapshot.resolutionId?.value) return;
  }
  throw new ProviderError("provider_catalog_live_receipt");
}

/** Build the entire candidate before publishing or changing a runtime route. */
export async function build_catalog(
  config: Deployment,
  plugin: Uint8Array,
  history: readonly CatalogGeneration[],
  secrets: Readonly<Record<string, string | undefined>>,
  signal: AbortSignal,
  fetcher: typeof fetch = fetch,
): Promise<CatalogGeneration> {
  const options = config.catalog;
  if (!options) throw new ProviderError("provider_catalog_missing");
  const sources = catalog_sources.parse(parse_json(await read_private(options.sources)));
  check_validity(sources, Date.now());
  if (
    new Set(sources.discovery).size !== sources.discovery.length ||
    new Set(sources.remotes.map((source) => source.source_id)).size !== sources.remotes.length
  )
    throw new ProviderError("provider_catalog_duplicate");
  unique_patches(sources.overrides);
  const entries = new Map(
    config.models.map((model) => [
      model.id,
      {
        route: structuredClone(model),
        availability: "unknown" as CatalogStatus["availability"],
      },
    ]),
  );
  const receipts: SourceReceipt[] = [await builtin_receipt(plugin)];
  for (const id of sources.discovery) {
    signal.throwIfAborted();
    const entry = entries.get(id);
    if (!entry) throw new ProviderError("provider_catalog_route");
    const inventory = await discover_models(entry.route, secrets, signal, fetcher);
    merge_discovery(
      entry,
      inventory.models.find((item) => item.id === entry.route.model),
    );
    const source_id = `discovery-${hex_digest(digest_json("loop.catalog-route/v1", id))}`;
    const revision =
      Math.max(
        0,
        ...history.flatMap((item) =>
          item.sources
            .filter((source) => source.source_id === source_id)
            .map((source) => source.revision),
        ),
      ) + 1;
    receipts.push({
      source_id,
      revision,
      issued_at: new Date().toISOString(),
      expires_at: sources.expires_at,
      sha256: inventory.sha256,
      kind: "discovery",
    });
  }
  for (const remote of sources.remotes) {
    signal.throwIfAborted();
    const document = await fetch_catalog(remote, options, signal, fetcher);
    unique_patches(document.entries);
    for (const patch of document.entries) merge_patch(entries, patch);
    receipts.push({
      source_id: document.source_id,
      revision: document.revision,
      issued_at: document.issued_at,
      expires_at: document.expires_at,
      sha256: hex_digest(json_digest(catalog_bytes(document))),
      kind: "remote",
    });
  }
  for (const { live_receipt: _live, ...patch } of sources.overrides) merge_patch(entries, patch);
  receipts.push({
    source_id: sources.source_id,
    revision: sources.revision,
    issued_at: sources.issued_at,
    expires_at: sources.expires_at,
    sha256: hex_digest(json_digest(catalog_bytes(sources))),
    kind: "local",
  });
  if (new Set(receipts.map((source) => source.source_id)).size !== receipts.length)
    throw new ProviderError("provider_catalog_duplicate");
  const resolved_at = new Date().toISOString();
  for (const receipt of receipts) check_validity(receipt, Date.parse(resolved_at));
  const candidate = validate_deployment({
    ...config,
    resolved_at,
    models: [...entries.values()].map((entry) => entry.route),
  });
  const statuses: CatalogStatus[] = [];
  for (const route of candidate.models) {
    const live = sources.overrides.find((patch) => patch.route_id === route.id)?.live_receipt;
    if (live) await verify_live(config, history, route, plugin, live);
    statuses.push({
      route_id: route.id,
      model: route.model,
      availability: entries.get(route.id)?.availability ?? "unknown",
      verification: live ? "live_verified" : "contract_verified",
      profile_sha256: profile_digest(route),
      ...(live ? { live_receipt: live } : {}),
    });
  }
  signal.throwIfAborted();
  const previous = history.at(-1);
  return {
    schema: "loop.provider-catalog-generation/v1",
    revision: history.length + 1,
    previous_sha256: previous ? catalog_digest(previous) : null,
    resolved_at,
    expires_at: new Date(
      Math.min(...receipts.map((item) => Date.parse(item.expires_at))),
    ).toISOString(),
    plugin_sha256: hex_digest(plugin),
    deployment_sha256: deployment_digest(config),
    sources: receipts,
    models: candidate.models,
    statuses,
  };
}
