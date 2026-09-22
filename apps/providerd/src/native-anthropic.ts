import Anthropic from "@anthropic-ai/sdk";
import { Code } from "@connectrpc/connect";
import { ErrorCategory } from "@loop-engine/protocol/provider";

import { native_error, ProviderError } from "./errors.js";
import { bounded_fetch, type NativeInput, type NativePlugin, token_count } from "./native.js";

function input_messages(input: NativeInput) {
  return {
    model: input.model.model,
    system: input.messages
      .filter((message) => message.role === "system")
      .map((message) => ({ type: "text" as const, text: message.text })),
    messages: input.messages.flatMap((message) =>
      message.role === "system" ? [] : [{ role: message.role, content: message.text }],
    ),
  };
}

export function anthropic_plugin(secret: string, fetcher: typeof fetch = fetch): NativePlugin {
  const client = new Anthropic({
    apiKey: secret,
    authToken: null,
    baseURL: "https://api.anthropic.com",
    maxRetries: 0,
    logLevel: "off",
    fetch: bounded_fetch(fetcher),
  });
  return {
    async count_input(input, signal) {
      try {
        const count = await client.messages.countTokens(input_messages(input), { signal });
        return token_count(count.input_tokens);
      } catch (error) {
        throw native_error(error, signal);
      }
    },
    async invoke(input, signal) {
      try {
        const reply = await client.messages.create(
          { ...input_messages(input), max_tokens: input.output_tokens, stream: false },
          { signal },
        );
        const finish = reply.stop_reason;
        if (
          reply.model !== input.model.model ||
          reply.role !== "assistant" ||
          !reply.usage ||
          !Array.isArray(reply.content) ||
          (finish !== "end_turn" &&
            finish !== "stop_sequence" &&
            finish !== "max_tokens" &&
            finish !== "refusal")
        ) {
          throw new ProviderError(
            "invalid_anthropic_output",
            Code.DataLoss,
            ErrorCategory.DEPENDENCY,
          );
        }
        const blocks = reply.content.map((block) => {
          if (block.type !== "text")
            throw new ProviderError(
              "unsupported_anthropic_content",
              Code.DataLoss,
              ErrorCategory.DEPENDENCY,
            );
          return {
            kind: finish === "refusal" ? ("refusal" as const) : ("text" as const),
            text: block.text,
          };
        });
        const created = token_count(reply.usage.cache_creation_input_tokens ?? 0);
        const cached = token_count(reply.usage.cache_read_input_tokens ?? 0);
        // Unit 1 never requests cache writes, whose distinct price is not in v1.
        if (created !== 0)
          throw new ProviderError(
            "unexpected_cache_write",
            Code.DataLoss,
            ErrorCategory.DEPENDENCY,
          );
        return {
          blocks,
          finish: finish === "max_tokens" ? "length" : finish === "refusal" ? "refusal" : "stop",
          usage: {
            input: token_count(reply.usage.input_tokens) + cached,
            output: token_count(reply.usage.output_tokens),
            cached,
            reasoning: 0,
          },
        };
      } catch (error) {
        throw native_error(error, signal);
      }
    },
  };
}
