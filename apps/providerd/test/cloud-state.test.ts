import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { create } from "@bufbuild/protobuf";
import { timestampNow } from "@bufbuild/protobuf/wkt";
import {
  ArtifactRefSchema,
  ImageDetail,
  ModelFinishReason,
  ModelMessageSchema,
  type ModelResponse,
  ModelRole,
  StreamModelRequestSchema,
  ToolChoiceMode,
  ToolResultStatus,
} from "@loop-engine/protocol/provider";
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { model_snapshot, validate_deployment } from "../src/config.js";
import { ProviderHost } from "../src/host.js";
import { hex_digest } from "../src/identity.js";
import { json_digest } from "../src/json.js";
import { actor_directory, prompt_schema } from "../src/private-state.js";
import {
  bedrock_events,
  CLOUD_SCHEMA,
  CLOUD_SECRETS,
  CLOUDS,
  type CloudRoute,
  cloud_config,
  cloud_fixture,
  cloud_reply,
  event_bytes,
} from "./cloud-fixture.js";
import { TEST_SECRET, test_config, test_request } from "./fixture.js";
import { tool_request } from "./rich-fixture.js";

let directory: string;
let fixture: Awaited<ReturnType<typeof cloud_fixture>>;
beforeAll(async () => {
  directory = await mkdtemp(join(tmpdir(), "loop-provider-cloud-state-"));
  fixture = await cloud_fixture(directory, (config) => {
    const route = config.models.find((model) => model.plugin === "bedrock_converse");
    if (route?.cloud?.kind !== "bedrock") throw new Error("missing_route");
    route.reasoning = "adaptive";
    route.cloud.reasoning_dialect = "anthropic";
  });
}, 30_000);
afterAll(async () => {
  await fixture?.close();
  if (directory) await rm(directory, { recursive: true, force: true });
});
beforeEach(() => {
  fixture.requests.length = 0;
  fixture.state.reply = undefined;
  fixture.state.body = undefined;
  fixture.state.events = undefined;
  fixture.state.chunk_delay = 0;
  fixture.state.stream_closed = false;
});
async function collect(
  command = test_request(fixture.host, "bedrock_converse"),
  signal?: AbortSignal,
) {
  let result: ModelResponse | undefined;
  for await (const response of fixture.client().streamModel(
    create(StreamModelRequestSchema, {
      context: command.context,
      invocation: command.invocation,
    }),
    { timeoutMs: 4500, signal },
  )) {
    if (response.event?.event.case === "completed") result = response.event.event.value.response;
  }
  return result;
}
async function artifact(media: string, bytes: Buffer) {
  const root = fixture.config.prompts;
  if (!root) throw new Error("missing_prompts");
  const digest = json_digest(bytes);
  const id = hex_digest(digest);
  const view = join(root, actor_directory(fixture.principal.actor_id));
  await mkdir(view, { recursive: true, mode: 0o700 });
  await writeFile(join(view, id), bytes, { mode: 0o600 });
  return create(ArtifactRefSchema, {
    artifactId: { value: id },
    uri: `loop-prompt://sha256/${id}`,
    sha256: { value: digest },
    schema: {
      name: "loop.prompt-artifact",
      version: 1,
      schemaSha256: { value: prompt_schema(media) },
    },
    mediaType: media,
    byteSize: BigInt(bytes.length),
    createdAt: timestampNow(),
  });
}
function config_route(plugin: CloudRoute) {
  const config = test_config("/cloud-config-fixture");
  cloud_config(config);
  const route = config.models.find((model) => model.plugin === plugin);
  if (!route) throw new Error("missing_model");
  return { config, route };
}

