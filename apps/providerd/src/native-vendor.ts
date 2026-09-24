import Anthropic from "@anthropic-ai/sdk";
import OpenAI from "openai";
import { anthropic_parameters, anthropic_result } from "./anthropic-content.js";
import { anthropic_stream, type MessageDialect } from "./anthropic-stream.js";
import type { ModelRoute } from "./config.js";
import { native_error, ProviderError } from "./errors.js";
import { bounded_fetch, input_ceiling, type NativeInput, type NativePlugin } from "./native.js";
import { openai_route } from "./native-openai.js";
import { chat_stream } from "./openai-stream.js";
import { vendor_dialect, vendor_parameters } from "./vendor-chat.js";
import { VENDORS, vendor_endpoint, vendor_id } from "./vendor-registry.js";
import { vendor_reply } from "./vendor-replies.js";

function minimax_parameters(input: NativeInput): ReturnType<typeof anthropic_parameters> {
  if (input.tools.some((tool) => tool.strict) || input.structured)
    throw new ProviderError("vendor_strict_tool_denied");
  const parameters = anthropic_parameters(input);
  for (const tool of parameters.tools ?? []) if ("strict" in tool) delete tool.strict;
  if (parameters.thinking && "display" in parameters.thinking) delete parameters.thinking.display;
  // MiniMax caches automatically; an Anthropic ephemeral directive is not proof
  // of a MiniMax retention policy. Still account for measured reads/writes.
  delete parameters.cache_control;
  return parameters;
}

const minimax_dialect: MessageDialect = {
  parameters: minimax_parameters,
  reply: (value, input, fragments) => anthropic_result(value, input, fragments, true),
};

export function vendor_plugin(
  model: ModelRoute,
  secret: string,
  fetcher: typeof fetch,
): NativePlugin {
  if (!vendor_id(model.plugin)) throw new ProviderError("invalid_vendor_route");
  const profile = VENDORS[model.plugin];
  const endpoint = vendor_endpoint(model);
  if (profile.wire === "messages") {
    const sdk = new Anthropic({
      apiKey: secret,
      authToken: null,
      baseURL: endpoint,
      maxRetries: 0,
      logLevel: "off",
      fetch: bounded_fetch(fetcher),
    });
    return {
      async count_input(input, signal) {
        signal.throwIfAborted();
        minimax_parameters(input);
        return input_ceiling(input);
      },
      async invoke(input, signal) {
        try {
          return minimax_dialect.reply(
            await sdk.messages.create({ ...minimax_parameters(input), stream: false }, { signal }),
            input,
          );
        } catch (error) {
          throw native_error(error, signal);
        }
      },
      async *stream(input, signal) {
        try {
          yield* anthropic_stream(sdk, input, signal, minimax_dialect);
        } catch (error) {
          throw native_error(error, signal);
        }
      },
    };
  }
  const sdk = new OpenAI({
    apiKey: secret,
    organization: null,
    project: null,
    baseURL: endpoint,
    maxRetries: 0,
    logLevel: "off",
    fetch: bounded_fetch(async (input, init) => {
      if (model.plugin !== "perplexity") return fetcher(input, init);
      const request = new Request(input, init);
      const url = new URL(request.url);
      if (url.origin !== new URL(endpoint).origin || url.pathname !== "/v1/chat/completions")
        throw new ProviderError("invalid_vendor_endpoint");
      url.pathname = "/v1/sonar";
      return fetcher(new Request(url, request));
    }),
  });
  if (profile.wire === "responses") return openai_route(sdk, false, model.model);
  return {
    async count_input(input, signal) {
      signal.throwIfAborted();
      vendor_parameters(input, false);
      return input_ceiling(input);
    },
    async invoke(input, signal) {
      try {
        if (model.plugin === "qwen" && input.model.reasoning !== "off") {
          // Some Qwen thinking models only offer streaming; retain the unary
          // RPC contract by returning only its validated terminal response.
          for await (const event of chat_stream(
            sdk,
            input,
            signal,
            undefined,
            vendor_dialect(input),
          ))
            if (event.kind === "complete") return event.reply;
          throw new ProviderError("incomplete_vendor_output");
        }
        return vendor_reply(
          await sdk.chat.completions.create(
            { ...vendor_parameters(input, false), stream: false },
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
        yield* chat_stream(sdk, input, signal, undefined, vendor_dialect(input));
      } catch (error) {
        throw native_error(error, signal);
      }
    },
  };
}
