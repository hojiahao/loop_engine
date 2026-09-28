import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { create } from "@bufbuild/protobuf";
import {
  ModelFinishReason,
  ModelMessageSchema,
  ModelRole,
  StreamModelRequestSchema,
  StructuredOutputDefinitionSchema,
  ToolChoiceMode,
  ToolResultStatus,
} from "@loop-engine/protocol/provider";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { validate_deployment } from "../src/config.js";
import { ProviderHost } from "../src/host.js";
import { VENDOR_IDS, VENDORS, vendor_endpoint } from "../src/vendor-registry.js";
import { TEST_SECRET } from "./fixture.js";
import { sse_bytes, TEST_SCHEMA } from "./rich-fixture.js";
import {
  vendor_events,
  vendor_fixture,
  vendor_request,
  vendor_response,
} from "./vendor-fixture.js";

let directory: string;
let fixture: Awaited<ReturnType<typeof vendor_fixture>>;
beforeAll(async () => {
  directory = await mkdtemp(join(tmpdir(), "loop-provider-vendors-"));
  fixture = await vendor_fixture(directory);
}, 30_000);
afterAll(async () => {
  await fixture?.close();
  if (directory) await rm(directory, { recursive: true, force: true });
});
beforeEach(() => {
  fixture.requests.length = 0;
  fixture.state.status = 200;
  fixture.state.reply = undefined;
  fixture.state.body = undefined;
  fixture.state.events = undefined;
  fixture.state.delay = 0;
  fixture.state.chunk_delay = 0;
});
async function invoke(command: ReturnType<typeof vendor_request>) {
  return (await fixture.client().invokeModel(command, { timeoutMs: 4500 })).response;
}
async function collect(command: ReturnType<typeof vendor_request>, signal?: AbortSignal) {
  const events = [];
  for await (const result of fixture.client().streamModel(
    create(StreamModelRequestSchema, {
      context: command.context,
      invocation: command.invocation,
    }),
    { timeoutMs: 4500, signal },
  ))
    events.push(result.event);
  return events;
}

