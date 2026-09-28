import { constants } from "node:fs";
import { open, opendir } from "node:fs/promises";
import { join } from "node:path";
import { z } from "zod";
import { catalog_status, source_receipt } from "./catalog-types.js";
import { type Deployment, model_schema, read_private, validate_deployment } from "./config.js";
import { ProviderError } from "./errors.js";
import { canonical_json, digest_json, hex_digest } from "./identity.js";
import { finish_invocation } from "./journal.js";
import { json_digest, parse_json } from "./json.js";

const digest = z.string().regex(/^[a-f0-9]{64}$/);
export const generation_schema = z.strictObject({
  schema: z.literal("loop.provider-catalog-generation/v1"),
  revision: z.number().int().min(1).max(256),
  previous_sha256: digest.nullable(),
  resolved_at: z.iso.datetime(),
  expires_at: z.iso.datetime(),
  plugin_sha256: digest,
  deployment_sha256: digest,
  sources: z.array(source_receipt).min(2).max(26),
  models: z.array(model_schema).min(1).max(128),
  statuses: z.array(catalog_status).min(1).max(128),
});
export type CatalogGeneration = z.infer<typeof generation_schema>;

/** Strip absent optional values; canonical pins still reject lossy numbers. */
export function catalog_bytes(value: unknown): Uint8Array {
  const encoded = JSON.stringify(value, (_key, item) => {
    if (
      (typeof item === "number" && !Number.isSafeInteger(item)) ||
      (Array.isArray(item) && item.some((entry) => entry === undefined))
    )
      throw new ProviderError("invalid_catalog_value");
    return item;
  });
  return Buffer.from(canonical_json(JSON.parse(encoded)), "utf8");
}

export function catalog_digest(value: CatalogGeneration): string {
  return hex_digest(
    digest_json("loop.provider-catalog-generation/v1", parse_json(catalog_bytes(value), 524_288)),
  );
}

export function deployment_digest(config: Deployment): string {
  const { catalog: _catalog, ...authority } = config;
  return hex_digest(
    digest_json("loop.catalog-deployment/v1", parse_json(catalog_bytes(authority))),
  );
}

export function profile_digest(model: Deployment["models"][number]): string {
  return hex_digest(digest_json("loop.catalog-profile/v1", parse_json(catalog_bytes(model))));
}

function catalog_failure(): never {
  throw new ProviderError("provider_catalog_corrupt");
}

function validate_generation(
  config: Deployment,
  record: CatalogGeneration,
  history: readonly CatalogGeneration[],
): void {
  const previous = history.at(-1);
  const resolved = new Date(record.resolved_at).getTime();
  if (
    record.revision !== history.length + 1 ||
    record.previous_sha256 !== (previous ? catalog_digest(previous) : null) ||
    resolved >= new Date(record.expires_at).getTime() ||
    (previous && resolved < new Date(previous.resolved_at).getTime()) ||
    new Set(record.sources.map((source) => source.source_id)).size !== record.sources.length
  )
    catalog_failure();
  const watermarks = new Map(
    history.flatMap((item) => item.sources.map((source) => [source.source_id, source] as const)),
  );
  for (const source of record.sources) {
    const last = watermarks.get(source.source_id);
    if (
      new Date(source.issued_at).getTime() > resolved ||
      new Date(source.expires_at).getTime() < new Date(record.expires_at).getTime() ||
      (last &&
        (source.kind !== last.kind ||
          source.revision < last.revision ||
          (source.revision === last.revision && source.sha256 !== last.sha256)))
    )
      catalog_failure();
  }
  const parsed = validate_deployment({
    ...config,
    resolved_at: record.resolved_at,
    models: record.models,
  });
  if (
    new Set(record.statuses.map((status) => status.route_id)).size !== record.models.length ||
    record.statuses.length !== record.models.length ||
    new Set(parsed.models.map((model) => model.alias)).size !== record.models.length
  )
    catalog_failure();
  for (const model of parsed.models) {
    const status = record.statuses.find((status) => status.route_id === model.id);
    const seed = config.models.find((seed) => seed.id === model.id);
    const authority = (value: Deployment["models"][number]) =>
      catalog_bytes({
        id: value.id,
        plugin: value.plugin,
        secret_env: value.secret_env,
        cloud: value.cloud,
        vendor: value.vendor,
        compatible: value.compatible,
      });
    if (
      !status ||
      !seed ||
      !Buffer.from(authority(seed)).equals(authority(model)) ||
      status.model !== model.model ||
      status.profile_sha256 !== profile_digest(model) ||
      (status.verification === "live_verified") !== Boolean(status.live_receipt)
    )
      catalog_failure();
    if (
      status.availability !== "retired" &&
      history.some((item) =>
        item.statuses.some(
          (prior) =>
            prior.route_id === model.id &&
            prior.model === model.model &&
            prior.availability === "retired",
        ),
      )
    )
      throw new ProviderError("provider_catalog_retired");
  }
}

