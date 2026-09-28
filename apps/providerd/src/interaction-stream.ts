import type { GoogleGenAI, Interactions } from "@google/genai";
import {
  interaction_blocks,
  interaction_parameters,
  interaction_result,
} from "./interaction-content.js";
import { json_bytes, parse_json } from "./json.js";
import type { NativeBlock, NativeEvent, NativeInput, NativeReply } from "./native.js";
import { stream_invalid, text_delta, tool_delta } from "./stream.js";

export async function* interaction_stream(
  client: GoogleGenAI,
  input: NativeInput,
  signal: AbortSignal,
): AsyncGenerator<NativeEvent> {
  const source = await client.interactions.create(
    { ...interaction_parameters(input), stream: true },
    { signal, maxRetries: 0 },
  );
  const slots = new Map<
    number,
    { step: Interactions.Step; index: number; closed: boolean; text: string }
  >();
  const blocks: NativeBlock[] = [];
  let id = "";
  let model = "";
  let final: NativeReply | undefined;
  for await (const event of source) {
    if (final) stream_invalid();
    if (event.event_type === "interaction.created") {
      if (
        id ||
        !event.interaction.id ||
        event.interaction.status !== "in_progress" ||
        event.interaction.agent ||
        (event.interaction.model !== undefined && event.interaction.model !== input.model.model)
      )
        stream_invalid();
      id = event.interaction.id;
      model = event.interaction.model ?? "";
      continue;
    }
    if (!id) stream_invalid();
    if (event.event_type === "step.start") {
      if (
        !Number.isSafeInteger(event.index) ||
        event.index < 0 ||
        event.index >= 256 ||
        slots.has(event.index) ||
        blocks.length >= 256
      )
        stream_invalid();
      const step = event.step;
      let block: NativeBlock;
      if (step.type === "model_output") {
        if (
          step.error ||
          (step.content?.length ?? 0) > 1 ||
          step.content?.some(
            (part) =>
              part.type !== "text" || typeof part.text !== "string" || part.annotations?.length,
          )
        )
          stream_invalid();
        block = {
          kind: "text",
          text: step.content?.[0]?.type === "text" ? step.content[0].text : "",
        };
      } else if (step.type === "function_call") {
        if (!step.id || !step.name || (step.arguments && Object.keys(step.arguments).length))
          stream_invalid();
        block = { kind: "tool_call", id: step.id, name: step.name, arguments: "" };
      } else if (step.type === "thought") {
        if (input.model.reasoning === "off") stream_invalid();
        if (
          (step.summary ?? []).some((part) => part.type !== "text" || typeof part.text !== "string")
        )
          stream_invalid();
        block = {
          kind: "reasoning",
          text: (step.summary ?? [])
            .map((part) => (part.type === "text" ? part.text : ""))
            .join(""),
          state: null,
        };
      } else stream_invalid();
      const index = blocks.length;
      blocks.push(block);
      slots.set(event.index, {
        step,
        index,
        closed: false,
        text: block.kind === "tool_call" ? "" : block.text,
      });
      yield {
        kind: "delta",
        delta:
          block.kind === "tool_call"
            ? tool_delta(index, block.id, block.name, "")
            : text_delta(index, block.text, block.kind === "reasoning"),
      };
    } else if (event.event_type === "step.delta" || event.event_type === "step.stop") {
      const slot = slots.get(event.index);
      if (!slot || slot.closed) stream_invalid();
      const block = blocks[slot.index];
      if (!block) stream_invalid();
      if (event.event_type === "step.stop") {
        slot.closed = true;
        if (slot.step.type === "function_call") {
          const args = parse_json(slot.text || "{}");
          if (args === null || typeof args !== "object" || Array.isArray(args)) stream_invalid();
          slot.step = { ...slot.step, arguments: args };
          if (block.kind !== "tool_call") stream_invalid();
          if (!slot.text) {
            slot.text = "{}";
            yield { kind: "delta", delta: tool_delta(slot.index, block.id, block.name, "{}") };
          }
          blocks[slot.index] = { ...block, arguments: slot.text };
        } else if (slot.step.type === "model_output")
          slot.step = { type: "model_output", content: [{ type: "text", text: slot.text }] };
        else if (slot.step.type === "thought") {
          slot.step = {
            ...slot.step,
            ...(slot.text ? { summary: [{ type: "text", text: slot.text }] } : {}),
          };
          const checked = interaction_blocks([slot.step])[0];
          if (!checked) stream_invalid();
          blocks[slot.index] = checked;
        }
      } else {
        const delta = event.delta;
        if (delta.type === "thought_signature") {
          if (slot.step.type !== "thought" || !delta.signature || slot.step.signature)
            stream_invalid();
          slot.step = { ...slot.step, signature: delta.signature };
        } else {
          const text =
            delta.type === "text" && block.kind === "text"
              ? delta.text
              : delta.type === "arguments_delta" && block.kind === "tool_call"
                ? delta.arguments
                : delta.type === "thought_summary" &&
                    block.kind === "reasoning" &&
                    delta.content?.type === "text"
                  ? delta.content.text
                  : undefined;
          if (typeof text !== "string") stream_invalid();
          slot.text += text;
          blocks[slot.index] =
            block.kind === "tool_call"
              ? { ...block, arguments: slot.text }
              : { ...block, text: slot.text };
          yield {
            kind: "delta",
            delta:
              block.kind === "tool_call"
                ? tool_delta(slot.index, block.id, block.name, text)
                : text_delta(slot.index, text, block.kind === "reasoning"),
          };
        }
      }
    } else if (event.event_type === "interaction.status_update") {
      // Status updates are advisory; completion still needs the terminal event.
      if (!["in_progress", "requires_action"].includes(event.status)) stream_invalid();
    } else if (event.event_type === "interaction.completed") {
      const reply = event.interaction;
      if (
        reply.id !== id ||
        [...slots.values()].some((slot) => !slot.closed) ||
        reply.agent ||
        (reply.model !== undefined && reply.model !== input.model.model)
      )
        stream_invalid();
      model = reply.model ?? model;
      const steps = [...slots.values()].map((slot) => slot.step);
      if (
        reply.steps &&
        !Buffer.from(json_bytes(parse_json(JSON.stringify(reply.steps)))).equals(
          json_bytes(parse_json(JSON.stringify(steps))),
        )
      )
        stream_invalid();
      final = interaction_result({ ...reply, model, steps }, input, blocks);
    } else stream_invalid();
  }
  if (!final) stream_invalid();
  yield { kind: "complete", reply: final };
}
