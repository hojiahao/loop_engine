import type OpenAI from "openai";
import type { ModelRoute } from "./config.js";
import { ProviderError } from "./errors.js";
import { type JsonValue, parse_json } from "./json.js";
import { type NativeBlock, type NativeInput, type NativeReply, token_count } from "./native.js";
import { chat_result } from "./openai-content.js";
import { VENDORS, vendor_id } from "./vendor-registry.js";

export function vendor_record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new ProviderError("invalid_vendor_output");
  return value as Record<string, unknown>;
}

export function reasoning_field(model: ModelRoute): string {
  if (!vendor_id(model.plugin)) throw new ProviderError("invalid_vendor_route");
  return model.vendor?.reasoning_field ?? VENDORS[model.plugin].reasoning_field;
}

function measured_count(primary: unknown, alternate: unknown): number {
  if (primary !== undefined && alternate !== undefined && primary !== alternate)
    throw new ProviderError("conflicting_vendor_usage");
  return token_count(primary ?? alternate ?? 0);
}

/** Validate the implemented Mistral thinking variant without discarding signatures. */
export function thinking_parts(value: unknown): Record<string, JsonValue>[] {
  if (!Array.isArray(value) || !value.length || value.length > 256)
    throw new ProviderError("invalid_vendor_thinking");
  return value.map((value) => {
    const part = vendor_record(value);
    if (
      part.type !== "thinking" ||
      !Array.isArray(part.thinking) ||
      Object.keys(part).some((key) => !["type", "thinking", "signature", "closed"].includes(key)) ||
      (part.signature != null && typeof part.signature !== "string") ||
      (part.closed !== undefined && typeof part.closed !== "boolean")
    )
      throw new ProviderError("invalid_vendor_thinking");
    for (const value of part.thinking) {
      const chunk = vendor_record(value);
      if (
        chunk.type !== "text" ||
        typeof chunk.text !== "string" ||
        Object.keys(chunk).some((key) => !["type", "text"].includes(key))
      )
        throw new ProviderError("unsupported_vendor_thinking");
    }
    return parse_json(JSON.stringify(part), 524_288, 64) as Record<string, JsonValue>;
  });
}

export function thinking_text(parts: readonly Record<string, JsonValue>[]): string {
  return parts
    .flatMap((part) => part.thinking as { text: string }[])
    .map((part) => part.text)
    .join("");
}

function vendor_usage(value: unknown, model: ModelRoute): OpenAI.CompletionUsage {
  const usage = vendor_record(value);
  const prompt = token_count(usage.prompt_tokens);
  const completion = token_count(usage.completion_tokens);
  if (token_count(usage.total_tokens) !== prompt + completion)
    throw new ProviderError("invalid_vendor_usage");
  const prompt_details =
    usage.prompt_tokens_details == null ? {} : vendor_record(usage.prompt_tokens_details);
  const completion_details =
    usage.completion_tokens_details == null ? {} : vendor_record(usage.completion_tokens_details);
  const cached = measured_count(
    prompt_details.cached_tokens,
    model.plugin === "deepseek" ? usage.prompt_cache_hit_tokens : usage.cached_tokens,
  );
  const reasoning = measured_count(completion_details.reasoning_tokens, usage.reasoning_tokens);
  if (
    cached > prompt ||
    reasoning > completion ||
    (usage.prompt_cache_miss_tokens !== undefined &&
      token_count(usage.prompt_cache_miss_tokens) + cached !== prompt)
  )
    throw new ProviderError("invalid_vendor_usage");
  if (
    model.plugin === "perplexity" &&
    (token_count(usage.citation_tokens ?? 0) !== 0 ||
      token_count(usage.num_search_queries ?? 0) !== 0)
  )
    throw new ProviderError("unexpected_vendor_search");
  return {
    prompt_tokens: prompt,
    completion_tokens: completion,
    total_tokens: prompt + completion,
    prompt_tokens_details: { cached_tokens: cached },
    completion_tokens_details: { reasoning_tokens: reasoning },
  };
}

function vendor_message(value: unknown, model: ModelRoute): Record<string, unknown> {
  const message = { ...vendor_record(value) };
  const field = reasoning_field(model);
  let thought = message[field];
  if (field === "content") {
    thought = undefined;
    if (Array.isArray(message.content)) {
      let text = "";
      const parts: Record<string, JsonValue>[] = [];
      for (const item of message.content) {
        const part = vendor_record(item);
        if (
          part.type === "text" &&
          typeof part.text === "string" &&
          Object.keys(part).every((key) => ["type", "text"].includes(key))
        )
          text += part.text;
        else if (part.type === "thinking") {
          if (text) throw new ProviderError("unsupported_vendor_content_order");
          parts.push(...thinking_parts([part]));
        } else throw new ProviderError("unsupported_vendor_content");
      }
      message.content = text || null;
      if (parts.length) {
        thought = thinking_text(parts);
        message.reasoning_parts = parts;
      }
    }
  }
  for (const name of ["reasoning", "reasoning_content"]) {
    if (name !== field && message[name] !== undefined && message[name] !== null)
      throw new ProviderError("unexpected_vendor_thinking");
    delete message[name];
  }
  if (thought !== undefined && thought !== null) {
    if (typeof thought !== "string" || (thought && model.reasoning === "off"))
      throw new ProviderError("unexpected_vendor_thinking");
    message.reasoning_content = thought;
  }
  for (const key of ["audio", "annotations", "citations", "images", "function_call"])
    if (
      message[key] !== undefined &&
      message[key] !== null &&
      (!Array.isArray(message[key]) || message[key].length)
    )
      throw new ProviderError("unsupported_vendor_content");
  return message;
}

