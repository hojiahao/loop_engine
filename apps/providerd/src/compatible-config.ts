import type { ModelRoute } from "./config.js";

export const COMPATIBLE_IDS = [
  "openai_compatible",
  "anthropic_compatible",
  "ollama",
  "vllm",
  "sglang",
  "llamacpp",
  "lmstudio",
  "nim",
  "litellm",
  "portkey",
  "openrouter",
] as const;
export type CompatibleId = (typeof COMPATIBLE_IDS)[number];
export const GATEWAY_IDS: readonly string[] = ["litellm", "portkey", "openrouter"];

export function compatible_id(value: string): value is CompatibleId {
  return (COMPATIBLE_IDS as readonly string[]).includes(value);
}

/** Deployment-owned destinations; plaintext is restricted to literal loopback. */
export function compatible_url(value: string): boolean {
  try {
    const url = new URL(value);
    return (
      value === url.href.replace(/\/$/, "") &&
      !url.username &&
      !url.password &&
      !url.search &&
      !url.hash &&
      /^\/[A-Za-z0-9._/-]*$/.test(url.pathname) &&
      !url.pathname.split("/").some((part) => part === "." || part === "..") &&
      url.port !== "0" &&
      (url.protocol === "https:" ||
        (url.protocol === "http:" && ["127.0.0.1", "[::1]"].includes(url.hostname)))
    );
  } catch {
    return false;
  }
}

export function compatible_valid(model: ModelRoute): boolean {
  if (!compatible_id(model.plugin)) return model.compatible === undefined;
  const route = model.compatible;
  if (
    !route ||
    !compatible_url(route.base_url) ||
    model.cloud ||
    model.vendor ||
    model.input_token_limit === undefined ||
    model.input_token_limit > model.context_tokens
  )
    return false;
  if (route.auth === "none" ? model.secret_env !== undefined : model.secret_env === undefined)
    return false;
  const gateway = GATEWAY_IDS.includes(model.plugin);
  if (
    gateway !== Boolean(route.gateway) ||
    (gateway && (route.wire !== "chat" || route.auth !== "bearer"))
  )
    return false;
  if (
    model.plugin === "openrouter" &&
    (route.base_url !== "https://openrouter.ai/api/v1" ||
      model.reasoning !== "off" ||
      [model.model, route.request_model ?? model.model].some((selector) =>
        /(?:^openrouter\/auto$|:(?:online|nitro|floor)$)/.test(selector),
      ))
  )
    return false;
  if (model.plugin === "portkey" && !route.gateway?.upstream_key_env) return false;
  if (model.plugin !== "portkey" && route.gateway?.upstream_key_env !== undefined) return false;
  if (model.plugin === "anthropic_compatible") {
    if (route.wire !== "messages") return false;
  } else if (model.plugin === "openai_compatible") {
    if (route.wire === "messages") return false;
  } else if (route.wire !== "chat") return false;
  if (
    route.wire !== "chat" &&
    (route.strict_tools || route.output_limit !== "max_tokens" || route.stream_usage !== "separate")
  )
    return false;
  if (route.wire === "messages")
    return (
      route.auth !== "bearer" &&
      route.thinking === "none" &&
      route.reasoning_field === "none" &&
      ["off", "adaptive", "enabled"].includes(model.reasoning) &&
      (!model.features.prompt_caching || model.cache_creation_usd !== undefined)
    );
  if (model.cache_creation_usd !== undefined || model.thinking_tokens !== undefined) return false;
  if (route.wire === "responses")
    return (
      route.auth !== "api_key" &&
      route.thinking === "none" &&
      route.reasoning_field === "none" &&
      ["off", "low", "medium", "high"].includes(model.reasoning)
    );
  if (route.auth === "api_key" || model.features.documents) return false;
  if (model.reasoning !== "off" && (route.thinking === "none" || route.reasoning_field === "none"))
    return false;
  if (route.thinking === "template" && !["off", "adaptive"].includes(model.reasoning)) return false;
  return (
    ["off", "adaptive", "low", "medium", "high", "max"].includes(model.reasoning) &&
    !(model.plugin === "ollama" && route.strict_tools)
  );
}
