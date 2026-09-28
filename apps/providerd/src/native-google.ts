import { createRequire } from "node:module";
import { native_error } from "./errors.js";
import { google_counting, google_parameters, google_result } from "./google-content.js";
import { google_stream } from "./google-stream.js";
import { interaction_parameters, interaction_result } from "./interaction-content.js";
import { interaction_stream } from "./interaction-stream.js";
import { bounded_fetch, input_ceiling, type NativePlugin, token_count } from "./native.js";

export function google_plugin(
  secret: string,
  interactions: boolean,
  fetcher: typeof fetch = fetch,
): NativePlugin {
  // Load only a configured native SDK so unrelated deployments and health-only
  // startup avoid its memory cost. Both pinned SDKs publish Node CJS builds.
  const { GoogleGenAI } = createRequire(import.meta.url)(
    "@google/genai",
  ) as typeof import("@google/genai");
  const transport = bounded_fetch(fetcher);
  const client = new GoogleGenAI({
    apiKey: secret,
    vertexai: false,
    apiVersion: "v1beta",
    httpOptions: {
      baseUrl: "https://generativelanguage.googleapis.com",
      retryOptions: { attempts: 1 },
      fetch: transport,
    },
  });
  return {
    async count_input(input, signal) {
      if (interactions) {
        signal.throwIfAborted();
        interaction_parameters(input);
        return input_ceiling(input);
      }
      const parameters = google_parameters(input);
      try {
        const response = await transport(
          `https://generativelanguage.googleapis.com/v1beta/models/${encodeURIComponent(input.model.model)}:countTokens`,
          {
            method: "POST",
            headers: { "x-goog-api-key": secret, "content-type": "application/json" },
            body: JSON.stringify(google_counting(parameters)),
            signal,
          },
        );
        if (!response.ok) throw { status: response.status };
        const body = (await response.json()) as { totalTokens?: unknown };
        return token_count(body.totalTokens);
      } catch (error) {
        throw native_error(error, signal);
      }
    },
    async invoke(input, signal) {
      try {
        if (interactions)
          return interaction_result(
            await client.interactions.create(
              { ...interaction_parameters(input), stream: false },
              { signal, maxRetries: 0 },
            ),
            input,
          );
        const parameters = google_parameters(input);
        return google_result(
          await client.models.generateContent({
            ...parameters,
            config: { ...parameters.config, abortSignal: signal },
          }),
          input,
        );
      } catch (error) {
        throw native_error(error, signal);
      }
    },
    async *stream(input, signal) {
      try {
        if (interactions) yield* interaction_stream(client, input, signal);
        else yield* google_stream(client, input, signal);
      } catch (error) {
        throw native_error(error, signal);
      }
    },
  };
}
