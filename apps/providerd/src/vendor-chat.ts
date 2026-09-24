import type OpenAI from "openai";
import { ProviderError } from "./errors.js";
import type { JsonValue } from "./json.js";
import type { NativeInput } from "./native.js";
import { chat_parameters } from "./openai-content.js";
import type { ChatDialect } from "./openai-stream.js";
import { VENDORS, vendor_id } from "./vendor-registry.js";
import {
  normalized_reply,
  reasoning_field,
  thinking_parts,
  thinking_text,
  vendor_chunks,
  vendor_record,
} from "./vendor-replies.js";

export function vendor_parameters(
  input: NativeInput,
  streaming: boolean,
): OpenAI.Chat.Completions.ChatCompletionCreateParamsNonStreaming {
  const model = input.model;
  if (!vendor_id(model.plugin) || VENDORS[model.plugin].wire !== "chat")
    throw new ProviderError("invalid_vendor_route");
  const profile = VENDORS[model.plugin];
  if (!profile.strict && input.tools.some((tool) => tool.strict))
    throw new ProviderError("vendor_strict_tool_denied");
  const base = chat_parameters({
    ...input,
    messages: input.messages.map((message) => ({
      ...message,
      content: message.content.filter((block) => block.kind !== "reasoning"),
    })),
  });
  const parameters: Record<string, unknown> = { ...base };
  delete parameters.store;
  delete parameters.prompt_cache_retention;
  delete parameters.max_completion_tokens;
  parameters[
    model.plugin === "groq" || model.plugin === "cerebras" ? "max_completion_tokens" : "max_tokens"
  ] = input.output_tokens;
  if (streaming && profile.stream_usage === "separate")
    parameters.stream_options = { include_usage: true };
  if (!profile.parallel) delete parameters.parallel_tool_calls;
  const messages = base.messages.map((message) => ({ ...vendor_record(message) }));
  parameters.messages = messages;
  const assistants = input.messages.filter((message) => message.role === "assistant");
  let assistant = 0;
  for (const message of messages) {
    if (Array.isArray(message.content)) {
      if (message.content.every((block: unknown) => vendor_record(block).type === "text"))
        message.content = message.content
          .map((block: unknown) => vendor_record(block).text)
          .join("");
      else
        for (const value of message.content) {
          const block = vendor_record(value);
          if (block.type === "image_url") {
            const image = vendor_record(block.image_url);
            if (image.detail !== "auto") throw new ProviderError("vendor_image_detail_denied");
            delete image.detail;
          }
        }
    }
    if (message.role !== "assistant") continue;
    const previous = assistants[assistant++];
    if (!previous) throw new ProviderError("invalid_vendor_history");
    const thoughts = previous.content.filter((block) => block.kind === "reasoning");
    if (thoughts.length > 1) throw new ProviderError("invalid_vendor_history");
    const thought = thoughts[0];
    if (thought) {
      const state = vendor_record(thought.state);
      const field = reasoning_field(model);
      if (
        state.protocol !== "loop.vendor-chat/v1" ||
        state.field !== field ||
        state.text !== thought.text
      )
        throw new ProviderError("vendor_continuation_denied");
      if (field === "content") {
        const parts = thinking_parts(state.parts);
        if (thinking_text(parts) !== thought.text)
          throw new ProviderError("vendor_continuation_denied");
        message.content = [
          ...parts,
          ...(typeof message.content === "string" && message.content
            ? [{ type: "text", text: message.content }]
            : []),
        ];
      } else message[field] = thought.text;
    }
    if (model.plugin === "deepseek" && message.content === null) message.content = "";
  }
  if (!profile.strict && Array.isArray(parameters.tools))
    for (const value of parameters.tools)
      delete vendor_record(vendor_record(value).function).strict;
  const thinking = model.reasoning !== "off";
  const effort = ["low", "medium", "high", "max"].includes(model.reasoning)
    ? model.reasoning
    : undefined;
  switch (model.plugin) {
    case "mistral":
      parameters.reasoning_effort = effort ?? (thinking ? "high" : "none");
      parameters.service_tier = "standard_only";
      break;
    case "deepseek":
      parameters.thinking = { type: thinking ? "enabled" : "disabled" };
      if (effort) parameters.reasoning_effort = effort;
      if (thinking && input.tools.length) {
        if (input.choice !== "auto") throw new ProviderError("vendor_tool_choice_denied");
        delete parameters.tool_choice;
      }
      delete parameters.parallel_tool_calls;
      break;
    case "qwen":
      if (input.output_tokens <= 10) throw new ProviderError("vendor_output_budget_denied");
      delete parameters.max_tokens;
      // DashScope documents a possible ten-token overshoot. Its newer complete
      // output limit includes thinking, unlike legacy max_tokens on Qwen.
      parameters.max_completion_tokens = input.output_tokens - 10;
      parameters.enable_thinking = thinking;
      parameters.enable_search = false;
      if (effort) parameters.reasoning_effort = effort;
      if (
        input.structured &&
        input.messages.some((message) => message.content.some((block) => block.kind === "image"))
      )
        throw new ProviderError("vendor_schema_media_denied");
      break;
    case "groq":
      if (model.model.startsWith("openai/gpt-oss")) parameters.include_reasoning = thinking;
      else parameters.reasoning_format = thinking ? "parsed" : "hidden";
      if (effort) parameters.reasoning_effort = effort;
      break;
    case "together":
      if (effort) parameters.reasoning_effort = effort;
      else parameters.reasoning = { enabled: thinking };
      if (thinking && model.model.startsWith("zai-org/"))
        parameters.chat_template_kwargs = { clear_thinking: false };
      break;
    case "fireworks":
      parameters.context_length_exceeded_behavior = "error";
      parameters.reasoning_effort = effort ?? thinking;
      if (thinking) parameters.thinking = { type: "enabled", keep: "all" };
      break;
    case "cerebras":
      parameters.reasoning_format = thinking ? "parsed" : "hidden";
      if (effort) parameters.reasoning_effort = effort;
      break;
    case "perplexity":
      parameters.disable_search = true;
      parameters.return_images = false;
      parameters.return_related_questions = false;
      delete parameters.n;
      break;
    case "glm":
      parameters.thinking = { type: thinking ? "enabled" : "disabled", clear_thinking: false };
      if (effort) parameters.reasoning_effort = effort;
      if (input.tools.length && input.choice !== "auto")
        throw new ProviderError("vendor_tool_choice_denied");
      if (streaming && input.tools.length) parameters.tool_stream = true;
      delete parameters.n;
      break;
    case "kimi":
      if (model.model.startsWith("kimi-k3")) {
        if (!thinking || model.reasoning === "medium")
          throw new ProviderError("vendor_thinking_denied");
        if (effort) parameters.reasoning_effort = effort;
      } else {
        if (effort || (model.model.startsWith("kimi-k2.7") && !thinking))
          throw new ProviderError("vendor_thinking_denied");
        parameters.thinking = {
          type: thinking ? "enabled" : "disabled",
          ...(thinking ? { keep: "all" } : {}),
        };
        if (input.tools.length && (input.choice === "required" || typeof input.choice === "object"))
          throw new ProviderError("vendor_tool_choice_denied");
      }
      delete parameters.parallel_tool_calls;
      break;
  }
  return parameters as unknown as OpenAI.Chat.Completions.ChatCompletionCreateParamsNonStreaming;
}

export function vendor_dialect(input: NativeInput): ChatDialect {
  const parts: Record<string, JsonValue>[] = [];
  let answer = false;
  return {
    parameters: vendor_parameters,
    chunks(event) {
      const chunks = vendor_chunks(event, input.model);
      for (const chunk of chunks) {
        const delta = chunk.choices[0]?.delta;
        if (!delta) continue;
        const raw = vendor_record(delta).reasoning_parts;
        if (raw !== undefined) {
          if (answer) throw new ProviderError("unsupported_vendor_content_order");
          for (const part of thinking_parts(raw)) {
            const previous = parts.at(-1);
            if (previous && previous.closed !== true) {
              previous.thinking = [{ type: "text", text: thinking_text([previous, part]) }];
              if (typeof part.signature === "string")
                previous.signature =
                  (typeof previous.signature === "string" ? previous.signature : "") +
                  part.signature;
              if (part.closed !== undefined) previous.closed = part.closed;
            } else parts.push(part);
          }
        }
        if (delta.content) answer = true;
      }
      return chunks;
    },
    reply: (value, request) => normalized_reply(value, request, parts.length ? parts : undefined),
  };
}
