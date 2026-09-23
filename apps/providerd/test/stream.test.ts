import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { create } from "@bufbuild/protobuf";
import { Code, ConnectError } from "@connectrpc/connect";
import {
  ModelFinishReason,
  type ModelStreamEvent,
  StreamModelRequestSchema,
  StructuredOutputDefinitionSchema,
} from "@loop-engine/protocol/provider";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { stream_body } from "../src/stream.js";
import { TEST_SECRET, test_request } from "./fixture.js";
import {
  reasoning_events,
  rich_fixture,
  sse_bytes,
  TEST_SCHEMA,
  text_events,
  tool_events,
  tool_request,
} from "./rich-fixture.js";

let directory: string;
let fixture: Awaited<ReturnType<typeof rich_fixture>>;
let thinking: Awaited<ReturnType<typeof rich_fixture>>;
beforeAll(async () => {
  directory = await mkdtemp(join(tmpdir(), "loop-provider-stream-"));
  fixture = await rich_fixture(directory);
  const private_path = join(directory, "thinking");
  await mkdir(private_path, { mode: 0o700 });
  thinking = await rich_fixture(private_path, (config) => {
    for (const model of config.models) {
      if (model.plugin === "anthropic") model.reasoning = "adaptive";
      else if (model.plugin === "openai_responses") model.reasoning = "medium";
    }
  });
});
afterAll(async () => {
  await fixture?.close();
  await thinking?.close();
  if (directory) await rm(directory, { recursive: true, force: true });
});
beforeEach(() => {
  fixture.requests.length = 0;
  fixture.state.events = undefined;
  fixture.state.chunk_delay = 0;
  fixture.state.stream_closed = false;
});

async function collect(
  command: ReturnType<typeof test_request>,
  signal?: AbortSignal,
  source = fixture,
) {
  const output: ModelStreamEvent[] = [];
  const request = create(StreamModelRequestSchema, {
    context: command.context,
    invocation: command.invocation,
  });
  for await (const result of source.client().streamModel(request, { timeoutMs: 4500, signal })) {
    if (!result.event) throw new Error("missing_event");
    output.push(result.event);
  }
  return output;
}

