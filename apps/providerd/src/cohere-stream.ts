import type { CohereClientV2 } from "cohere-ai";
import { cohere_parameters, cohere_result } from "./cohere-content.js";
import { parse_json } from "./json.js";
import type { NativeBlock, NativeEvent, NativeInput, NativeReply } from "./native.js";
import { stream_invalid, text_delta, tool_delta } from "./stream.js";

export async function* cohere_stream(
  client: CohereClientV2,
  input: NativeInput,
  signal: AbortSignal,
): AsyncGenerator<NativeEvent> {
  const source = await client.chatStream(cohere_parameters(input), {
    abortSignal: signal,
    maxRetries: 0,
  });
  const blocks: NativeBlock[] = [];
  const slots = new Map<string, { index: number; closed: boolean }>();
  let started = false;
  let final: NativeReply | undefined;
  let plan = -1;
  for await (const event of source) {
    if (final) stream_invalid();
    if (event.type === "message-start") {
      if (started || event.delta?.message?.role !== "assistant") stream_invalid();
      started = true;
      continue;
    }
    if (!started) stream_invalid();
    if (event.type === "tool-plan-delta") {
      const text = event.delta?.message?.toolPlan;
      if (typeof text !== "string") stream_invalid();
      if (plan < 0) {
        if (blocks.length >= 256) stream_invalid();
        plan = blocks.length;
        blocks.push({ kind: "text", text: "" });
      }
      const block = blocks[plan];
      if (block?.kind !== "text") stream_invalid();
      blocks[plan] = { kind: "text", text: block.text + text };
      yield { kind: "delta", delta: text_delta(plan, text) };
    } else if (event.type === "message-end") {
      if (
        [...slots.values()].some((slot) => !slot.closed) ||
        event.delta?.error ||
        !event.delta?.finishReason
      )
        stream_invalid();
      final = cohere_result(
        {
          id: "stream",
          finishReason: event.delta.finishReason,
          message: { role: "assistant" },
          ...(event.delta.usage ? { usage: event.delta.usage } : {}),
        },
        blocks,
      );
    } else if (event.type.startsWith("content-") || event.type.startsWith("tool-call-")) {
      if (
        !("index" in event) ||
        !Number.isSafeInteger(event.index) ||
        (event.index ?? -1) < 0 ||
        (event.index ?? 256) >= 256
      )
        stream_invalid();
      const tool = event.type.startsWith("tool-call-");
      const key = `${tool ? "tool" : "text"}:${event.index}`;
      let slot = slots.get(key);
      if (event.type === "content-start" || event.type === "tool-call-start") {
        if (slot || blocks.length >= 256) stream_invalid();
        slot = { index: blocks.length, closed: false };
        slots.set(key, slot);
        if (event.type === "content-start") {
          const content = event.delta?.message?.content;
          if (
            content?.type === "thinking" &&
            typeof content.thinking === "string" &&
            content.text === undefined
          ) {
            if (input.model.reasoning !== "enabled") stream_invalid();
            blocks.push({
              kind: "reasoning",
              text: content.thinking,
              state: { type: "thinking", thinking: content.thinking },
            });
            yield { kind: "delta", delta: text_delta(slot.index, content.thinking, true) };
          } else if (
            content?.type === "text" &&
            content.thinking === undefined &&
            typeof content.text === "string"
          ) {
            blocks.push({ kind: "text", text: content.text });
            yield { kind: "delta", delta: text_delta(slot.index, content.text) };
          } else stream_invalid();
        } else {
          const call = event.delta?.message?.toolCalls;
          if (call?.type !== "function" || !call.id || !call.function?.name) stream_invalid();
          const args = call.function.arguments ?? "";
          blocks.push({
            kind: "tool_call",
            id: call.id,
            name: call.function.name,
            arguments: args,
          });
          yield { kind: "delta", delta: tool_delta(slot.index, call.id, call.function.name, args) };
        }
      } else {
        if (!slot || slot.closed) stream_invalid();
        const block = blocks[slot.index];
        if (event.type === "content-end" || event.type === "tool-call-end") {
          if (block?.kind === "tool_call") parse_json(block.arguments);
          slot.closed = true;
        } else if (event.type === "content-delta") {
          const content = event.delta?.message?.content;
          if (
            block?.kind === "reasoning" &&
            typeof content?.thinking === "string" &&
            content.text === undefined
          ) {
            const text = block.text + content.thinking;
            blocks[slot.index] = {
              kind: "reasoning",
              text,
              state: { type: "thinking", thinking: text },
            };
            yield { kind: "delta", delta: text_delta(slot.index, content.thinking, true) };
          } else if (
            block?.kind === "text" &&
            typeof content?.text === "string" &&
            content.thinking === undefined
          ) {
            blocks[slot.index] = { kind: "text", text: block.text + content.text };
            yield { kind: "delta", delta: text_delta(slot.index, content.text) };
          } else stream_invalid();
        } else if (event.type === "tool-call-delta") {
          const text = event.delta?.message?.toolCalls?.function?.arguments;
          if (block?.kind !== "tool_call" || typeof text !== "string") stream_invalid();
          blocks[slot.index] = { ...block, arguments: block.arguments + text };
          yield { kind: "delta", delta: tool_delta(slot.index, block.id, block.name, text) };
        } else stream_invalid();
      }
    } else stream_invalid();
  }
  if (!final) stream_invalid();
  yield { kind: "complete", reply: final };
}
