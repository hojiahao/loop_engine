import { createHash } from "node:crypto";
import { Code } from "@connectrpc/connect";
import type {
  Content,
  FunctionCallingConfigMode,
  GenerateContentParameters,
  GenerateContentResponse,
  Part,
  ThinkingLevel,
} from "@google/genai";
import { ErrorCategory } from "@loop-engine/protocol/provider";
import { ProviderError } from "./errors.js";
import { json_bytes, parse_json } from "./json.js";
import {
  type NativeBlock,
  type NativeContent,
  type NativeInput,
  type NativeReply,
  token_count,
} from "./native.js";

export function invalid_google(): never {
  throw new ProviderError("invalid_google_output", Code.DataLoss, ErrorCategory.DEPENDENCY);
}

function call_identity(part: Part, id: string, index: number): string {
  if (part.functionCall?.id) return part.functionCall.id;
  if (!id) invalid_google();
  return `gemini-local-${createHash("sha256").update(`${id}:${index}`).digest("hex").slice(0, 32)}`;
}

/** Preserve complete signed Parts, not just thought text, for Gemini tool turns. */
export function google_blocks(
  parts: readonly Part[],
  id: string,
  input: NativeInput,
): NativeBlock[] {
  const blocks: NativeBlock[] = [];
  let summary = "";
  for (const [index, part] of parts.entries()) {
    if (
      Object.keys(part).some(
        (key) => !["text", "thought", "thoughtSignature", "functionCall"].includes(key),
      ) ||
      (part.functionCall !== undefined && part.text !== undefined)
    )
      invalid_google();
    if (
      part.thoughtSignature !== undefined &&
      (typeof part.thoughtSignature !== "string" || !part.thoughtSignature)
    )
      invalid_google();
    if (part.thought === true) {
      if (typeof part.text !== "string" || part.functionCall) invalid_google();
      summary += part.text;
    } else if (typeof part.text === "string") {
      const previous = blocks.at(-1);
      if (previous?.kind === "text")
        blocks[blocks.length - 1] = { kind: "text", text: previous.text + part.text };
      else blocks.push({ kind: "text", text: part.text });
    } else if (part.functionCall) {
      const call = part.functionCall;
      if (
        !call.name ||
        call.partialArgs ||
        call.willContinue !== undefined ||
        (call.id?.startsWith("gemini-local-") ?? false)
      )
        invalid_google();
      blocks.push({
        kind: "tool_call",
        id: call_identity(part, id, index),
        name: call.name,
        arguments: JSON.stringify(call.args ?? {}),
      });
    } else if (!part.thoughtSignature) invalid_google();
  }
  if (input.model.reasoning !== "off")
    blocks.unshift({
      kind: "reasoning",
      text: summary,
      state: parse_json(JSON.stringify({ protocol: "google_generate", id, parts })),
    });
  else if (parts.some((part) => part.thought || part.thoughtSignature)) invalid_google();
  return blocks;
}

export function google_parameters(input: NativeInput): GenerateContentParameters {
  const system: string[] = [];
  const contents: Content[] = [];
  const calls = new Map<string, { name: string; id?: string }>();
  let previous_role = "";
  for (const message of input.messages) {
    if (message.role === "system") {
      for (const block of message.content) {
        if (block.kind !== "text") throw new ProviderError("google_content_denied");
        system.push(block.text);
      }
      continue;
    }
    let parts: Part[] = [];
    const saved = message.content.filter((block) => block.kind === "reasoning");
    if (saved.length) {
      const state = saved[0]?.state;
      if (
        saved.length !== 1 ||
        !state ||
        typeof state !== "object" ||
        Array.isArray(state) ||
        state.protocol !== "google_generate" ||
        typeof state.id !== "string" ||
        !Array.isArray(state.parts)
      )
        throw new ProviderError("google_continuation_denied");
      parts = state.parts as Part[];
      const expected = google_blocks(parts, state.id, input).map(project_block);
      const actual = message.content.map(project_block);
      if (
        !Buffer.from(json_bytes(parse_json(JSON.stringify(expected)))).equals(
          json_bytes(parse_json(JSON.stringify(actual))),
        )
      )
        throw new ProviderError("google_continuation_denied");
      for (const [index, part] of parts.entries())
        if (part.functionCall?.name)
          calls.set(call_identity(part, state.id, index), {
            name: part.functionCall.name,
            ...(part.functionCall.id ? { id: part.functionCall.id } : {}),
          });
    } else {
      for (const block of message.content) {
        if (block.kind === "text" || block.kind === "refusal") parts.push({ text: block.text });
        else if (block.kind === "image" || block.kind === "document") {
          if (block.kind === "image" && block.detail !== "auto")
            throw new ProviderError("google_image_detail_denied");
          parts.push({ inlineData: { mimeType: block.media, data: block.data } });
        } else if (block.kind === "tool_call") {
          if (input.model.reasoning !== "off")
            throw new ProviderError("google_continuation_required");
          const call = {
            name: block.name,
            ...(block.id.startsWith("gemini-local-") ? {} : { id: block.id }),
          };
          calls.set(block.id, call);
          parts.push({
            functionCall: { ...call, args: parse_json(block.arguments) as Record<string, unknown> },
          });
        } else if (block.kind === "tool_result") {
          const call = calls.get(block.id);
          if (!call) throw new ProviderError("google_tool_result_denied");
          parts.push({
            functionResponse: {
              ...call,
              response: block.error ? { error: block.text } : { output: block.text },
            },
          });
        } else throw new ProviderError("google_content_denied");
      }
    }
    const previous = contents.at(-1);
    if (message.role === "tool" && previous_role === "tool" && previous?.parts)
      previous.parts.push(...parts);
    else contents.push({ role: message.role === "assistant" ? "model" : "user", parts });
    previous_role = message.role;
  }
  const mode =
    typeof input.choice === "object" || input.choice === "required"
      ? "ANY"
      : input.choice === "none"
        ? "NONE"
        : input.tools.some((tool) => tool.strict)
          ? "VALIDATED"
          : "AUTO";
  return {
    model: input.model.model,
    contents,
    config: {
      maxOutputTokens: input.output_tokens,
      candidateCount: 1,
      systemInstruction: { parts: [{ text: system.join("\n\n") }] },
      automaticFunctionCalling: { disable: true },
      ...(input.tools.length
        ? {
            tools: [
              {
                functionDeclarations: input.tools.map((tool) => ({
                  name: tool.name,
                  description: tool.description,
                  parametersJsonSchema: tool.schema.value,
                })),
              },
            ],
            toolConfig: {
              functionCallingConfig: {
                mode: mode as FunctionCallingConfigMode,
                ...(typeof input.choice === "object"
                  ? { allowedFunctionNames: [input.choice.name] }
                  : {}),
              },
            },
          }
        : {}),
      ...(input.structured
        ? {
            responseMimeType: "application/json",
            responseJsonSchema: input.structured.schema.value,
          }
        : {}),
      ...(input.model.reasoning !== "off"
        ? {
            thinkingConfig: {
              includeThoughts: true,
              thinkingLevel: input.model.reasoning.toUpperCase() as ThinkingLevel,
            },
          }
        : {}),
    },
  };
}

