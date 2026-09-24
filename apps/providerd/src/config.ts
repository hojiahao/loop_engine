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
import { VENDOR_IDS, VENDORS, vendor_id, vendor_valid } from "./vendor-registry.js";

const token = z.string().regex(/^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$/);
const decimal = z.string().regex(/^(?:0|[1-9][0-9]{0,5})(?:\.[0-9]{0,8}[1-9])?$/);
const private_path = z.string().refine(isAbsolute);
const secret_ref = z.string().regex(/^LOOP_LLM_[A-Z0-9_]{1,64}$/);
const cloud_schema = z.discriminatedUnion("kind", [
  z.strictObject({
    kind: z.literal("azure"),
    resource: z.string().regex(/^[a-z0-9][a-z0-9-]{1,62}[a-z0-9]$/),
    domain: z.enum(["openai.azure.com", "services.ai.azure.com"]).default("openai.azure.com"),
    deployment: z.string().regex(/^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/),
    auth: z.enum(["api_key", "entra"]),
  }),
  z.strictObject({
    kind: z.literal("vertex"),
    project: z.string().regex(/^[a-z][a-z0-9-]{4,28}[a-z0-9]$/),
    location: z.string().regex(/^(?:global|[a-z]+-[a-z]+[0-9]{1,2})$/),
  }),
  z.strictObject({
    kind: z.literal("bedrock"),
    region: z.string().regex(/^(?:us|eu|ap|ca|sa|me|af|il|mx)-(?:[a-z]+-)?[a-z]+-[0-9]$/),
    model_id: z.string().regex(/^[A-Za-z0-9][A-Za-z0-9._:/-]{0,255}$/),
    credentials: z
      .strictObject({
        access_key_env: secret_ref,
        secret_key_env: secret_ref,
        session_token_env: secret_ref.optional(),
      })
      .optional(),
    guardrail: z
      .strictObject({
        id: z
          .string()
          .regex(/^(?:[a-z0-9]+|arn:aws:bedrock:[a-z0-9-]+:[0-9]{12}:guardrail\/[a-z0-9]+)$/),
        version: z.string().regex(/^[1-9][0-9]{0,7}$/),
        maximum_usd: decimal,
      })
      .optional(),
    reasoning_dialect: z.enum(["none", "anthropic"]).default("none"),
  }),
]);
const model_schema = z.strictObject({
  id: token,
  plugin: z.enum([
    "openai_responses",
    "openai_chat",
    "anthropic",
    "google_generate",
    "google_interactions",
    "cohere",
    "azure_responses",
    "azure_chat",
    "vertex_generate",
    "bedrock_converse",
    ...VENDOR_IDS,
  ]),
  model: token,
  alias: token,
  context_tokens: z.number().int().min(1).max(2_000_000),
  // Vendor-documented maximum input, required where no full request counter exists.
  input_token_limit: z.number().int().min(1).max(2_000_000).optional(),
  output_tokens: z.number().int().min(1).max(100_000),
  input_usd: decimal,
  output_usd: decimal,
  cached_usd: decimal,
  cache_creation_usd: decimal.optional(),
  features: z
    .strictObject({
      streaming: z.boolean().default(false),
      tools: z.boolean().default(false),
      parallel_tools: z.boolean().default(false),
      structured_output: z.boolean().default(false),
      vision: z.boolean().default(false),
      documents: z.boolean().default(false),
      prompt_caching: z.boolean().default(false),
    })
    .prefault({}),
  reasoning: z.enum(["off", "low", "medium", "high", "max", "adaptive", "enabled"]).default("off"),
  thinking_tokens: z.number().int().min(1024).max(99_999).optional(),
  secret_env: secret_ref.optional(),
  cloud: cloud_schema.optional(),
  vendor: z
    .strictObject({
      region: z.enum(["global", "cn", "us", "jp"]).default("global"),
      workspace: z
        .string()
        .regex(/^[a-z0-9][a-z0-9-]{0,62}$/)
        .optional(),
      reasoning_field: z.enum(["reasoning_content", "reasoning"]).optional(),
      maximum_extra_usd: decimal.optional(),
    })
    .optional(),
});

