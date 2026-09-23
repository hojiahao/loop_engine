import type Anthropic from "@anthropic-ai/sdk";
import { anthropic_parameters, anthropic_result } from "./anthropic-content.js";
import { parse_json } from "./json.js";
import type { NativeEvent, NativeInput } from "./native.js";
import { stream_invalid, text_delta, tool_delta } from "./stream.js";

export async function* anthropic_stream(
  client: Anthropic,
  input: NativeInput,
  signal: AbortSignal,
): AsyncGenerator<NativeEvent> {
  const events = await client.messages.create(
    { ...anthropic_parameters(input), stream: true },
    { signal },
  );
  let message: Anthropic.Messages.Message | undefined;
  const closed = new Set<number>();
  const fragments = new Map<number, string>();
  let ending = false;
  let ended = false;
  let usage = false;
  try {
    for await (const event of events) {
      if (ended) stream_invalid();
      if (event.type === "message_start") {
        if (
          message ||
          event.message.content.length ||
          event.message.model !== input.model.model ||
          event.message.role !== "assistant"
        )
          stream_invalid();
        message = event.message;
        continue;
      }
      if (!message) stream_invalid();
      if (event.type === "content_block_start") {
        if (
          ending ||
          event.index !== message.content.length ||
          event.index >= 256 ||
          !["text", "thinking", "redacted_thinking", "tool_use"].includes(event.content_block.type)
        )
          stream_invalid();
        const block = event.content_block;
        message.content.push(block);
        if (block.type === "text" && block.text)
          yield { kind: "delta", delta: text_delta(event.index, block.text) };
        if (block.type === "thinking" && block.thinking)
          yield { kind: "delta", delta: text_delta(event.index, block.thinking, true) };
        if (block.type === "tool_use") {
          const text = JSON.stringify(block.input);
          fragments.set(event.index, text === "{}" ? "" : text);
          if (text !== "{}")
            yield { kind: "delta", delta: tool_delta(event.index, block.id, block.name, text) };
        }
      } else if (event.type === "content_block_delta") {
        const block = message.content[event.index];
        if (ending || !Number.isSafeInteger(event.index) || !block || closed.has(event.index))
          stream_invalid();
        const delta = event.delta;
        if (block.type === "text" && delta.type === "text_delta") {
          block.text += delta.text;
          yield { kind: "delta", delta: text_delta(event.index, delta.text) };
        } else if (block.type === "thinking" && delta.type === "thinking_delta") {
          block.thinking += delta.thinking;
          yield { kind: "delta", delta: text_delta(event.index, delta.thinking, true) };
        } else if (block.type === "thinking" && delta.type === "signature_delta")
          block.signature += delta.signature;
        else if (block.type === "tool_use" && delta.type === "input_json_delta") {
          fragments.set(event.index, (fragments.get(event.index) ?? "") + delta.partial_json);
          yield {
            kind: "delta",
            delta: tool_delta(event.index, block.id, block.name, delta.partial_json),
          };
        } else stream_invalid();
      } else if (event.type === "content_block_stop") {
        const block = message.content[event.index];
        if (ending || !block || closed.has(event.index)) stream_invalid();
        if (block.type === "tool_use") {
          let text = fragments.get(event.index) ?? "";
          if (!text) {
            text = "{}";
            fragments.set(event.index, text);
            yield { kind: "delta", delta: tool_delta(event.index, block.id, block.name, text) };
          }
          block.input = parse_json(text);
        }
        closed.add(event.index);
      } else if (event.type === "message_delta") {
        if (
          closed.size !== message.content.length ||
          (message.stop_reason &&
            event.delta.stop_reason &&
            message.stop_reason !== event.delta.stop_reason)
        )
          stream_invalid();
        ending = true;
        message.stop_reason = event.delta.stop_reason ?? message.stop_reason;
        message.stop_sequence = event.delta.stop_sequence ?? message.stop_sequence;
        if (event.usage) {
          if (event.usage.output_tokens !== undefined && event.usage.output_tokens !== null) {
            if (event.usage.output_tokens < message.usage.output_tokens) stream_invalid();
            usage = true;
          }
          message.usage = { ...message.usage, ...event.usage } as Anthropic.Messages.Usage;
        }
      } else if (event.type === "message_stop") {
        if (!ending || !usage || !message.stop_reason || closed.size !== message.content.length)
          stream_invalid();
        ended = true;
      } else stream_invalid();
    }
    if (!message || !ended || signal.aborted) stream_invalid();
    yield { kind: "complete", reply: anthropic_result(message, input, fragments) };
  } finally {
    events.controller.abort();
  }
}