describe("cloud boundaries", () => {
  it.each(CLOUDS)("pins %s cloud routing in the catalog", (plugin) => {
    const { config, route } = config_route(plugin);
    const before = model_snapshot(config, route, new Uint8Array(32));
    if (route.cloud?.kind === "azure") route.cloud.deployment = "changed-deployment";
    else if (route.cloud?.kind === "vertex") route.cloud.location = "global";
    else if (route.cloud?.kind === "bedrock") route.cloud.region = "us-west-2";
    const after = model_snapshot(config, route, new Uint8Array(32));
    expect(before.catalogSha256).not.toEqual(after.catalogSha256);
    expect(before.snapshotSha256).not.toEqual(after.snapshotSha256);
    expect(before.modelId).toEqual(after.modelId);
  });

  it.each(CLOUDS)("requires %s cloud binding and full input ceiling", (plugin) => {
    const { config, route } = config_route(plugin);
    expect(() => validate_deployment(config)).not.toThrow();
    const cloud = route.cloud;
    delete route.cloud;
    expect(() => validate_deployment(config)).toThrow();
    route.cloud = cloud;
    delete route.input_token_limit;
    expect(() => validate_deployment(config)).toThrow();
  });

  it.each([
    [
      "azure_responses",
      {
        kind: "azure",
        resource: "evil.example/redirect",
        domain: "openai.azure.com",
        deployment: "d",
        auth: "api_key",
      },
    ],
    [
      "azure_chat",
      {
        kind: "azure",
        resource: "loop-test",
        domain: "localhost",
        deployment: "d",
        auth: "api_key",
      },
    ],
    ["vertex_generate", { kind: "vertex", project: "loop-fixture", location: "global/../evil" }],
    [
      "bedrock_converse",
      { kind: "bedrock", region: "cn-north-1", model_id: "model", reasoning_dialect: "none" },
    ],
    [
      "bedrock_converse",
      {
        kind: "bedrock",
        region: "us-east-1",
        model_id: "arn:aws:bedrock:us-east-1:123456789012:prompt/hidden",
        reasoning_dialect: "none",
      },
    ],
    [
      "bedrock_converse",
      {
        kind: "bedrock",
        region: "us-east-1",
        model_id: "arn:aws:bedrock:us-west-2:123456789012:inference-profile/profile",
        reasoning_dialect: "none",
      },
    ],
  ] as const)("denies invalid %s deployment identifiers", (plugin, cloud) => {
    const { config, route } = config_route(plugin);
    expect(() =>
      validate_deployment({
        ...config,
        models: config.models.map((model) => (model === route ? { ...model, cloud } : model)),
      }),
    ).toThrow();
  });

  it("rejects conflicting Azure credential modes", () => {
    const { config, route } = config_route("azure_chat");
    if (route.cloud?.kind !== "azure") throw new Error("missing_route");
    route.cloud.auth = "entra";
    expect(() => validate_deployment(config)).toThrow();
    delete route.secret_env;
    expect(() => validate_deployment(config)).not.toThrow();
  });

  it("denies undeclared Bedrock reasoning dialects", () => {
    const { config, route } = config_route("bedrock_converse");
    route.reasoning = "low";
    expect(() => validate_deployment(config)).toThrow();
    route.reasoning = "adaptive";
    expect(() => validate_deployment(config)).toThrow();
  });

  it.each(CLOUDS)("transmits only authorized %s image and PDF bytes", async (plugin) => {
    const command = test_request(fixture.host, plugin);
    const message = command.invocation?.messages.at(-1);
    if (!message) throw new Error("missing_message");
    const image = Buffer.from(
      "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aCkkAAAAASUVORK5CYII=",
      "base64",
    );
    const pdf = Buffer.from("%PDF-1.4\nfixture\n%%EOF\n");
    message.content.push(
      {
        $typeName: "loop.v1.ContentBlock",
        content: {
          case: "image",
          value: {
            $typeName: "loop.v1.ImageContent",
            artifact: await artifact("image/png", image),
            detail: ImageDetail.AUTO,
          },
        },
      },
      {
        $typeName: "loop.v1.ContentBlock",
        content: {
          case: "document",
          value: {
            $typeName: "loop.v1.DocumentContent",
            artifact: await artifact("application/pdf", pdf),
          },
        },
      },
    );
    fixture.state.reply = cloud_reply(plugin);
    await fixture.client().invokeModel(command, { timeoutMs: 4500 });
    const sent = fixture.requests.at(-1)?.raw;
    expect(sent).toContain(image.toString("base64"));
    expect(sent).toContain(pdf.toString("base64"));
    expect(sent).not.toContain("loop-prompt:");
  });

  it("restores signed Bedrock reasoning after host restart", async () => {
    fixture.state.events = event_bytes(bedrock_events(true, true));
    const command = tool_request(fixture, "bedrock_converse");
    if (!command.invocation?.tools[0]) throw new Error("missing_tool");
    command.invocation.tools[0].inputSchema = CLOUD_SCHEMA;
    const result = await collect(command);
    expect(result?.finishReason).toBe(ModelFinishReason.TOOL_CALL);
    expect(
      JSON.stringify(result, (_key, value: unknown) =>
        typeof value === "bigint" ? String(value) : value,
      ),
    ).not.toContain("signed-thinking");
    const next = tool_request(fixture, "bedrock_converse");
    if (!next.invocation?.tools[0]) throw new Error("missing_tool");
    next.invocation.tools[0].inputSchema = CLOUD_SCHEMA;
    next.invocation.messages.push(
      create(ModelMessageSchema, { role: ModelRole.ASSISTANT, content: result?.content }),
      create(ModelMessageSchema, {
        role: ModelRole.TOOL,
        content: [
          {
            content: {
              case: "toolResult",
              value: {
                toolCallId: "call_1",
                status: ToolResultStatus.SUCCESS,
                result: { case: "text", value: { text: "done" } },
              },
            },
          },
        ],
      }),
    );
    fixture.state.events = undefined;
    fixture.state.reply = cloud_reply("bedrock_converse");
    const restarted = new ProviderHost(
      fixture.config,
      new Uint8Array(32).fill(1),
      { ...CLOUD_SECRETS, LOOP_LLM_TEST: TEST_SECRET },
      fixture.fetcher,
      { google: async () => "fixture-token" },
    );
    const replayed = await restarted.invoke(next, fixture.principal, new AbortController().signal);
    expect(replayed.finishReason).toBe(ModelFinishReason.STOP);
    expect(fixture.requests.at(-1)?.raw).toContain("signed-thinking");
    expect(fixture.requests.at(-1)?.body.additionalModelRequestFields).toEqual({
      thinking: { type: "adaptive" },
    });
  });

  it("accounts separately for Bedrock cache writes", async () => {
    fixture.state.reply = {
      output: { message: { role: "assistant", content: [{ text: "idea" }] } },
      stopReason: "end_turn",
      usage: {
        inputTokens: 7,
        cacheReadInputTokens: 3,
        cacheWriteInputTokens: 2,
        outputTokens: 5,
        totalTokens: 17,
        cacheDetails: [{ ttl: "5m", inputTokens: 2 }],
      },
    };
    const result = await fixture
      .client()
      .invokeModel(test_request(fixture.host, "bedrock_converse"), { timeoutMs: 4500 });
    expect(result.response?.usage).toMatchObject({
      inputTokens: 12n,
      cachedInputTokens: 3n,
      cacheCreationInputTokens: 2n,
    });
    expect(fixture.requests.at(-1)?.body.system).toContainEqual({
      cachePoint: { type: "default", ttl: "5m" },
    });
  });

  it("rejects unsupported Bedrock tool selection before dispatch", async () => {
    const command = tool_request(fixture, "bedrock_converse");
    if (!command.invocation?.tools[0] || !command.invocation.toolChoice)
      throw new Error("missing_tool");
    command.invocation.tools[0].inputSchema = CLOUD_SCHEMA;
    command.invocation.toolChoice.mode = ToolChoiceMode.NONE;
    await expect(fixture.client().invokeModel(command, { timeoutMs: 4500 })).rejects.toThrow(
      "bedrock_tool_choice_denied",
    );
    expect(fixture.requests).toHaveLength(0);
  });

  it("reserves Guardrail cost before dispatch", async () => {
    const command = test_request(fixture.host, "bedrock_converse");
    if (!command.invocation?.budget?.maximumCost?.amount) throw new Error("missing_budget");
    // Covers the token allowance exactly but omits the pinned ancillary charge.
    command.invocation.budget.maximumCost.amount.value = "0.000384";
    await expect(fixture.client().invokeModel(command, { timeoutMs: 4500 })).rejects.toThrow(
      "provider_budget_denied",
    );
    expect(fixture.requests).toHaveLength(0);
  });

  it("requires a priced and published Guardrail", () => {
    const { config, route } = config_route("bedrock_converse");
    if (route.cloud?.kind !== "bedrock") throw new Error("missing_route");
    const cloud = route.cloud;
    for (const guardrail of [
      { id: "guardrail123", version: "3" },
      { id: "guardrail123", version: "DRAFT", maximum_usd: "0.001" },
    ])
      expect(() =>
        validate_deployment({
          ...config,
          models: config.models.map((model) =>
            model === route ? { ...model, cloud: { ...cloud, guardrail } } : model,
          ),
        }),
      ).toThrow();
  });

  it("rejects unknown unary Bedrock content before SDK projection", async () => {
    fixture.state.reply = {
      output: { message: { role: "assistant", content: [{ text: "idea", futureOutput: true }] } },
      stopReason: "end_turn",
      usage: { inputTokens: 12, outputTokens: 5, totalTokens: 17 },
    };
    await expect(
      fixture
        .client()
        .invokeModel(test_request(fixture.host, "bedrock_converse"), { timeoutMs: 4500 }),
    ).rejects.toThrow();
    expect(fixture.requests).toHaveLength(1);
  });

  it.each([
    "truncated",
    "checksum",
    "oversize",
    "missing_usage",
    "duplicate_end",
    "unknown",
    "unknown_delta",
    "exception",
  ])("rejects Bedrock %s streams", async (mode) => {
    const events = bedrock_events();
    if (mode === "missing_usage") events.pop();
    if (mode === "duplicate_end") events.push({ messageStop: { stopReason: "end_turn" } });
    if (mode === "unknown") events.splice(1, 0, { newOutputType: {} });
    if (mode === "unknown_delta")
      events[1] = {
        contentBlockDelta: { contentBlockIndex: 0, delta: { text: "hidden", futureOutput: true } },
      };
    if (mode === "exception")
      events.splice(1, 0, { throttlingException: { message: TEST_SECRET } });
    let bytes = Buffer.concat(event_bytes(events));
    if (mode === "truncated") bytes = bytes.subarray(0, bytes.length - 1);
    if (mode === "checksum") bytes[bytes.length - 1] = (bytes.at(-1) ?? 0) ^ 1;
    if (mode === "oversize") {
      bytes = Buffer.alloc(4);
      bytes.writeUInt32BE(524_289);
    }
    fixture.state.events = [bytes];
    await expect(collect()).rejects.toThrow();
    expect(fixture.requests).toHaveLength(1);
  });

  it("cancels Bedrock transport and preserves ambiguous replay", async () => {
    fixture.state.events = event_bytes(bedrock_events());
    fixture.state.chunk_delay = 40;
    const command = test_request(fixture.host, "bedrock_converse");
    await expect(collect(command, AbortSignal.timeout(150))).rejects.toThrow();
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(fixture.state.stream_closed).toBe(true);
    await expect(collect(command)).rejects.toThrow();
    expect(fixture.requests).toHaveLength(1);
  });

  it("uses the AWS default chain with SigV4 despite ambient bearer configuration", async () => {
    const config = structuredClone(fixture.config);
    const route = config.models.find((model) => model.plugin === "bedrock_converse");
    if (route?.cloud?.kind !== "bedrock") throw new Error("missing_route");
    delete route.cloud.credentials;
    vi.stubEnv("AWS_ACCESS_KEY_ID", CLOUD_SECRETS.LOOP_LLM_AWS_ACCESS);
    vi.stubEnv("AWS_SECRET_ACCESS_KEY", CLOUD_SECRETS.LOOP_LLM_AWS_SECRET);
    vi.stubEnv("AWS_SESSION_TOKEN", CLOUD_SECRETS.LOOP_LLM_AWS_SESSION);
    vi.stubEnv("AWS_PROFILE", "");
    vi.stubEnv("AWS_BEARER_TOKEN_BEDROCK", "fixture-bearer-must-not-be-used");
    try {
      const host = new ProviderHost(
        config,
        new Uint8Array(32).fill(1),
        { LOOP_LLM_TEST: TEST_SECRET },
        fixture.fetcher,
        { google: async () => "fixture-token" },
      );
      fixture.state.reply = cloud_reply("bedrock_converse");
      await host.invoke(
        test_request(host, "bedrock_converse"),
        config.principals[0] ?? fixture.principal,
        new AbortController().signal,
      );
      expect(fixture.requests.at(-1)?.authorization).toMatch(/^AWS4-HMAC-SHA256 /);
    } finally {
      vi.unstubAllEnvs();
    }
  });

  it("never falls back from missing explicit AWS credentials", async () => {
    const host = new ProviderHost(
      fixture.config,
      new Uint8Array(32).fill(1),
      { LOOP_LLM_TEST: TEST_SECRET },
      fixture.fetcher,
      { google: async () => "fixture-token" },
    );
    await expect(
      host.invoke(
        test_request(host, "bedrock_converse"),
        fixture.principal,
        new AbortController().signal,
      ),
    ).rejects.toThrow("cloud_identity_unavailable");
    expect(fixture.requests).toHaveLength(0);
  });
});
