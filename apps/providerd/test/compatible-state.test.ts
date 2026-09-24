import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { create } from "@bufbuild/protobuf";
import {
  ModelMessageSchema,
  ModelRole,
  StreamModelRequestSchema,
} from "@loop-engine/protocol/provider";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { model_snapshot, validate_deployment } from "../src/config.js";
import { ProviderHost } from "../src/host.js";
import {
  COMPATIBLE_ROUTES,
  type CompatibleRoute,
  compatible_events,
  compatible_fixture,
  compatible_kind,
  compatible_reply,
  compatible_request,
} from "./compatible-fixture.js";
import { TEST_SECRET } from "./fixture.js";
import { reasoning_events, sse_bytes } from "./rich-fixture.js";

let directory: string;
let fixture: Awaited<ReturnType<typeof compatible_fixture>>;
const thinking = COMPATIBLE_ROUTES.filter((id) => id !== "openrouter");
const secrets = { LOOP_LLM_TEST: TEST_SECRET, LOOP_LLM_UPSTREAM: "fixture-upstream-key" };
beforeAll(async () => {
  directory = await mkdtemp(join(tmpdir(), "loop-provider-compatible-state-"));
  fixture = await compatible_fixture(directory, (config) => {
    for (const model of config.models) {
      const route = model.compatible;
      if (!route || model.id === "openrouter") continue;
      route.request_model = `served/${model.id}`;
      model.reasoning = route.wire === "messages" ? "adaptive" : "low";
      if (route.wire !== "chat") continue;
      route.thinking = model.plugin === "sglang" ? "template" : "effort";
      if (route.thinking === "template") model.reasoning = "adaptive";
      route.reasoning_field = model.plugin === "lmstudio" ? "reasoning" : "reasoning_content";
      if (model.plugin === "vllm") route.stream_usage = "terminal";
    }
  });
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
  fixture.state.on_request = undefined;
});

function thinking_reply(id: CompatibleRoute) {
  const reply = compatible_reply(id);
  const kind = compatible_kind(id);
  if (kind === "chat") {
    const message = (reply.choices as { message: Record<string, unknown> }[])[0]?.message;
    if (!message) throw new Error("missing_message");
    message[id === "lmstudio" ? "reasoning" : "reasoning_content"] = "public summary";
  } else if (kind === "claude")
    (reply.content as unknown[]).unshift({
      type: "thinking",
      thinking: "public summary",
      signature: "private-signature",
    });
  else
    (reply.output as unknown[]).unshift({
      type: "reasoning",
      id: "rs1",
      summary: [{ type: "summary_text", text: "public summary" }],
      encrypted_content: "private-ciphertext",
    });
  return reply;
}

function thinking_stream(id: CompatibleRoute): Record<string, unknown>[] {
  const kind = compatible_kind(id);
  if (kind !== "chat")
    return JSON.parse(
      JSON.stringify(reasoning_events(kind)).replaceAll(
        `${kind}-fixture-20260901`,
        `${id}-fixture-20260901`,
      ),
    );
  const events = compatible_events(id);
  const first = events[0];
  if (!first) throw new Error("missing_start");
  events.splice(1, 0, {
    ...first,
    choices: [
      {
        index: 0,
        delta: { [id === "lmstudio" ? "reasoning" : "reasoning_content"]: "public summary" },
        finish_reason: null,
      },
    ],
  });
  if (id === "vllm") {
    const usage = events.pop()?.usage;
    const last = events.at(-1);
    if (!last || !usage) throw new Error("missing_usage");
    last.usage = usage;
  }
  return events;
}

async function invoke(command: ReturnType<typeof compatible_request>) {
  return (await fixture.client().invokeModel(command, { timeoutMs: 4500 })).response;
}

