import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { create } from "@bufbuild/protobuf";
import { Code, ConnectError } from "@connectrpc/connect";
import {
  ModelFinishReason,
  ModelMessageSchema,
  ModelRole,
  type ModelStreamEvent,
  StreamModelRequestSchema,
  StructuredOutputDefinitionSchema,
  ToolChoiceMode,
  ToolResultStatus,
} from "@loop-engine/protocol/provider";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import {
  ADDITIONAL,
  additional_events,
  additional_fixture,
  additional_reply,
} from "./additional-fixture.js";
import { TEST_SECRET, test_request } from "./fixture.js";
import { sse_bytes, TEST_SCHEMA, tool_request } from "./rich-fixture.js";

let directory: string;
let fixture: Awaited<ReturnType<typeof additional_fixture>>;
let thinking: Awaited<ReturnType<typeof additional_fixture>>;
beforeAll(async () => {
  directory = await mkdtemp(join(tmpdir(), "loop-provider-additional-"));
  fixture = await additional_fixture(directory);
  await mkdir(join(directory, "thinking"), { mode: 0o700 });
  thinking = await additional_fixture(join(directory, "thinking"), (config) => {
    for (const model of config.models)
      if (model.plugin.startsWith("google")) model.reasoning = "low";
      else {
        model.reasoning = "enabled";
        model.thinking_tokens = 1024;
        model.output_tokens = 2048;
      }
    config.policy.output_tokens = 2048;
  });
});
afterAll(async () => {
  await fixture?.close();
  await thinking?.close();
  if (directory) await rm(directory, { recursive: true, force: true });
});
beforeEach(() => {
  for (const source of [fixture, thinking]) {
    source.requests.length = 0;
    source.state.reply = undefined;
    source.state.body = undefined;
    source.state.status = 200;
    source.state.count = undefined;
    source.state.events = undefined;
    source.state.chunk_delay = 0;
    source.state.stream_closed = false;
  }
});

async function invoke(command: ReturnType<typeof test_request>, source = fixture) {
  return (await source.client().invokeModel(command, { timeoutMs: 4500 })).response;
}
async function collect(
  command: ReturnType<typeof test_request>,
  signal?: AbortSignal,
  source = fixture,
) {
  const events: ModelStreamEvent[] = [];
  for await (const result of source.client().streamModel(
    create(StreamModelRequestSchema, {
      context: command.context,
      invocation: command.invocation,
    }),
    { timeoutMs: 4500, signal },
  )) {
    if (!result.event) throw new Error("missing_event");
    events.push(result.event);
  }
  return events;
}

