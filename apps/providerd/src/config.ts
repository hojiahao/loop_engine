import { createHash } from "node:crypto";
import { constants } from "node:fs";
import { open, readFile } from "node:fs/promises";
import { isAbsolute } from "node:path";
import { create, toJson } from "@bufbuild/protobuf";
import { timestampFromDate } from "@bufbuild/protobuf/wkt";
import {
  ModelProtocolFamily,
  ModelResolutionSnapshotSchema,
  PolicyReferenceSchema,
} from "@loop-engine/protocol/provider";
import { z } from "zod";

import { digest_json, hex_digest } from "./identity.js";

const token = z.string().regex(/^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$/);
const decimal = z.string().regex(/^(?:0|[1-9][0-9]{0,5})(?:\.[0-9]{0,8}[1-9])?$/);
const private_path = z.string().refine(isAbsolute);
const model_schema = z.strictObject({
  id: token,
  plugin: z.enum(["openai_responses", "openai_chat", "anthropic"]),
  model: token,
  alias: token,
  context_tokens: z.number().int().min(1).max(2_000_000),
  output_tokens: z.number().int().min(1).max(100_000),
  input_usd: decimal,
  output_usd: decimal,
  cached_usd: decimal,
  secret_env: z.string().regex(/^LOOP_LLM_[A-Z0-9_]{1,64}$/),
});

export const deployment_schema = z.strictObject({
  schema: z.literal("loop.provider-deployment/v1"),
  resolved_at: z.iso.datetime(),
  port: z.number().int().min(1024).max(65535),
  tls: z.strictObject({ ca: private_path, certificate: private_path, key: private_path }),
  journal: private_path,
  principals: z
    .array(
      z.strictObject({
        certificate_sha256: z.string().regex(/^[a-f0-9]{64}$/),
        actor_id: token,
        actor_kind: z.enum(["service", "agent"]),
        model_ids: z.array(token).min(1).max(128),
      }),
    )
    .min(1)
    .max(128),
  policy: z.strictObject({
    id: token,
    revision: z.string().regex(/^[1-9][0-9]{0,9}$/),
    input_tokens: z.number().int().min(1).max(2_000_000),
    output_tokens: z.number().int().min(1).max(100_000),
    maximum_usd: decimal,
    wall_time_ms: z.number().int().min(100).max(300_000),
    concurrency: z.number().int().min(1).max(16),
  }),
  models: z.array(model_schema).min(1).max(128),
});

export type Deployment = z.infer<typeof deployment_schema>;
export type ModelRoute = z.infer<typeof model_schema>;
export type Principal = Deployment["principals"][number];

export async function read_private(path: string, limit = 1_048_576): Promise<Buffer> {
  const handle = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW);
  try {
    const info = await handle.stat();
    if (
      !info.isFile() ||
      info.size < 1 ||
      info.size > limit ||
      (info.mode & 0o077) !== 0 ||
      info.uid !== process.getuid?.()
    ) {
      throw new Error("invalid_private_file");
    }
    const bytes = await handle.readFile();
    if (bytes.length !== info.size) throw new Error("invalid_private_file");
    return bytes;
  } finally {
    await handle.close();
  }
}

export function validate_deployment(value: unknown): Deployment {
  const config = deployment_schema.parse(value);
  const models = new Set(config.models.map((model) => model.id));
  const certificates = new Set(config.principals.map((principal) => principal.certificate_sha256));
  if (
    models.size !== config.models.length ||
    certificates.size !== config.principals.length ||
    config.models.some((model) => model.output_tokens > model.context_tokens) ||
    config.principals.some(
      (principal) =>
        new Set(principal.model_ids).size !== principal.model_ids.length ||
        principal.model_ids.some((id) => !models.has(id)),
    )
  ) {
    throw new Error("invalid_provider_deployment");
  }
  return config;
}

export async function load_deployment(path: string): Promise<Deployment> {
  return validate_deployment(JSON.parse((await read_private(path)).toString("utf8")));
}

/** Bind installed host bytes, package metadata and the workspace dependency lock. */
export async function plugin_digest(): Promise<Uint8Array> {
  const extension = import.meta.url.endsWith(".ts") ? "ts" : "js";
  const hash = createHash("sha256").update("loop.provider-plugin/v1\0");
  for (const name of [
    "index",
    "config",
    "identity",
    "errors",
    "native",
    "native-openai",
    "native-anthropic",
    "host",
    "journal",
    "rpc",
    "server",
    "health",
  ]) {
    hash
      .update(name)
      .update("\0")
      .update(await readFile(new URL(`./${name}.${extension}`, import.meta.url)));
  }
  for (const path of ["../package.json", "../../../pnpm-lock.yaml"]) {
    hash
      .update(path)
      .update("\0")
      .update(await readFile(new URL(path, import.meta.url)));
  }
  return hash.digest();
}

export function model_snapshot(config: Deployment, model: ModelRoute, plugin: Uint8Array) {
  const family = {
    openai_responses: ModelProtocolFamily.OPENAI_RESPONSES,
    openai_chat: ModelProtocolFamily.OPENAI_CHAT_COMPLETIONS,
    anthropic: ModelProtocolFamily.ANTHROPIC_MESSAGES,
  }[model.plugin];
  const capabilities = {
    contextWindowTokens: BigInt(model.context_tokens),
    maximumOutputTokens: BigInt(model.output_tokens),
  };
  const capability = digest_json("loop.provider-capabilities/v1", {
    context_tokens: String(model.context_tokens),
    output_tokens: String(model.output_tokens),
    profile: "unary-text.1",
  });
  const catalog = digest_json(
    "loop.provider-catalog/v1",
    config.models.map(({ secret_env: _secret, ...entry }) => entry),
  );
  const identity = digest_json("loop.provider-resolution/v1", {
    model: model.id,
    catalog: hex_digest(catalog),
    plugin: hex_digest(plugin),
    resolved_at: config.resolved_at,
  });
  const snapshot = create(ModelResolutionSnapshotSchema, {
    resolutionId: { value: `resolution-${hex_digest(identity)}` },
    providerId: { value: model.plugin.startsWith("openai") ? "openai" : "anthropic" },
    modelId: { value: model.model },
    requestedAlias: model.alias,
    protocolFamily: family,
    capabilities,
    pricing: {
      inputPerMillionTokens: { currencyCode: "USD", amount: { value: model.input_usd } },
      outputPerMillionTokens: { currencyCode: "USD", amount: { value: model.output_usd } },
      cachedInputPerMillionTokens: { currencyCode: "USD", amount: { value: model.cached_usd } },
    },
    capabilitySha256: { value: capability },
    catalogSha256: { value: catalog },
    resolvedAt: timestampFromDate(new Date(config.resolved_at)),
    providerPluginName: model.plugin,
    providerPluginVersion: "0.1.0",
    providerPluginSha256: { value: plugin },
  });
  snapshot.snapshotSha256 = {
    $typeName: "loop.v1.Sha256Digest",
    value: digest_json(
      "loop.provider-model/v1",
      toJson(ModelResolutionSnapshotSchema, snapshot, { alwaysEmitImplicit: true }),
    ),
  };
  return snapshot;
}

export function request_policy(config: Deployment) {
  return create(PolicyReferenceSchema, {
    policyId: { value: config.policy.id },
    revision: config.policy.revision,
    sha256: { value: digest_json("loop.provider-request-policy/v1", config.policy) },
  });
}
