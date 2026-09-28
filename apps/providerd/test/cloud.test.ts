import { createHash, createHmac } from "node:crypto";
import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { clone, create } from "@bufbuild/protobuf";
import {
  ModelFinishReason,
  ModelMessageSchema,
  ModelResolutionSnapshotSchema,
  ModelRole,
  type ModelStreamEvent,
  StreamModelRequestSchema,
  StructuredOutputDefinitionSchema,
  ToolResultStatus,
} from "@loop-engine/protocol/provider";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { additional_events } from "./additional-fixture.js";
import {
  bedrock_events,
  CLOUD_SCHEMA,
  CLOUD_SECRETS,
  CLOUDS,
  type CloudRoute,
  cloud_fixture,
  cloud_reply,
  event_bytes,
} from "./cloud-fixture.js";
import { TEST_SECRET, test_request } from "./fixture.js";
import { sse_bytes, text_events, tool_request } from "./rich-fixture.js";

let directory: string;
let fixture: Awaited<ReturnType<typeof cloud_fixture>>;
let identity: Awaited<ReturnType<typeof cloud_fixture>>;
let auth_failure = false;
let auth_delay = 0;
beforeAll(async () => {
  directory = await mkdtemp(join(tmpdir(), "loop-provider-cloud-"));
  fixture = await cloud_fixture(directory);
  await mkdir(join(directory, "identity"), { mode: 0o700 });
  identity = await cloud_fixture(
    join(directory, "identity"),
    (config) => {
      for (const model of config.models) {
        if (model.cloud?.kind === "azure") {
          model.cloud.auth = "entra";
          delete model.secret_env;
        }
        if (model.cloud?.kind === "vertex") model.cloud.location = "global";
      }
    },
    { azure: resolve_token, google: resolve_token },
  );
}, 30_000);
afterAll(async () => {
  await fixture?.close();
  await identity?.close();
  if (directory) await rm(directory, { recursive: true, force: true });
});
beforeEach(() => {
  auth_failure = false;
  auth_delay = 0;
  for (const source of [fixture, identity]) {
    source.requests.length = 0;
    source.state.status = 200;
    source.state.reply = undefined;
    source.state.body = undefined;
    source.state.events = undefined;
    source.state.delay = 0;
    source.state.chunk_delay = 0;
    source.state.stream_closed = false;
  }
});

async function resolve_token() {
  if (auth_delay) await new Promise((resolve) => setTimeout(resolve, auth_delay));
  if (auth_failure) throw new Error(TEST_SECRET);
  return "fixture-cloud-access-token";
}
function request(plugin: CloudRoute, tools = false, source = fixture) {
  const command = tools ? tool_request(source, plugin) : test_request(source.host, plugin);
  if (command.invocation?.tools[0]) command.invocation.tools[0].inputSchema = CLOUD_SCHEMA;
  return command;
}
async function invoke(command: ReturnType<typeof request>, source = fixture) {
  return (await source.client().invokeModel(command, { timeoutMs: 4500 })).response;
}
async function collect(
  command: ReturnType<typeof request>,
  signal?: AbortSignal,
  source = fixture,
) {
  const events: ModelStreamEvent[] = [];
  for await (const response of source.client().streamModel(
    create(StreamModelRequestSchema, {
      context: command.context,
      invocation: command.invocation,
    }),
    { timeoutMs: 4500, signal },
  )) {
    if (!response.event) throw new Error("missing_event");
    events.push(response.event);
  }
  return events;
}
function digest(text: string) {
  return createHash("sha256").update(text).digest("hex");
}
function sign(key: string | Uint8Array, text: string) {
  return createHmac("sha256", key).update(text).digest();
}

