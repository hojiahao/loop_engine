import { createRequire } from "node:module";
import { cohere_parameters, cohere_result } from "./cohere-content.js";
import { cohere_stream } from "./cohere-stream.js";
import { native_error } from "./errors.js";
import { bounded_fetch, input_ceiling, type NativePlugin } from "./native.js";

export function cohere_plugin(secret: string, fetcher: typeof fetch = fetch): NativePlugin {
  const { CohereClientV2 } = createRequire(import.meta.url)(
    "cohere-ai",
  ) as typeof import("cohere-ai");
  const client = new CohereClientV2({
    token: secret,
    baseUrl: "https://api.cohere.com",
    maxRetries: 0,
    fetch: bounded_fetch(fetcher),
  });
  return {
    async count_input(input, signal) {
      signal.throwIfAborted();
      cohere_parameters(input);
      return input_ceiling(input);
    },
    async invoke(input, signal) {
      try {
        return cohere_result(
          await client.chat(cohere_parameters(input), { abortSignal: signal, maxRetries: 0 }),
        );
      } catch (error) {
        throw native_error(error, signal);
      }
    },
    async *stream(input, signal) {
      try {
        yield* cohere_stream(client, input, signal);
      } catch (error) {
        throw native_error(error, signal);
      }
    },
  };
}
