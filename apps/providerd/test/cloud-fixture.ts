import { writeFileSync } from "node:fs";
import { clone } from "@bufbuild/protobuf";
import { JsonSchemaSchema } from "@loop-engine/protocol/provider";
import { EventStreamCodec } from "@smithy/core/event-streams";
import type { CloudIdentity } from "../src/cloud-auth.js";
import { type Deployment, validate_deployment } from "../src/config.js";
import { hex_digest } from "../src/identity.js";
import { json_bytes, json_digest } from "../src/json.js";
import { additional_reply } from "./additional-fixture.js";
import { test_reply } from "./fixture.js";
import { rich_fixture, TEST_SCHEMA, tool_reply } from "./rich-fixture.js";

export const CLOUDS = [
  "azure_responses",
  "azure_chat",
  "vertex_generate",
  "bedrock_converse",
] as const;
export type CloudRoute = (typeof CLOUDS)[number];
export const CLOUD_SCHEMA = clone(JsonSchemaSchema, TEST_SCHEMA);
CLOUD_SCHEMA.canonicalJson = json_bytes({
  type: "object",
  properties: { window: { type: "integer" } },
  required: ["window"],
  additionalProperties: false,
});
CLOUD_SCHEMA.schemaSha256 = {
  $typeName: "loop.v1.Sha256Digest",
  value: json_digest(CLOUD_SCHEMA.canonicalJson),
};
export const CLOUD_SECRETS = {
  LOOP_LLM_AWS_ACCESS: "AKIAFIXTURE0000000000",
  LOOP_LLM_AWS_SECRET: "fixture-signing-secret-not-real",
  LOOP_LLM_AWS_SESSION: "fixture-session-token-not-real",
};

export function cloud_config(config: Deployment): void {
  const base = config.models[0];
  if (!base) throw new Error("fixture_missing_model");
  config.models = CLOUDS.map((plugin) => ({
    ...base,
    id: plugin,
    alias: plugin,
    plugin,
    input_token_limit: 128,
    model:
      plugin === "azure_responses"
        ? "responses-fixture-20260901"
        : plugin === "azure_chat"
          ? "chat-fixture-20260901"
          : plugin === "vertex_generate"
            ? "google_generate-fixture-20260901"
            : "anthropic.claude-fixture-v1:0",
    secret_env: plugin.startsWith("azure") ? "LOOP_LLM_TEST" : undefined,
    ...(plugin === "bedrock_converse" ? { cache_creation_usd: "2" } : {}),
    cloud: plugin.startsWith("azure")
      ? {
          kind: "azure",
          resource: "loop-test",
          domain: "openai.azure.com",
          deployment: `deployment-${plugin}`,
          auth: "api_key",
        }
      : plugin === "vertex_generate"
        ? { kind: "vertex", project: "loop-fixture", location: "us-central1" }
        : {
            kind: "bedrock",
            region: "us-east-1",
            model_id: "us.anthropic.claude-fixture-v1:0",
            reasoning_dialect: "none",
            credentials: {
              access_key_env: "LOOP_LLM_AWS_ACCESS",
              secret_key_env: "LOOP_LLM_AWS_SECRET",
              session_token_env: "LOOP_LLM_AWS_SESSION",
            },
            guardrail: { id: "guardrail123", version: "3", maximum_usd: "0.0001" },
          },
  }));
  const principal = config.principals[0];
  if (!principal) throw new Error("fixture_missing_principal");
  principal.model_ids = [...CLOUDS];
}

export async function cloud_fixture(
  directory: string,
  configure?: (config: Deployment) => void,
  identity: CloudIdentity = {
    azure: async () => "fixture-entra-token",
    google: async () => "fixture-google-token",
  },
  secrets: Readonly<Record<string, string | undefined>> = CLOUD_SECRETS,
) {
  return rich_fixture(
    directory,
    (config) => {
      cloud_config(config);
      const schema = config.schemas[0];
      if (!schema) throw new Error("fixture_missing_schema");
      writeFileSync(schema.path, CLOUD_SCHEMA.canonicalJson, { mode: 0o600 });
      schema.sha256 = hex_digest(json_digest(CLOUD_SCHEMA.canonicalJson));
      configure?.(config);
      validate_deployment(config);
    },
    { identity, secrets },
  );
}

