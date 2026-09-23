import { Readable } from "node:stream";
import { EventStreamCodec } from "@smithy/core/event-streams";
import { type HttpHandlerOptions, type HttpRequest, HttpResponse } from "@smithy/core/protocols";
import { type JsonValue, parse_json } from "./json.js";
import { bounded_fetch } from "./native.js";
import { stream_invalid } from "./stream.js";

function checked_union(
  value: JsonValue | undefined,
  names: readonly string[],
): Record<string, JsonValue> {
  if (!value || typeof value !== "object" || Array.isArray(value)) stream_invalid();
  const keys = Object.keys(value);
  if (keys.length !== 1 || !names.includes(keys[0] ?? "")) stream_invalid();
  return value;
}

/** Frame validation precedes the SDK decoder so a declared huge frame cannot
 * cause an unbounded allocation. The official codec validates both CRCs. */
async function* bedrock_frames(response: Response): AsyncGenerator<Uint8Array> {
  const reader = response.body?.getReader();
  if (!reader) stream_invalid();
  const codec = new EventStreamCodec(
    (bytes) => new TextDecoder("utf-8", { fatal: true }).decode(bytes),
    (text) => new TextEncoder().encode(text),
  );
  let pending: Buffer<ArrayBufferLike> = Buffer.alloc(0);
  let size = 0;
  let frames = 0;
  try {
    for (;;) {
      const chunk = await reader.read();
      if (chunk.done) break;
      size += chunk.value.length;
      if (size > 8_388_608) stream_invalid();
      pending = Buffer.concat([pending, chunk.value]);
      while (pending.length >= 4) {
        const length = pending.readUInt32BE(0);
        if (length < 16 || length > 524_288) stream_invalid();
        if (pending.length < length) break;
        if (++frames > 16_384) stream_invalid();
        const frame = pending.subarray(0, length);
        const event = codec.decode(frame);
        const kind = event.headers[":message-type"];
        if (kind?.type !== "string" || !["event", "exception", "error"].includes(kind.value))
          stream_invalid();
        const name = event.headers[kind.value === "event" ? ":event-type" : ":exception-type"];
        // The SDK silently skips unknown event names. Reject before decoding:
        // a future output type must not disappear from a successful receipt.
        const allowed =
          kind.value === "event"
            ? [
                "messageStart",
                "contentBlockStart",
                "contentBlockDelta",
                "contentBlockStop",
                "messageStop",
                "metadata",
              ]
            : [
                "internalServerException",
                "modelStreamErrorException",
                "serviceUnavailableException",
                "throttlingException",
                "validationException",
              ];
        if (name?.type !== "string" || !allowed.includes(name.value)) stream_invalid();
        const payload = parse_json(event.body, 524_288, 64);
        if (!payload || typeof payload !== "object" || Array.isArray(payload)) stream_invalid();
        if (name.value === "contentBlockDelta") {
          const delta = checked_union(payload.delta, ["text", "toolUse", "reasoningContent"]);
          if (delta.reasoningContent)
            checked_union(delta.reasoningContent, ["text", "signature", "redactedContent"]);
        } else if (name.value === "contentBlockStart") checked_union(payload.start, ["toolUse"]);
        yield frame;
        pending = pending.subarray(length);
      }
    }
    if (pending.length || !frames) stream_invalid();
  } finally {
    await reader.cancel().catch(() => undefined);
    reader.releaseLock();
  }
}

/** Transport only; AWS's SDK retains signing and protocol deserialization. */
export function bedrock_transport(origin: string, fetcher: typeof fetch) {
  const transport = bounded_fetch(fetcher);
  return {
    async handle(request: HttpRequest, options?: HttpHandlerOptions) {
      const target = `${request.protocol}//${request.hostname}${request.port ? `:${request.port}` : ""}${request.path}`;
      const headers = new Headers(request.headers);
      const payload: unknown = request.body;
      if (
        new URL(target).origin !== origin ||
        request.method !== "POST" ||
        (typeof payload !== "string" && !(payload instanceof Uint8Array)) ||
        Buffer.byteLength(payload) > 8_388_608 ||
        !headers.get("authorization")?.startsWith("AWS4-HMAC-SHA256 ") ||
        (request.query && Object.keys(request.query).length)
      )
        stream_invalid();
      const signal = options?.abortSignal as AbortSignal | undefined;
      signal?.throwIfAborted();
      const init = {
        method: request.method,
        headers,
        // Smithy's JSON adapter is a Uint8Array subclass. Send the signed
        // bytes verbatim, without coercing the adapter or serializing again.
        body: typeof payload === "string" ? payload : Buffer.from(payload),
        signal,
        redirect: "error" as const,
      };
      const streaming = request.path.endsWith("/converse-stream");
      const response = await (streaming ? fetcher : transport)(target, init);
      let body: Uint8Array | Readable;
      if (streaming && response.ok) {
        if (
          response.headers.get("content-type")?.split(";")[0]?.trim() !==
          "application/vnd.amazon.eventstream"
        ) {
          await response.body?.cancel();
          stream_invalid();
        }
        body = Readable.from(bedrock_frames(response), { objectMode: false });
      } else {
        // Errors on the binary route need the same JSON byte/parser bounds.
        const bounded = streaming
          ? await bounded_fetch(async () => response)(target, init)
          : response;
        body = new Uint8Array(await bounded.arrayBuffer());
        if (response.ok) {
          const payload = parse_json(body, 524_288, 64);
          if (!payload || typeof payload !== "object" || Array.isArray(payload)) stream_invalid();
          const output = checked_union(payload.output, ["message"]);
          const message = output.message;
          if (
            !message ||
            typeof message !== "object" ||
            Array.isArray(message) ||
            !Array.isArray(message.content)
          )
            stream_invalid();
          for (const value of message.content) {
            const block = checked_union(value, ["text", "toolUse", "reasoningContent"]);
            if (block.reasoningContent)
              checked_union(block.reasoningContent, ["reasoningText", "redactedContent"]);
          }
        }
      }
      return {
        response: new HttpResponse({
          statusCode: response.status,
          headers: Object.fromEntries(response.headers),
          body,
        }),
      };
    },
  };
}
