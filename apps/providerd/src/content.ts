import { create } from "@bufbuild/protobuf";
import { Code } from "@connectrpc/connect";
import {
  type ContentBlock,
  ContentBlockSchema,
  ErrorCategory,
  ImageDetail,
  type JsonSchema,
  JsonSchemaSchema,
  type ModelInvocation,
  type ModelResolutionSnapshot,
  ModelRole,
  ToolChoiceMode,
  ToolResultStatus,
} from "@loop-engine/protocol/provider";

import { type Deployment, type ModelRoute, read_private } from "./config.js";
import { ProviderError } from "./errors.js";
import { hex_digest } from "./identity.js";
import { check_document, compile_schema, make_document, type RegisteredSchema } from "./json.js";
import type {
  NativeContent,
  NativeInput,
  NativeMessage,
  NativeReply,
  NativeTool,
} from "./native.js";
import { read_continuation, read_prompt, save_continuation } from "./private-state.js";

function content_denied(): never {
  throw new ProviderError("unsupported_request_content");
}
function valid_text(value: string, empty = false): string {
  if (
    typeof value !== "string" ||
    !value.isWellFormed() ||
    (!empty && !value.length) ||
    Buffer.byteLength(value) > 262_144
  )
    content_denied();
  return value;
}
function valid_name(value: string): string {
  if (!/^[A-Za-z0-9_-]{1,128}$/.test(value)) content_denied();
  return value;
}

/** Only registered schemas and actor-private prompt views enter native requests. */
export class ProviderContent {
  private readonly schemas = new Map<string, RegisteredSchema>();
  constructor(private readonly config: Deployment) {}

  async schema(reference: JsonSchema | undefined): Promise<RegisteredSchema> {
    if (!reference) content_denied();
    const registration = this.config.schemas.find((entry) => entry.id === reference.schemaId);
    if (
      !registration ||
      registration.version !== reference.schemaVersion ||
      registration.sha256 !== hex_digest(reference.schemaSha256?.value ?? new Uint8Array())
    )
      throw new ProviderError("provider_schema_denied");
    let loaded = this.schemas.get(registration.id);
    if (!loaded) {
      const bytes = await read_private(registration.path, 65_536);
      loaded = compile_schema(
        create(JsonSchemaSchema, {
          schemaId: registration.id,
          schemaVersion: registration.version,
          canonicalJson: bytes,
          schemaSha256: { value: Buffer.from(registration.sha256, "hex") },
        }),
      );
      this.schemas.set(registration.id, loaded);
    }
    if (!Buffer.from(reference.canonicalJson).equals(loaded.reference.canonicalJson))
      throw new ProviderError("provider_schema_denied");
    return loaded;
  }

  async document_schema(id: string): Promise<RegisteredSchema> {
    const registration = this.config.schemas.find((entry) => entry.id === id);
    if (!registration) throw new ProviderError("provider_schema_denied");
    return this.schema(
      create(JsonSchemaSchema, {
        schemaId: id,
        schemaVersion: registration.version,
        canonicalJson: await read_private(registration.path, 65_536),
        schemaSha256: { value: Buffer.from(registration.sha256, "hex") },
      }),
    );
  }