describe("native ordered streaming", () => {
  it("bounds reads for slow consumers and cancels the source", async () => {
    let reads = 0;
    let cancelled = false;
    const source = new ReadableStream<Uint8Array>(
      {
        pull(controller) {
          reads++;
          controller.enqueue(Buffer.from(`data: {"part":${reads}}\n\n`));
        },
        cancel() {
          cancelled = true;
        },
      },
      { highWaterMark: 0 },
    );
    const reader = stream_body(new Response(source), false).getReader();
    expect(reads).toBe(0);
    await reader.read();
    await new Promise((resolve) => setImmediate(resolve));
    expect(reads).toBe(1);
    await reader.read();
    expect(reads).toBe(2);
    await reader.cancel();
    expect(cancelled).toBe(true);
    reader.releaseLock();
  });
  it.each(["responses", "claude"])(
    "streams %s summaries without disclosing continuation bytes",
    async (model) => {
      thinking.state.events = sse_bytes(reasoning_events(model));
      const events = await collect(test_request(thinking.host, model), undefined, thinking);
      const summary = events.find(
        (event) =>
          event.event.case === "contentDelta" && event.event.value.delta.case === "reasoning",
      );
      expect(summary).toMatchObject({
        event: {
          case: "contentDelta",
          value: { delta: { case: "reasoning", value: { text: "public summary" } } },
        },
      });
      const final = events.at(-1)?.event;
      expect(final).toMatchObject({
        case: "completed",
        value: {
          response: {
            content: [
              {
                content: {
                  case: "reasoning",
                  value: {
                    text: "public summary",
                    continuation: {
                      providerId: { value: model === "claude" ? "anthropic" : "openai" },
                    },
                  },
                },
              },
              { content: { case: "text" } },
            ],
          },
        },
      });
      expect(
        JSON.stringify(events, (_key, value) =>
          typeof value === "bigint" ? value.toString() : value,
        ),
      ).not.toContain("private-");
    },
  );
  it.each(["responses", "chat", "claude"])(
    "streams %s text through TLS and replays without another charge",
    async (model) => {
      fixture.state.events = sse_bytes(text_events(model), model === "chat");
      const command = test_request(fixture.host, model);
      const events = await collect(command);
      expect(events.map((event) => event.sequence)).toEqual(
        events.map((_, index) => BigInt(index + 1)),
      );
      expect(events.map((event) => event.event.case)).toEqual([
        "started",
        "contentDelta",
        "usageUpdate",
        "completed",
      ]);
      const final = events.at(-1)?.event;
      expect(final).toMatchObject({
        case: "completed",
        value: {
          response: {
            finishReason: ModelFinishReason.STOP,
            content: [{ content: { case: "text", value: { text: "研究 idea" } } }],
            usage: { inputTokens: 12n, outputTokens: 5n },
          },
        },
      });
      expect((await collect(command)).map((event) => event.event.case)).toEqual([
        "started",
        "usageUpdate",
        "completed",
      ]);
      expect(fixture.requests).toHaveLength(2);
      expect(fixture.requests.at(-1)?.body.stream).toBe(true);
    },
  );

  it.each(["responses", "chat", "claude"])(
    "validates interleaved %s tool argument fragments",
    async (model) => {
      fixture.state.events = sse_bytes(tool_events(model, 2), model === "chat");
      const events = await collect(tool_request(fixture, model));
      const final = events.at(-1)?.event;
      if (final?.case !== "completed" || !final.value.response)
        throw new Error("missing_completion");
      expect(final.value.response.finishReason).toBe(ModelFinishReason.TOOL_CALL);
      expect(final.value.response.content).toHaveLength(2);
      for (const block of final.value.response.content) {
        if (block.content.case !== "toolCall") throw new Error("missing_tool_call");
        expect(Buffer.from(block.content.value.arguments?.utf8Json ?? []).toString()).toBe(
          '{"window":20}',
        );
      }
      expect(
        new Set(
          events.flatMap((event) =>
            event.event.case === "contentDelta" ? [event.event.value.contentIndex] : [],
          ),
        ),
      ).toEqual(new Set([0, 1]));
    },
  );

  it.each(["responses", "chat", "claude"])(
    "rejects %s truncated completion and fences replay",
    async (model) => {
      const all = text_events(model);
      // Chat still has finish and usage; absence of DONE must remain a failure.
      fixture.state.events = sse_bytes(model === "chat" ? all : all.slice(0, -1));
      const command = test_request(fixture.host, model);
      await expect(collect(command)).rejects.toBeInstanceOf(ConnectError);
      const requests = fixture.requests.length;
      fixture.state.events = sse_bytes(all, model === "chat");
      await expect(collect(command)).rejects.toThrow("invocation_ambiguous");
      expect(fixture.requests).toHaveLength(requests);
    },
  );

  it.each(["responses", "chat", "claude"])(
    "rejects content after the %s terminal",
    async (model) => {
      const chunks = sse_bytes(text_events(model), model === "chat");
      fixture.state.events = [
        ...chunks,
        ...sse_bytes([{ type: "error", error: { message: TEST_SECRET } }]),
      ];
      try {
        await collect(test_request(fixture.host, model));
        throw new Error("unexpected_success");
      } catch (error) {
        expect(error).toBeInstanceOf(ConnectError);
        expect(ConnectError.from(error).rawMessage).not.toContain(TEST_SECRET);
      }
    },
  );

  it("rejects missing and duplicated Responses sequence numbers", async () => {
    for (const sequence_number of [0, 2]) {
      const events = JSON.parse(JSON.stringify(text_events("responses")));
      events[1].sequence_number = sequence_number;
      fixture.state.events = sse_bytes(events);
      await expect(collect(test_request(fixture.host))).rejects.toThrow("invalid_provider_stream");
    }
  });

  it("rejects a final response that disagrees with emitted text", async () => {
    const events = JSON.parse(JSON.stringify(text_events("responses")));
    events.at(-1).response.output[0].content[0].text = "different result";
    fixture.state.events = sse_bytes(events);
    await expect(collect(test_request(fixture.host))).rejects.toThrow("invalid_provider_stream");
  });

  it.each(["responses", "chat", "claude"])(
    "validates %s structured JSON only after completion",
    async (model) => {
      fixture.state.events = sse_bytes(text_events(model, '{"window":20}'), model === "chat");
      const command = test_request(fixture.host, model);
      if (!command.invocation) throw new Error("missing_invocation");
      command.invocation.structuredOutput = create(StructuredOutputDefinitionSchema, {
        name: "window",
        jsonSchema: TEST_SCHEMA,
        strict: true,
      });
      const final = (await collect(command)).at(-1)?.event;
      expect(final).toMatchObject({
        case: "completed",
        value: { response: { content: [{ content: { case: "structuredOutput" } }] } },
      });
    },
  );

  it.each(["responses", "chat", "claude"])("cancels %s and fences retries", async (model) => {
    fixture.state.events = sse_bytes(text_events(model), model === "chat");
    fixture.state.chunk_delay = 10;
    const command = test_request(fixture.host, model);
    const controller = new AbortController();
    const request = create(StreamModelRequestSchema, {
      context: command.context,
      invocation: command.invocation,
    });
    const output: string[] = [];
    try {
      for await (const item of fixture
        .client()
        .streamModel(request, { timeoutMs: 4500, signal: controller.signal })) {
        output.push(item.event?.event.case ?? "");
        if (item.event?.event.case === "contentDelta") controller.abort();
      }
      throw new Error("unexpected_success");
    } catch (error) {
      expect(ConnectError.from(error).code).toBe(Code.Canceled);
    }
    expect(output).not.toContain("completed");
    await expect(collect(command)).rejects.toThrow("invocation_ambiguous");
    expect(fixture.requests).toHaveLength(2);
  });

  it.each(["responses", "chat", "claude"])("rejects duplicate %s completion", async (model) => {
    const events = text_events(model);
    fixture.state.events = sse_bytes([...events, events.at(-1)], model === "chat");
    await expect(collect(test_request(fixture.host, model))).rejects.toThrow();
  });

  it("preserves Claude's terminal refusal classification", async () => {
    const events = JSON.parse(JSON.stringify(text_events("claude", "Cannot complete request")));
    events.at(-2).delta.stop_reason = "refusal";
    fixture.state.events = sse_bytes(events);
    const final = (await collect(test_request(fixture.host, "claude"))).at(-1)?.event;
    expect(final).toMatchObject({
      case: "completed",
      value: {
        response: {
          finishReason: ModelFinishReason.CONTENT_FILTER,
          content: [{ content: { case: "refusal", value: { reason: "Cannot complete request" } } }],
        },
      },
    });
  });

  it.each(["responses", "chat", "claude"])("rejects %s missing usage", async (model) => {
    const events = JSON.parse(JSON.stringify(text_events(model)));
    if (model === "chat") events.pop();
    else if (model === "claude") delete events.at(-2).usage;
    else delete events.at(-1).response.usage;
    fixture.state.events = sse_bytes(events, model === "chat");
    await expect(collect(test_request(fixture.host, model))).rejects.toThrow();
  });

  it.each(["responses", "chat", "claude"])("rejects %s usage overruns", async (model) => {
    const events = JSON.parse(JSON.stringify(text_events(model)));
    if (model === "chat") events.at(-1).usage.completion_tokens = 500;
    else if (model === "claude") events.at(-2).usage.output_tokens = 500;
    else events.at(-1).response.usage.output_tokens = 500;
    fixture.state.events = sse_bytes(events, model === "chat");
    await expect(collect(test_request(fixture.host, model))).rejects.toThrow(
      "provider_usage_exceeded",
    );
  });

  it.each(["responses", "chat", "claude"])("rejects %s incomplete tool JSON", async (model) => {
    const events: unknown[] = JSON.parse(JSON.stringify(tool_events(model)));
    // Keep native lifecycle and final argument text consistent; only the JSON is broken.
    const bytes = sse_bytes(events, model === "chat");
    const malformed = Buffer.concat(bytes).toString().replaceAll("20}", "20");
    fixture.state.events = [Buffer.from(malformed)];
    await expect(collect(tool_request(fixture, model))).rejects.toMatchObject({
      code: Code.DataLoss,
    });
  });

  it.each([false, true])("keeps Chat tool/text order with trailing text: %s", async (text) => {
    const events = JSON.parse(JSON.stringify(tool_events("chat")));
    // An empty role/content chunk does not allocate a phantom text block.
    events[0].choices[0].delta.content = "";
    if (text)
      events.splice(1, 0, {
        id: "c1",
        model: "chat-fixture-20260901",
        choices: [{ index: 0, delta: { content: "proposal" }, finish_reason: null }],
      });
    fixture.state.events = sse_bytes(events, true);
    const final = (await collect(tool_request(fixture, "chat"))).at(-1)?.event;
    if (final?.case !== "completed") throw new Error("missing_completion");
    expect(final.value.response?.content.map((block) => block.content.case)).toEqual(
      text ? ["toolCall", "text"] : ["toolCall"],
    );
  });
});
