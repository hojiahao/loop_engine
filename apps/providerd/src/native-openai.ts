import OpenAI from "openai";
import { native_error } from "./errors.js";
import { bounded_fetch, type NativePlugin, token_count } from "./native.js";
import {
  chat_parameters,
  chat_result,
  response_parameters,
  response_result,
} from "./openai-content.js";
import { chat_stream, response_stream } from "./openai-stream.js";

export function openai_plugin(
  secret: string,
  chat: boolean,
  fetcher: typeof fetch = fetch,
): NativePlugin {
  const client = new OpenAI({
    apiKey: secret,
    organization: null,
    project: null,
    baseURL: "https://api.openai.com/v1",
    maxRetries: 0,
    logLevel: "off",
    fetch: bounded_fetch(fetcher),
  });
  return {
    async count_input(input, signal) {
      try {
        const {
          input: messages,
          tools,
          tool_choice,
          parallel_tool_calls,
          reasoning,
          text,
        } = response_parameters(input);
        const count = await client.responses.inputTokens.count(
          {
            model: input.model.model,
            input: messages,
            tools,
            tool_choice,
            parallel_tool_calls,
            reasoning,
            text,
          },
          { signal },
        );
        return token_count(count.input_tokens);
      } catch (error) {
        throw native_error(error, signal);
      }
    },
    async invoke(input, signal) {
      try {
        if (chat)
          return chat_result(
            await client.chat.completions.create(
              { ...chat_parameters(input), stream: false },
              { signal },
            ),
            input,
          );
        return response_result(
          await client.responses.create(
            { ...response_parameters(input), stream: false },
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
        const events = chat
          ? chat_stream(client, input, signal)
          : response_stream(client, input, signal);
        yield* events;
      } catch (error) {
        throw native_error(error, signal);
      }
    },
  };
}
