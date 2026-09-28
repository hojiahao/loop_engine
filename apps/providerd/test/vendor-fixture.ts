import { type Deployment, validate_deployment } from "../src/config.js";
import { VENDOR_IDS, VENDORS, type VendorId } from "../src/vendor-registry.js";
import { test_reply, test_request } from "./fixture.js";
import {
  reasoning_events,
  rich_fixture,
  text_events,
  tool_events,
  tool_reply,
  tool_request,
} from "./rich-fixture.js";

export async function vendor_fixture(directory: string, configure?: (config: Deployment) => void) {
  return rich_fixture(directory, (config) => {
    const base = config.models[0];
    if (!base) throw new Error("missing_model");
    config.models = VENDOR_IDS.map((plugin) => ({
      ...base,
      id: plugin,
      alias: plugin,
      plugin,
      model: `${plugin}-fixture-20260901`,
      input_token_limit: 128,
      ...(plugin === "minimax" ? { cache_creation_usd: "2" } : {}),
      ...(plugin === "perplexity"
        ? { vendor: { region: "global" as const, maximum_extra_usd: "0.0001" } }
        : {}),
      features: {
        ...base.features,
        documents: false,
        tools: VENDORS[plugin].tools,
        parallel_tools: VENDORS[plugin].parallel,
        structured_output: VENDORS[plugin].schema,
        vision: VENDORS[plugin].vision,
      },
    }));
    const principal = config.principals[0];
    if (!principal) throw new Error("missing_principal");
    principal.model_ids = [...VENDOR_IDS];
    configure?.(config);
    validate_deployment(config);
  });
}

export function vendor_request(
  fixture: Awaited<ReturnType<typeof vendor_fixture>>,
  plugin: VendorId,
  tools = false,
) {
  const command = tools ? tool_request(fixture, plugin) : test_request(fixture.host, plugin);
  const tool = command.invocation?.tools[0];
  if (tool) tool.strict = VENDORS[plugin].strict;
  return command;
}

export function vendor_response(
  plugin: VendorId,
  tools = false,
  thought = false,
): Record<string, unknown> {
  const wire = VENDORS[plugin].wire;
  const kind = wire === "messages" ? "claude" : wire === "responses" ? "responses" : "chat";
  const reply = structuredClone(
    tools
      ? tool_reply(kind)
      : test_reply(
          wire === "messages"
            ? "/v1/messages"
            : wire === "responses"
              ? "/v1/responses"
              : "/v1/chat/completions",
          { model: `${plugin}-fixture-20260901` },
        ),
  ) as Record<string, unknown>;
  reply.model = `${plugin}-fixture-20260901`;
  if (wire === "chat") {
    reply.usage = {
      prompt_tokens: 12,
      completion_tokens: 5,
      total_tokens: 17,
      ...(plugin === "deepseek"
        ? { prompt_cache_hit_tokens: 3, prompt_cache_miss_tokens: 9 }
        : { prompt_tokens_details: { cached_tokens: 3 } }),
    };
    if (thought) {
      const choices = reply.choices as { message: Record<string, unknown> }[];
      const message = choices[0]?.message;
      if (!message) throw new Error("missing_message");
      const field = VENDORS[plugin].reasoning_field;
      if (field === "content")
        message.content = [
          {
            type: "thinking",
            thinking: [{ type: "text", text: "private thought" }],
            signature: "opaque-mistral-signature",
            closed: true,
          },
          ...(typeof message.content === "string" ? [{ type: "text", text: message.content }] : []),
        ];
      else message[field] = "private thought";
    }
  } else if (wire === "messages" && thought) {
    (reply.content as unknown[]).unshift({ type: "thinking", thinking: "private thought" });
  } else if (wire === "responses" && thought) {
    (reply.output as unknown[]).unshift({
      type: "reasoning",
      id: "rs1",
      summary: [{ type: "summary_text", text: "private thought" }],
      encrypted_content: "opaque-xai-state",
    });
  }
  return reply;
}

export function vendor_events(
  plugin: VendorId,
  thought = false,
  tools = false,
): Record<string, unknown>[] {
  const wire = VENDORS[plugin].wire;
  if (wire !== "chat") {
    const kind = wire === "messages" ? "claude" : "responses";
    return JSON.parse(
      JSON.stringify(
        tools ? tool_events(kind) : thought ? reasoning_events(kind) : text_events(kind),
      ).replaceAll(`${kind}-fixture-20260901`, `${plugin}-fixture-20260901`),
    );
  }
  const envelope = {
    id: "vendor-stream",
    object: "chat.completion.chunk",
    model: `${plugin}-fixture-20260901`,
    created: 1,
  };
  const chunk = (delta: unknown, finish_reason: string | null = null) => ({
    ...envelope,
    choices: [{ index: 0, delta, finish_reason }],
  });
  const events: Record<string, unknown>[] = [chunk({ role: "assistant", content: "" })];
  if (thought) {
    const field = VENDORS[plugin].reasoning_field;
    events.push(
      chunk(
        field === "content"
          ? {
              content: [
                { type: "thinking", thinking: [{ type: "text", text: "private thought" }] },
              ],
            }
          : { [field]: "private thought" },
      ),
    );
    if (field === "content")
      events.push(
        chunk({
          content: [
            { type: "thinking", thinking: [], signature: "opaque-mistral-signature", closed: true },
          ],
        }),
      );
  }
  if (tools)
    events.push(
      chunk({
        tool_calls: [
          {
            index: 0,
            type: "function",
            id: "call_1",
            function: { name: "propose_factor", arguments: '{"window":' },
          },
        ],
      }),
      chunk({ tool_calls: [{ index: 0, function: { arguments: "20}" } }] }),
    );
  else events.push(chunk({ content: "研究 idea" }));
  const usage = { prompt_tokens: 12, completion_tokens: 5, total_tokens: 17 };
  if (VENDORS[plugin].stream_usage === "terminal") {
    const ending: Record<string, unknown> = chunk({}, tools ? "tool_calls" : "stop");
    if (plugin === "groq") ending.x_groq = { id: "fixture", usage };
    else ending.usage = usage;
    events.push(ending);
  } else {
    events.push(chunk({}, tools ? "tool_calls" : "stop"), { ...envelope, choices: [], usage });
  }
  return events;
}
