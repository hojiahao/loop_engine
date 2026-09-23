import { Code } from "@connectrpc/connect";
import { ErrorCategory } from "@loop-engine/protocol/provider";
import type OpenAI from "openai";
import { ProviderError } from "./errors.js";
import { parse_json } from "./json.js";
import { type NativeBlock, type NativeInput, type NativeReply, token_count } from "./native.js";

type ResponseItem = OpenAI.Responses.ResponseInputItem;
type ChatMessage = OpenAI.Chat.Completions.ChatCompletionMessageParam;

export function invalid_openai(): never {
  throw new ProviderError("invalid_openai_output", Code.DataLoss, ErrorCategory.DEPENDENCY);
}

export function response_parameters(
  input: NativeInput,
): Omit<OpenAI.Responses.ResponseCreateParamsNonStreaming, "stream"> {
  const messages: ResponseItem[] = [];
  for (const message of input.messages) {
    const parts: OpenAI.Responses.ResponseInputContent[] = [];
    const flush_message = () => {
      if (!parts.length) return;
      if (message.role === "tool") invalid_openai();
      messages.push({ role: message.role, content: parts.splice(0) });
    };
    for (const block of message.content) {
      if (["tool_call", "tool_result", "reasoning"].includes(block.kind)) flush_message();
      if (block.kind === "tool_call")
        messages.push({
          type: "function_call",
          call_id: block.id,
          name: block.name,
          arguments: block.arguments,
        });
      else if (block.kind === "tool_result")
        messages.push({
          type: "function_call_output",
          call_id: block.id,
          output: block.error
            ? JSON.stringify({ status: "error", content: block.text })
            : block.text,
        });
      else if (block.kind === "reasoning")
        messages.push(block.state as unknown as OpenAI.Responses.ResponseReasoningItem);
      else {
        if (message.role === "tool") invalid_openai();
        const content: OpenAI.Responses.ResponseInputContent =
          block.kind === "image"
            ? {
                type: "input_image",
                detail: block.detail,
                image_url: `data:${block.media};base64,${block.data}`,
              }
            : block.kind === "document"
              ? {
                  type: "input_file",
                  filename: "prompt.pdf",
                  file_data: `data:application/pdf;base64,${block.data}`,
                }
              : { type: "input_text", text: block.text };
        parts.push(content);
      }
    }
    flush_message();
  }
  const tools: OpenAI.Responses.Tool[] = input.tools.map((tool) => ({
    type: "function",
    name: tool.name,
    description: tool.description,
    strict: tool.strict,
    parameters: tool.schema.value,
  }));
  const structured = input.structured;
  return {
    model: input.model.model,
    input: messages,
    max_output_tokens: input.output_tokens,
    store: false,
    ...(tools.length
      ? {
          tools,
          tool_choice:
            typeof input.choice === "object"
              ? { type: "function", name: input.choice.name }
              : input.choice,
          parallel_tool_calls: input.model.features.parallel_tools,
        }
      : {}),
    ...(structured
      ? {
          text: {
            format: {
              type: "json_schema",
              name: structured.name,
              description: structured.description,
              strict: structured.strict,
              schema: structured.schema.value,
            },
          },
        }
      : {}),
    ...(["low", "medium", "high"].includes(input.model.reasoning)
      ? {
          reasoning: {
            effort: input.model.reasoning as "low" | "medium" | "high",
            summary: "auto",
          },
          include: ["reasoning.encrypted_content" as const],
        }
      : {}),
    ...(input.model.features.prompt_caching
      ? { prompt_cache_retention: "in_memory" as const }
      : {}),
  };
}

export function chat_parameters(
  input: NativeInput,
): Omit<OpenAI.Chat.Completions.ChatCompletionCreateParamsNonStreaming, "stream"> {
  const messages: ChatMessage[] = [];
  for (const message of input.messages) {
    if (message.role === "tool") {
      for (const block of message.content) {
        if (block.kind !== "tool_result") invalid_openai();
        messages.push({
          role: "tool",
          tool_call_id: block.id,
          content: block.error
            ? JSON.stringify({ status: "error", content: block.text })
            : block.text,
        });
      }
    } else if (message.role === "assistant") {
      const calls: OpenAI.Chat.Completions.ChatCompletionMessageFunctionToolCall[] = [];
      const texts: string[] = [];
      let refusal: string | undefined;
      for (const block of message.content) {
        if (block.kind === "tool_call")
          calls.push({
            type: "function",
            id: block.id,
            function: { name: block.name, arguments: block.arguments },
          });
        else if (block.kind === "text") texts.push(block.text);
        else if (block.kind === "refusal") refusal = block.text;
        else invalid_openai();
      }
      messages.push({
        role: "assistant",
        content: texts.length ? texts.join("") : null,
        ...(calls.length ? { tool_calls: calls } : {}),
        ...(refusal ? { refusal } : {}),
      });
    } else if (message.role === "system") {
      messages.push({
        role: "system",
        content: message.content
          .map((block) => {
            if (block.kind !== "text") invalid_openai();
            return block.text;
          })
          .join(""),
      });
    } else {
      const content: OpenAI.Chat.Completions.ChatCompletionContentPart[] = message.content.map(
        (block) => {
          if (block.kind === "text") return { type: "text", text: block.text };
          if (block.kind === "image")
            return {
              type: "image_url",
              image_url: { url: `data:${block.media};base64,${block.data}`, detail: block.detail },
            };
          if (block.kind === "document")
            return {
              type: "file",
              file: {
                filename: "prompt.pdf",
                file_data: `data:application/pdf;base64,${block.data}`,
              },
            };
          return invalid_openai();
        },
      );
      messages.push({ role: "user", content });
    }
  }
  const structured = input.structured;
  return {
    model: input.model.model,
    messages,
    max_completion_tokens: input.output_tokens,
    store: false,
    n: 1,
    ...(input.tools.length
      ? {
          tools: input.tools.map((tool) => ({
            type: "function" as const,
            function: {
              name: tool.name,
              description: tool.description,
              strict: tool.strict,
              parameters: tool.schema.value,
            },
          })),
          tool_choice:
            typeof input.choice === "object"
              ? { type: "function", function: { name: input.choice.name } }
              : input.choice,
          parallel_tool_calls: input.model.features.parallel_tools,
        }
      : {}),
    ...(structured
      ? {
          response_format: {
            type: "json_schema",
            json_schema: {
              name: structured.name,
              description: structured.description,
              strict: structured.strict,
              schema: structured.schema.value,
            },
          },
        }
      : {}),
    ...(input.model.features.prompt_caching
      ? { prompt_cache_retention: "in_memory" as const }
      : {}),
  };
}