export const deployment_schema = z.strictObject({
  schema: z.literal("loop.provider-deployment/v1"),
  resolved_at: z.iso.datetime(),
  port: z.number().int().min(1024).max(65535),
  tls: z.strictObject({ ca: private_path, certificate: private_path, key: private_path }),
  journal: private_path,
  prompts: private_path.optional(),
  schemas: z
    .array(
      z.strictObject({
        id: token,
        version: z.number().int().min(1),
        sha256: z.string().regex(/^[a-f0-9]{64}$/),
        path: private_path,
      }),
    )
    .max(128)
    .default([]),
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

function cloud_valid(model: ModelRoute): boolean {
  const cloud = model.cloud;
  if (model.plugin.startsWith("azure"))
    return (
      cloud?.kind === "azure" &&
      (cloud.auth === "api_key" ? model.secret_env !== undefined : model.secret_env === undefined)
    );
  if (model.plugin === "vertex_generate")
    return cloud?.kind === "vertex" && model.secret_env === undefined;
  if (model.plugin === "bedrock_converse")
    return (
      cloud?.kind === "bedrock" &&
      model.secret_env === undefined &&
      !cloud.region.startsWith("us-gov-") &&
      (cloud.model_id.startsWith("arn:")
        ? new RegExp(
            `^arn:aws:bedrock:${cloud.region}:(?:[0-9]{12})?:(?:foundation-model|inference-profile|application-inference-profile|provisioned-model)/[A-Za-z0-9._:-]+$`,
          ).test(cloud.model_id)
        : /^[A-Za-z0-9][A-Za-z0-9._:-]*$/.test(cloud.model_id)) &&
      (model.reasoning === "off"
        ? cloud.reasoning_dialect === "none"
        : cloud.reasoning_dialect === "anthropic" &&
          model.model.startsWith("anthropic.") &&
          ["adaptive", "enabled"].includes(model.reasoning))
    );
  return cloud === undefined && model.secret_env !== undefined;
}

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
    new Set(config.schemas.map((schema) => schema.id)).size !== config.schemas.length ||
    config.models.some(
      (model) =>
        !cloud_valid(model) ||
        !vendor_valid(model) ||
        model.output_tokens > model.context_tokens ||
        (vendor_id(model.plugin) ||
        [
          "cohere",
          "google_interactions",
          "azure_responses",
          "azure_chat",
          "vertex_generate",
          "bedrock_converse",
        ].includes(model.plugin)
          ? model.input_token_limit === undefined || model.input_token_limit > model.context_tokens
          : model.input_token_limit !== undefined) ||
        ((model.plugin.startsWith("google") || model.plugin === "vertex_generate") &&
          !/^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(model.model)) ||
        (model.plugin === "cohere" &&
          (model.features.documents || !["off", "enabled"].includes(model.reasoning))) ||
        (model.features.parallel_tools && !model.features.tools) ||
        (model.features.documents && !model.features.vision) ||
        (["anthropic", "bedrock_converse", "minimax"].includes(model.plugin) &&
          model.features.prompt_caching &&
          model.cache_creation_usd === undefined) ||
        (!["anthropic", "bedrock_converse", "minimax"].includes(model.plugin) &&
          model.cache_creation_usd !== undefined) ||
        (["openai_chat", "azure_chat"].includes(model.plugin) && model.reasoning !== "off") ||
        (model.plugin === "anthropic" &&
          !["off", "adaptive", "enabled"].includes(model.reasoning)) ||
        (!vendor_id(model.plugin) &&
          ((!["anthropic", "cohere", "bedrock_converse"].includes(model.plugin) &&
            ["adaptive", "enabled"].includes(model.reasoning)) ||
            model.reasoning === "max")) ||
        (model.reasoning === "enabled" &&
          (model.thinking_tokens === undefined || model.thinking_tokens >= model.output_tokens)) ||
        (model.reasoning !== "enabled" && model.thinking_tokens !== undefined),
    ) ||
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
    "openai-content",
    "openai-stream",
    "anthropic-content",
    "anthropic-stream",
    "native-google",
    "google-content",
    "google-stream",
    "interaction-content",
    "interaction-stream",
    "native-cohere",
    "cohere-content",
    "cohere-stream",
    "cloud-auth",
    "cloud-plugins",
    "native-bedrock",
    "bedrock-content",
    "bedrock-stream",
    "bedrock-transport",
    "vendor-registry",
    "vendor-chat",
    "vendor-replies",
    "native-vendor",
    "json",
    "content",
    "private-state",
    "stream",
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
  const family = vendor_id(model.plugin)
    ? {
        chat: ModelProtocolFamily.OPENAI_CHAT_COMPLETIONS,
        responses: ModelProtocolFamily.OPENAI_RESPONSES,
        messages: ModelProtocolFamily.ANTHROPIC_MESSAGES,
      }[VENDORS[model.plugin].wire]
    : {
        openai_responses: ModelProtocolFamily.OPENAI_RESPONSES,
        openai_chat: ModelProtocolFamily.OPENAI_CHAT_COMPLETIONS,
        anthropic: ModelProtocolFamily.ANTHROPIC_MESSAGES,
        google_generate: ModelProtocolFamily.GOOGLE_GENERATE_CONTENT,
        google_interactions: ModelProtocolFamily.GOOGLE_INTERACTIONS,
        cohere: ModelProtocolFamily.COHERE_V2_CHAT,
        azure_responses: ModelProtocolFamily.OPENAI_RESPONSES,
        azure_chat: ModelProtocolFamily.OPENAI_CHAT_COMPLETIONS,
        vertex_generate: ModelProtocolFamily.GOOGLE_GENERATE_CONTENT,
        bedrock_converse: ModelProtocolFamily.AWS_BEDROCK_CONVERSE,
      }[model.plugin];
  const capabilities = {
    contextWindowTokens: BigInt(model.context_tokens),
    maximumOutputTokens: BigInt(model.output_tokens),
    supportsTools: model.features.tools,
    supportsParallelTools: model.features.parallel_tools,
    supportsStructuredOutput: model.features.structured_output,
    supportsVision: model.features.vision,
    supportsDocuments: model.features.documents,
    supportsReasoning: model.reasoning !== "off",
    supportsPromptCaching: model.features.prompt_caching,
    supportsStreaming: model.features.streaming,
  };
  const capability = digest_json("loop.provider-capabilities/v1", {
    context_tokens: String(model.context_tokens),
    input_token_limit: model.input_token_limit ?? null,
    output_tokens: String(model.output_tokens),
    features: model.features,
    reasoning: model.reasoning,
    thinking_tokens: model.thinking_tokens ?? null,
    profile: "native-content.1",
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
    providerId: {
      value: vendor_id(model.plugin)
        ? model.plugin
        : model.plugin.startsWith("openai")
          ? "openai"
          : model.plugin.startsWith("azure")
            ? "azure_openai"
            : model.plugin === "vertex_generate"
              ? "google_vertex"
              : model.plugin === "bedrock_converse"
                ? "aws_bedrock"
                : model.plugin.startsWith("google")
                  ? "google"
                  : model.plugin,
    },
    modelId: { value: model.model },
    requestedAlias: model.alias,
    protocolFamily: family,
    capabilities,
    pricing: {
      inputPerMillionTokens: { currencyCode: "USD", amount: { value: model.input_usd } },
      outputPerMillionTokens: { currencyCode: "USD", amount: { value: model.output_usd } },
      cachedInputPerMillionTokens: { currencyCode: "USD", amount: { value: model.cached_usd } },
      ...(model.cache_creation_usd === undefined
        ? {}
        : {
            cacheCreationPerMillionTokens: {
              currencyCode: "USD",
              amount: { value: model.cache_creation_usd },
            },
          }),
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
    sha256: {
      value: digest_json("loop.provider-request-policy/v2", {
        ...config.policy,
        schemas: config.schemas.map(({ path: _path, ...schema }) => schema),
        prompt_artifacts: config.prompts !== undefined,
      }),
    },
  });
}
