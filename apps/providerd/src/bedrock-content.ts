import type {
  ContentBlock,
  ConverseCommandInput,
  ConverseResponse,
  Message,
  SystemContentBlock,
} from "@aws-sdk/client-bedrock-runtime";
import { Code } from "@connectrpc/connect";
import { ErrorCategory } from "@loop-engine/protocol/provider";
import { ProviderError } from "./errors.js";
import { parse_json } from "./json.js";
import { type NativeBlock, type NativeInput, type NativeReply, token_count } from "./native.js";

export function invalid_bedrock(): never {
  throw new ProviderError("invalid_bedrock_output", Code.DataLoss, ErrorCategory.DEPENDENCY);
}

export function bedrock_parameters(input: NativeInput): ConverseCommandInput {
  const cloud = input.model.cloud;
  if (cloud?.kind !== "bedrock") throw new ProviderError("invalid_cloud_route");
  if (
    input.model.reasoning === "enabled" &&
    (input.model.thinking_tokens ?? 0) >= input.output_tokens
  )
    throw new ProviderError("provider_thinking_budget");
  if (
    input.model.reasoning !== "off" &&
    (input.choice === "required" || typeof input.choice === "object")
  )
    throw new ProviderError("bedrock_thinking_choice_denied");
  if (input.choice === "none" && input.tools.length)
    throw new ProviderError("bedrock_tool_choice_denied");
  const system: SystemContentBlock[] = [];
  const messages: Message[] = [];
  let previous = "";
  for (const message of input.messages) {
    if (message.role === "system") {
      for (const block of message.content) {
        if (block.kind !== "text") throw new ProviderError("bedrock_content_denied");
        system.push({ text: block.text });
      }
      continue;
    }
    const content: ContentBlock[] = message.content.flatMap((block): ContentBlock[] => {
      if (block.kind === "text" || block.kind === "refusal") return [{ text: block.text }];
      if (block.kind === "tool_call")
        return [
          {
            toolUse: { toolUseId: block.id, name: block.name, input: parse_json(block.arguments) },
          },
        ];
      if (block.kind === "tool_result")
        return [
          {
            toolResult: {
              toolUseId: block.id,
              status: block.error ? "error" : "success",
              content: [{ text: block.text }],
            },
          },
        ];
      if (block.kind === "image") {
        if (block.detail !== "auto") throw new ProviderError("bedrock_image_detail_denied");
        return [
          {
            image: {
              format: block.media === "image/png" ? "png" : "jpeg",
              source: { bytes: Buffer.from(block.data, "base64") },
            },
          },
        ];
      }
      if (block.kind === "document")
        return [
          { text: "Attached document." },
          {
            document: {
              format: "pdf",
              name: "document",
              source: { bytes: Buffer.from(block.data, "base64") },
            },
          },
        ];
      if (block.kind === "reasoning") {
        const state = block.state;
        if (
          !state ||
          typeof state !== "object" ||
          Array.isArray(state) ||
          state.protocol !== "bedrock_converse"
        )
          throw new ProviderError("bedrock_continuation_denied");
        if (
          typeof state.text === "string" &&
          typeof state.signature === "string" &&
          state.signature &&
          state.text === block.text
        )
          return [
            {
              reasoningContent: { reasoningText: { text: state.text, signature: state.signature } },
            },
          ];
        if (typeof state.redacted === "string" && state.redacted && block.text === "")
          return [{ reasoningContent: { redactedContent: Buffer.from(state.redacted, "base64") } }];
        throw new ProviderError("bedrock_continuation_denied");
      }
      throw new ProviderError("bedrock_content_denied");
    });
    const last = messages.at(-1);
    if (message.role === "tool" && previous === "tool" && last?.content)
      last.content.push(...content);
    else messages.push({ role: message.role === "assistant" ? "assistant" : "user", content });
    previous = message.role;
  }
  if (input.model.features.prompt_caching) {
    if (system.length) system.push({ cachePoint: { type: "default", ttl: "5m" } });
    else
      messages
        .findLast((message) => message.role === "user")
        ?.content?.push({ cachePoint: { type: "default", ttl: "5m" } });
  }
  const tools = input.tools;
  return {
    modelId: cloud.model_id,
    system,
    messages,
    inferenceConfig: { maxTokens: input.output_tokens },
    ...(tools.length
      ? {
          toolConfig: {
            tools: tools.map((tool) => ({
              toolSpec: {
                name: tool.name,
                description: tool.description,
                strict: tool.strict,
                inputSchema: { json: tool.schema.value },
              },
            })),
            toolChoice:
              typeof input.choice === "object"
                ? { tool: { name: input.choice.name } }
                : input.choice === "required"
                  ? { any: {} }
                  : { auto: {} },
          },
        }
      : {}),
    ...(input.structured
      ? {
          outputConfig: {
            textFormat: {
              type: "json_schema",
              structure: {
                jsonSchema: {
                  name: input.structured.name,
                  description: input.structured.description,
                  schema: JSON.stringify(input.structured.schema.value),
                },
              },
            },
          },
        }
      : {}),
    ...(cloud.guardrail
      ? {
          guardrailConfig: {
            guardrailIdentifier: cloud.guardrail.id,
            guardrailVersion: cloud.guardrail.version,
            trace: "disabled",
          },
        }
      : {}),
    ...(input.model.reasoning !== "off"
      ? {
          additionalModelRequestFields: {
            thinking:
              input.model.reasoning === "adaptive"
                ? { type: "adaptive" }
                : { type: "enabled", budget_tokens: input.model.thinking_tokens ?? 0 },
          },
        }
      : {}),
  };
}

