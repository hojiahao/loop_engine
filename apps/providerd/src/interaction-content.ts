import type { Interactions } from "@google/genai";
import { ProviderError } from "./errors.js";
import { invalid_google } from "./google-content.js";
import { parse_json } from "./json.js";
import { type NativeBlock, type NativeInput, type NativeReply, token_count } from "./native.js";

export function interaction_parameters(
  input: NativeInput,
): Interactions.CreateModelInteractionParamsNonStreaming {
  const system: string[] = [];
  const steps: Interactions.Step[] = [];
  for (const message of input.messages) {
    if (message.role === "system") {
      for (const block of message.content) {
        if (block.kind !== "text") throw new ProviderError("google_content_denied");
        system.push(block.text);
      }
      continue;
    }
    let content: Interactions.Content[] = [];
    function flush() {
      if (content.length)
        steps.push({ type: message.role === "assistant" ? "model_output" : "user_input", content });
      content = [];
    }
    for (const block of message.content) {
      if (block.kind === "text" || block.kind === "refusal")
        content.push({ type: "text", text: block.text });
      else if (block.kind === "image" || block.kind === "document") {
        if (block.kind === "image" && block.detail !== "auto")
          throw new ProviderError("google_image_detail_denied");
        content.push({ type: block.kind, mime_type: block.media, data: block.data });
      } else {
        flush();
        if (block.kind === "tool_call")
          steps.push({
            type: "function_call",
            id: block.id,
            name: block.name,
            arguments: parse_json(block.arguments) as Record<string, unknown>,
          });
        else if (block.kind === "tool_result")
          steps.push({
            type: "function_result",
            call_id: block.id,
            is_error: block.error,
            result: block.text,
          });
        else if (block.kind === "reasoning") {
          const state = block.state;
          if (
            !state ||
            typeof state !== "object" ||
            Array.isArray(state) ||
            state.type !== "thought"
          )
            throw new ProviderError("google_continuation_denied");
          steps.push(state as Interactions.ThoughtStep);
        } else throw new ProviderError("google_content_denied");
      }
    }
    flush();
  }
  const choice =
    typeof input.choice === "object"
      ? { allowed_tools: { mode: "any" as const, tools: [input.choice.name] } }
      : input.choice === "required"
        ? "any"
        : input.choice === "auto" && input.tools.some((tool) => tool.strict)
          ? "validated"
          : input.choice;
  return {
    model: input.model.model,
    input: steps,
    store: false,
    background: false,
    system_instruction: system.join("\n\n"),
    generation_config: {
      max_output_tokens: input.output_tokens,
      ...(input.tools.length ? { tool_choice: choice } : {}),
      ...(input.model.reasoning !== "off"
        ? { thinking_level: input.model.reasoning, thinking_summaries: "auto" }
        : { thinking_summaries: "none" }),
    },
    ...(input.tools.length
      ? {
          tools: input.tools.map((tool) => ({
            type: "function" as const,
            name: tool.name,
            description: tool.description,
            parameters: tool.schema.value,
          })),
        }
      : {}),
    ...(input.structured
      ? {
          response_format: {
            type: "text" as const,
            mime_type: "application/json",
            schema: input.structured.schema.value,
          },
        }
      : {}),
  };
}

export function interaction_blocks(steps: readonly Interactions.Step[]): NativeBlock[] {
  const blocks: NativeBlock[] = [];
  for (const step of steps) {
    if (step.type === "model_output") {
      if (step.error || !Array.isArray(step.content)) invalid_google();
      for (const part of step.content) {
        if (part.type !== "text" || typeof part.text !== "string" || part.annotations?.length)
          invalid_google();
        blocks.push({ kind: "text", text: part.text });
      }
    } else if (step.type === "function_call") {
      if (
        !step.id ||
        !step.name ||
        !step.arguments ||
        typeof step.arguments !== "object" ||
        Array.isArray(step.arguments)
      )
        invalid_google();
      blocks.push({
        kind: "tool_call",
        id: step.id,
        name: step.name,
        arguments: JSON.stringify(step.arguments),
      });
    } else if (step.type === "thought") {
      if (
        !step.signature ||
        (step.summary ?? []).some((part) => part.type !== "text" || typeof part.text !== "string")
      )
        invalid_google();
      const text = (step.summary ?? [])
        .map((part) => (part.type === "text" ? part.text : ""))
        .join("");
      blocks.push({ kind: "reasoning", text, state: parse_json(JSON.stringify(step)) });
    } else invalid_google();
  }
  return blocks;
}

export function interaction_result(
  reply: Interactions.Interaction,
  input: NativeInput,
  ordered?: readonly NativeBlock[],
): NativeReply {
  if (
    !reply.id ||
    reply.model !== input.model.model ||
    reply.agent ||
    !Array.isArray(reply.steps) ||
    !reply.usage ||
    !["completed", "requires_action", "incomplete"].includes(reply.status)
  )
    invalid_google();
  const usage = reply.usage;
  const incoming =
    token_count(usage.total_input_tokens) + token_count(usage.total_tool_use_tokens ?? 0);
  const reasoning = token_count(usage.total_thought_tokens ?? 0);
  const outgoing = token_count(usage.total_output_tokens) + reasoning;
  if (token_count(usage.total_tokens) !== incoming + outgoing || usage.grounding_tool_count?.length)
    invalid_google();
  return {
    blocks: ordered ?? interaction_blocks(reply.steps),
    finish:
      reply.status === "requires_action"
        ? "tool_call"
        : reply.status === "incomplete"
          ? "length"
          : "stop",
    usage: {
      input: incoming,
      output: outgoing,
      reasoning,
      created: 0,
      cached: token_count(usage.total_cached_tokens ?? 0),
    },
  };
}
