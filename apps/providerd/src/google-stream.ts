import type { GenerateContentResponse, GoogleGenAI, Part } from "@google/genai";
import { google_blocks, google_parameters, google_result } from "./google-content.js";
import type { NativeEvent, NativeInput } from "./native.js";
import { stream_invalid, text_delta, tool_delta } from "./stream.js";

export async function* google_stream(
  client: GoogleGenAI,
  input: NativeInput,
  signal: AbortSignal,
): AsyncGenerator<NativeEvent> {
  const parameters = google_parameters(input);
  const source = await client.models.generateContentStream({
    ...parameters,
    config: { ...parameters.config, abortSignal: signal },
  });
  const parts: Part[] = [];
  let id = "";
  let model = "";
  let usage: GenerateContentResponse["usageMetadata"];
  let final: GenerateContentResponse["candidates"];
  let public_size = input.model.reasoning === "off" ? 0 : 1;
  let text_index = -1;
  for await (const chunk of source) {
    if (chunk.responseId) {
      if (id && chunk.responseId !== id) stream_invalid();
      id = chunk.responseId;
    }
    if (chunk.modelVersion) {
      if (chunk.modelVersion !== input.model.model) stream_invalid();
      model = chunk.modelVersion;
    }
    if (!id || !model || chunk.promptFeedback?.blockReason) stream_invalid();
    if (chunk.usageMetadata) usage = chunk.usageMetadata;
    if (chunk.candidates?.length) {
      if (final || chunk.candidates.length !== 1) stream_invalid();
      const candidate = chunk.candidates[0];
      if (
        !candidate ||
        (candidate.index ?? 0) !== 0 ||
        (candidate.content?.role !== undefined && candidate.content.role !== "model") ||
        candidate.groundingMetadata ||
        candidate.citationMetadata
      )
        stream_invalid();
      for (const part of candidate.content?.parts ?? []) {
        parts.push(part);
        // Validate each native part before exposing its preview.
        const converted = google_blocks([part], id, input);
        if (part.thought) {
          if (typeof part.text !== "string") stream_invalid();
          yield { kind: "delta", delta: text_delta(0, part.text, true) };
        } else if (typeof part.text === "string") {
          if (text_index < 0) text_index = public_size++;
          yield { kind: "delta", delta: text_delta(text_index, part.text) };
        } else if (part.functionCall) {
          // IDs of legacy responses without native call IDs depend on the full
          // part offset, not a chunk-local counter.
          const call = google_blocks(parts, id, input).at(-1);
          if (call?.kind !== "tool_call" || !converted.length) stream_invalid();
          yield {
            kind: "delta",
            delta: tool_delta(public_size++, call.id, call.name, call.arguments),
          };
          text_index = -1;
        }
        if (public_size > 256 || parts.length > 16_384) stream_invalid();
      }
      if (candidate.finishReason) final = [{ ...candidate, content: { role: "model", parts } }];
    }
  }
  if (!final || !usage) stream_invalid();
  yield {
    kind: "complete",
    reply: google_result(
      {
        responseId: id,
        modelVersion: model,
        candidates: final,
        usageMetadata: usage,
      } as GenerateContentResponse,
      input,
    ),
  };
}
