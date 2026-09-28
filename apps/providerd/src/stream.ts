import { create } from "@bufbuild/protobuf";
import { Code } from "@connectrpc/connect";
import {
  type ContentDelta,
  ContentDeltaSchema,
  ErrorCategory,
} from "@loop-engine/protocol/provider";
import { ProviderError } from "./errors.js";
import { parse_json } from "./json.js";
import type { NativeReply } from "./native.js";

export function stream_invalid(): never {
  throw new ProviderError("invalid_provider_stream", Code.DataLoss, ErrorCategory.DEPENDENCY);
}

/** Preserve backpressure; SDK parsing alone does not require Chat's DONE marker. */
export function stream_body(response: Response, require_done: boolean): ReadableStream<Uint8Array> {
  const iterator = stream_chunks(response, require_done);
  return new ReadableStream(
    {
      async pull(controller) {
        try {
          const item = await iterator.next();
          if (item.done) controller.close();
          else controller.enqueue(item.value);
        } catch (error) {
          controller.error(error);
        }
      },
      async cancel() {
        await iterator.return();
      },
    },
    { highWaterMark: 0 },
  );
}

async function* stream_chunks(
  response: Response,
  require_done: boolean,
): AsyncGenerator<Uint8Array, void> {
  const reader = response.body?.getReader();
  if (!reader) stream_invalid();
  const decoder = new TextDecoder("utf-8", { fatal: true });
  const encoder = new TextEncoder();
  let pending = "";
  let lines: string[] = [];
  let event_size = 0;
  let size = 0;
  let events = 0;
  let terminal = "";
  try {
    for (;;) {
      const chunk = await reader.read();
      size += chunk.value?.length ?? 0;
      if (size > 8_388_608) stream_invalid();
      pending += chunk.done ? decoder.decode() : decoder.decode(chunk.value, { stream: true });
      if (chunk.done && pending.endsWith("\r")) pending += "\n";
      let boundary = /\r\n|\n|\r(?!$)/.exec(pending);
      while (boundary) {
        const line = pending.slice(0, boundary.index);
        pending = pending.slice(boundary.index + boundary[0].length);
        if (line !== "") {
          lines.push(line);
          event_size += Buffer.byteLength(line);
        } else if (lines.length) {
          if (++events > 16_384) stream_invalid();
          const data = lines
            .filter((value) => value.startsWith("data:"))
            .map((value) => value.slice(5).replace(/^ /, ""))
            .join("\n");
          const encoded = `${lines.join("\n")}\n\n`;
          if (terminal && data) stream_invalid();
          if (data === "[DONE]") terminal = encoded;
          else if (!terminal) {
            if (data) {
              try {
                parse_json(data, 524_288, 64);
              } catch {
                stream_invalid();
              }
            }
            yield encoder.encode(encoded);
          }
          lines = [];
          event_size = 0;
        }
        if (lines.length > 4096 || event_size > 524_288) stream_invalid();
        boundary = /\r\n|\n|\r(?!$)/.exec(pending);
      }
      if (pending.length > 524_288) stream_invalid();
      if (chunk.done) break;
    }
    if (pending || lines.length || (require_done && !terminal)) stream_invalid();
    // Delay DONE until EOF so malformed/trailing data cannot be hidden by the SDK.
    if (terminal) yield encoder.encode(terminal);
  } finally {
    await reader.cancel().catch(() => undefined);
    reader.releaseLock();
  }
}

export function text_delta(index: number, text: string, reasoning = false): ContentDelta {
  return create(ContentDeltaSchema, {
    contentIndex: index,
    delta: reasoning ? { case: "reasoning", value: { text } } : { case: "text", value: { text } },
  });
}
export function tool_delta(
  index: number,
  id: string,
  name: string,
  fragment: string,
): ContentDelta {
  return create(ContentDeltaSchema, {
    contentIndex: index,
    delta: {
      case: "toolCall",
      value: {
        toolCallId: id,
        toolName: name,
        argumentsJsonFragment: new TextEncoder().encode(fragment),
      },
    },
  });
}

/** A completed reply must agree with every preview already exposed to a client. */
export class StreamEvidence {
  private readonly content = new Map<
    number,
    { kind: string; text: string; id?: string; name?: string }
  >();
  private size = 0;
  record(delta: ContentDelta): void {
    if (!Number.isSafeInteger(delta.contentIndex) || delta.contentIndex >= 256) stream_invalid();
    const value = delta.delta;
    const text =
      value.case === "toolCall"
        ? new TextDecoder("utf-8", { fatal: true }).decode(value.value.argumentsJsonFragment)
        : value.case === "text" || value.case === "reasoning"
          ? value.value.text
          : undefined;
    if (text === undefined || !text.isWellFormed()) stream_invalid();
    this.size += Buffer.byteLength(text);
    if (this.size > 262_144) stream_invalid();
    let previous = this.content.get(delta.contentIndex);
    if (!previous) {
      previous = {
        kind: value.case ?? "",
        text: "",
        ...(value.case === "toolCall"
          ? { id: value.value.toolCallId, name: value.value.toolName }
          : {}),
      };
      this.content.set(delta.contentIndex, previous);
    }
    if (
      previous.kind !== value.case ||
      (value.case === "toolCall" &&
        (previous.id !== value.value.toolCallId || previous.name !== value.value.toolName))
    )
      stream_invalid();
    previous.text += text;
  }
  verify(reply: NativeReply): void {
    for (const [index, seen] of this.content) {
      const block = reply.blocks[index];
      if (!block) stream_invalid();
      if (seen.kind === "toolCall") {
        if (
          block.kind !== "tool_call" ||
          block.id !== seen.id ||
          block.name !== seen.name ||
          block.arguments !== seen.text
        )
          stream_invalid();
      } else if (
        (block.kind !== seen.kind && !(seen.kind === "text" && block.kind === "refusal")) ||
        !("text" in block) ||
        block.text !== seen.text
      )
        stream_invalid();
    }
  }
}
