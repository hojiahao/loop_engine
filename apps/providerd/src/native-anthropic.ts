import Anthropic from "@anthropic-ai/sdk";
import { anthropic_parameters, anthropic_result } from "./anthropic-content.js";
import { anthropic_stream } from "./anthropic-stream.js";
import { native_error } from "./errors.js";
import { bounded_fetch, type NativePlugin, token_count } from "./native.js";

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
        const { max_tokens: _maximum, ...parameters } = anthropic_parameters(input);
        return token_count(
          (await client.messages.countTokens(parameters, { signal })).input_tokens,
        );
      } catch (error) {
        throw native_error(error, signal);
      }
    },
    async invoke(input, signal) {
      try {
        return anthropic_result(
          await client.messages.create(
            { ...anthropic_parameters(input), stream: false },
            { signal },
          ),
          input,
        );
      } catch (error) {
        throw native_error(error, signal);
      }
    },
    async *stream(input, signal) {
      try {
        yield* anthropic_stream(client, input, signal);
      } catch (error) {
        throw native_error(error, signal);
      }
    },
  };
}