describe("cloud deployments", () => {
  it.each(CLOUDS)("invokes %s through the authenticated service", async (plugin) => {
    fixture.state.reply = cloud_reply(plugin);
    const result = await invoke(request(plugin));
    expect(result).toMatchObject({
      finishReason: ModelFinishReason.STOP,
      usage: {
        inputTokens: 12n,
        outputTokens: 5n,
        cachedInputTokens: plugin === "vertex_generate" ? 0n : 3n,
      },
    });
    expect(result?.usage?.chargedCost).toBeUndefined();
    expect(fixture.requests).toHaveLength(1);
    const sent = fixture.requests[0];
    if (plugin.startsWith("azure")) {
      expect(sent?.body.model).toBe(`deployment-${plugin}`);
      expect(sent?.headers["api-key"]).toBe(TEST_SECRET);
      expect(sent?.authorization).toBeUndefined();
      expect(sent?.path).toBe(
        `/openai/v1/${plugin === "azure_chat" ? "chat/completions" : "responses"}`,
      );
      expect(fixture.seen_urls.at(-1)).toBe("https://loop-test.openai.azure.com");
    } else if (plugin === "vertex_generate") {
      expect(sent?.path).toBe(
        "/v1/projects/loop-fixture/locations/us-central1/publishers/google/models/google_generate-fixture-20260901:generateContent",
      );
      expect(sent?.authorization).toBe("Bearer fixture-google-token");
      expect(sent?.google_key).toBeUndefined();
      expect(fixture.seen_urls.at(-1)).toBe("https://us-central1-aiplatform.googleapis.com");
    } else {
      expect(sent?.path).toBe("/model/us.anthropic.claude-fixture-v1%3A0/converse");
      expect(sent?.body).toMatchObject({
        inferenceConfig: { maxTokens: 64 },
        guardrailConfig: {
          guardrailIdentifier: "guardrail123",
          guardrailVersion: "3",
          trace: "disabled",
        },
      });
      expect(sent?.authorization).toMatch(/^AWS4-HMAC-SHA256 /);
      expect(sent?.headers["x-amz-security-token"]).toBe(CLOUD_SECRETS.LOOP_LLM_AWS_SESSION);
    }
  });

  it("verifies the AWS signature against received bytes", async () => {
    fixture.state.reply = cloud_reply("bedrock_converse");
    await invoke(request("bedrock_converse"));
    const sent = fixture.requests[0];
    if (!sent) throw new Error("missing_request");
    const auth = sent.authorization ?? "";
    const match =
      /^AWS4-HMAC-SHA256 Credential=([^/]+)\/([^,]+), SignedHeaders=([^,]+), Signature=([a-f0-9]{64})$/.exec(
        auth,
      );
    expect(match).not.toBeNull();
    const [, key, scope, headers, signature] = match as RegExpExecArray;
    expect(key).toBe(CLOUD_SECRETS.LOOP_LLM_AWS_ACCESS);
    expect(scope).toMatch(/^[0-9]{8}\/us-east-1\/bedrock\/aws4_request$/);
    const signed = (headers ?? "").split(";");
    // The fixture redirects transport to localhost; the signed host remains the
    // pinned AWS destination, not that local HTTP listener's Host header.
    expect(fixture.seen_urls.at(-1)).toBe("https://bedrock-runtime.us-east-1.amazonaws.com");
    const canonical = signed
      .map(
        (name) =>
          `${name}:${String(
            name === "host" ? "bedrock-runtime.us-east-1.amazonaws.com" : sent.headers[name],
          )
            .trim()
            .replace(/\s+/g, " ")}\n`,
      )
      .join("");
    const path = sent.path
      .split("/")
      .map((part) => encodeURIComponent(part))
      .join("/");
    const body_hash = digest(sent.raw);
    expect(sent.headers["x-amz-content-sha256"]).toBe(body_hash);
    const canonical_request = ["POST", path, "", canonical, headers, body_hash].join("\n");
    const date = String(sent.headers["x-amz-date"]);
    const string_to_sign = ["AWS4-HMAC-SHA256", date, scope, digest(canonical_request)].join("\n");
    const day_key = sign(`AWS4${CLOUD_SECRETS.LOOP_LLM_AWS_SECRET}`, date.slice(0, 8));
    const signing_key = sign(sign(sign(day_key, "us-east-1"), "bedrock"), "aws4_request");
    expect(sign(signing_key, string_to_sign).toString("hex")).toBe(signature);
    expect(digest(`${sent.raw} `)).not.toBe(body_hash);
  });

  it.each(["azure_responses", "azure_chat", "vertex_generate"] as const)(
    "uses OAuth for %s",
    async (plugin) => {
      identity.state.reply = cloud_reply(plugin);
      await invoke(request(plugin, false, identity), identity);
      expect(identity.requests[0]?.authorization).toBe("Bearer fixture-cloud-access-token");
      expect(identity.requests[0]?.headers["api-key"]).toBeUndefined();
      if (plugin === "vertex_generate") {
        expect(identity.requests[0]?.path).toContain("/locations/global/");
        expect(identity.seen_urls.at(-1)).toBe("https://aiplatform.googleapis.com");
      }
    },
  );

  it.each(CLOUDS)("round trips %s tools", async (plugin) => {
    fixture.state.reply = cloud_reply(plugin, true);
    const result = await invoke(request(plugin, true));
    expect(result?.finishReason).toBe(ModelFinishReason.TOOL_CALL);
    const block = result?.content.find((item) => item.content.case === "toolCall");
    if (block?.content.case !== "toolCall") throw new Error("missing_tool");
    const next = request(plugin, true);
    next.invocation?.messages.push(
      create(ModelMessageSchema, { role: ModelRole.ASSISTANT, content: result?.content }),
      create(ModelMessageSchema, {
        role: ModelRole.TOOL,
        content: [
          {
            content: {
              case: "toolResult",
              value: {
                toolCallId: block.content.value.toolCallId,
                status: ToolResultStatus.ERROR,
                result: { case: "text", value: { text: "worker unavailable" } },
              },
            },
          },
        ],
      }),
    );
    fixture.state.reply = cloud_reply(plugin);
    expect((await invoke(next))?.finishReason).toBe(ModelFinishReason.STOP);
    expect(fixture.requests.at(-1)?.raw).toContain("worker unavailable");
  });

  it.each(CLOUDS)("checks %s structured output", async (plugin) => {
    const command = request(plugin);
    if (!command.invocation) throw new Error("missing_invocation");
    command.invocation.structuredOutput = create(StructuredOutputDefinitionSchema, {
      name: "window",
      strict: true,
      jsonSchema: CLOUD_SCHEMA,
    });
    fixture.state.reply = JSON.parse(
      JSON.stringify(cloud_reply(plugin)).replaceAll("diagnostic idea", '{\\"window\\":20}'),
    );
    const result = await invoke(command);
    expect(result?.content[0]?.content.case).toBe("structuredOutput");
    expect(fixture.requests[0]?.raw).toContain("window");
  });

  it.each(CLOUDS)("streams %s to a validated receipt", async (plugin) => {
    fixture.state.events =
      plugin === "bedrock_converse"
        ? event_bytes(bedrock_events())
        : plugin === "vertex_generate"
          ? sse_bytes(additional_events("google_generate"))
          : sse_bytes(
              text_events(plugin === "azure_chat" ? "chat" : "responses"),
              plugin === "azure_chat",
            );
    const events = await collect(request(plugin));
    expect(events.at(-1)?.event.case).toBe("completed");
    expect(events.filter((event) => event.event.case === "completed")).toHaveLength(1);
    expect(fixture.requests).toHaveLength(1);
  });

  it("preserves Bedrock tool JSON fragments", async () => {
    fixture.state.events = event_bytes(bedrock_events(true));
    const events = await collect(request("bedrock_converse", true));
    const last = events.at(-1)?.event;
    expect(last?.case).toBe("completed");
    if (last?.case === "completed")
      expect(last.value.response?.finishReason).toBe(ModelFinishReason.TOOL_CALL);
  });

  it.each(CLOUDS)("reserves %s input before dispatch", async (plugin) => {
    const command = request(plugin);
    if (!command.invocation?.budget) throw new Error("missing_budget");
    command.invocation.budget.maximumInputTokens = 127n;
    await expect(invoke(command)).rejects.toThrow();
    expect(fixture.requests).toHaveLength(0);
  });

  it.each(CLOUDS)("rejects %s missing usage", async (plugin) => {
    const reply = cloud_reply(plugin) as Record<string, unknown>;
    delete reply.usage;
    delete reply.usageMetadata;
    fixture.state.reply = reply;
    await expect(invoke(request(plugin))).rejects.toThrow();
    expect(fixture.requests).toHaveLength(1);
  });

  it.each(CLOUDS)("redacts %s failure without retry", async (plugin) => {
    fixture.state.status = 429;
    fixture.state.body = { message: TEST_SECRET, __type: "ThrottlingException" };
    await expect(invoke(request(plugin))).rejects.toThrow("provider_rate_limited");
    expect(fixture.requests).toHaveLength(1);
  });

  it.each(CLOUDS)("refuses %s redirects", async (plugin) => {
    fixture.state.status = 307;
    await expect(invoke(request(plugin))).rejects.toThrow();
    expect(fixture.requests).toHaveLength(1);
  });

  it.each(["azure_responses", "vertex_generate"] as const)(
    "denies %s identity failure before dispatch",
    async (plugin) => {
      auth_failure = true;
      await expect(invoke(request(plugin, false, identity), identity)).rejects.toThrow();
      expect(identity.requests).toHaveLength(0);
    },
  );

  it.each(["azure_responses", "vertex_generate"] as const)(
    "bounds %s identity resolution",
    async (plugin) => {
      auth_delay = 150;
      const command = request(plugin, false, identity);
      if (!command.invocation?.budget) throw new Error("missing_budget");
      command.invocation.budget.maximumWallTime = {
        $typeName: "google.protobuf.Duration",
        seconds: 0n,
        nanos: 100_000_000,
      };
      await expect(invoke(command, identity)).rejects.toThrow();
      await new Promise((resolve) => setTimeout(resolve, 200));
      expect(identity.requests).toHaveLength(0);
    },
  );

  it.each(["azure_responses", "azure_chat", "vertex_generate"] as const)(
    "rejects %s model drift",
    async (plugin) => {
      const reply = cloud_reply(plugin) as Record<string, unknown>;
      if (plugin === "vertex_generate") reply.modelVersion = "changed-model";
      else reply.model = `deployment-${plugin}`;
      fixture.state.reply = reply;
      await expect(invoke(request(plugin))).rejects.toThrow();
    },
  );

  it("rejects changed resolution pins without transport", async () => {
    const command = request("bedrock_converse");
    if (!command.invocation?.model) throw new Error("missing_model");
    command.invocation.model = clone(ModelResolutionSnapshotSchema, command.invocation.model);
    command.invocation.model.modelId = { $typeName: "loop.v1.ModelId", value: "other-model" };
    await expect(invoke(command)).rejects.toThrow();
    expect(fixture.requests).toHaveLength(0);
  });
});
