import { create } from "@bufbuild/protobuf";
import { Code } from "@connectrpc/connect";
import {
  type ContentDelta,
  ErrorCategory,
  ModelFinishReason,
  ModelUsageSchema,
} from "@loop-engine/protocol/provider";

import type { ModelRoute } from "./config.js";
import { ProviderError } from "./errors.js";
import { type JsonValue, parse_json, type RegisteredSchema } from "./json.js";
import { stream_body } from "./stream.js";

export type NativeBlock =
  | { readonly kind: "text" | "refusal"; readonly text: string }
  | {
      readonly kind: "tool_call";
      readonly id: string;
      readonly name: string;
      readonly arguments: string;
    }
  | { readonly kind: "reasoning"; readonly text: string; readonly state: JsonValue };
export type NativeContent =
  | NativeBlock
  | {
      readonly kind: "tool_result";
      readonly id: string;
      readonly error: boolean;
      readonly text: string;
    }
  | {
      readonly kind: "image";
      readonly media: "image/png" | "image/jpeg";
      readonly data: string;
      readonly detail: "auto" | "low" | "high";
    }
  | { readonly kind: "document"; readonly media: "application/pdf"; readonly data: string };
export interface NativeMessage {
  readonly role: "system" | "user" | "assistant" | "tool";
  readonly content: readonly NativeContent[];
}
export interface NativeTool {
  readonly name: string;
  readonly description: string;
  readonly strict: boolean;
  readonly schema: RegisteredSchema;
}
export interface NativeInput {
  readonly model: ModelRoute;
  readonly messages: readonly NativeMessage[];
  readonly output_tokens: number;
  readonly tools: readonly NativeTool[];
  readonly choice: "none" | "auto" | "required" | { readonly name: string };
  readonly structured?: NativeTool;
}
export interface NativeUsage {
  readonly input: number;
  readonly output: number;
  readonly cached: number;
  readonly reasoning: number;
  readonly created: number;
}
export interface NativeReply {
  readonly blocks: readonly NativeBlock[];
  readonly finish: "stop" | "length" | "refusal" | "tool_call";
  readonly usage: NativeUsage;
}
export type NativeEvent =
  | { readonly kind: "delta"; readonly delta: ContentDelta }
  | { readonly kind: "complete"; readonly reply: NativeReply };
export interface NativePlugin {
  count_input(input: NativeInput, signal: AbortSignal): Promise<number>;
  invoke(input: NativeInput, signal: AbortSignal): Promise<NativeReply>;
  stream(input: NativeInput, signal: AbortSignal): AsyncIterable<NativeEvent>;
}

/** Conservative reservation, not a measured count. The deployment pins the
 * vendor's full input limit separately from its combined context capacity. */
export function input_ceiling(input: NativeInput): number {
  const limit = input.model.input_token_limit;
  if (!limit) throw new ProviderError("provider_input_limit_missing");
  return limit;
}

export function token_count(value: unknown): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0 || value > 2_000_000) {
    throw new ProviderError("invalid_provider_usage", Code.DataLoss, ErrorCategory.DEPENDENCY);
  }
  return value;
}

export function response_usage(usage: NativeUsage) {
  for (const value of Object.values(usage)) token_count(value);
  if (usage.cached + usage.created > usage.input || usage.reasoning > usage.output)
    throw new ProviderError("invalid_provider_usage", Code.DataLoss, ErrorCategory.DEPENDENCY);
  return create(ModelUsageSchema, {
    inputTokens: BigInt(usage.input),
    outputTokens: BigInt(usage.output),
    cachedInputTokens: BigInt(usage.cached),
    reasoningTokens: BigInt(usage.reasoning),
    cacheCreationInputTokens: BigInt(usage.created),
    // Token-price accounting is not evidence of an actual supplier charge.
  });
}

export function response_finish(reply: NativeReply): ModelFinishReason {
  return {
    stop: ModelFinishReason.STOP,
    length: ModelFinishReason.LENGTH,
    refusal: ModelFinishReason.CONTENT_FILTER,
    tool_call: ModelFinishReason.TOOL_CALL,
  }[reply.finish];
}

/** Bound decoded bytes too, and refuse redirects instead of forwarding keys. */
export function bounded_fetch(fetcher: typeof fetch = fetch): typeof fetch {
  return async (input, init) => {
    const response = await fetcher(input, { ...init, redirect: "error" });
    if (!response.body)
      throw new ProviderError("provider_empty_response", Code.DataLoss, ErrorCategory.DEPENDENCY);
    if (response.headers.get("content-type")?.split(";")[0]?.trim() === "text/event-stream") {
      const url = new URL(input instanceof Request ? input.url : String(input));
      return new Response(stream_body(response, url.pathname.endsWith("/chat/completions")), {
        status: response.status,
        statusText: response.statusText,
        headers: response.headers,
      });
    }
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
    const bytes = Buffer.concat(chunks);
    if (response.headers.get("content-type")?.split(";")[0]?.trim() === "application/json") {
      try {
        parse_json(bytes, 524_288, 64);
      } catch {
        throw new ProviderError("invalid_provider_json", Code.DataLoss, ErrorCategory.DEPENDENCY);
      }
    }
    return new Response(bytes, {
      status: response.status,
      statusText: response.statusText,
      headers: response.headers,
    });
  };
}
