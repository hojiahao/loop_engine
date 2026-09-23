import type {
  ContentBlock,
  ConverseResponse,
  ConverseStreamOutput,
} from "@aws-sdk/client-bedrock-runtime";
import { Code } from "@connectrpc/connect";
import { ErrorCategory } from "@loop-engine/protocol/provider";
import { bedrock_result } from "./bedrock-content.js";
import { ProviderError } from "./errors.js";
import { parse_json } from "./json.js";
import type { NativeEvent, NativeInput } from "./native.js";
import { stream_invalid, text_delta, tool_delta } from "./stream.js";

interface BedrockBlock {
  kind: "text" | "tool" | "reasoning";
  text: string;
  signature: string;
  redacted: Uint8Array[];
  closed: boolean;
  id?: string;
  name?: string;
}

export async function* bedrock_stream(
  source: AsyncIterable<ConverseStreamOutput>,
  input: NativeInput,
): AsyncGenerator<NativeEvent> {
  let started = false;
  let stopped = false;
  let metadata = false;
  const blocks: BedrockBlock[] = [];
  const native: ContentBlock[] = [];
  const reply: ConverseResponse = {
    output: { message: { role: "assistant", content: native } },
    stopReason: undefined,
    usage: undefined,
    metrics: undefined,
  };
  for await (const event of source) {
    if (event.throttlingException)
      throw new ProviderError(
        "provider_rate_limited",
        Code.ResourceExhausted,
        ErrorCategory.RATE_LIMIT,
      );
    if (
      metadata ||
      Object.keys(event).filter((key) => event[key as keyof ConverseStreamOutput] !== undefined)
        .length !== 1
    )
      stream_invalid();
    if (event.messageStart) {
      if (started || event.messageStart.role !== "assistant") stream_invalid();
      started = true;
    } else if (!started) stream_invalid();
    else if (event.metadata) {
      if (!stopped || event.metadata.trace?.promptRouter) stream_invalid();
      metadata = true;
      reply.usage = event.metadata.usage;
      reply.metrics = event.metadata.metrics;
    } else if (stopped) stream_invalid();
    else if (event.contentBlockStart) {
      const start = event.contentBlockStart;
      const call = start.start?.toolUse;
      if (
        start.contentBlockIndex !== blocks.length ||
        blocks.length >= 256 ||
        !call?.toolUseId ||
        !call.name ||
        call.type !== undefined
      )
        stream_invalid();
      blocks.push({
        kind: "tool",
        id: call.toolUseId,
        name: call.name,
        text: "",
        signature: "",
        redacted: [],
        closed: false,
      });
      yield { kind: "delta", delta: tool_delta(blocks.length - 1, call.toolUseId, call.name, "") };
    } else if (event.contentBlockDelta) {
      const { contentBlockIndex: index, delta } = event.contentBlockDelta;
      if (
        index === undefined ||
        !Number.isSafeInteger(index) ||
        index < 0 ||
        index > blocks.length ||
        index >= 256 ||
        !delta ||
        Object.keys(delta).filter((key) => delta[key as keyof typeof delta] !== undefined)
          .length !== 1
      )
        stream_invalid();
      if (index === blocks.length) {
        if (delta.text === undefined && delta.reasoningContent === undefined) stream_invalid();
        blocks.push({
          kind: delta.text === undefined ? "reasoning" : "text",
          text: "",
          signature: "",
          redacted: [],
          closed: false,
        });
      }
      const block = blocks[index];
      if (!block || block.closed) stream_invalid();
      if (typeof delta.text === "string" && block.kind === "text") {
        block.text += delta.text;
        yield { kind: "delta", delta: text_delta(index, delta.text) };
      } else if (
        typeof delta.toolUse?.input === "string" &&
        block.kind === "tool" &&
        block.id &&
        block.name
      ) {
        block.text += delta.toolUse.input;
        yield {
          kind: "delta",
          delta: tool_delta(index, block.id, block.name, delta.toolUse.input),
        };
      } else if (
        delta.reasoningContent &&
        block.kind === "reasoning" &&
        input.model.reasoning !== "off"
      ) {
        const value = delta.reasoningContent;
        if (
          Object.keys(value).filter((key) => value[key as keyof typeof value] !== undefined)
            .length !== 1
        )
          stream_invalid();
        if (typeof value.text === "string" && !block.signature && !block.redacted.length) {
          block.text += value.text;
          yield { kind: "delta", delta: text_delta(index, value.text, true) };
        } else if (typeof value.signature === "string" && !block.redacted.length)
          block.signature += value.signature;
        else if (value.redactedContent?.length && !block.text && !block.signature)
          block.redacted.push(value.redactedContent);
        else stream_invalid();
      } else stream_invalid();
    } else if (event.contentBlockStop) {
      const index = event.contentBlockStop.contentBlockIndex;
      const block = index === undefined ? undefined : blocks[index];
      if (!block || block.closed || index !== native.length) stream_invalid();
      block.closed = true;
      if (block.kind === "text") native.push({ text: block.text });
      else if (block.kind === "tool")
        native.push({
          toolUse: { toolUseId: block.id, name: block.name, input: parse_json(block.text) },
        });
      else if (block.redacted.length)
        native.push({ reasoningContent: { redactedContent: Buffer.concat(block.redacted) } });
      else if (block.signature)
        native.push({
          reasoningContent: { reasoningText: { text: block.text, signature: block.signature } },
        });
      else stream_invalid();
    } else if (event.messageStop) {
      if (blocks.some((block) => !block.closed)) stream_invalid();
      stopped = true;
      reply.stopReason = event.messageStop.stopReason;
    } else stream_invalid();
  }
  if (!metadata) stream_invalid();
  const result = bedrock_result(reply, input);
  // Preserve original JSON fragments for the shared preview/completion check.
  const completed = result.blocks.map((block, index) =>
    block.kind === "tool_call" ? { ...block, arguments: blocks[index]?.text ?? "" } : block,
  );
  yield { kind: "complete", reply: { ...result, blocks: completed } };
}
