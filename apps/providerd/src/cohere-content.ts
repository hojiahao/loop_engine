import { Code } from "@connectrpc/connect";
import { ErrorCategory } from "@loop-engine/protocol/provider";
import type { Cohere } from "cohere-ai";
import { ProviderError } from "./errors.js";
import { parse_json } from "./json.js";
import { type NativeBlock, type NativeInput, type NativeReply, token_count } from "./native.js";

export function invalid_cohere(): never {
  throw new ProviderError("invalid_cohere_output", Code.DataLoss, ErrorCategory.DEPENDENCY);
}

export function cohere_parameters(input: NativeInput): Cohere.V2ChatRequest {
  // These combinations have no lossless representation in V2 Chat.
  if (
    typeof input.choice === "object" ||
    (input.structured && input.tools.length) ||
    new Set(input.tools.map((tool) => tool.strict)).size > 1
  )
    throw new ProviderError("cohere_capability_denied");
  const messages: Cohere.ChatMessageV2[] = [];
  for (const message of input.messages) {
    if (message.role === "tool") {
      for (const block of message.content) {
        if (block.kind !== "tool_result") throw new ProviderError("cohere_content_denied");
        messages.push({
          role: "tool",
          toolCallId: block.id,
          content: JSON.stringify(block.error ? { error: block.text } : { output: block.text }),
        });
      }
    } else if (message.role === "assistant") {
      const content: Cohere.AssistantMessageV2ContentOneItem[] = [];
      const toolCalls: Cohere.ToolCallV2[] = [];
      for (const block of message.content) {
        if (block.kind === "text" || block.kind === "refusal")
          content.push({ type: "text", text: block.text });
        else if (block.kind === "tool_call")
          toolCalls.push({
            type: "function",
            id: block.id,
            function: { name: block.name, arguments: block.arguments },
          });
        else if (block.kind === "reasoning") {
          const state = block.state;
          if (
            !state ||
            typeof state !== "object" ||
            Array.isArray(state) ||
            state.type !== "thinking" ||
            state.thinking !== block.text
          )
            throw new ProviderError("cohere_continuation_denied");
          content.push({ type: "thinking", thinking: block.text });
        } else throw new ProviderError("cohere_content_denied");
      }
      messages.push({ role: "assistant", content, ...(toolCalls.length ? { toolCalls } : {}) });
    } else if (message.role === "system") {
      messages.push({
        role: "system",
        content: message.content
          .map((block) => {
            if (block.kind !== "text") throw new ProviderError("cohere_content_denied");
            return block.text;
          })
          .join("\n\n"),
      });
    } else {
      messages.push({
        role: "user",
        content: message.content.map((block): Cohere.Content => {
          if (block.kind === "text") return { type: "text", text: block.text };
          if (block.kind === "image")
            return {
              type: "image_url",
              imageUrl: { url: `data:${block.media};base64,${block.data}`, detail: block.detail },
            };
          throw new ProviderError("cohere_content_denied");
        }),
      });
    }
  }
  if (input.structured)
    messages.unshift({
      role: "system",
      content: "Return only a JSON object conforming to the supplied response schema.",
    });
  return {
    model: input.model.model,
    messages,
    maxTokens: input.output_tokens,
    ...(input.model.reasoning === "enabled"
      ? { thinking: { type: "enabled" as const, tokenBudget: input.model.thinking_tokens } }
      : {}),
    ...(input.tools.length
      ? {
          tools: input.tools.map((tool) => ({
            type: "function" as const,
            function: {
              name: tool.name,
              description: tool.description,
              parameters: tool.schema.value,
            },
          })),
          strictTools: input.tools[0]?.strict,
          ...(input.choice === "required"
            ? { toolChoice: "REQUIRED" as const }
            : input.choice === "none"
              ? { toolChoice: "NONE" as const }
              : {}),
        }
      : {}),
    ...(input.structured
      ? {
          responseFormat: {
            type: "json_object" as const,
            jsonSchema: input.structured.schema.value,
          },
        }
      : {}),
  };
}

export function cohere_result(
  reply: Cohere.V2ChatResponse,
  ordered?: readonly NativeBlock[],
): NativeReply {
  if (
    !reply.id ||
    reply.message?.role !== "assistant" ||
    !reply.usage?.tokens ||
    !["COMPLETE", "STOP_SEQUENCE", "MAX_TOKENS", "TOOL_CALL"].includes(reply.finishReason) ||
    reply.message.citations?.length
  )
    invalid_cohere();
  const blocks: NativeBlock[] = [];
  // Tool plans are ordinary, replayable assistant text, not signed reasoning state.
  if (reply.message.toolPlan) blocks.push({ kind: "text", text: reply.message.toolPlan });
  for (const block of reply.message.content ?? []) {
    if (block.type === "thinking" && typeof block.thinking === "string")
      blocks.push({
        kind: "reasoning",
        text: block.thinking,
        state: parse_json(JSON.stringify(block)),
      });
    else if (block.type === "text" && typeof block.text === "string")
      blocks.push({ kind: "text", text: block.text });
    else invalid_cohere();
  }
  for (const call of reply.message.toolCalls ?? []) {
    if (
      call.type !== "function" ||
      !call.id ||
      !call.function?.name ||
      typeof call.function.arguments !== "string"
    )
      invalid_cohere();
    parse_json(call.function.arguments);
    blocks.push({
      kind: "tool_call",
      id: call.id,
      name: call.function.name,
      arguments: call.function.arguments,
    });
  }
  return {
    blocks: ordered ?? blocks,
    finish:
      reply.finishReason === "TOOL_CALL"
        ? "tool_call"
        : reply.finishReason === "MAX_TOKENS"
          ? "length"
          : "stop",
    usage: {
      input: token_count(reply.usage.tokens.inputTokens),
      output: token_count(reply.usage.tokens.outputTokens),
      cached: token_count(reply.usage.cachedTokens ?? 0),
      created: 0,
      reasoning: 0,
    },
  };
}
