import type Anthropic from "@anthropic-ai/sdk";
import { Code } from "@connectrpc/connect";
import { ErrorCategory } from "@loop-engine/protocol/provider";
import { ProviderError } from "./errors.js";
import { parse_json } from "./json.js";
import { type NativeBlock, type NativeInput, type NativeReply, token_count } from "./native.js";

export function invalid_anthropic(): never {
  throw new ProviderError("invalid_anthropic_output", Code.DataLoss, ErrorCategory.DEPENDENCY);
}

export function anthropic_parameters(
  input: NativeInput,
): Omit<Anthropic.Messages.MessageCreateParamsNonStreaming, "stream"> {
  const system: Anthropic.Messages.TextBlockParam[] = [];
  const messages: Anthropic.Messages.MessageParam[] = [];
  for (const message of input.messages) {
    if (message.role === "system") {
      for (const block of message.content) {
        if (block.kind !== "text") invalid_anthropic();
        system.push({ type: "text", text: block.text });
      }
      continue;
    }
    const content: Anthropic.Messages.ContentBlockParam[] = message.content.map((block) => {
      if (block.kind === "text" || block.kind === "refusal")
        return { type: "text", text: block.text };
      if (block.kind === "tool_call")
        return {
          type: "tool_use",
          id: block.id,
          name: block.name,
          input: parse_json(block.arguments),
        };
      if (block.kind === "tool_result")
        return {
          type: "tool_result",
          tool_use_id: block.id,
          is_error: block.error,
          content: block.text,
        };
      if (block.kind === "reasoning")
        return block.state as unknown as
          | Anthropic.Messages.ThinkingBlockParam
          | Anthropic.Messages.RedactedThinkingBlockParam;
      if (block.kind === "image")
        return {
          type: "image",
          source: { type: "base64", media_type: block.media, data: block.data },
        };
      if (block.kind === "document")
        return {
          type: "document",
          source: { type: "base64", media_type: "application/pdf", data: block.data },
        };
      return invalid_anthropic();
    });
    messages.push({
      role: message.role === "tool" ? "user" : message.role,
      content: content.length === 1 && content[0]?.type === "text" ? content[0].text : content,
    });
  }
  const structured = input.structured;
  const parallel = !input.model.features.parallel_tools;
  const choice: Anthropic.Messages.ToolChoice =
    typeof input.choice === "object"
      ? { type: "tool", name: input.choice.name, disable_parallel_tool_use: parallel }
      : input.choice === "none"
        ? { type: "none" }
        : {
            type: input.choice === "required" ? "any" : "auto",
            disable_parallel_tool_use: parallel,
          };
  if (
    input.model.reasoning === "enabled" &&
    (input.model.thinking_tokens ?? 0) >= input.output_tokens
  )
    throw new ProviderError("provider_thinking_budget");
  return {
    model: input.model.model,
    max_tokens: input.output_tokens,
    system,
    messages,
    ...(input.tools.length
      ? {
          tools: input.tools.map((tool) => ({
            name: tool.name,
            description: tool.description,
            strict: tool.strict,
            input_schema: { ...tool.schema.value, type: "object" as const },
          })),
          tool_choice: choice,
        }
      : {}),
    ...(structured
      ? { output_config: { format: { type: "json_schema", schema: structured.schema.value } } }
      : {}),
    ...(input.model.reasoning === "adaptive"
      ? { thinking: { type: "adaptive", display: "summarized" } as const }
      : input.model.reasoning === "enabled"
        ? {
            thinking: {
              type: "enabled",
              budget_tokens: input.model.thinking_tokens ?? 0,
              display: "summarized",
            } as const,
          }
        : {}),
    ...(input.model.features.prompt_caching
      ? { cache_control: { type: "ephemeral", ttl: "5m" } as const }
      : {}),
  };
}

export function anthropic_result(
  reply: Anthropic.Messages.Message,
  input: NativeInput,
  fragments?: ReadonlyMap<number, string>,
): NativeReply {
  const finish = reply.stop_reason;
  if (
    reply.model !== input.model.model ||
    reply.role !== "assistant" ||
    !reply.usage ||
    !Array.isArray(reply.content) ||
    !["end_turn", "stop_sequence", "max_tokens", "refusal", "tool_use"].includes(finish ?? "")
  )
    invalid_anthropic();
  const blocks: NativeBlock[] = reply.content.map((block, index) => {
    if (block.type === "text" && typeof block.text === "string")
      return { kind: finish === "refusal" ? "refusal" : "text", text: block.text };
    if (
      block.type === "tool_use" &&
      typeof block.id === "string" &&
      typeof block.name === "string"
    ) {
      const args = fragments?.get(index) ?? JSON.stringify(block.input);
      parse_json(args);
      return { kind: "tool_call", id: block.id, name: block.name, arguments: args };
    }
    if (
      block.type === "thinking" &&
      typeof block.thinking === "string" &&
      typeof block.signature === "string" &&
      block.signature
    )
      return { kind: "reasoning", text: block.thinking, state: parse_json(JSON.stringify(block)) };
    if (block.type === "redacted_thinking" && typeof block.data === "string" && block.data)
      return { kind: "reasoning", text: "", state: parse_json(JSON.stringify(block)) };
    return invalid_anthropic();
  });
  const created = token_count(reply.usage.cache_creation_input_tokens ?? 0);
  const cached = token_count(reply.usage.cache_read_input_tokens ?? 0);
  if (created && !input.model.features.prompt_caching)
    throw new ProviderError("unexpected_cache_write", Code.DataLoss, ErrorCategory.DEPENDENCY);
  if (
    reply.usage.cache_creation &&
    (token_count(reply.usage.cache_creation.ephemeral_1h_input_tokens) !== 0 ||
      token_count(reply.usage.cache_creation.ephemeral_5m_input_tokens) !== created)
  )
    invalid_anthropic();
  return {
    blocks,
    finish:
      finish === "max_tokens"
        ? "length"
        : finish === "refusal"
          ? "refusal"
          : finish === "tool_use"
            ? "tool_call"
            : "stop",
    usage: {
      input: token_count(reply.usage.input_tokens) + cached + created,
      output: token_count(reply.usage.output_tokens),
      cached,
      created,
      // Messages reports combined output, not a separate measured thinking count.
      reasoning: 0,
    },
  };
}
