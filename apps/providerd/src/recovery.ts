import { fromBinary } from "@bufbuild/protobuf";
import { Code } from "@connectrpc/connect";
import {
  ErrorCategory,
  type JsonDocument,
  ModelFinishReason,
  ModelResponseSchema,
} from "@loop-engine/protocol/provider";
import { ProviderError } from "./errors.js";
import { json_bytes, json_digest, parse_json } from "./json.js";

function receipt_invalid(): never {
  throw new Error("invalid_receipt");
}

function receipt_id(value: string | undefined): string {
  if (!value || !/^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$/.test(value)) receipt_invalid();
  return value;
}

function receipt_json(document: JsonDocument | undefined): number {
  if (!document?.schemaId || document.schemaSha256?.value.length !== 32) receipt_invalid();
  const value = parse_json(document.utf8Json);
  if (
    !Buffer.from(json_digest(json_bytes(value))).equals(
      document.canonicalSha256?.value ?? new Uint8Array(),
    )
  )
    receipt_invalid();
  return document.utf8Json.length;
}

/** Validate stored transport evidence without current schemas, model or secrets.
 * A valid receipt is historical evidence, never tool/admission authorization.
 */
export function receipt_response(bytes: Uint8Array) {
  try {
    const response = fromBinary(ModelResponseSchema, bytes);
    receipt_id(response.requestId?.value);
    receipt_id(response.resolutionId?.value);
    const usage = response.usage;
    if (
      !usage ||
      response.content.length < 1 ||
      response.content.length > 256 ||
      ![
        ModelFinishReason.STOP,
        ModelFinishReason.LENGTH,
        ModelFinishReason.TOOL_CALL,
        ModelFinishReason.CONTENT_FILTER,
      ].includes(response.finishReason) ||
      usage.inputTokens > 2_000_000n ||
      usage.outputTokens > 2_000_000n ||
      usage.cachedInputTokens + usage.cacheCreationInputTokens > usage.inputTokens ||
      usage.reasoningTokens > usage.outputTokens
    )
      receipt_invalid();
    const calls = new Set<string>();
    let size = 0;
    for (const block of response.content) {
      const { content } = block;
      switch (content.case) {
        case "text":
          size += Buffer.byteLength(content.value.text);
          break;
        case "refusal":
          if (!content.value.reason) receipt_invalid();
          size += Buffer.byteLength(content.value.reason);
          break;
        case "reasoning": {
          const continuation = content.value.continuation;
          if (
            !continuation ||
            continuation.modelResolutionId?.value !== response.resolutionId?.value ||
            continuation.stateSha256?.value.length !== 32 ||
            !continuation.expiresAt ||
            continuation.expiresAt.seconds < 0n ||
            continuation.expiresAt.seconds > 253_402_300_799n ||
            continuation.expiresAt.nanos < 0 ||
            continuation.expiresAt.nanos >= 1_000_000_000
          )
            receipt_invalid();
          receipt_id(continuation.providerContinuationId?.value);
          receipt_id(continuation.providerId?.value);
          size += Buffer.byteLength(content.value.text);
          break;
        }
        case "toolCall": {
          const call = content.value;
          if (
            !/^[A-Za-z0-9_-]{1,128}$/.test(call.toolCallId) ||
            !/^[A-Za-z0-9_-]{1,128}$/.test(call.toolName) ||
            calls.has(call.toolCallId)
          )
            receipt_invalid();
          calls.add(call.toolCallId);
          size += receipt_json(call.arguments);
          break;
        }
        case "structuredOutput":
          size += receipt_json(content.value.output);
          break;
        default:
          receipt_invalid();
      }
      if (size > 262_144) receipt_invalid();
    }
    if (calls.size > 0 !== (response.finishReason === ModelFinishReason.TOOL_CALL))
      receipt_invalid();
    return response;
  } catch {
    throw new ProviderError("provider_receipt_corrupt", Code.DataLoss, ErrorCategory.INTERNAL);
  }
}