  async prepare(
    invocation: ModelInvocation,
    model: ModelRoute,
    actor: string,
    snapshot: ModelResolutionSnapshot,
  ): Promise<NativeInput> {
    if (
      invocation.messages.length < 1 ||
      invocation.messages.length > 512 ||
      invocation.tools.length > 128 ||
      (invocation.tools.length > 0 && !model.features.tools)
    )
      content_denied();
    if (
      model.reasoning === "enabled" &&
      BigInt(model.thinking_tokens ?? 0) >= (invocation.budget?.maximumOutputTokens ?? 0n)
    )
      throw new ProviderError("provider_thinking_budget");
    const tools: NativeTool[] = [];
    for (const definition of invocation.tools) {
      if (typeof definition.strict !== "boolean") content_denied();
      const schema = await this.schema(definition.inputSchema);
      if (schema.value.type !== "object") content_denied();
      tools.push({
        name: valid_name(definition.name),
        description: valid_text(definition.description, true),
        strict: definition.strict,
        schema,
      });
    }
    if (new Set(tools.map((tool) => tool.name)).size !== tools.length) content_denied();
    let structured: NativeTool | undefined;
    if (invocation.structuredOutput) {
      const definition = invocation.structuredOutput;
      if (!model.features.structured_output || typeof definition.strict !== "boolean")
        content_denied();
      if (model.plugin === "anthropic" && !definition.strict) content_denied();
      structured = {
        name: valid_name(definition.name),
        description: valid_text(definition.description, true),
        strict: definition.strict,
        schema: await this.schema(definition.jsonSchema),
      };
    }
    let choice: NativeInput["choice"] = tools.length ? "auto" : "none";
    if (invocation.toolChoice) {
      const selection = invocation.toolChoice;
      if (selection.mode !== ToolChoiceMode.NAMED && selection.namedTool) content_denied();
      if (selection.mode === ToolChoiceMode.NONE) choice = "none";
      else if (selection.mode === ToolChoiceMode.AUTO && tools.length) choice = "auto";
      else if (selection.mode === ToolChoiceMode.REQUIRED && tools.length) choice = "required";
      else if (
        selection.mode === ToolChoiceMode.NAMED &&
        tools.some((tool) => tool.name === selection.namedTool)
      )
        choice = { name: selection.namedTool };
      else content_denied();
    }
    if (
      model.plugin === "anthropic" &&
      model.reasoning !== "off" &&
      (choice === "required" || typeof choice === "object")
    )
      content_denied();
    const messages: NativeMessage[] = [];
    const pending = new Set<string>();
    const calls = new Set<string>();
    let bytes = 0;
    let artifacts = 0;
    let non_system = false;
    for (const message of invocation.messages) {
      const role = (
        {
          [ModelRole.SYSTEM]: "system",
          [ModelRole.USER]: "user",
          [ModelRole.ASSISTANT]: "assistant",
          [ModelRole.TOOL]: "tool",
        } as const
      )[message.role as ModelRole.SYSTEM | ModelRole.USER | ModelRole.ASSISTANT | ModelRole.TOOL];
      if (
        !role ||
        (role === "system" && non_system) ||
        (pending.size > 0 && role !== "tool") ||
        message.content.length < 1 ||
        message.content.length > 256
      )
        content_denied();
      non_system ||= role !== "system";
      const content: NativeContent[] = [];
      for (const { content: block } of message.content) {
        if (block.case === "text" && role !== "tool")
          content.push({ kind: "text", text: valid_text(block.value.text, true) });
        else if (block.case === "refusal" && role === "assistant")
          content.push({ kind: "refusal", text: valid_text(block.value.reason) });
        else if (block.case === "structuredOutput" && role === "assistant") {
          const document = block.value.output;
          if (!document) content_denied();
          content.push({
            kind: "text",
            text: check_document(document, await this.document_schema(document.schemaId)),
          });
        } else if (block.case === "toolCall" && role === "assistant" && model.features.tools) {
          const call = block.value;
          const tool = tools.find((entry) => entry.name === call.toolName);
          if (!tool || calls.has(call.toolCallId)) content_denied();
          valid_name(call.toolCallId);
          calls.add(call.toolCallId);
          pending.add(call.toolCallId);
          content.push({
            kind: "tool_call",
            id: call.toolCallId,
            name: tool.name,
            arguments: check_document(call.arguments, tool.schema),
          });
        } else if (block.case === "toolResult" && role === "tool" && model.features.tools) {
          const result = block.value;
          if (
            !pending.delete(result.toolCallId) ||
            ![ToolResultStatus.SUCCESS, ToolResultStatus.ERROR].includes(result.status)
          )
            content_denied();
          let text: string;
          if (result.result.case === "text") text = valid_text(result.result.value.text, true);
          else if (result.result.case === "json")
            text = check_document(
              result.result.value,
              await this.document_schema(result.result.value.schemaId),
            );
          else if (result.result.case === "artifact") {
            const artifact = await read_prompt(this.config, actor, result.result.value);
            if (!["text/plain", "application/json"].includes(artifact.media)) content_denied();
            text = new TextDecoder("utf-8", { fatal: true }).decode(artifact.bytes);
          } else content_denied();
          content.push({
            kind: "tool_result",
            id: result.toolCallId,
            error: result.status === ToolResultStatus.ERROR,
            text,
          });
        } else if (
          block.case === "reasoning" &&
          role === "assistant" &&
          model.reasoning !== "off"
        ) {
          content.push({
            kind: "reasoning",
            text: valid_text(block.value.text, true),
            state: await read_continuation(this.config, actor, snapshot, block.value),
          });
        } else if (block.case === "image" && role === "user" && model.features.vision) {
          const artifact = await read_prompt(this.config, actor, block.value.artifact);
          const detail = (
            {
              [ImageDetail.AUTO]: "auto",
              [ImageDetail.LOW]: "low",
              [ImageDetail.HIGH]: "high",
            } as const
          )[block.value.detail as ImageDetail.AUTO | ImageDetail.LOW | ImageDetail.HIGH];
          if (
            !detail ||
            !["image/png", "image/jpeg"].includes(artifact.media) ||
            (model.plugin === "anthropic" && detail !== "auto")
          )
            content_denied();
          artifacts += artifact.bytes.length;
          if (artifacts > 4_194_304) content_denied();
          content.push({
            kind: "image",
            media: artifact.media as "image/png" | "image/jpeg",
            data: artifact.bytes.toString("base64"),
            detail,
          });
        } else if (block.case === "document" && role === "user" && model.features.documents) {
          const artifact = await read_prompt(this.config, actor, block.value.artifact);
          if (artifact.media !== "application/pdf") content_denied();
          artifacts += artifact.bytes.length;
          if (artifacts > 4_194_304) content_denied();
          content.push({
            kind: "document",
            media: "application/pdf",
            data: artifact.bytes.toString("base64"),
          });
        } else content_denied();
        const added = content.at(-1);
        if (added && "text" in added) bytes += Buffer.byteLength(added.text);
        if (added?.kind === "tool_call") bytes += Buffer.byteLength(added.arguments);
        if (bytes > 262_144) content_denied();
      }
      if (
        !model.features.parallel_tools &&
        content.filter((block) => block.kind === "tool_call").length > 1
      )
        content_denied();
      messages.push({ role, content });
    }
    if (
      pending.size ||
      bytes < 1 ||
      bytes > 262_144 ||
      artifacts > 4_194_304 ||
      !messages.some((message) => message.role === "user") ||
      !["user", "tool"].includes(messages.at(-1)?.role ?? "")
    )
      content_denied();
    return {
      model,
      messages,
      tools,
      choice,
      ...(structured ? { structured } : {}),
      output_tokens: Number(invocation.budget?.maximumOutputTokens ?? 0n),
    };
  }

