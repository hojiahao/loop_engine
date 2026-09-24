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
  ToolResultStatus,
} from "@loop-engine/protocol/provider";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { compatible_url } from "../src/compatible-config.js";
import { validate_deployment } from "../src/config.js";
import { ProviderHost } from "../src/host.js";
import {
  COMPATIBLE_ROUTES,
  compatible_events,
  compatible_fixture,
  compatible_kind,
  compatible_reply,
  compatible_request,
} from "./compatible-fixture.js";
import { TEST_SECRET } from "./fixture.js";
import { sse_bytes, TEST_SCHEMA } from "./rich-fixture.js";

let directory: string;
let fixture: Awaited<ReturnType<typeof compatible_fixture>>;
beforeAll(async () => {
  directory = await mkdtemp(join(tmpdir(), "loop-provider-compatible-"));
  fixture = await compatible_fixture(directory);
}, 30_000);
afterAll(async () => {
  await fixture?.close();
  if (directory) await rm(directory, { recursive: true, force: true });
});
beforeEach(() => {
  fixture.requests.length = 0;
  fixture.state.reply = undefined;
  fixture.state.events = undefined;
  fixture.state.body = undefined;
  fixture.state.status = 200;
  fixture.state.delay = 0;
  fixture.state.chunk_delay = 0;
});
async function invoke(command: ReturnType<typeof compatible_request>) {
  return (await fixture.client().invokeModel(command, { timeoutMs: 4500 })).response;
}
async function collect(command: ReturnType<typeof compatible_request>) {
  const events = [];
  for await (const reply of fixture.client().streamModel(
    create(StreamModelRequestSchema, {
      context: command.context,
      invocation: command.invocation,
    }),
    { timeoutMs: 4500 },
  ))
    events.push(reply.event);
  return events;
}

