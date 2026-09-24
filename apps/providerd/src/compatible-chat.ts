import type OpenAI from "openai";
import { ProviderError } from "./errors.js";
import type { NativeInput } from "./native.js";
import { chat_parameters } from "./openai-content.js";
import type { ChatDialect } from "./openai-stream.js";
import { normalized_reply, vendor_chunks, vendor_record } from "./vendor-replies.js";

/** Only a declared wire dialect can add fields to the native Chat request. */
export function compatible_parameters(input: NativeInput, streaming: boolean) {
  const route = input.model.compatible;
  if (route?.wire !== "chat") throw new ProviderError("compatible_route_missing");
  if (!route.strict_tools && input.tools.some((tool) => tool.strict))
    throw new ProviderError("compatible_strict_denied");
  const base = chat_parameters({
    ...input,
    messages: input.messages.map((message) => ({
      ...message,
      content: message.content.filter((block) => block.kind !== "reasoning"),
    })),
  });
  const body: Record<string, unknown> = { ...base };
  delete body.store;
  delete body.prompt_cache_retention;
  delete body.max_completion_tokens;
  body[route.output_limit] = input.output_tokens;
  if (streaming && route.stream_usage === "separate") body.stream_options = { include_usage: true };
  const assistants = input.messages.filter((message) => message.role === "assistant");
  let index = 0;
  body.messages = base.messages.map((value) => {
    const message = { ...vendor_record(value) };
    if (Array.isArray(message.content)) {
      if (message.content.every((part) => vendor_record(part).type === "text"))
        message.content = message.content.map((part) => vendor_record(part).text).join("");
      else
        for (const value of message.content) {
          const part = vendor_record(value);
          if (part.type === "image_url") {
            const image = vendor_record(part.image_url);
            if (image.detail !== "auto") throw new ProviderError("compatible_image_denied");
            delete image.detail;
          }
        }
    }
    if (message.role !== "assistant") return message;
    const previous = assistants[index++];
    if (!previous) throw new ProviderError("compatible_history_denied");
    const thoughts = previous.content.filter((block) => block.kind === "reasoning");
    if (thoughts.length > 1) throw new ProviderError("compatible_history_denied");
    const thought = thoughts[0];
    if (thought) {
      const state = vendor_record(thought.state);
      if (
        route.reasoning_field === "none" ||
        state.protocol !== "loop.vendor-chat/v1" ||
        state.field !== route.reasoning_field ||
        state.text !== thought.text ||
        state.parts !== undefined
      )
        throw new ProviderError("compatible_continuation_denied");
      message[route.reasoning_field] = thought.text;
    }
    return message;
  });
  if (!route.strict_tools && Array.isArray(body.tools))
    for (const tool of body.tools) delete vendor_record(vendor_record(tool).function).strict;
  if (route.thinking === "effort")
    body.reasoning_effort =
      input.model.reasoning === "off"
        ? "none"
        : input.model.reasoning === "adaptive"
          ? "high"
          : input.model.reasoning;
  if (route.thinking === "template")
    body.chat_template_kwargs = { enable_thinking: input.model.reasoning !== "off" };
  if (input.model.plugin === "llamacpp")
    body.reasoning_format = route.reasoning_field === "none" ? "none" : "deepseek";
  if (input.model.plugin === "openrouter") {
    body.provider = {
      only: [route.gateway?.upstream_provider],
      allow_fallbacks: false,
      require_parameters: true,
    };
    body.reasoning = { enabled: false };
    body.plugins = [];
  }
  if (input.model.plugin === "litellm") {
    body.disable_fallbacks = true;
    body.num_retries = 0;
  }
  return body as unknown as OpenAI.Chat.Completions.ChatCompletionCreateParamsNonStreaming;
}

export function compatible_dialect(input: NativeInput): ChatDialect {
  const route = input.model.compatible;
  if (!route) throw new ProviderError("compatible_route_missing");
  return {
    parameters: compatible_parameters,
    chunks: (event) => vendor_chunks(event, input.model, route),
    reply: (value, request) => normalized_reply(value, request, undefined, route),
  };
}