export function response_result(reply: OpenAI.Responses.Response, input: NativeInput): NativeReply {
  if (
    reply.model !== input.model.model ||
    !reply.usage ||
    !Array.isArray(reply.output) ||
    reply.error
  )
    invalid_openai();
  let finish: NativeReply["finish"] = "stop";
  if (reply.status === "incomplete" && reply.incomplete_details?.reason === "max_output_tokens")
    finish = "length";
  else if (reply.status !== "completed") invalid_openai();
  const blocks: NativeBlock[] = [];
  for (const item of reply.output) {
    if (item.type === "reasoning") {
      if (!Array.isArray(item.summary)) invalid_openai();
      if (item.summary.length === 0 && !item.encrypted_content) continue;
      if (
        typeof item.encrypted_content !== "string" ||
        !item.encrypted_content ||
        typeof item.id !== "string"
      )
        invalid_openai();
      const text = item.summary
        .map((part) => {
          if (part.type !== "summary_text" || typeof part.text !== "string") invalid_openai();
          return part.text;
        })
        .join("");
      blocks.push({ kind: "reasoning", text, state: parse_json(JSON.stringify(item)) });
    } else if (item.type === "function_call") {
      if (
        typeof item.arguments !== "string" ||
        typeof item.name !== "string" ||
        typeof item.call_id !== "string"
      )
        invalid_openai();
      blocks.push({
        kind: "tool_call",
        id: item.call_id,
        name: item.name,
        arguments: item.arguments,
      });
      if (finish === "stop") finish = "tool_call";
    } else if (
      item.type === "message" &&
      item.role === "assistant" &&
      Array.isArray(item.content)
    ) {
      for (const content of item.content) {
        if (content.type === "output_text" && typeof content.text === "string")
          blocks.push({ kind: "text", text: content.text });
        else if (content.type === "refusal" && typeof content.refusal === "string") {
          blocks.push({ kind: "refusal", text: content.refusal });
          finish = "refusal";
        } else invalid_openai();
      }
    } else invalid_openai();
  }
  return {
    blocks,
    finish,
    usage: {
      input: token_count(reply.usage.input_tokens),
      output: token_count(reply.usage.output_tokens),
      cached: token_count(reply.usage.input_tokens_details?.cached_tokens ?? 0),
      reasoning: token_count(reply.usage.output_tokens_details?.reasoning_tokens ?? 0),
      created: 0,
    },
  };
}

export function chat_result(
  reply: OpenAI.Chat.Completions.ChatCompletion,
  input: NativeInput,
): NativeReply {
  const choice = reply.choices?.[0];
  if (
    reply.model !== input.model.model ||
    reply.choices?.length !== 1 ||
    !choice ||
    !reply.usage ||
    choice.message.role !== "assistant" ||
    choice.message.function_call ||
    choice.message.audio
  )
    invalid_openai();
  const blocks: NativeBlock[] = [];
  if (choice.message.content !== null) blocks.push({ kind: "text", text: choice.message.content });
  if (choice.message.refusal) blocks.push({ kind: "refusal", text: choice.message.refusal });
  for (const call of choice.message.tool_calls ?? []) {
    if (
      call.type !== "function" ||
      typeof call.id !== "string" ||
      typeof call.function?.name !== "string" ||
      typeof call.function.arguments !== "string"
    )
      invalid_openai();
    blocks.push({
      kind: "tool_call",
      id: call.id,
      name: call.function.name,
      arguments: call.function.arguments,
    });
  }
  const finish = choice.finish_reason;
  if (!["stop", "length", "content_filter", "tool_calls"].includes(finish)) invalid_openai();
  return {
    blocks,
    finish:
      choice.message.refusal || finish === "content_filter"
        ? "refusal"
        : finish === "tool_calls"
          ? "tool_call"
          : (finish as "stop" | "length"),
    usage: {
      input: token_count(reply.usage.prompt_tokens),
      output: token_count(reply.usage.completion_tokens),
      cached: token_count(reply.usage.prompt_tokens_details?.cached_tokens ?? 0),
      reasoning: token_count(reply.usage.completion_tokens_details?.reasoning_tokens ?? 0),
      created: 0,
    },
  };
}
