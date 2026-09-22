import { create } from "@bufbuild/protobuf";
import { Code } from "@connectrpc/connect";
import {
  ContentBlockSchema,
  ErrorCategory,
  ModelFinishReason,
  ModelUsageSchema,
} from "@loop-engine/protocol/provider";

import type { ModelRoute } from "./config.js";
import { ProviderError } from "./errors.js";

export interface TextMessage {
  readonly role: "system" | "user" | "assistant";
  readonly text: string;
}
export interface NativeInput {
  readonly model: ModelRoute;
  readonly messages: readonly TextMessage[];
  readonly output_tokens: number;
}
export interface NativeUsage {
  readonly input: number;
  readonly output: number;
  readonly cached: number;
  readonly reasoning: number;
}
export interface NativeReply {
  readonly blocks: readonly { kind: "text" | "refusal"; text: string }[];
  readonly finish: "stop" | "length" | "refusal";
  readonly usage: NativeUsage;
}
export interface NativePlugin {
  count_input(input: NativeInput, signal: AbortSignal): Promise<number>;
  invoke(input: NativeInput, signal: AbortSignal): Promise<NativeReply>;
}

export function token_count(value: unknown): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0 || value > 2_000_000) {
    throw new ProviderError("invalid_provider_usage", Code.DataLoss, ErrorCategory.DEPENDENCY);
  }
  return value;
}

export function response_blocks(reply: NativeReply) {
  if (reply.blocks.length < 1 || reply.blocks.length > 256)
    throw new ProviderError("invalid_provider_content", Code.DataLoss, ErrorCategory.DEPENDENCY);
  return reply.blocks.map((block) => {
    if (
      typeof block.text !== "string" ||
      !block.text.isWellFormed() ||
      Buffer.byteLength(block.text) > 262_144
    ) {
      throw new ProviderError("invalid_provider_content", Code.DataLoss, ErrorCategory.DEPENDENCY);
    }
    return create(ContentBlockSchema, {
      content:
        block.kind === "text"
          ? { case: "text", value: { text: block.text } }
          : { case: "refusal", value: { reason: block.text } },
    });
  });
}

export function response_usage(usage: NativeUsage) {
  for (const value of Object.values(usage)) token_count(value);
  if (usage.cached > usage.input || usage.reasoning > usage.output)
    throw new ProviderError("invalid_provider_usage", Code.DataLoss, ErrorCategory.DEPENDENCY);
  return create(ModelUsageSchema, {
    inputTokens: BigInt(usage.input),
    outputTokens: BigInt(usage.output),
    cachedInputTokens: BigInt(usage.cached),
    reasoningTokens: BigInt(usage.reasoning),
    // Token-price accounting is not evidence of an actual supplier charge.
  });
}

export function response_finish(reply: NativeReply): ModelFinishReason {
  return {
    stop: ModelFinishReason.STOP,
    length: ModelFinishReason.LENGTH,
    refusal: ModelFinishReason.CONTENT_FILTER,
  }[reply.finish];
}

/** Bound decoded bytes too, and refuse redirects instead of forwarding keys. */
export function bounded_fetch(fetcher: typeof fetch = fetch): typeof fetch {
  return async (input, init) => {
    const response = await fetcher(input, { ...init, redirect: "error" });
    if (!response.body)
      throw new ProviderError("provider_empty_response", Code.DataLoss, ErrorCategory.DEPENDENCY);
    const reader = response.body.getReader();
    const chunks: Uint8Array[] = [];
    let size = 0;
    try {
      for (;;) {
        const next = await reader.read();
        if (next.done) break;
        size += next.value.length;
        if (size > 524_288)
          throw new ProviderError(
            "provider_response_too_large",
            Code.ResourceExhausted,
            ErrorCategory.DEPENDENCY,
          );
        chunks.push(next.value);
      }
    } finally {
      await reader.cancel().catch(() => undefined);
      reader.releaseLock();
    }
    return new Response(Buffer.concat(chunks), {
      status: response.status,
      statusText: response.statusText,
      headers: response.headers,
    });
  };
}
