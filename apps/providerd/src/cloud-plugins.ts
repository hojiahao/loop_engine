import { createRequire } from "node:module";
import OpenAI from "openai";
import { azure_identity, type CloudIdentity, cloud_token, google_identity } from "./cloud-auth.js";
import type { ModelRoute } from "./config.js";
import { native_error, ProviderError } from "./errors.js";
import { google_parameters, google_result } from "./google-content.js";
import { google_stream } from "./google-stream.js";
import { bounded_fetch, input_ceiling, type NativePlugin } from "./native.js";
import { openai_route } from "./native-openai.js";

export function azure_plugin(
  model: ModelRoute,
  secret: string | undefined,
  fetcher: typeof fetch,
  identity: CloudIdentity,
): NativePlugin {
  const cloud = model.cloud;
  if (cloud?.kind !== "azure") throw new ProviderError("invalid_cloud_route");
  const token = cloud.auth === "entra" ? (identity.azure ?? azure_identity()) : undefined;
  const transport = bounded_fetch(fetcher);
  const client = new OpenAI({
    apiKey: "cloud-identity-resolved-by-transport",
    organization: null,
    project: null,
    baseURL: `https://${cloud.resource}.${cloud.domain}/openai/v1`,
    maxRetries: 0,
    logLevel: "off",
    fetch: async (input, init) => {
      const request = new Request(input, init);
      request.headers.delete("authorization");
      request.headers.delete("api-key");
      if (token)
        request.headers.set("authorization", `Bearer ${await cloud_token(token, request.signal)}`);
      else if (secret) request.headers.set("api-key", secret);
      else throw new ProviderError("cloud_identity_unavailable");
      request.signal.throwIfAborted();
      return transport(request);
    },
  });
  return openai_route(client, model.plugin === "azure_chat", cloud.deployment);
}

export function vertex_plugin(
  model: ModelRoute,
  fetcher: typeof fetch,
  identity: CloudIdentity,
): NativePlugin {
  const cloud = model.cloud;
  if (cloud?.kind !== "vertex") throw new ProviderError("invalid_cloud_route");
  const { project, location } = cloud;
  const token = identity.google ?? google_identity();
  const { GoogleGenAI } = createRequire(import.meta.url)(
    "@google/genai",
  ) as typeof import("@google/genai");
  const { OAuth2Client } = createRequire(import.meta.url)(
    "google-auth-library",
  ) as typeof import("google-auth-library");
  async function client(signal: AbortSignal) {
    const credential = new OAuth2Client();
    credential.setCredentials({ access_token: await cloud_token(token, signal) });
    signal.throwIfAborted();
    return new GoogleGenAI({
      vertexai: true,
      project,
      location,
      apiVersion: "v1",
      googleAuthOptions: { authClient: credential },
      httpOptions: {
        baseUrl:
          location === "global"
            ? "https://aiplatform.googleapis.com"
            : `https://${location}-aiplatform.googleapis.com`,
        retryOptions: { attempts: 1 },
        fetch: bounded_fetch(fetcher),
      },
    });
  }
  return {
    async count_input(input, signal) {
      signal.throwIfAborted();
      google_parameters(input);
      return input_ceiling(input);
    },
    async invoke(input, signal) {
      try {
        const sdk = await client(signal);
        const parameters = google_parameters(input);
        return google_result(
          await sdk.models.generateContent({
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
        yield* google_stream(await client(signal), input, signal);
      } catch (error) {
        throw native_error(error, signal);
      }
    },
  };
}