  async response(
    reply: NativeReply,
    input: NativeInput,
    actor: string,
    snapshot: ModelResolutionSnapshot,
  ): Promise<ContentBlock[]> {
    function invalid_reply(): never {
      throw new ProviderError("invalid_provider_content", Code.DataLoss, ErrorCategory.DEPENDENCY);
    }
    if (reply.blocks.length < 1 || reply.blocks.length > 256) invalid_reply();
    let output_size = 0;
    for (const block of reply.blocks) {
      output_size += Buffer.byteLength(block.kind === "tool_call" ? block.arguments : block.text);
      if (output_size > 262_144) invalid_reply();
    }
    const calls = reply.blocks.filter((block) => block.kind === "tool_call");
    if (
      (calls.length > 0 && (reply.finish !== "tool_call" || input.choice === "none")) ||
      (reply.finish === "tool_call" && calls.length === 0) ||
      (!input.model.features.parallel_tools && calls.length > 1) ||
      (reply.finish === "stop" &&
        (input.choice === "required" || typeof input.choice === "object")) ||
      new Set(calls.map((call) => call.id)).size !== calls.length
    )
      invalid_reply();
    const output: ContentBlock[] = [];
    for (const block of reply.blocks) {
      if (block.kind === "text")
        output.push(
          create(ContentBlockSchema, {
            content: { case: "text", value: { text: valid_text(block.text, true) } },
          }),
        );
      else if (block.kind === "refusal")
        output.push(
          create(ContentBlockSchema, {
            content: { case: "refusal", value: { reason: valid_text(block.text) } },
          }),
        );
      else if (block.kind === "tool_call") {
        const tool = input.tools.find((entry) => entry.name === block.name);
        if (!tool || (typeof input.choice === "object" && input.choice.name !== tool.name))
          invalid_reply();
        output.push(
          create(ContentBlockSchema, {
            content: {
              case: "toolCall",
              value: {
                toolCallId: valid_name(block.id),
                toolName: tool.name,
                arguments: make_document(block.arguments, tool.schema),
              },
            },
          }),
        );
      } else if (block.kind === "reasoning") {
        if (input.model.reasoning === "off") invalid_reply();
        output.push(
          create(ContentBlockSchema, {
            content: {
              case: "reasoning",
              value: {
                text: valid_text(block.text, true),
                continuation: await save_continuation(
                  this.config,
                  actor,
                  snapshot,
                  block.text,
                  block.state,
                ),
              },
            },
          }),
        );
      } else invalid_reply();
    }
    if (input.structured && reply.finish === "stop") {
      const texts = output.filter((block) => block.content.case === "text");
      if (texts.length !== 1) invalid_reply();
      const block = texts[0];
      if (block?.content.case !== "text") invalid_reply();
      block.content = {
        case: "structuredOutput",
        value: {
          $typeName: "loop.v1.StructuredOutputContent",
          output: make_document(block.content.value.text, input.structured.schema),
        },
      };
    }
    return output;
  }
}
