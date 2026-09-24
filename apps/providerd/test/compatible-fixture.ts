import { COMPATIBLE_IDS, GATEWAY_IDS } from "../src/compatible-config.js";
import { type Deployment, validate_deployment } from "../src/config.js";
import { TEST_SECRET, test_reply, test_request } from "./fixture.js";
import {
  rich_fixture,
  text_events,
  tool_events,
  tool_reply,
  tool_request,
} from "./rich-fixture.js";

export const COMPATIBLE_ROUTES = [...COMPATIBLE_IDS, "compatible_responses"] as const;
export type CompatibleRoute = (typeof COMPATIBLE_ROUTES)[number];

export function compatible_kind(id: CompatibleRoute) {
  return id === "anthropic_compatible"
    ? "claude"
    : id === "compatible_responses"
      ? "responses"
      : "chat";
}

export async function compatible_fixture(
  directory: string,
  configure?: (config: Deployment) => void,
) {
  return rich_fixture(
    directory,
    (config) => {
      const base = config.models[0];
      if (!base) throw new Error("missing_model");
      config.models = COMPATIBLE_ROUTES.map((id) => {
        const plugin = id === "compatible_responses" ? "openai_compatible" : id;
        const wire =
          compatible_kind(id) === "claude"
            ? "messages"
            : compatible_kind(id) === "responses"
              ? "responses"
              : "chat";
        return {
          ...base,
          id,
          alias: id,
          plugin,
          model: `${id}-fixture-20260901`,
          input_token_limit: 128,
          secret_env: id === "ollama" ? undefined : "LOOP_LLM_TEST",
          ...(wire === "messages" ? { cache_creation_usd: "2" } : {}),
          features: { ...base.features, documents: wire !== "chat" },
          compatible: {
            base_url:
              id === "openrouter"
                ? "https://openrouter.ai/api/v1"
                : id === "ollama"
                  ? "http://127.0.0.1:11434/v1"
                  : wire === "messages"
                    ? "https://messages.fixture.invalid"
                    : "https://compatible.fixture.invalid/v1",
            wire,
            auth: wire === "messages" ? "api_key" : id === "ollama" ? "none" : "bearer",
            strict_tools: wire === "chat" && id !== "ollama",
            output_limit: "max_tokens",
            thinking: "none",
            reasoning_field: "none",
            stream_usage: "separate",
            ...(GATEWAY_IDS.includes(id)
              ? {
                  gateway: {
                    upstream_provider: "openai",
                    route_sha256: "b".repeat(64),
                    maximum_extra_usd: "0.0001",
                    ...(id === "portkey" ? { upstream_key_env: "LOOP_LLM_UPSTREAM" } : {}),
                  },
                }
              : {}),
          },
        };
      });
      const principal = config.principals[0];
      if (!principal) throw new Error("missing_principal");
      principal.model_ids = [...COMPATIBLE_ROUTES];
      configure?.(config);
      validate_deployment(config);
    },
    { secrets: { LOOP_LLM_TEST: TEST_SECRET, LOOP_LLM_UPSTREAM: "fixture-upstream-key" } },
  );
}

export function compatible_request(
  fixture: Awaited<ReturnType<typeof compatible_fixture>>,
  id: CompatibleRoute,
  tools = false,
) {
  const command = tools ? tool_request(fixture, id) : test_request(fixture.host, id);
  const tool = command.invocation?.tools[0];
  if (tool && id === "ollama") tool.strict = false;
  return command;
}

export function compatible_reply(id: CompatibleRoute, tools = false): Record<string, unknown> {
  const kind = compatible_kind(id);
  const value = tools
    ? tool_reply(kind)
    : test_reply(
        kind === "claude"
          ? "/v1/messages"
          : kind === "responses"
            ? "/v1/responses"
            : "/v1/chat/completions",
        { model: `${id}-fixture-20260901` },
      );
  const reply = structuredClone(value) as Record<string, unknown>;
  reply.model = `${id}-fixture-20260901`;
  if (kind === "chat") reply.usage = { prompt_tokens: 12, completion_tokens: 5, total_tokens: 17 };
  return reply;
}

export function compatible_events(id: CompatibleRoute, tools = false): Record<string, unknown>[] {
  const kind = compatible_kind(id);
  const events: Record<string, unknown>[] = JSON.parse(
    JSON.stringify(tools ? tool_events(kind) : text_events(kind)).replaceAll(
      `${kind}-fixture-20260901`,
      `${id}-fixture-20260901`,
    ),
  );
  if (kind === "chat")
    for (const event of events)
      if (event.usage) event.usage = { prompt_tokens: 12, completion_tokens: 5, total_tokens: 17 };
  return events;
}
