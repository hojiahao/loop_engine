import Anthropic from "@anthropic-ai/sdk";
import OpenAI from "openai";
import { anthropic_parameters, anthropic_result } from "./anthropic-content.js";
import { anthropic_stream } from "./anthropic-stream.js";
import { compatible_dialect, compatible_parameters } from "./compatible-chat.js";
import type { ModelRoute } from "./config.js";
import { native_error, ProviderError } from "./errors.js";
import { bounded_fetch, input_ceiling, type NativePlugin } from "./native.js";
import { openai_route } from "./native-openai.js";
import { chat_stream } from "./openai-stream.js";
import { vendor_reply } from "./vendor-replies.js";

/** The private endpoint, header policy and gateway supplier are not caller input. */
function compatible_fetch(
  model: ModelRoute,
  secret: string | undefined,
  upstream: string | undefined,
  fetcher: typeof fetch,
): typeof fetch {
  const route = model.compatible;
  if (!route) throw new ProviderError("compatible_route_missing");
  const base = new URL(route.base_url);
  const suffix =
    route.wire === "chat"
      ? "/chat/completions"
      : route.wire === "messages"
        ? "/v1/messages"
        : "/responses";
  const expected = `${base.pathname.replace(/\/$/, "")}${suffix}`;
  return bounded_fetch(async (input, init) => {
    const request = new Request(input, init);
    const url = new URL(request.url);
    if (
      url.origin !== base.origin ||
      url.pathname !== expected ||
      url.search ||
      request.method !== "POST"
    )
      throw new ProviderError("compatible_endpoint_denied");
    const headers = new Headers(request.headers);
    headers.delete("authorization");
    headers.delete("x-api-key");
    if (model.plugin === "portkey") {
      if (!secret || !upstream || !route.gateway)
        throw new ProviderError("provider_credentials_missing");
      headers.set("x-portkey-api-key", secret);
      headers.set("x-portkey-provider", route.gateway.upstream_provider);
      headers.set("authorization", `Bearer ${upstream}`);
      headers.set("x-portkey-config", JSON.stringify({ retry: { attempts: 0 } }));
    } else if (route.auth !== "none") {
      if (!secret) throw new ProviderError("provider_credentials_missing");
      headers.set(
        route.auth === "api_key" ? "x-api-key" : "authorization",
        route.auth === "api_key" ? secret : `Bearer ${secret}`,
      );
    }
    const response = await fetcher(new Request(request, { headers, redirect: "error" }));
    const retries = response.headers.get("x-portkey-retry-attempt-count");
    if (model.plugin === "portkey" && retries !== null && retries !== "0") {
      await response.body?.cancel().catch(() => undefined);
      throw new ProviderError("gateway_retry_denied");
    }
    return response;
  });
}

export function compatible_plugin(
  model: ModelRoute,
  secret: string | undefined,
  secrets: Readonly<Record<string, string | undefined>>,
  fetcher: typeof fetch,
): NativePlugin {
  const route = model.compatible;
  if (!route) throw new ProviderError("compatible_route_missing");
  const upstream = route.gateway?.upstream_key_env
    ? secrets[route.gateway.upstream_key_env]
    : undefined;
  const transport = compatible_fetch(model, secret, upstream, fetcher);
  const request_model = route.request_model ?? model.model;
  if (route.wire === "messages") {
    const client = new Anthropic({
      apiKey: secret ?? "anonymous",
      authToken: null,
      baseURL: route.base_url,
      maxRetries: 0,
      logLevel: "off",
      fetch: transport,
    });
    return {
      async count_input(input, signal) {
        signal.throwIfAborted();
        anthropic_parameters(input);
        return input_ceiling(input);
      },
      async invoke(input, signal) {
        try {
          return anthropic_result(
            await client.messages.create(
              { ...anthropic_parameters(input), model: request_model, stream: false },
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
          yield* anthropic_stream(client, input, signal, {
            parameters: (request) => ({
              ...anthropic_parameters(request),
              model: request_model,
            }),
            reply: anthropic_result,
          });
        } catch (error) {
          throw native_error(error, signal);
        }
      },
    };
  }
  const client = new OpenAI({
    apiKey: secret ?? "anonymous",
    organization: null,
    project: null,
    baseURL: route.base_url,
    maxRetries: 0,
    logLevel: "off",
    fetch: transport,
  });
  if (route.wire === "responses") return openai_route(client, false, request_model);
  return {
    async count_input(input, signal) {
      signal.throwIfAborted();
      compatible_parameters(input, false);
      return input_ceiling(input);
    },
    async invoke(input, signal) {
      try {
        return vendor_reply(
          await client.chat.completions.create(
            {
              ...compatible_parameters(input, false),
              model: request_model,
              stream: false,
            },
            { signal },
          ),
          input,
          route,
        );
      } catch (error) {
        throw native_error(error, signal);
      }
    },
    async *stream(input, signal) {
      try {
        yield* chat_stream(client, input, signal, request_model, compatible_dialect(input));
      } catch (error) {
        throw native_error(error, signal);
      }
    },
  };
}