describe("compatible transports", () => {
  it.each(COMPATIBLE_ROUTES)("invokes %s with explicit authentication", async (id) => {
    fixture.state.reply = compatible_reply(id);
    const command = compatible_request(fixture, id);
    const reply = await invoke(command);
    expect(reply?.finishReason).toBe(ModelFinishReason.STOP);
    expect(reply?.usage?.inputTokens).toBe(12n);
    expect(fixture.requests).toHaveLength(1);
    const sent = fixture.requests[0];
    if (id === "ollama") {
      expect(sent?.authorization).toBeUndefined();
      expect(sent?.key).toBeUndefined();
    } else if (id === "anthropic_compatible") expect(sent?.key).toBe(TEST_SECRET);
    else if (id === "portkey") {
      expect(sent?.headers["x-portkey-api-key"]).toBe(TEST_SECRET);
      expect(sent?.authorization).toBe("Bearer fixture-upstream-key");
      expect(sent?.headers["x-portkey-provider"]).toBe("openai");
      expect(sent?.headers["x-portkey-config"]).toBe('{"retry":{"attempts":0}}');
    } else expect(sent?.authorization).toBe(`Bearer ${TEST_SECRET}`);
    if (id === "openrouter")
      expect(sent?.body.provider).toEqual({
        only: ["openai"],
        allow_fallbacks: false,
        require_parameters: true,
      });
    if (id === "litellm")
      expect(sent?.body).toMatchObject({ disable_fallbacks: true, num_retries: 0 });
    expect(command.invocation?.model?.providerId?.value).toBe(
      id === "compatible_responses" ? "openai_compatible" : id,
    );
  });

  it.each(COMPATIBLE_ROUTES.flatMap((id) => [false, true].map((tools) => ({ id, tools }))))(
    "streams $id text/tools=$tools through the shared lifecycle",
    async ({ id, tools }) => {
      fixture.state.events = sse_bytes(
        compatible_events(id, tools),
        compatible_kind(id) === "chat",
      );
      const events = await collect(compatible_request(fixture, id, tools));
      const final = events.at(-1)?.event;
      expect(final?.case).toBe("completed");
      if (final?.case !== "completed") throw new Error("missing_result");
      expect(final.value.response?.finishReason).toBe(
        tools ? ModelFinishReason.TOOL_CALL : ModelFinishReason.STOP,
      );
    },
  );

  it.each(COMPATIBLE_ROUTES)("round trips %s tool results", async (id) => {
    fixture.state.reply = compatible_reply(id, true);
    const result = await invoke(compatible_request(fixture, id, true));
    const block = result?.content[0];
    if (block?.content.case !== "toolCall") throw new Error("missing_tool");
    const command = compatible_request(fixture, id, true);
    command.invocation?.messages.push(
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
                result: { case: "text", value: { text: "evaluated" } },
              },
            },
          },
        ],
      }),
    );
    fixture.state.reply = compatible_reply(id);
    await invoke(command);
    expect(JSON.stringify(fixture.requests.at(-1)?.body)).toContain("evaluated");
  });

  it.each(COMPATIBLE_ROUTES)("validates %s registered JSON schema output", async (id) => {
    const command = compatible_request(fixture, id);
    if (!command.invocation) throw new Error("missing_invocation");
    command.invocation.structuredOutput = create(StructuredOutputDefinitionSchema, {
      name: "factor_window",
      jsonSchema: TEST_SCHEMA,
      strict: true,
    });
    const reply = compatible_reply(id);
    const text = '{"window":20}';
    if (compatible_kind(id) === "claude") reply.content = [{ type: "text", text }];
    else if (compatible_kind(id) === "responses")
      reply.output = [
        { type: "message", role: "assistant", content: [{ type: "output_text", text }] },
      ];
    else {
      const choice = (reply.choices as { message: { content: string } }[])[0];
      if (!choice) throw new Error("missing_choice");
      choice.message.content = text;
    }
    fixture.state.reply = reply;
    expect((await invoke(command))?.content[0]?.content.case).toBe("structuredOutput");
  });

  it.each(COMPATIBLE_ROUTES)("fails closed on missing %s usage", async (id) => {
    const reply = compatible_reply(id);
    delete reply.usage;
    fixture.state.reply = reply;
    await expect(invoke(compatible_request(fixture, id))).rejects.toThrow();
  });

  it.each(COMPATIBLE_ROUTES)("refuses %s redirects", async (id) => {
    fixture.state.status = 302;
    await expect(invoke(compatible_request(fixture, id))).rejects.toThrow();
    expect(fixture.requests).toHaveLength(1);
  });

  it.each(COMPATIBLE_ROUTES)("redacts %s upstream errors without retry", async (id) => {
    fixture.state.status = 429;
    fixture.state.body = { error: { message: TEST_SECRET } };
    await expect(invoke(compatible_request(fixture, id))).rejects.toThrow("provider_rate_limited");
    expect(fixture.requests).toHaveLength(1);
  });

  it.each(["missing", "invalid"])("denies a %s Portkey upstream key", async (mode) => {
    const host = new ProviderHost(
      fixture.config,
      new Uint8Array(32).fill(1),
      {
        LOOP_LLM_TEST: TEST_SECRET,
        LOOP_LLM_UPSTREAM: mode === "invalid" ? "invalid\nkey" : undefined,
      },
      fixture.fetcher,
    );
    await expect(
      host.invoke(
        compatible_request({ ...fixture, host }, "portkey"),
        fixture.principal,
        new AbortController().signal,
      ),
    ).rejects.toThrow("provider_credentials_missing");
    expect(fixture.requests).toHaveLength(0);
  });

  it.each([
    "http://public.example/v1",
    "https://user:pass@example.com/v1",
    "https://example.com/v1?q=1",
    "https://example.com/v1#tag",
    "https://example.com/%2e%2e/v1",
    "http://localhost:8080/v1",
    "https://example.com:0/v1",
    "file:///tmp/model",
  ])("rejects unsafe endpoint %s", (value) => {
    expect(compatible_url(value)).toBe(false);
  });

  it("rejects a gateway with no upstream identity", () => {
    const config = structuredClone(fixture.config);
    const route = config.models.find((model) => model.id === "openrouter");
    if (!route?.compatible) throw new Error("missing_route");
    delete route.compatible.gateway;
    expect(() => validate_deployment(config)).toThrow();
  });
});
