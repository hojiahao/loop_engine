import { isAbsolute } from "node:path";
import { z } from "zod";

const identifier = z.string().regex(/^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$/);
const digest = z.string().regex(/^[a-f0-9]{64}$/);
const amount = z.string().regex(/^(?:0|[1-9][0-9]{0,5})(?:\.[0-9]{0,8}[1-9])?$/);
const timestamp = z.iso.datetime();

export const catalog_options = z.strictObject({
  directory: z.string().refine(isAbsolute),
  sources: z.string().refine(isAbsolute),
  maximum_generations: z.number().int().min(2).max(256).default(64),
  trusted_keys: z
    .array(
      z.strictObject({
        id: identifier,
        public_key: z
          .string()
          .min(32)
          .max(4096)
          .regex(/^-----BEGIN PUBLIC KEY-----\n[\s\S]+\n-----END PUBLIC KEY-----\n?$/),
      }),
    )
    .max(16)
    .default([]),
});

export const availability_schema = z.enum([
  "unknown",
  "active",
  "unavailable",
  "deprecated",
  "retired",
]);
export const verification_schema = z.enum(["implemented", "contract_verified", "live_verified"]);

/** Catalog input describes a model; it cannot change a route's authority. */
export const model_patch = z.strictObject({
  route_id: identifier,
  model: identifier.optional(),
  alias: identifier.optional(),
  context_tokens: z.number().int().min(1).max(2_000_000).optional(),
  input_token_limit: z.number().int().min(1).max(2_000_000).optional(),
  output_tokens: z.number().int().min(1).max(100_000).optional(),
  input_usd: amount.optional(),
  output_usd: amount.optional(),
  cached_usd: amount.optional(),
  cache_creation_usd: amount.optional(),
  features: z
    .strictObject({
      streaming: z.boolean().optional(),
      tools: z.boolean().optional(),
      parallel_tools: z.boolean().optional(),
      structured_output: z.boolean().optional(),
      vision: z.boolean().optional(),
      documents: z.boolean().optional(),
      prompt_caching: z.boolean().optional(),
    })
    .optional(),
  reasoning: z.enum(["off", "low", "medium", "high", "max", "adaptive", "enabled"]).optional(),
  thinking_tokens: z.number().int().min(1024).max(99_999).optional(),
  availability: availability_schema.optional(),
});

export const catalog_document = z.strictObject({
  schema: z.literal("loop.model-catalog/v1"),
  source_id: identifier,
  revision: z.number().int().min(1).max(Number.MAX_SAFE_INTEGER),
  issued_at: timestamp,
  expires_at: timestamp,
  entries: z.array(model_patch).max(128),
});

export const catalog_envelope = z.strictObject({
  schema: z.literal("loop.signed-catalog/v1"),
  key_id: identifier,
  payload: catalog_document,
  signature: z.string().regex(/^[A-Za-z0-9+/]{86}==$/),
});

export const catalog_sources = z.strictObject({
  schema: z.literal("loop.catalog-sources/v1"),
  source_id: identifier,
  revision: z.number().int().min(1).max(Number.MAX_SAFE_INTEGER),
  issued_at: timestamp,
  expires_at: timestamp,
  discovery: z.array(identifier).max(16).default([]),
  remotes: z
    .array(z.strictObject({ source_id: identifier, url: z.string().max(2048), key_id: identifier }))
    .max(8)
    .default([]),
  overrides: z
    .array(model_patch.extend({ live_receipt: digest.optional() }))
    .max(128)
    .default([]),
});

export const source_receipt = z.strictObject({
  source_id: identifier,
  revision: z.number().int().min(1).max(Number.MAX_SAFE_INTEGER),
  issued_at: timestamp,
  expires_at: timestamp,
  sha256: digest,
  kind: z.enum(["builtin", "discovery", "remote", "local"]),
});

export const catalog_status = z.strictObject({
  route_id: identifier,
  model: identifier,
  availability: availability_schema,
  verification: verification_schema,
  profile_sha256: digest,
  live_receipt: digest.optional(),
});

export type CatalogOptions = z.infer<typeof catalog_options>;
export type CatalogDocument = z.infer<typeof catalog_document>;
export type CatalogSources = z.infer<typeof catalog_sources>;
export type SourceReceipt = z.infer<typeof source_receipt>;
export type CatalogStatus = z.infer<typeof catalog_status>;
export type ModelPatch = z.infer<typeof model_patch>;
