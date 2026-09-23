import type OpenAI from "openai";
import { json_bytes, parse_json } from "./json.js";
import type { NativeEvent, NativeInput, NativeReply } from "./native.js";
import {
  chat_parameters,
  chat_result,
  response_parameters,
  response_result,
} from "./openai-content.js";
import { stream_invalid, text_delta, tool_delta } from "./stream.js";

interface OutputItem {
  type: string;
  id: string;
  closed: boolean;
  final?: Uint8Array;
  index?: number;
  call_id?: string;
  name?: string;
  arguments: string;
  parts: Map<number, { index: number; type: string; text: string; closed: boolean }>;
}

export async function* response_stream(
  client: OpenAI,
  input: NativeInput,
  signal: AbortSignal,
): AsyncGenerator<NativeEvent> {
  const events = await client.responses.create(
    { ...response_parameters(input), stream: true },
    { signal },
  );
  const items = new Map<number, OutputItem>();
  let sequence = 0;
  let next_index = 0;
  let id = "";
  let reply: NativeReply | undefined;
  try {
    for await (const event of events) {
      if (
        reply ||
        !Number.isSafeInteger(event.sequence_number) ||
        event.sequence_number !== sequence++
      )
        stream_invalid();
      if (event.type === "response.created") {
        if (id || sequence !== 1 || event.response.model !== input.model.model) stream_invalid();
        id = event.response.id;
      } else if (!id) stream_invalid();
      else if (event.type === "response.in_progress") {
        if (event.response.id !== id) stream_invalid();
      } else if (event.type === "response.output_item.added") {
        if (
          event.output_index !== items.size ||
          items.size >= 256 ||
          !["message", "function_call", "reasoning"].includes(event.item.type)
        )
          stream_invalid();
        const item = event.item;
        if (!item.id) stream_invalid();
        const state: OutputItem = {
          id: item.id,
          type: item.type,
          closed: false,
          arguments: "",
          parts: new Map(),
        };
        if (item.type === "function_call") {
          state.index = next_index++;
          state.call_id = item.call_id;
          state.name = item.name;
          state.arguments = item.arguments;
          yield {
            kind: "delta",
            delta: tool_delta(state.index, item.call_id, item.name, item.arguments),
          };
        } else if (item.type === "reasoning" && input.model.reasoning !== "off")
          state.index = next_index++;
        items.set(event.output_index, state);
      } else if (event.type === "response.content_part.added") {
        const item = items.get(event.output_index);
        if (
          !item ||
          item.closed ||
          item.type !== "message" ||
          item.id !== event.item_id ||
          event.content_index !== item.parts.size
        )
          stream_invalid();
        const part = event.part;
        if (part.type !== "output_text" && part.type !== "refusal") stream_invalid();
        const index = next_index++;
        const text = part.type === "output_text" ? part.text : part.refusal;
        item.parts.set(event.content_index, { index, type: part.type, text, closed: false });
        if (part.type === "output_text" && text)
          yield { kind: "delta", delta: text_delta(index, text) };
      } else if (
        event.type === "response.output_text.delta" ||
        event.type === "response.refusal.delta"
      ) {
        const item = items.get(event.output_index);
        const part = item?.parts.get(event.content_index);
        if (
          !item ||
          item.closed ||
          item.id !== event.item_id ||
          !part ||
          part.closed ||
          typeof event.delta !== "string" ||
          part.type !== (event.type === "response.output_text.delta" ? "output_text" : "refusal")
        )
          stream_invalid();
        part.text += event.delta;
        if (part.type === "output_text")
          yield { kind: "delta", delta: text_delta(part.index, event.delta) };
      } else if (
        event.type === "response.output_text.done" ||
        event.type === "response.refusal.done"
      ) {
        const item = items.get(event.output_index);
        const part = item?.parts.get(event.content_index);
        const text = event.type === "response.output_text.done" ? event.text : event.refusal;
        if (
          !item ||
          item.closed ||
          !part ||
          part.closed ||
          item.id !== event.item_id ||
          part.text !== text
        )
          stream_invalid();
      } else if (event.type === "response.content_part.done") {
        const item = items.get(event.output_index);
        const part = item?.parts.get(event.content_index);
        if (!item || item.closed || !part || part.closed || item.id !== event.item_id)
          stream_invalid();
        const text =
          event.part.type === "output_text"
            ? event.part.text
            : event.part.type === "refusal"
              ? event.part.refusal
              : undefined;
        if (part.type !== event.part.type || part.text !== text) stream_invalid();
        part.closed = true;
      } else if (
        event.type === "response.function_call_arguments.delta" ||
        event.type === "response.function_call_arguments.done"
      ) {
        const item = items.get(event.output_index);
        if (
          !item ||
          item.closed ||
          item.type !== "function_call" ||
          item.id !== event.item_id ||
          item.index === undefined ||
          !item.call_id ||
          !item.name
        )
          stream_invalid();
        if (event.type === "response.function_call_arguments.delta") {
          if (typeof event.delta !== "string") stream_invalid();
          item.arguments += event.delta;
          yield {
            kind: "delta",
            delta: tool_delta(item.index, item.call_id, item.name, event.delta),
          };
        } else if (item.arguments !== event.arguments) stream_invalid();
      } else if (event.type === "response.reasoning_summary_part.added") {
        const item = items.get(event.output_index);
        if (
          !item ||
          item.closed ||
          item.type !== "reasoning" ||
          item.id !== event.item_id ||
          item.index === undefined ||
          event.summary_index !== item.parts.size
        )
          stream_invalid();
        item.parts.set(event.summary_index, {
          index: item.index,
          type: "summary_text",
          text: event.part.text,
          closed: false,
        });
        if (event.part.text)
          yield { kind: "delta", delta: text_delta(item.index, event.part.text, true) };
      } else if (
        event.type === "response.reasoning_summary_text.delta" ||
        event.type === "response.reasoning_summary_text.done" ||
        event.type === "response.reasoning_summary_part.done"
      ) {
        const item = items.get(event.output_index);
        const part = item?.parts.get(event.summary_index);
        if (
          !item ||
          item.closed ||
          item.id !== event.item_id ||
          !part ||
          part.closed ||
          item.index === undefined
        )
          stream_invalid();
        if (event.type === "response.reasoning_summary_text.delta") {
          part.text += event.delta;
          yield { kind: "delta", delta: text_delta(item.index, event.delta, true) };
        } else {
          const text =
            event.type === "response.reasoning_summary_text.done" ? event.text : event.part.text;
          if (text !== part.text) stream_invalid();
          if (event.type === "response.reasoning_summary_part.done") part.closed = true;
        }
      } else if (event.type === "response.output_item.done") {
        const item = items.get(event.output_index);
        if (
          !item ||
          item.closed ||
          item.id !== event.item.id ||
          item.type !== event.item.type ||
          [...item.parts.values()].some((part) => !part.closed)
        )
          stream_invalid();
        item.closed = true;
        item.final = json_bytes(parse_json(JSON.stringify(event.item), 524_288, 64));
      } else if (event.type === "response.completed" || event.type === "response.incomplete") {
        if (
          event.response.id !== id ||
          event.response.output.length !== items.size ||
          [...items.values()].some((item) => !item.closed)
        )
          stream_invalid();
        for (const [index, state] of items) {
          if (
            !state.final ||
            !Buffer.from(state.final).equals(
              json_bytes(parse_json(JSON.stringify(event.response.output[index]), 524_288, 64)),
            )
          )
            stream_invalid();
        }
        reply = response_result(event.response, input);
      } else stream_invalid();
    }
    if (!reply || signal.aborted) stream_invalid();
    yield { kind: "complete", reply };
  } finally {
    events.controller.abort();
  }
}