describe("additional native protocols", () => {
  it.each(ADDITIONAL)(
    "invokes %s with native authentication and measured usage",
    async (plugin) => {
      const result = await invoke(test_request(fixture.host, plugin));
      expect(result).toMatchObject({
        finishReason: ModelFinishReason.STOP,
        usage: { inputTokens: 12n, outputTokens: 5n, cachedInputTokens: 3n },
        content: [{ content: { case: "text", value: { text: "diagnostic idea" } } }],
      });
      expect(result?.usage?.chargedCost).toBeUndefined();
      const sent = fixture.requests.at(-1);
      expect(sent?.path).toContain(
        plugin === "cohere"
          ? "/v2/chat"
          : plugin === "google_generate"
            ? ":generateContent"
            : "/interactions",
      );
      expect(plugin === "cohere" ? sent?.authorization : sent?.google_key).toBe(
        plugin === "cohere" ? `Bearer ${TEST_SECRET}` : TEST_SECRET,
      );
      expect(fixture.seen_urls.at(-1)).toBe(
        plugin === "cohere"
          ? "https://api.cohere.com"
          : "https://generativelanguage.googleapis.com",
      );
      if (plugin === "google_generate") {
        expect(fixture.requests[0]?.body).toMatchObject({
          generateContentRequest: {
            model: `models/${plugin}-fixture-20260901`,
            systemInstruction: { parts: [{ text: "Propose a research idea." }] },
            contents: [{ role: "user" }],
            generationConfig: { maxOutputTokens: 64 },
          },
        });
        expect(fixture.requests[0]?.body.contents).toBeUndefined();
      } else if (plugin === "google_interactions") {
        expect(sent?.body).toMatchObject({
          store: false,
          background: false,
          input: [{ type: "user_input" }],
          generation_config: { max_output_tokens: 64 },
        });
        expect(sent?.body.previous_interaction_id).toBeUndefined();
      } else expect(sent?.body.max_tokens).toBe(64);
    },
  );

  it.each(ADDITIONAL)("round trips %s tools through the same service", async (plugin) => {
    fixture.state.reply = additional_reply(plugin, true);
    const response = await invoke(tool_request(fixture, plugin));
    expect(response?.finishReason).toBe(ModelFinishReason.TOOL_CALL);
    const call = response?.content.find((block) => block.content.case === "toolCall");
    if (call?.content.case !== "toolCall") throw new Error("missing_call");
    const next = tool_request(fixture, plugin);
    next.invocation?.messages.push(
      create(ModelMessageSchema, { role: ModelRole.ASSISTANT, content: response?.content }),
      create(ModelMessageSchema, {
        role: ModelRole.TOOL,
        content: [
          {
            content: {
              case: "toolResult",
              value: {
                toolCallId: call.content.value.toolCallId,
                status: ToolResultStatus.ERROR,
                result: { case: "text", value: { text: "worker unavailable" } },
              },
            },
          },
        ],
      }),
    );
    fixture.state.reply = undefined;
    expect((await invoke(next))?.finishReason).toBe(ModelFinishReason.STOP);
    expect(JSON.stringify(fixture.requests.at(-1)?.body)).toContain("worker unavailable");
    if (plugin === "google_generate") {
      expect(fixture.requests[0]?.body).toMatchObject({
        generateContentRequest: { tools: [{ functionDeclarations: [{ name: "propose_factor" }] }] },
      });
      // The local correlation ID is not invented as a native Gemini call ID.
      expect(JSON.stringify(fixture.requests.at(-1)?.body)).not.toContain("gemini-local-");
    }
  });

  it.each(ADDITIONAL)("checks %s registered structured output", async (plugin) => {
    const command = test_request(fixture.host, plugin);
    if (!command.invocation) throw new Error("missing_invocation");
    command.invocation.structuredOutput = create(StructuredOutputDefinitionSchema, {
      name: "window",
      strict: true,
      jsonSchema: TEST_SCHEMA,
    });
    fixture.state.reply = JSON.parse(
      JSON.stringify(additional_reply(plugin)).replaceAll("diagnostic idea", '{\\"window\\":20}'),
    );
    expect((await invoke(command))?.content[0]?.content.case).toBe("structuredOutput");
    const body = fixture.requests.at(-1)?.body;
    if (plugin === "cohere")
      expect(body).toMatchObject({
        response_format: { type: "json_object", json_schema: { type: "object" } },
      });
    else if (plugin === "google_interactions")
      expect(body).toMatchObject({
        response_format: {
          type: "text",
          mime_type: "application/json",
          schema: { type: "object" },
        },
      });
    else
      expect(body).toMatchObject({
        generationConfig: {
          responseMimeType: "application/json",
          responseJsonSchema: { type: "object" },
        },
      });
  });

  it.each(ADDITIONAL)("streams %s text incrementally and replays the receipt", async (plugin) => {
    fixture.state.events = sse_bytes(additional_events(plugin));
    const command = test_request(fixture.host, plugin);
    const events = await collect(command);
    expect(events.map((event) => event.sequence)).toEqual(
      events.map((_, index) => BigInt(index + 1)),
    );
    expect(
      events.filter((event) => event.event.case === "contentDelta").length,
    ).toBeGreaterThanOrEqual(2);
    expect(events.at(-1)?.event).toMatchObject({
      case: "completed",
      value: {
        response: {
          finishReason: ModelFinishReason.STOP,
          content: [{ content: { case: "text", value: { text: "diagnostic idea" } } }],
        },
      },
    });
    const before = fixture.requests.length;
    expect((await collect(command)).at(-1)?.event).toEqual(events.at(-1)?.event);
    expect(fixture.requests).toHaveLength(before);
  });

  it.each(ADDITIONAL)("streams %s native tool calls", async (plugin) => {
    fixture.state.events = sse_bytes(additional_events(plugin, true));
    const events = await collect(tool_request(fixture, plugin));
    expect(events.at(-1)?.event).toMatchObject({
      case: "completed",
      value: { response: { finishReason: ModelFinishReason.TOOL_CALL } },
    });
    expect(
      events.some(
        (event) =>
          event.event.case === "contentDelta" && event.event.value.delta.case === "toolCall",
      ),
    ).toBe(true);
  });

  it.each(ADDITIONAL)("rejects a truncated %s stream without a receipt", async (plugin) => {
    fixture.state.events = sse_bytes(additional_events(plugin).slice(0, -1));
    const command = test_request(fixture.host, plugin);
    await expect(collect(command)).rejects.toMatchObject({ code: Code.DataLoss });
    const before = fixture.requests.length;
    await expect(collect(command)).rejects.toThrow("invocation_ambiguous");
    expect(fixture.requests).toHaveLength(before);
  });

  it.each(ADDITIONAL)("rejects trailing %s events", async (plugin) => {
    const events = additional_events(plugin);
    fixture.state.events = sse_bytes([...events, events[0]]);
    await expect(collect(test_request(fixture.host, plugin))).rejects.toMatchObject({
      code: Code.DataLoss,
    });
  });

  it.each(ADDITIONAL)("cancels %s after a preview and fences resubmission", async (plugin) => {
    fixture.state.events = sse_bytes(additional_events(plugin));
    fixture.state.chunk_delay = 10;
    const command = test_request(fixture.host, plugin);
    const controller = new AbortController();
    const output: string[] = [];
    try {
      for await (const result of fixture.client().streamModel(
        create(StreamModelRequestSchema, {
          context: command.context,
          invocation: command.invocation,
        }),
        { signal: controller.signal, timeoutMs: 4500 },
      )) {
        output.push(result.event?.event.case ?? "");
        if (result.event?.event.case === "contentDelta") controller.abort();
      }
      throw new Error("unexpected_success");
    } catch (error) {
      expect(ConnectError.from(error).code).toBe(Code.Canceled);
    }
    expect(output).not.toContain("completed");
    const before = fixture.requests.length;
    await expect(collect(command)).rejects.toThrow("invocation_ambiguous");
    expect(fixture.requests).toHaveLength(before);
  });

  it.each(ADDITIONAL)("rejects %s missing usage", async (plugin) => {
    const reply = additional_reply(plugin);
    if (plugin === "google_generate") delete reply.usageMetadata;
    else delete reply.usage;
    fixture.state.reply = reply;
    await expect(invoke(test_request(fixture.host, plugin))).rejects.toMatchObject({
      code: Code.DataLoss,
    });
  });

  it.each(ADDITIONAL)("rejects %s usage above the caller budget", async (plugin) => {
    const reply = additional_reply(plugin);
    if (reply.usageMetadata) {
      reply.usageMetadata.candidatesTokenCount = 65;
      reply.usageMetadata.totalTokenCount = 77;
    } else if (reply.usage?.tokens) reply.usage.tokens.output_tokens = 65;
    else if (reply.usage) {
      reply.usage.total_output_tokens = 65;
      reply.usage.total_tokens = 77;
    }
    fixture.state.reply = reply;
    await expect(invoke(test_request(fixture.host, plugin))).rejects.toThrow(
      "provider_usage_exceeded",
    );
  });

  it.each(["google_generate", "google_interactions"] as const)(
    "rejects %s model drift",
    async (plugin) => {
      const reply = additional_reply(plugin);
      if (reply.modelVersion) reply.modelVersion = "different-model";
      else if (reply.model) reply.model = "different-model";
      fixture.state.reply = reply;
      await expect(invoke(test_request(fixture.host, plugin))).rejects.toThrow(
        "invalid_google_output",
      );
    },
  );

  it.each(ADDITIONAL)("redacts %s rate limits and never retries", async (plugin) => {
    fixture.state.status = 429;
    fixture.state.body = {
      message: TEST_SECRET,
      error: { code: 429, message: TEST_SECRET, status: "RESOURCE_EXHAUSTED" },
    };
    await expect(invoke(test_request(fixture.host, plugin))).rejects.toMatchObject({
      code: Code.ResourceExhausted,
      rawMessage: "provider_rate_limited",
    });
    expect(fixture.requests).toHaveLength(1);
  });

  it.each(["google_interactions", "cohere"] as const)(
    "requires the full %s input ceiling before spending",
    async (plugin) => {
      const command = test_request(fixture.host, plugin);
      if (!command.invocation?.budget) throw new Error("missing_budget");
      command.invocation.budget.maximumInputTokens = 127n;
      await expect(invoke(command)).rejects.toThrow("provider_input_budget");
      expect(fixture.requests).toHaveLength(0);
    },
  );

  it("rejects unsupported Cohere combinations before the vendor call", async () => {
    const command = tool_request(fixture, "cohere");
    if (!command.invocation) throw new Error("missing_invocation");
    command.invocation.toolChoice = {
      $typeName: "loop.v1.ToolChoice",
      mode: ToolChoiceMode.NAMED,
      namedTool: "propose_factor",
    };
    await expect(invoke(command)).rejects.toThrow("cohere_capability_denied");
    expect(fixture.requests).toHaveLength(0);
  });

  it.each(["google_generate", "google_interactions"] as const)(
    "preserves %s signed tool continuation privately",
    async (plugin) => {
      thinking.state.reply = additional_reply(plugin, true, true);
      const result = await invoke(tool_request(thinking, plugin), thinking);
      expect(result?.usage).toMatchObject({ outputTokens: 8n, reasoningTokens: 3n });
      expect(
        JSON.stringify(result, (_key, value) =>
          typeof value === "bigint" ? value.toString() : value,
        ),
      ).not.toContain("signed-thinking");
      const call = result?.content.find((block) => block.content.case === "toolCall");
      if (call?.content.case !== "toolCall") throw new Error("missing_call");
      const next = tool_request(thinking, plugin);
      next.invocation?.messages.push(
        create(ModelMessageSchema, { role: ModelRole.ASSISTANT, content: result?.content }),
        create(ModelMessageSchema, {
          role: ModelRole.TOOL,
          content: [
            {
              content: {
                case: "toolResult",
                value: {
                  toolCallId: call.content.value.toolCallId,
                  status: ToolResultStatus.SUCCESS,
                  result: { case: "text", value: { text: "accepted" } },
                },
              },
            },
          ],
        }),
      );
      thinking.state.reply = additional_reply(plugin, false, true);
      expect((await invoke(next, thinking))?.finishReason).toBe(ModelFinishReason.STOP);
      expect(JSON.stringify(thinking.requests.at(-1)?.body)).toContain("signed-thinking");
    },
  );

  it("supports Cohere thinking without fabricating a separate token count", async () => {
    thinking.state.reply = {
      ...additional_reply("cohere"),
      message: {
        role: "assistant",
        content: [
          { type: "thinking", thinking: "reasoning fixture" },
          { type: "text", text: "idea" },
        ],
      },
    };
    const command = test_request(thinking.host, "cohere");
    if (!command.invocation?.budget?.maximumCost?.amount) throw new Error("missing_budget");
    command.invocation.budget.maximumOutputTokens = 1536n;
    command.invocation.budget.maximumCost.amount.value = "0.01";
    const result = await invoke(command, thinking);
    expect(result?.content[0]?.content.case).toBe("reasoning");
    expect(result?.usage).toMatchObject({ outputTokens: 5n, reasoningTokens: 0n });
    expect(thinking.requests.at(-1)?.body.thinking).toEqual({
      type: "enabled",
      token_budget: 1024,
    });
    const next = test_request(thinking.host, "cohere");
    if (!next.invocation) throw new Error("missing_invocation");
    next.invocation.budget = command.invocation.budget;
    next.invocation.messages.push(
      create(ModelMessageSchema, { role: ModelRole.ASSISTANT, content: result?.content }),
      create(ModelMessageSchema, {
        role: ModelRole.USER,
        content: [{ content: { case: "text", value: { text: "continue" } } }],
      }),
    );
    await invoke(next, thinking);
    expect(JSON.stringify(thinking.requests.at(-1)?.body.messages)).toContain(
      '"thinking":"reasoning fixture"',
    );
  });
});