/** A record uses the existing fsync+exclusive-link immutable result publisher. */
async function read_generation(path: string): Promise<CatalogGeneration> {
  const record = z
    .strictObject({ bytes: z.string(), sha256: digest })
    .parse(parse_json(await read_private(path), 1_048_576));
  const bytes = Buffer.from(record.bytes, "base64");
  if (
    bytes.length > 524_288 ||
    bytes.toString("base64") !== record.bytes ||
    hex_digest(json_digest(bytes)) !== record.sha256
  )
    catalog_failure();
  const generation = generation_schema.parse(parse_json(bytes, 524_288));
  if (!Buffer.from(catalog_bytes(generation)).equals(bytes)) catalog_failure();
  return generation;
}

/** Read a bounded contiguous chain. Old validity windows remain audit evidence. */
export async function load_catalog(config: Deployment): Promise<CatalogGeneration[]> {
  const options = config.catalog;
  if (!options) throw new ProviderError("provider_catalog_missing");
  const handle = await open(
    options.directory,
    constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW,
  );
  try {
    const info = await handle.stat();
    if ((info.mode & 0o077) !== 0 || info.uid !== process.getuid?.()) catalog_failure();
  } finally {
    await handle.close();
  }
  const names: string[] = [];
  let scanned = 0;
  const directory = await opendir(options.directory);
  for await (const entry of directory) {
    if (++scanned > options.maximum_generations * 2 + 16) catalog_failure();
    if (/^\.pending-[a-f0-9-]{36}$/.test(entry.name) && entry.isFile()) continue;
    if (!entry.isFile() || !/^[0-9]{4}\.result$/.test(entry.name)) catalog_failure();
    names.push(entry.name);
    if (names.length > options.maximum_generations) catalog_failure();
  }
  names.sort();
  const history: CatalogGeneration[] = [];
  for (const [index, name] of names.entries()) {
    if (name !== `${String(index + 1).padStart(4, "0")}.result`) catalog_failure();
    const record = await read_generation(join(options.directory, name));
    validate_generation(config, record, history);
    history.push(record);
  }
  return history;
}

/** Publish one expected successor. A competing writer cannot replace its bytes. */
export async function publish_catalog(
  config: Deployment,
  generation: CatalogGeneration,
): Promise<void> {
  const options = config.catalog;
  if (!options) throw new ProviderError("provider_catalog_missing");
  const history = await load_catalog(config);
  const previous = history.at(-1);
  if (
    history.length >= options.maximum_generations ||
    generation.revision !== history.length + 1 ||
    generation.previous_sha256 !== (previous ? catalog_digest(previous) : null)
  )
    throw new ProviderError("provider_catalog_conflict");
  generation = generation_schema.parse(generation);
  validate_generation(config, generation, history);
  const now = Date.now();
  if (
    now < new Date(generation.resolved_at).getTime() ||
    now >= new Date(generation.expires_at).getTime()
  )
    throw new ProviderError("provider_catalog_expired");
  const bytes = catalog_bytes(generation);
  const name = `${String(generation.revision).padStart(4, "0")}.result`;
  try {
    await finish_invocation(
      options.directory,
      { result_path: join(options.directory, name) },
      bytes,
    );
  } catch {
    // No automatic retry: the caller must reload and review the winning revision.
    throw new ProviderError("provider_catalog_publish_failed");
  }
}