function project_block(block: NativeContent): unknown {
  if (block.kind === "tool_call") return { ...block, arguments: parse_json(block.arguments) };
  if (block.kind === "text" || block.kind === "refusal") {
    // Client JSON documents are bound by canonical digest at the shared boundary.
    // Compare JSON meaning while replaying the unchanged signed native bytes.
    try {
      return { kind: "text", document: parse_json(block.text) };
    } catch {
      return { kind: "text", text: block.text };
    }
  }
  return block;
}

/** The Developer API SDK counter omits system/tools; send its documented full envelope. */
export function google_counting(parameters: GenerateContentParameters) {
  const config = parameters.config ?? {};
  const {
    systemInstruction,
    tools,
    toolConfig,
    automaticFunctionCalling: _automatic,
    ...generationConfig
  } = config;
  return {
    generateContentRequest: {
      model: `models/${parameters.model}`,
      contents: parameters.contents,
      systemInstruction,
      tools,
      toolConfig,
      generationConfig,
    },
  };
}

export function google_result(reply: GenerateContentResponse, input: NativeInput): NativeReply {
  const candidate = reply.candidates?.[0];
  const usage = reply.usageMetadata;
  if (
    reply.modelVersion !== input.model.model ||
    !usage ||
    !reply.responseId ||
    reply.candidates?.length !== 1 ||
    !candidate ||
    (candidate.index ?? 0) !== 0 ||
    candidate.content?.role !== "model" ||
    !Array.isArray(candidate.content.parts) ||
    candidate.groundingMetadata ||
    candidate.citationMetadata
  )
    invalid_google();
  const reason = candidate.finishReason;
  const refusal = ["SAFETY", "RECITATION", "BLOCKLIST", "PROHIBITED_CONTENT", "SPII"].includes(
    reason ?? "",
  );
  if (!refusal && reason !== "STOP" && reason !== "MAX_TOKENS") invalid_google();
  const blocks = google_blocks(candidate.content.parts, reply.responseId, input);
  if (refusal) {
    if (blocks.some((block) => block.kind !== "text" && block.kind !== "reasoning"))
      invalid_google();
    if (!blocks.some((block) => block.kind === "text"))
      blocks.push({ kind: "refusal", text: "Content blocked by provider." });
    for (const [index, block] of blocks.entries())
      if (block.kind === "text")
        blocks[index] = { kind: "refusal", text: block.text || "Content blocked by provider." };
  }
  const reasoning = token_count(usage.thoughtsTokenCount ?? 0);
  const incoming =
    token_count(usage.promptTokenCount) + token_count(usage.toolUsePromptTokenCount ?? 0);
  const outgoing = token_count(usage.candidatesTokenCount ?? (refusal ? 0 : undefined)) + reasoning;
  if (token_count(usage.totalTokenCount) !== incoming + outgoing) invalid_google();
  return {
    blocks,
    finish: refusal
      ? "refusal"
      : reason === "MAX_TOKENS"
        ? "length"
        : blocks.some((block) => block.kind === "tool_call")
          ? "tool_call"
          : "stop",
    usage: {
      input: incoming,
      output: outgoing,
      cached: token_count(usage.cachedContentTokenCount ?? 0),
      created: 0,
      reasoning,
    },
  };
}
