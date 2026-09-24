import { z } from "zod";
import { compatible_id, GATEWAY_IDS } from "./compatible-config.js";
import type { ModelRoute } from "./config.js";
import { native_error, ProviderError } from "./errors.js";
import { digest_json, hex_digest } from "./identity.js";
import { json_digest, parse_json } from "./json.js";
import { bounded_fetch } from "./native.js";

const identifier = z.string().regex(/^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$/);
const capacity = z.number().int().positive().max(100_000_000);
const next_token = z.string().min(1).max(4096);

export interface DiscoveredModel {
  readonly id: string;
  readonly availability: "active" | "deprecated" | "retired";
  readonly input_tokens?: number;
  readonly output_tokens?: number;
  readonly vision?: boolean;
  readonly documents?: boolean;
  readonly structured_output?: boolean;
  readonly reasoning?: boolean;
}
export interface ModelInventory {
  readonly route_id: string;
  readonly models: readonly DiscoveredModel[];
  readonly sha256: string;
}

type ListDialect = "openai" | "anthropic" | "google" | "cohere";

function discovery_route(model: ModelRoute) {
  const bearer = { dialect: "openai" as ListDialect, auth: "bearer" as const };
  if (["openai_responses", "openai_chat"].includes(model.plugin))
    return { ...bearer, url: "https://api.openai.com/v1/models" };
  if (model.plugin === "anthropic")
    return {
      dialect: "anthropic" as const,
      auth: "api_key" as const,
      url: "https://api.anthropic.com/v1/models",
    };
  if (["google_generate", "google_interactions"].includes(model.plugin))
    return {
      dialect: "google" as const,
      auth: "google_key" as const,
      url: "https://generativelanguage.googleapis.com/v1beta/models",
    };
  if (model.plugin === "cohere")
    return { ...bearer, dialect: "cohere" as const, url: "https://api.cohere.com/v1/models" };
  if (compatible_id(model.plugin) && model.compatible && !GATEWAY_IDS.includes(model.plugin))
    return {
      dialect: model.compatible.wire === "messages" ? ("anthropic" as const) : ("openai" as const),
      auth: model.compatible.auth,
      url: `${model.compatible.base_url}${model.compatible.wire === "messages" ? "/v1" : ""}/models`,
    };
  throw new ProviderError("provider_discovery_unavailable");
}

function optional_capacity(value: unknown): number | undefined {
  return value === undefined || value === null ? undefined : capacity.parse(value);
}

