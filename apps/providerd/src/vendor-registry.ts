import type { ModelRoute } from "./config.js";

export const VENDOR_IDS = [
  "mistral",
  "deepseek",
  "qwen",
  "xai",
  "groq",
  "together",
  "fireworks",
  "cerebras",
  "perplexity",
  "glm",
  "kimi",
  "minimax",
] as const;
export type VendorId = (typeof VENDOR_IDS)[number];

/** Protocol ceilings, not a claim that every hosted model supports each feature.
 * A deployment must narrow these ceilings to the exact model's capabilities. */
export interface VendorProfile {
  readonly endpoint: string;
  readonly wire: "chat" | "responses" | "messages";
  readonly vision: boolean;
  readonly schema: boolean;
  readonly tools: boolean;
  readonly strict: boolean;
  readonly parallel: boolean;
  readonly thinking: readonly string[];
  readonly reasoning_field: "reasoning_content" | "reasoning" | "content";
  readonly stream_usage: "separate" | "terminal";
}

const chat: VendorProfile = {
  endpoint: "",
  wire: "chat",
  vision: true,
  schema: true,
  tools: true,
  strict: true,
  parallel: true,
  thinking: ["off", "adaptive", "low", "medium", "high", "max"],
  reasoning_field: "reasoning_content",
  stream_usage: "separate",
};
export const VENDORS: Readonly<Record<VendorId, VendorProfile>> = {
  mistral: {
    ...chat,
    endpoint: "https://api.mistral.ai/v1",
    strict: false,
    thinking: ["off", "adaptive", "low", "medium", "high"],
    reasoning_field: "content",
    stream_usage: "terminal",
  },
  deepseek: {
    ...chat,
    endpoint: "https://api.deepseek.com",
    vision: false,
    schema: false,
    strict: false,
    thinking: ["off", "adaptive", "low", "high", "max"],
  },
  qwen: {
    ...chat,
    endpoint: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
    strict: false,
  },
  xai: {
    ...chat,
    endpoint: "https://api.x.ai/v1",
    wire: "responses",
    thinking: ["off", "low", "medium", "high"],
  },
  groq: {
    ...chat,
    endpoint: "https://api.groq.com/openai/v1",
    reasoning_field: "reasoning",
    thinking: ["off", "low", "medium", "high"],
    stream_usage: "terminal",
  },
  together: { ...chat, endpoint: "https://api.together.ai/v1" },
  fireworks: { ...chat, endpoint: "https://api.fireworks.ai/inference/v1" },
  cerebras: {
    ...chat,
    endpoint: "https://api.cerebras.ai/v1",
    vision: false,
    reasoning_field: "reasoning",
    thinking: ["off", "low", "medium", "high"],
    stream_usage: "terminal",
  },
  perplexity: {
    ...chat,
    endpoint: "https://api.perplexity.ai/v1",
    vision: false,
    tools: false,
    strict: false,
    parallel: false,
    thinking: ["off"],
    stream_usage: "terminal",
  },
  glm: {
    ...chat,
    endpoint: "https://api.z.ai/api/paas/v4",
    schema: false,
    strict: false,
    parallel: false,
    thinking: ["off", "adaptive", "low", "high", "max"],
    stream_usage: "terminal",
  },
  kimi: { ...chat, endpoint: "https://api.moonshot.ai/v1", strict: false },
  minimax: {
    ...chat,
    endpoint: "https://api.minimax.io/anthropic",
    wire: "messages",
    schema: false,
    strict: false,
    thinking: ["off", "adaptive"],
  },
};

export function vendor_id(value: string): value is VendorId {
  return (VENDOR_IDS as readonly string[]).includes(value);
}

/** Region is an administrative credential boundary, never an automatic fallback. */
export function vendor_endpoint(model: ModelRoute): string {
  if (!vendor_id(model.plugin)) throw new Error("invalid_vendor");
  const region = model.vendor?.region ?? "global";
  if (model.plugin === "qwen") {
    const workspace = model.vendor?.workspace;
    if (region === "us") {
      if (workspace) throw new Error("invalid_vendor_region");
      return "https://dashscope-us.aliyuncs.com/compatible-mode/v1";
    }
    if (workspace) {
      const location = { global: "ap-southeast-1", cn: "cn-beijing", jp: "ap-northeast-1" }[region];
      if (!location) throw new Error("invalid_vendor_region");
      return `https://${workspace}.${location}.maas.aliyuncs.com/compatible-mode/v1`;
    }
    if (region === "cn") return "https://dashscope.aliyuncs.com/compatible-mode/v1";
  }
  if (region === "cn") {
    if (model.plugin === "glm") return "https://open.bigmodel.cn/api/paas/v4";
    if (model.plugin === "kimi") return "https://api.moonshot.cn/v1";
    if (model.plugin === "minimax") return "https://api.minimax.cn/anthropic";
  }
  if (region !== "global") throw new Error("invalid_vendor_region");
  return VENDORS[model.plugin].endpoint;
}

export function vendor_valid(model: ModelRoute): boolean {
  if (!vendor_id(model.plugin)) return model.vendor === undefined;
  const profile = VENDORS[model.plugin];
  try {
    vendor_endpoint(model);
  } catch {
    return false;
  }
  return (
    model.cloud === undefined &&
    model.secret_env !== undefined &&
    model.input_token_limit !== undefined &&
    model.input_token_limit <= model.context_tokens &&
    profile.thinking.includes(model.reasoning) &&
    model.thinking_tokens === undefined &&
    !model.features.documents &&
    (!model.features.vision || profile.vision) &&
    (!model.features.structured_output || profile.schema) &&
    (!model.features.tools || profile.tools) &&
    (!model.features.parallel_tools || profile.parallel) &&
    (model.vendor?.workspace === undefined || model.plugin === "qwen") &&
    (model.vendor?.reasoning_field === undefined || model.plugin === "together") &&
    (model.plugin === "perplexity"
      ? model.vendor?.maximum_extra_usd !== undefined
      : model.vendor?.maximum_extra_usd === undefined) &&
    (model.plugin !== "minimax" ||
      ((!model.model.startsWith("MiniMax-M2") ||
        (model.reasoning === "adaptive" && !model.features.vision)) &&
        (!model.features.prompt_caching || model.cache_creation_usd !== undefined))) &&
    (model.plugin === "minimax" || model.cache_creation_usd === undefined)
  );
}