export async function* chat_stream(
  client: OpenAI,
  input: NativeInput,
  signal: AbortSignal,
): AsyncGenerator<NativeEvent> {
  const events = await client.chat.completions.create(
    { ...chat_parameters(input), stream: true, stream_options: { include_usage: true } },
    { signal },
  );
  const calls = new Map<number, { id: string; name: string; arguments: string; index: number }>();
  let text: string | null = null;
  let refusal: string | null = null;
  let text_index: number | undefined;
  let next_index = 0;
  let id = "";
  let role = false;
  let finish: OpenAI.Chat.Completions.ChatCompletion.Choice["finish_reason"] | undefined;
  let usage: OpenAI.CompletionUsage | undefined;
  try {
    for await (const event of events) {
      if (
        event.model !== input.model.model ||
        !Array.isArray(event.choices) ||
        (id && event.id !== id) ||
        usage
      )
        stream_invalid();
      id = event.id;
      if (event.usage) {
        if (!finish || event.choices.length) stream_invalid();
        usage = event.usage;
        continue;
      }
      const choice = event.choices[0];
      if (finish || event.choices.length !== 1 || !choice || choice.index !== 0) stream_invalid();
      const delta = choice.delta;
      if (delta.role) {
        if (role || delta.role !== "assistant") stream_invalid();
        role = true;
      }
      if (!role || delta.function_call) stream_invalid();
      if (delta.content !== undefined && delta.content !== null) {
        if (typeof delta.content !== "string") stream_invalid();
        if (delta.content) {
          text_index ??= next_index++;
          text = (text ?? "") + delta.content;
          yield { kind: "delta", delta: text_delta(text_index, delta.content) };
        }
      }
      if (delta.refusal) refusal = (refusal ?? "") + delta.refusal;
      for (const change of delta.tool_calls ?? []) {
        if (
          !Number.isSafeInteger(change.index) ||
          change.index < 0 ||
          change.index >= 128 ||
          (change.type && change.type !== "function")
        )
          stream_invalid();
        let call = calls.get(change.index);
        if (!call) {
          if (change.index !== calls.size || !change.id || !change.function?.name) stream_invalid();
          call = { id: change.id, name: change.function.name, arguments: "", index: next_index++ };
          calls.set(change.index, call);
        } else if (
          (change.id && change.id !== call.id) ||
          (change.function?.name && change.function.name !== call.name)
        )
          stream_invalid();
        const fragment = change.function?.arguments ?? "";
        call.arguments += fragment;
        yield { kind: "delta", delta: tool_delta(call.index, call.id, call.name, fragment) };
      }
      if (choice.finish_reason) finish = choice.finish_reason;
    }
    if (!finish || !usage || !role || signal.aborted) stream_invalid();
    const reply = chat_result(
      {
        id,
        model: input.model.model,
        object: "chat.completion",
        created: 0,
        choices: [
          {
            index: 0,
            logprobs: null,
            finish_reason: finish,
            message: {
              role: "assistant",
              content: text,
              refusal,
              ...(calls.size
                ? {
                    tool_calls: [...calls.values()].map((call) => ({
                      type: "function" as const,
                      id: call.id,
                      function: { name: call.name, arguments: call.arguments },
                    })),
                  }
                : {}),
            },
          },
        ],
        usage,
      },
      input,
    );
    // Chat supplies separate text/tool fields, not ordered content blocks. Keep
    // the first-fragment order already exposed in this stream's content indices.
    const indices = [
      ...(text ? [text_index] : []),
      ...(refusal ? [next_index++] : []),
      ...[...calls.values()].map((call) => call.index),
    ];
    if (indices.length !== reply.blocks.length || indices.some((index) => index === undefined))
      stream_invalid();
    const blocks = reply.blocks
      .map((block, index) => ({ block, index: indices[index] ?? -1 }))
      .sort((left, right) => left.index - right.index)
      .map(({ block }) => block);
    yield { kind: "complete", reply: { ...reply, blocks } };
  } finally {
    events.controller.abort();
  }
}