export function cloud_reply(plugin: CloudRoute, tools = false): unknown {
  if (plugin === "vertex_generate") return additional_reply("google_generate", tools);
  if (plugin === "azure_responses" || plugin === "azure_chat") {
    const name = plugin === "azure_chat" ? "chat" : "responses";
    return tools
      ? tool_reply(name)
      : test_reply(`/v1/${name === "chat" ? "chat/completions" : "responses"}`, {
          model: `${name}-fixture-20260901`,
        });
  }
  return {
    output: {
      message: {
        role: "assistant",
        content: [
          tools
            ? { toolUse: { toolUseId: "call_1", name: "propose_factor", input: { window: 20 } } }
            : { text: "diagnostic idea" },
        ],
      },
    },
    stopReason: tools ? "tool_use" : "end_turn",
    usage: { inputTokens: 9, outputTokens: 5, cacheReadInputTokens: 3, totalTokens: 17 },
    metrics: { latencyMs: 10 },
  };
}

export function bedrock_events(tools = false, thinking = false): Record<string, unknown>[] {
  const index = thinking ? 1 : 0;
  return [
    { messageStart: { role: "assistant" } },
    ...(thinking
      ? [
          {
            contentBlockDelta: {
              contentBlockIndex: 0,
              delta: { reasoningContent: { text: "Summary" } },
            },
          },
          {
            contentBlockDelta: {
              contentBlockIndex: 0,
              delta: { reasoningContent: { signature: "signed-thinking" } },
            },
          },
          { contentBlockStop: { contentBlockIndex: 0 } },
        ]
      : []),
    ...(tools
      ? [
          {
            contentBlockStart: {
              contentBlockIndex: index,
              start: { toolUse: { toolUseId: "call_1", name: "propose_factor" } },
            },
          },
          {
            contentBlockDelta: {
              contentBlockIndex: index,
              delta: { toolUse: { input: '{ "window":' } },
            },
          },
          {
            contentBlockDelta: { contentBlockIndex: index, delta: { toolUse: { input: "20 }" } } },
          },
        ]
      : [
          { contentBlockDelta: { contentBlockIndex: index, delta: { text: "研究 " } } },
          { contentBlockDelta: { contentBlockIndex: index, delta: { text: "idea" } } },
        ]),
    { contentBlockStop: { contentBlockIndex: index } },
    { messageStop: { stopReason: tools ? "tool_use" : "end_turn" } },
    {
      metadata: {
        usage: { inputTokens: 9, outputTokens: 5, cacheReadInputTokens: 3, totalTokens: 17 },
        metrics: { latencyMs: 10 },
      },
    },
  ];
}

export function event_bytes(events: readonly Record<string, unknown>[]): Uint8Array[] {
  const codec = new EventStreamCodec(
    (bytes) => new TextDecoder().decode(bytes),
    (text) => new TextEncoder().encode(text),
  );
  const frames = events.map((event) => {
    const name = Object.keys(event)[0];
    if (!name) throw new Error("fixture_missing_event");
    return codec.encode({
      headers: {
        ":message-type": {
          type: "string",
          value: name.endsWith("Exception") ? "exception" : "event",
        },
        [name.endsWith("Exception") ? ":exception-type" : ":event-type"]: {
          type: "string",
          value: name,
        },
        ":content-type": { type: "string", value: "application/json" },
      },
      body: new TextEncoder().encode(JSON.stringify(event[name])),
    });
  });
  // Deliberately split inside the prelude, headers, UTF-8 and CRC.
  return frames.flatMap((frame) => [frame.slice(0, 7), frame.slice(7, 41), frame.slice(41)]);
}