export function bedrock_result(reply: ConverseResponse, input: NativeInput): NativeReply {
  const message = reply.output?.message;
  const usage = reply.usage;
  const reason = reply.stopReason;
  if (
    message?.role !== "assistant" ||
    !Array.isArray(message.content) ||
    !usage ||
    ![
      "end_turn",
      "stop_sequence",
      "max_tokens",
      "tool_use",
      "guardrail_intervened",
      "content_filtered",
    ].includes(reason ?? "") ||
    reply.trace?.promptRouter
  )
    invalid_bedrock();
  const refusal = reason === "guardrail_intervened" || reason === "content_filtered";
  const blocks = message.content.map((block): NativeBlock => {
    if (
      Object.keys(block).filter((key) => block[key as keyof ContentBlock] !== undefined).length !==
      1
    )
      invalid_bedrock();
    if (typeof block.text === "string")
      return { kind: refusal ? "refusal" : "text", text: block.text };
    if (
      block.toolUse &&
      !refusal &&
      typeof block.toolUse.toolUseId === "string" &&
      typeof block.toolUse.name === "string" &&
      block.toolUse.type === undefined
    ) {
      const args = JSON.stringify(block.toolUse.input);
      parse_json(args);
      return {
        kind: "tool_call",
        id: block.toolUse.toolUseId,
        name: block.toolUse.name,
        arguments: args,
      };
    }
    if (block.reasoningContent && input.model.reasoning !== "off") {
      const value = block.reasoningContent;
      if (
        value.reasoningText &&
        typeof value.reasoningText.text === "string" &&
        value.reasoningText.signature
      )
        return {
          kind: "reasoning",
          text: value.reasoningText.text,
          state: {
            protocol: "bedrock_converse",
            text: value.reasoningText.text,
            signature: value.reasoningText.signature,
          },
        };
      if (value.redactedContent?.length)
        return {
          kind: "reasoning",
          text: "",
          state: {
            protocol: "bedrock_converse",
            redacted: Buffer.from(value.redactedContent).toString("base64"),
          },
        };
    }
    return invalid_bedrock();
  });
  if (refusal && !blocks.length)
    blocks.push({ kind: "refusal", text: "Content blocked by provider." });
  const cached = token_count(usage.cacheReadInputTokens ?? 0);
  const created = token_count(usage.cacheWriteInputTokens ?? 0);
  const incoming = token_count(usage.inputTokens) + cached + created;
  const outgoing = token_count(usage.outputTokens);
  if (
    token_count(usage.totalTokens) !== incoming + outgoing ||
    (created && !input.model.features.prompt_caching)
  )
    invalid_bedrock();
  if (
    usage.cacheDetails &&
    (usage.cacheDetails.some((entry) => entry.ttl !== "5m") ||
      usage.cacheDetails.reduce((sum, entry) => sum + token_count(entry.inputTokens), 0) !==
        created)
  )
    invalid_bedrock();
  return {
    blocks,
    finish: refusal
      ? "refusal"
      : reason === "max_tokens"
        ? "length"
        : reason === "tool_use"
          ? "tool_call"
          : "stop",
    // Converse reports combined output; never manufacture a thinking-token count.
    usage: { input: incoming, output: outgoing, cached, created, reasoning: 0 },
  };
}
