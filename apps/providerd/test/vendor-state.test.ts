import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { create } from "@bufbuild/protobuf";
import {
  ModelMessageSchema,
  ModelRole,
  StreamModelRequestSchema,
  ToolChoiceMode,
} from "@loop-engine/protocol/provider";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { model_snapshot, validate_deployment } from "../src/config.js";
import { ProviderHost } from "../src/host.js";
import { VENDOR_IDS, VENDORS, type VendorId, vendor_endpoint } from "../src/vendor-registry.js";
import { TEST_SECRET } from "./fixture.js";
import { sse_bytes } from "./rich-fixture.js";
import {
  vendor_events,
  vendor_fixture,
  vendor_request,
  vendor_response,
} from "./vendor-fixture.js";

let directory: string;
let fixture: Awaited<ReturnType<typeof vendor_fixture>>;
const thinking = VENDOR_IDS.filter((plugin) => plugin !== "perplexity");
beforeAll(async () => {
  directory = await mkdtemp(join(tmpdir(), "loop-provider-vendor-state-"));
  fixture = await vendor_fixture(directory, (config) => {
    for (const model of config.models) {
      if (model.plugin === "perplexity") continue;
      model.reasoning = ["xai", "groq", "cerebras"].includes(model.plugin) ? "low" : "adaptive";
    }
  });
}, 30_000);
afterAll(async () => {
  await fixture?.close();
  if (directory) await rm(directory, { recursive: true, force: true });
});
beforeEach(() => {
  fixture.requests.length = 0;
  fixture.state.events = undefined;
  fixture.state.reply = undefined;
  fixture.state.status = 200;
  fixture.state.delay = 0;
  fixture.state.chunk_delay = 0;
});

async function invoke(plugin: VendorId) {
  return (await fixture.client().invokeModel(vendor_request(fixture, plugin), { timeoutMs: 4500 }))
    .response;
}