function parse_models(value: unknown, dialect: ListDialect, now: number) {
  const object = z.record(z.string(), z.unknown()).parse(value);
  const raw = z
    .array(z.record(z.string(), z.unknown()))
    .max(2048)
    .parse(dialect === "google" || dialect === "cohere" ? object.models : object.data);
  const models: DiscoveredModel[] = [];
  for (const item of raw) {
    if (dialect === "google") {
      const name = z
        .string()
        .regex(/^models\/[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/)
        .parse(item.name);
      const methods = z.array(z.string()).max(32).parse(item.supportedGenerationMethods);
      if (!methods.includes("generateContent")) continue;
      models.push({
        id: name.slice(7),
        availability: "active",
        input_tokens: optional_capacity(item.inputTokenLimit),
        output_tokens: optional_capacity(item.outputTokenLimit),
        ...(item.thinking === undefined ? {} : { reasoning: z.boolean().parse(item.thinking) }),
      });
    } else if (dialect === "cohere") {
      const methods = z.array(z.string()).max(32).parse(item.endpoints);
      if (!methods.includes("chat")) continue;
      models.push({
        id: identifier.parse(item.name),
        availability: z.boolean().parse(item.is_deprecated) ? "deprecated" : "active",
        input_tokens: optional_capacity(item.context_length),
      });
    } else if (dialect === "anthropic") {
      const capabilities =
        item.capabilities == null
          ? undefined
          : z.record(z.string(), z.unknown()).parse(item.capabilities);
      const flags: Partial<DiscoveredModel> = {};
      for (const [native, field] of [
        ["image_input", "vision"],
        ["pdf_input", "documents"],
        ["structured_outputs", "structured_output"],
        ["thinking", "reasoning"],
      ] as const) {
        if (capabilities?.[native] == null) continue;
        Object.assign(flags, {
          [field]: z.object({ supported: z.boolean() }).parse(capabilities[native]).supported,
        });
      }
      models.push({
        ...flags,
        id: identifier.parse(item.id),
        availability: "active",
        input_tokens: optional_capacity(item.max_input_tokens),
        output_tokens: optional_capacity(item.max_tokens),
      });
    } else {
      const shutdown =
        item.shutdown_date == null ? undefined : z.iso.date().parse(item.shutdown_date);
      models.push({
        id: identifier.parse(item.id),
        availability: shutdown
          ? Date.parse(shutdown) <= now
            ? "retired"
            : "deprecated"
          : "active",
      });
    }
  }
  let next: string | undefined;
  if (dialect === "anthropic") {
    if (z.boolean().parse(object.has_more)) next = next_token.parse(object.last_id);
  } else if (dialect === "google" || dialect === "cohere") {
    const token = dialect === "google" ? object.nextPageToken : object.next_page_token;
    if (token !== undefined && token !== null && token !== "") next = next_token.parse(token);
  } else if (object.has_more === true || object.next_page || object.next_page_token)
    throw new ProviderError("provider_discovery_pagination");
  return { models, next };
}

/** Read-only inventory. Listing never establishes live generation or a price. */
export async function discover_models(
  model: ModelRoute,
  secrets: Readonly<Record<string, string | undefined>>,
  signal: AbortSignal,
  fetcher: typeof fetch = fetch,
): Promise<ModelInventory> {
  const route = discovery_route(model);
  const headers = new Headers({ accept: "application/json" });
  if (route.auth !== "none") {
    const secret = model.secret_env ? secrets[model.secret_env] : undefined;
    if (!secret || !/^[\x21-\x7e]{1,4096}$/.test(secret))
      throw new ProviderError("provider_credentials_missing");
    headers.set(
      route.auth === "api_key"
        ? "x-api-key"
        : route.auth === "google_key"
          ? "x-goog-api-key"
          : "authorization",
      route.auth === "bearer" ? `Bearer ${secret}` : secret,
    );
  }
  if (route.dialect === "anthropic") headers.set("anthropic-version", "2023-06-01");
  const transport = bounded_fetch(fetcher);
  const models: DiscoveredModel[] = [];
  const pages: string[] = [];
  const cursors = new Set<string>();
  let next: string | undefined;
  try {
    for (let page = 0; page < 8; page++) {
      const url = new URL(route.url);
      if (route.dialect === "anthropic") url.searchParams.set("limit", "1000");
      if (route.dialect === "google") url.searchParams.set("pageSize", "1000");
      if (route.dialect === "cohere") {
        url.searchParams.set("page_size", "1000");
        url.searchParams.set("endpoint", "chat");
      }
      if (next)
        url.searchParams.set(
          route.dialect === "anthropic"
            ? "after_id"
            : route.dialect === "google"
              ? "pageToken"
              : "page_token",
          next,
        );
      const response = await transport(url, {
        method: "GET",
        headers,
        redirect: "error",
        signal: AbortSignal.any([signal, AbortSignal.timeout(5000)]),
      });
      if (!response.ok) throw { status: response.status };
      if (response.headers.get("content-type")?.split(";")[0]?.trim() !== "application/json")
        throw new ProviderError("provider_discovery_content");
      const bytes = new Uint8Array(await response.arrayBuffer());
      pages.push(hex_digest(json_digest(bytes)));
      const parsed = parse_models(parse_json(bytes, 524_288), route.dialect, Date.now());
      models.push(...parsed.models);
      if (models.length > 2048 || new Set(models.map((item) => item.id)).size !== models.length)
        throw new ProviderError("provider_discovery_limit");
      next = parsed.next;
      if (!next)
        return {
          route_id: model.id,
          models: models.sort((left, right) =>
            left.id < right.id ? -1 : left.id > right.id ? 1 : 0,
          ),
          sha256: hex_digest(digest_json("loop.model-discovery/v1", pages)),
        };
      if (cursors.has(next)) throw new ProviderError("provider_discovery_cycle");
      cursors.add(next);
    }
    throw new ProviderError("provider_discovery_limit");
  } catch (error) {
    throw native_error(error, signal);
  }
}