function vendor_envelope(value: unknown): Record<string, unknown> {
  const reply = vendor_record(value);
  if (reply.error) throw new ProviderError("vendor_error_response");
  for (const key of ["citations", "search_results", "web_search", "images", "related_questions"])
    if (reply[key] !== undefined && (!Array.isArray(reply[key]) || reply[key].length))
      throw new ProviderError("unsupported_vendor_content");
  return reply;
}

function vendor_finish(value: unknown, model: ModelRoute): unknown {
  if (model.plugin === "glm" && value === "sensitive") return "content_filter";
  return value;
}

/** Normalized reasoning is still private continuation state, not ordinary text. */
export function vendor_reply(
  value: OpenAI.Chat.Completions.ChatCompletion,
  input: NativeInput,
): NativeReply {
  const reply = vendor_envelope(value);
  if (!Array.isArray(reply.choices) || reply.choices.length !== 1)
    throw new ProviderError("invalid_vendor_output");
  const choice = vendor_record(reply.choices[0]);
  const message = vendor_message(choice.message, input.model);
  return normalized_reply(
    {
      ...reply,
      usage: vendor_usage(reply.usage, input.model),
      choices: [
        {
          ...choice,
          message: { content: null, ...message },
          finish_reason: vendor_finish(choice.finish_reason, input.model),
        },
      ],
    } as unknown as OpenAI.Chat.Completions.ChatCompletion,
    input,
  );
}

export function normalized_reply(
  value: OpenAI.Chat.Completions.ChatCompletion,
  input: NativeInput,
  parts?: readonly Record<string, JsonValue>[],
): NativeReply {
  const reply = chat_result(value, input);
  const message = vendor_record(value.choices[0]?.message);
  const text = message.reasoning_content;
  const raw = parts ?? message.reasoning_parts;
  const thinking = raw === undefined ? undefined : thinking_parts(raw);
  if (thinking && thinking_text(thinking) !== (text ?? ""))
    throw new ProviderError("invalid_vendor_thinking");
  if ((text === undefined || text === "" || text === null) && !thinking) return reply;
  if (typeof text !== "string" || input.model.reasoning === "off")
    throw new ProviderError("unexpected_vendor_thinking");
  const reasoning: NativeBlock = {
    kind: "reasoning",
    text,
    state: {
      protocol: "loop.vendor-chat/v1",
      field: reasoning_field(input.model),
      text,
      ...(thinking ? { parts: thinking } : {}),
    },
  };
  return { ...reply, blocks: [reasoning, ...reply.blocks] };
}

/** Split terminal content+usage into the shared lifecycle's two explicit steps.
 * Mistral/Groq/Cerebras may carry usage on the terminal content event. */
export function vendor_chunks(
  value: OpenAI.Chat.Completions.ChatCompletionChunk,
  model: ModelRoute,
): readonly OpenAI.Chat.Completions.ChatCompletionChunk[] {
  if (!vendor_id(model.plugin)) throw new ProviderError("invalid_vendor_route");
  const source = vendor_envelope(value);
  if (!Array.isArray(source.choices) || source.choices.length > 1)
    throw new ProviderError("invalid_vendor_output");
  const choice = source.choices[0] === undefined ? undefined : vendor_record(source.choices[0]);
  const groq_usage = source.x_groq == null ? undefined : vendor_record(source.x_groq).usage;
  if (
    source.usage != null &&
    groq_usage != null &&
    JSON.stringify(vendor_usage(source.usage, model)) !==
      JSON.stringify(vendor_usage(groq_usage, model))
  )
    throw new ProviderError("conflicting_vendor_usage");
  const usage = source.usage ?? groq_usage;
  const result: Record<string, unknown>[] = [];
  if (choice) {
    result.push({
      ...source,
      usage: null,
      choices: [
        {
          ...choice,
          delta: vendor_message(choice.delta, model),
          finish_reason: vendor_finish(choice.finish_reason, model),
        },
      ],
    });
    if (usage != null && !choice.finish_reason) {
      if (VENDORS[model.plugin].stream_usage !== "terminal")
        throw new ProviderError("early_vendor_usage");
      vendor_usage(usage, model);
    }
  }
  if (usage != null && (!choice || choice.finish_reason))
    result.push({ ...source, choices: [], usage: vendor_usage(usage, model) });
  if (!result.length) throw new ProviderError("invalid_vendor_output");
  return result as unknown as OpenAI.Chat.Completions.ChatCompletionChunk[];
}