async function collect(plugin: VendorId) {
  const command = vendor_request(fixture, plugin);
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

describe("vendor continuation and boundaries", () => {
  it.each(thinking.flatMap((plugin) => [false, true].map((stream) => ({ plugin, stream }))))(
    "restores $plugin thinking after restart (stream=$stream)",
    async ({ plugin, stream }) => {
      if (stream || plugin === "qwen")
        fixture.state.events = sse_bytes(
          vendor_events(plugin, true),
          VENDORS[plugin].wire === "chat",
        );
      else fixture.state.reply = vendor_response(plugin, false, true);
      const events = stream ? await collect(plugin) : [];
      const last = events.at(-1)?.event;
      const response = stream
        ? last?.case === "completed"
          ? last.value.response
          : undefined
        : await invoke(plugin);
      const block = response?.content[0];
      if (block?.content.case !== "reasoning") throw new Error("missing_reasoning");
      expect(block.content.value.continuation?.providerId?.value).toBe(plugin);
      expect(
        JSON.stringify(response, (_key, value) =>
          typeof value === "bigint" ? String(value) : value,
        ),
      ).not.toContain("opaque-");
      const restarted = new ProviderHost(
        fixture.config,
        new Uint8Array(32).fill(1),
        { LOOP_LLM_TEST: TEST_SECRET },
        fixture.fetcher,
      );
      const command = vendor_request({ ...fixture, host: restarted }, plugin);
      command.invocation?.messages.push(
        create(ModelMessageSchema, { role: ModelRole.ASSISTANT, content: response?.content }),
        create(ModelMessageSchema, {
          role: ModelRole.USER,
          content: [{ content: { case: "text", value: { text: "Continue" } } }],
        }),
      );
      fixture.state.events =
        plugin === "qwen" ? sse_bytes(vendor_events(plugin, true), true) : undefined;
      fixture.state.reply = vendor_response(plugin, false, true);
      await restarted.invoke(command, fixture.principal, new AbortController().signal);
      const sent = JSON.stringify(fixture.requests.at(-1)?.body);
      expect(sent).toContain(block.content.value.text);
      if (plugin === "mistral") expect(sent).toContain("opaque-mistral-signature");
      if (plugin === "minimax" && !stream) expect(sent).not.toContain("signature");
      const count = fixture.requests.length;
      block.content.value.text = "tampered";
      const changed = vendor_request({ ...fixture, host: restarted }, plugin);
      changed.invocation?.messages.push(...(command.invocation?.messages.slice(1) ?? []));
      await expect(
        restarted.invoke(changed, fixture.principal, new AbortController().signal),
      ).rejects.toThrow("provider_private_state_denied");
      expect(fixture.requests).toHaveLength(count);
    },
  );

  it.each(VENDOR_IDS)("rejects truncated %s streams", async (plugin) => {
    const events = vendor_events(plugin);
    events.pop();
    fixture.state.events = sse_bytes(events, VENDORS[plugin].wire === "chat");
    await expect(collect(plugin)).rejects.toThrow();
  });

  it("preserves DeepSeek tool thinking without a forbidden selector", async () => {
    fixture.state.reply = vendor_response("deepseek", true, true);
    const command = vendor_request(fixture, "deepseek", true);
    const response = await fixture.client().invokeModel(command, { timeoutMs: 4500 });
    expect(response.response?.content[0]?.content.case).toBe("reasoning");
    expect(fixture.requests[0]?.body.tool_choice).toBeUndefined();
    const denied = vendor_request(fixture, "deepseek", true);
    if (!denied.invocation?.toolChoice) throw new Error("missing_choice");
    denied.invocation.toolChoice.mode = ToolChoiceMode.REQUIRED;
    await expect(fixture.client().invokeModel(denied, { timeoutMs: 4500 })).rejects.toThrow(
      "vendor_tool_choice_denied",
    );
    expect(fixture.requests).toHaveLength(1);
  });

  it.each(["total", "cached", "reasoning", "miss"])(
    "denies conflicting %s accounting",
    async (field) => {
      const reply = vendor_response("deepseek", false, true);
      const usage = reply.usage as Record<string, unknown>;
      if (field === "total") usage.total_tokens = 13;
      if (field === "cached") usage.prompt_tokens_details = { cached_tokens: 4 };
      if (field === "reasoning") usage.completion_tokens_details = { reasoning_tokens: 6 };
      if (field === "miss") usage.prompt_cache_miss_tokens = 4;
      fixture.state.reply = reply;
      await expect(invoke("deepseek")).rejects.toThrow();
    },
  );

  it("rejects unsupported Mistral thinking references", async () => {
    const reply = vendor_response("mistral", false, true);
    const message = (reply.choices as { message: Record<string, unknown> }[])[0]?.message;
    if (!message) throw new Error("missing_message");
    message.content = [{ type: "thinking", thinking: [{ type: "reference", reference_ids: [0] }] }];
    fixture.state.reply = reply;
    await expect(invoke("mistral")).rejects.toThrow("unsupported_vendor_thinking");
  });

  it("rejects conflicting Groq terminal usage fields", async () => {
    const events = vendor_events("groq");
    const last = events.at(-1);
    if (!last) throw new Error("missing_event");
    last.usage = { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 };
    fixture.state.events = sse_bytes(events, true);
    await expect(collect("groq")).rejects.toThrow("conflicting_vendor_usage");
  });

  it("rejects search charges outside the configured Sonar profile", async () => {
    const reply = vendor_response("perplexity");
    (reply.usage as Record<string, unknown>).num_search_queries = 1;
    fixture.state.reply = reply;
    await expect(invoke("perplexity")).rejects.toThrow("unexpected_vendor_search");
  });

  it("binds the Together thinking dialect to the resolution", () => {
    const route = fixture.config.models.find((model) => model.plugin === "together");
    if (!route) throw new Error("missing_model");
    const changed = {
      ...route,
      vendor: { region: "global" as const, reasoning_field: "reasoning" as const },
    };
    const config = {
      ...fixture.config,
      models: fixture.config.models.map((model) => (model === route ? changed : model)),
    };
    const plugin = new Uint8Array(32).fill(1);
    expect(model_snapshot(fixture.config, route, plugin)).not.toEqual(
      model_snapshot(config, changed, plugin),
    );
  });

  it.each([
    ["qwen", "cn", undefined, "https://dashscope.aliyuncs.com"],
    ["qwen", "us", undefined, "https://dashscope-us.aliyuncs.com"],
    ["qwen", "jp", "workspace", "https://workspace.ap-northeast-1.maas.aliyuncs.com"],
    ["glm", "cn", undefined, "https://open.bigmodel.cn"],
    ["kimi", "cn", undefined, "https://api.moonshot.cn"],
    ["minimax", "cn", undefined, "https://api.minimax.cn"],
  ] as const)(
    "uses %s %s credentials only at its pinned origin",
    async (plugin, region, workspace, origin) => {
      const config = structuredClone(fixture.config);
      const model = config.models.find((entry) => entry.plugin === plugin);
      if (!model) throw new Error("missing_model");
      model.vendor = { region, ...(workspace ? { workspace } : {}) };
      validate_deployment(config);
      expect(new URL(vendor_endpoint(model)).origin).toBe(origin);
      const host = new ProviderHost(
        config,
        new Uint8Array(32).fill(1),
        { LOOP_LLM_TEST: TEST_SECRET },
        fixture.fetcher,
      );
      const principal = config.principals[0];
      if (!principal) throw new Error("missing_principal");
      fixture.state.reply = vendor_response(plugin, false, true);
      if (plugin === "qwen") fixture.state.events = sse_bytes(vendor_events(plugin, true), true);
      await host.invoke(
        vendor_request({ ...fixture, host }, plugin),
        principal,
        new AbortController().signal,
      );
      expect(fixture.seen_urls.at(-1)).toBe(origin);
    },
  );
});