async function collect(id: CompatibleRoute) {
  const command = compatible_request(fixture, id);
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

describe("compatible continuation and routing boundaries", () => {
  it.each(thinking.flatMap((id) => [false, true].map((stream) => ({ id, stream }))))(
    "restores $id reasoning after restart (stream=$stream)",
    async ({ id, stream }) => {
      if (stream)
        fixture.state.events = sse_bytes(thinking_stream(id), compatible_kind(id) === "chat");
      else fixture.state.reply = thinking_reply(id);
      const events = stream ? await collect(id) : [];
      const final = events.at(-1)?.event;
      const response = stream
        ? final?.case === "completed"
          ? final.value.response
          : undefined
        : await invoke(compatible_request(fixture, id));
      const block = response?.content.find((block) => block.content.case === "reasoning");
      if (block?.content.case !== "reasoning") throw new Error("missing_reasoning");
      expect(
        JSON.stringify(response, (_key, value) =>
          typeof value === "bigint" ? String(value) : value,
        ),
      ).not.toContain("private-");
      const restarted = new ProviderHost(
        fixture.config,
        new Uint8Array(32).fill(1),
        secrets,
        fixture.fetcher,
      );
      const command = compatible_request({ ...fixture, host: restarted }, id);
      command.invocation?.messages.push(
        create(ModelMessageSchema, { role: ModelRole.ASSISTANT, content: response?.content }),
        create(ModelMessageSchema, {
          role: ModelRole.USER,
          content: [{ content: { case: "text", value: { text: "Continue" } } }],
        }),
      );
      fixture.state.events = undefined;
      fixture.state.reply = thinking_reply(id);
      await restarted.invoke(command, fixture.principal, new AbortController().signal);
      const sent = fixture.requests.at(-1)?.body;
      expect(sent?.model).toBe(`served/${id}`);
      expect(JSON.stringify(sent)).toContain("public summary");
      if (id === "anthropic_compatible")
        expect(JSON.stringify(sent)).toContain("private-signature");
      if (id === "compatible_responses")
        expect(JSON.stringify(sent)).toContain("private-ciphertext");
      if (id === "sglang") expect(sent?.chat_template_kwargs).toEqual({ enable_thinking: true });
      else if (compatible_kind(id) === "chat") expect(sent?.reasoning_effort).toBe("low");
      const count = fixture.requests.length;
      block.content.value.text = "tampered";
      const changed = compatible_request({ ...fixture, host: restarted }, id);
      changed.invocation?.messages.push(...(command.invocation?.messages.slice(2) ?? []));
      await expect(
        restarted.invoke(changed, fixture.principal, new AbortController().signal),
      ).rejects.toThrow("provider_private_state_denied");
      expect(fixture.requests).toHaveLength(count);
    },
  );

  it.each(COMPATIBLE_ROUTES)("rejects truncated %s streams", async (id) => {
    const events = compatible_events(id);
    events.pop();
    fixture.state.events = sse_bytes(events, compatible_kind(id) === "chat");
    await expect(collect(id)).rejects.toThrow();
  });

  it.each(COMPATIBLE_ROUTES)("rejects a substituted %s response model", async (id) => {
    fixture.state.reply = { ...compatible_reply(id), model: "unexpected-fallback-model" };
    await expect(invoke(compatible_request(fixture, id))).rejects.toThrow();
    expect(fixture.requests).toHaveLength(1);
  });

  it.each(["openai_compatible", "anthropic_compatible", "compatible_responses"] as const)(
    "cancels %s without an automatic second generation",
    async (id) => {
      const command = compatible_request(fixture, id);
      const controller = new AbortController();
      fixture.state.delay = 100;
      fixture.state.on_request = () => controller.abort();
      await expect(
        fixture.host.invoke(command, fixture.principal, controller.signal),
      ).rejects.toThrow("provider_cancelled");
      await expect(invoke(command)).rejects.toThrow("invocation_ambiguous");
      expect(fixture.requests).toHaveLength(1);
    },
  );

  it("rejects undeclared strict tools before reaching Ollama", async () => {
    const command = compatible_request(fixture, "ollama", true);
    const tool = command.invocation?.tools[0];
    if (!tool) throw new Error("missing_tool");
    tool.strict = true;
    await expect(invoke(command)).rejects.toThrow("compatible_strict_denied");
    expect(fixture.requests).toHaveLength(0);
  });

  it("rejects opaque gateway reasoning instead of losing continuation", async () => {
    const reply = compatible_reply("openrouter");
    const message = (reply.choices as { message: Record<string, unknown> }[])[0]?.message;
    if (!message) throw new Error("missing_message");
    message.reasoning_details = [{ type: "reasoning.encrypted", data: "opaque" }];
    fixture.state.reply = reply;
    await expect(invoke(compatible_request(fixture, "openrouter"))).rejects.toThrow(
      "unsupported_vendor_content",
    );
  });

  it.each(["litellm", "portkey", "openrouter"] as const)(
    "reserves %s non-token charges before dispatch",
    async (id) => {
      const command = compatible_request(fixture, id);
      if (!command.invocation?.budget?.maximumCost?.amount) throw new Error("missing_budget");
      command.invocation.budget.maximumCost.amount.value = "0.000256";
      await expect(invoke(command)).rejects.toThrow("provider_budget_denied");
      expect(fixture.requests).toHaveLength(0);
    },
  );

  it.each(["0", "1", "-1", "invalid"])("checks the Portkey retry receipt %s", async (retries) => {
    fixture.state.reply = compatible_reply("portkey");
    const fetcher: typeof fetch = async (input, init) => {
      const response = await fixture.fetcher(input, init);
      const headers = new Headers(response.headers);
      headers.set("x-portkey-retry-attempt-count", retries);
      return new Response(response.body, { status: response.status, headers });
    };
    const host = new ProviderHost(fixture.config, new Uint8Array(32).fill(1), secrets, fetcher);
    const result = host.invoke(
      compatible_request({ ...fixture, host }, "portkey"),
      fixture.principal,
      new AbortController().signal,
    );
    if (retries === "0") await expect(result).resolves.toBeDefined();
    else await expect(result).rejects.toThrow();
    expect(fixture.requests).toHaveLength(1);
  });

  it("pins the administrator's gateway mapping into the model resolution", () => {
    const config = structuredClone(fixture.config);
    const original = fixture.config.models.find((model) => model.id === "openrouter");
    const changed = config.models.find((model) => model.id === "openrouter");
    if (!original || !changed?.compatible?.gateway) throw new Error("missing_route");
    changed.compatible.gateway.route_sha256 = "c".repeat(64);
    const plugin = new Uint8Array(32).fill(1);
    expect(model_snapshot(fixture.config, original, plugin)).not.toEqual(
      model_snapshot(config, changed, plugin),
    );
  });

  it.each(["openrouter/auto", "model:online", "model:nitro", "model:floor"])(
    "rejects dynamic gateway request selector %s",
    (selector) => {
      const config = structuredClone(fixture.config);
      const model = config.models.find((model) => model.id === "openrouter");
      if (!model?.compatible) throw new Error("missing_route");
      model.compatible.request_model = selector;
      expect(() => validate_deployment(config)).toThrow();
    },
  );

  it.each(["anthropic_compatible", "compatible_responses"] as const)(
    "rejects Chat-only dialect settings on %s",
    (id) => {
      const config = structuredClone(fixture.config);
      const model = config.models.find((model) => model.id === id);
      if (!model?.compatible) throw new Error("missing_route");
      model.compatible.stream_usage = "terminal";
      expect(() => validate_deployment(config)).toThrow();
    },
  );
});