describe("first-class vendor plugins", () => {
  it.each(VENDOR_IDS)(
    "invokes %s with its origin, identity and native parameters",
    async (plugin) => {
      fixture.state.reply = vendor_response(plugin);
      const reply = await invoke(vendor_request(fixture, plugin));
      expect(reply?.finishReason).toBe(ModelFinishReason.STOP);
      expect(reply?.usage).toMatchObject({ inputTokens: 12n, outputTokens: 5n });
      expect(fixture.requests).toHaveLength(1);
      const sent = fixture.requests[0];
      const model = fixture.config.models.find((value) => value.id === plugin);
      if (!model) throw new Error("missing_model");
      expect(fixture.seen_urls.at(-1)).toBe(new URL(vendor_endpoint(model)).origin);
      if (plugin === "minimax") expect(sent?.key).toBe(TEST_SECRET);
      else expect(sent?.authorization).toBe(`Bearer ${TEST_SECRET}`);
      if (plugin === "deepseek") expect(sent?.body.thinking).toEqual({ type: "disabled" });
      if (plugin === "qwen")
        expect(sent?.body).toMatchObject({ enable_thinking: false, enable_search: false });
      if (plugin === "perplexity") {
        expect(sent?.path).toBe("/v1/sonar");
        expect(sent?.body.disable_search).toBe(true);
      }
      if (plugin === "glm")
        expect(sent?.body.thinking).toEqual({ type: "disabled", clear_thinking: false });
      if (VENDORS[plugin].wire === "chat") {
        expect(sent?.body.store).toBeUndefined();
        expect(sent?.body.prompt_cache_retention).toBeUndefined();
        expect(
          sent?.body[
            ["groq", "cerebras", "qwen"].includes(plugin) ? "max_completion_tokens" : "max_tokens"
          ],
        ).toBe(plugin === "qwen" ? 54 : 64);
      }
      expect(reply?.usage?.chargedCost).toBeUndefined();
    },
  );

  it.each(VENDOR_IDS)("streams %s with a complete usage receipt", async (plugin) => {
    fixture.state.events = sse_bytes(vendor_events(plugin), VENDORS[plugin].wire === "chat");
    const events = await collect(vendor_request(fixture, plugin));
    expect(events.at(-1)?.event.case).toBe("completed");
    expect(events.filter((event) => event?.event.case === "completed")).toHaveLength(1);
    expect(fixture.requests).toHaveLength(1);
  });

  it.each(VENDOR_IDS.filter((plugin) => VENDORS[plugin].tools))(
    "streams %s tool argument fragments",
    async (plugin) => {
      fixture.state.events = sse_bytes(
        vendor_events(plugin, false, true),
        VENDORS[plugin].wire === "chat",
      );
      const events = await collect(vendor_request(fixture, plugin, true));
      const final = events.at(-1)?.event;
      expect(final?.case).toBe("completed");
      if (final?.case !== "completed") throw new Error("missing_result");
      expect(final.value.response?.finishReason).toBe(ModelFinishReason.TOOL_CALL);
      expect(final.value.response?.content[0]?.content.case).toBe("toolCall");
    },
  );

  it.each(["deepseek", "xai", "minimax"] as const)(
    "cancels %s without automatic replay",
    async (plugin) => {
      fixture.state.events = sse_bytes(vendor_events(plugin), VENDORS[plugin].wire === "chat");
      fixture.state.chunk_delay = 10;
      const command = vendor_request(fixture, plugin);
      const controller = new AbortController();
      const receive = async () => {
        for await (const reply of fixture.client().streamModel(
          create(StreamModelRequestSchema, {
            context: command.context,
            invocation: command.invocation,
          }),
          { timeoutMs: 4500, signal: controller.signal },
        )) {
          if (reply.event?.event.case === "contentDelta") controller.abort();
        }
      };
      await expect(receive()).rejects.toThrow();
      await expect(invoke(command)).rejects.toThrow("invocation_ambiguous");
      expect(fixture.requests).toHaveLength(1);
    },
  );

  it.each(VENDOR_IDS.filter((plugin) => VENDORS[plugin].tools))(
    "round trips a %s tool result",
    async (plugin) => {
      fixture.state.reply = vendor_response(plugin, true);
      const response = await invoke(vendor_request(fixture, plugin, true));
      expect(response?.finishReason).toBe(ModelFinishReason.TOOL_CALL);
      const block = response?.content.find((item) => item.content.case === "toolCall");
      if (block?.content.case !== "toolCall") throw new Error("missing_tool");
      const next = vendor_request(fixture, plugin, true);
      next.invocation?.messages.push(
        create(ModelMessageSchema, { role: ModelRole.ASSISTANT, content: [block] }),
        create(ModelMessageSchema, {
          role: ModelRole.TOOL,
          content: [
            {
              content: {
                case: "toolResult",
                value: {
                  toolCallId: block.content.value.toolCallId,
                  status: ToolResultStatus.SUCCESS,
                  result: { case: "text", value: { text: "evaluation complete" } },
                },
              },
            },
          ],
        }),
      );
      fixture.state.reply = vendor_response(plugin);
      await invoke(next);
      expect(fixture.requests).toHaveLength(2);
      expect(JSON.stringify(fixture.requests.at(-1)?.body)).toContain("evaluation complete");
    },
  );

  it.each(VENDOR_IDS.filter((plugin) => VENDORS[plugin].schema))(
    "validates %s native schema output",
    async (plugin) => {
      const command = vendor_request(fixture, plugin);
      if (!command.invocation) throw new Error("missing_invocation");
      command.invocation.structuredOutput = create(StructuredOutputDefinitionSchema, {
        name: "factor_window",
        jsonSchema: TEST_SCHEMA,
        strict: true,
      });
      const reply = vendor_response(plugin);
      if (plugin === "xai")
        reply.output = [
          {
            type: "message",
            role: "assistant",
            content: [{ type: "output_text", text: '{"window":20}' }],
          },
        ];
      else {
        const choice = (reply.choices as { message: { content: string } }[])[0];
        if (!choice) throw new Error("missing_choice");
        choice.message.content = '{"window":20}';
      }
      fixture.state.reply = reply;
      expect((await invoke(command))?.content[0]?.content.case).toBe("structuredOutput");
      const sent = fixture.requests.at(-1)?.body;
      expect(JSON.stringify(sent)).toContain("json_schema");
    },
  );

  it.each(VENDOR_IDS)("redacts %s rate-limit errors without retry", async (plugin) => {
    fixture.state.status = 429;
    fixture.state.body = { error: { message: TEST_SECRET } };
    await expect(invoke(vendor_request(fixture, plugin))).rejects.toThrow("provider_rate_limited");
    expect(fixture.requests).toHaveLength(1);
  });

  it.each(VENDOR_IDS)("rejects missing %s usage", async (plugin) => {
    const reply = vendor_response(plugin);
    delete reply.usage;
    fixture.state.reply = reply;
    await expect(invoke(vendor_request(fixture, plugin))).rejects.toThrow();
  });

  it.each(["mistral", "deepseek", "qwen", "glm", "kimi", "minimax"] as const)(
    "denies unimplemented %s strict tools",
    async (plugin) => {
      const command = vendor_request(fixture, plugin, true);
      if (!command.invocation?.tools[0]) throw new Error("missing_tool");
      command.invocation.tools[0].strict = true;
      await expect(invoke(command)).rejects.toThrow("vendor_strict_tool_denied");
      expect(fixture.requests).toHaveLength(0);
    },
  );

  it("denies unsupported GLM tool selection", async () => {
    const command = vendor_request(fixture, "glm", true);
    if (!command.invocation?.toolChoice) throw new Error("missing_choice");
    command.invocation.toolChoice.mode = ToolChoiceMode.REQUIRED;
    await expect(invoke(command)).rejects.toThrow("vendor_tool_choice_denied");
    expect(fixture.requests).toHaveLength(0);
  });

  it("reserves Perplexity non-token costs before dispatch", async () => {
    const command = vendor_request(fixture, "perplexity");
    if (!command.invocation?.budget?.maximumCost?.amount) throw new Error("missing_budget");
    command.invocation.budget.maximumCost.amount.value = "0.000256";
    await expect(invoke(command)).rejects.toThrow("provider_budget_denied");
    expect(fixture.requests).toHaveLength(0);
  });

  it("rejects unexpected search output", async () => {
    fixture.state.reply = {
      ...vendor_response("perplexity"),
      citations: ["https://fixture.invalid"],
    };
    await expect(invoke(vendor_request(fixture, "perplexity"))).rejects.toThrow(
      "unsupported_vendor_content",
    );
  });

  it.each(VENDOR_IDS)("denies %s invocation without its configured key", async (plugin) => {
    const host = new ProviderHost(fixture.config, new Uint8Array(32).fill(1), {}, fixture.fetcher);
    const command = vendor_request({ ...fixture, host }, plugin);
    await expect(
      host.invoke(command, fixture.principal, new AbortController().signal),
    ).rejects.toThrow("provider_credentials_missing");
    expect(fixture.requests).toHaveLength(0);
  });

  it("rejects unsupported vendor regions and invented capabilities", () => {
    for (const plugin of VENDOR_IDS) {
      const config = structuredClone(fixture.config);
      const model = config.models.find((value) => value.plugin === plugin);
      if (!model) throw new Error("missing_model");
      model.features.documents = true;
      expect(() => validate_deployment(config)).toThrow();
    }
  });
});
