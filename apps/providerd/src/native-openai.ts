import { Code } from "@connectrpc/connect";
import { ErrorCategory } from "@loop-engine/protocol/provider";
import OpenAI from "openai";

import { native_error, ProviderError } from "./errors.js";
import {
  bounded_fetch,
  type NativeInput,
  type NativePlugin,
  type NativeReply,
  token_count,
} from "./native.js";

function input_messages(input: NativeInput) {
  return input.messages.map((message) => ({ role: message.role, content: message.text }));
}

function invalid_output(): never {
  throw new ProviderError("invalid_openai_output", Code.DataLoss, ErrorCategory.DEPENDENCY);
}

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
        const count = await client.responses.inputTokens.count(
          { model: input.model.model, input: input_messages(input) },
          { signal },
        );
        return token_count(count.input_tokens);
      } catch (error) {
        throw native_error(error, signal);
      }
    },
    async invoke(input, signal) {
      try {
        if (chat) return await chat_reply(client, input, signal);
        const reply = await client.responses.create(
          {
            model: input.model.model,
            input: input_messages(input),
            max_output_tokens: input.output_tokens,
            store: false,
            stream: false,
          },
          { signal },
        );
        if (
          reply.model !== input.model.model ||
          !reply.usage ||
          !Array.isArray(reply.output) ||
          reply.error
        )
          invalid_output();
        let finish: NativeReply["finish"] = "stop";
        if (
          reply.status === "incomplete" &&
          reply.incomplete_details?.reason === "max_output_tokens"
        )
          finish = "length";
        else if (reply.status !== "completed") invalid_output();
        const blocks: { kind: "text" | "refusal"; text: string }[] = [];
        for (const item of reply.output) {
          if (item.type === "reasoning" && item.summary.length === 0 && !item.encrypted_content)
            continue;
          if (item.type !== "message" || item.role !== "assistant") invalid_output();
          for (const content of item.content) {
            if (content.type === "output_text") blocks.push({ kind: "text", text: content.text });
            else if (content.type === "refusal") {
              blocks.push({ kind: "refusal", text: content.refusal });
              finish = "refusal";
            } else invalid_output();
          }
        }
        return {
          blocks,
          finish,
          usage: {
            input: token_count(reply.usage.input_tokens),
            output: token_count(reply.usage.output_tokens),
            cached: token_count(reply.usage.input_tokens_details?.cached_tokens ?? 0),
            reasoning: token_count(reply.usage.output_tokens_details?.reasoning_tokens ?? 0),
          },
        };
      } catch (error) {
        throw native_error(error, signal);
      }
    },
  };
}

async function chat_reply(
  client: OpenAI,
  input: NativeInput,
  signal: AbortSignal,
): Promise<NativeReply> {
  const reply = await client.chat.completions.create(
    {
      model: input.model.model,
      messages: input_messages(input),
      max_completion_tokens: input.output_tokens,
      store: false,
      stream: false,
      n: 1,
    },
    { signal },
  );
  const choice = reply.choices?.[0];
  if (
    reply.model !== input.model.model ||
    reply.choices?.length !== 1 ||
    !choice ||
    !reply.usage ||
    choice.message.role !== "assistant" ||
    choice.message.tool_calls?.length ||
    choice.message.function_call ||
    choice.message.audio
  )
    invalid_output();
  const blocks: { kind: "text" | "refusal"; text: string }[] = [];
  if (choice.message.content !== null) blocks.push({ kind: "text", text: choice.message.content });
  if (choice.message.refusal) blocks.push({ kind: "refusal", text: choice.message.refusal });
  const finish = choice.finish_reason;
  if (finish !== "stop" && finish !== "length" && finish !== "content_filter") invalid_output();
  return {
    blocks,
    finish: choice.message.refusal || finish === "content_filter" ? "refusal" : finish,
    usage: {
      input: token_count(reply.usage.prompt_tokens),
      output: token_count(reply.usage.completion_tokens),
      cached: token_count(reply.usage.prompt_tokens_details?.cached_tokens ?? 0),
      reasoning: token_count(reply.usage.completion_tokens_details?.reasoning_tokens ?? 0),
    },
  };
}
