import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import { clone, create } from "@bufbuild/protobuf";
import {
  JsonSchemaSchema,
  ToolChoiceMode,
  ToolDefinitionSchema,
} from "@loop-engine/protocol/provider";
import type { Deployment } from "../src/config.js";
import { hex_digest } from "../src/identity.js";
import { json_bytes, json_digest } from "../src/json.js";
import { test_fixture, test_request } from "./fixture.js";

export const SCHEMA_BYTES = json_bytes({
  type: "object",
  properties: { window: { type: "integer", minimum: 1 } },
  required: ["window"],
  additionalProperties: false,
});
export const TEST_SCHEMA = create(JsonSchemaSchema, {
  schemaId: "factor-window",
  schemaVersion: 1,
  canonicalJson: SCHEMA_BYTES,
  schemaSha256: { value: json_digest(SCHEMA_BYTES) },
});

export async function rich_fixture(directory: string, configure?: (config: Deployment) => void) {
  const schema_path = join(directory, "window.json");
  await writeFile(schema_path, SCHEMA_BYTES, { mode: 0o600 });
  return test_fixture(directory, (config) => {
    config.prompts = join(directory, "prompts");
    config.schemas.push({
      id: "factor-window",
      version: 1,
      path: schema_path,
      sha256: hex_digest(json_digest(SCHEMA_BYTES)),
    });
    for (const model of config.models) {
      model.features = {
        streaming: true,
        tools: true,
        parallel_tools: true,
        structured_output: true,
        vision: true,
        documents: true,
        prompt_caching: true,
      };
      if (model.plugin === "anthropic") model.cache_creation_usd = "2";
    }
    configure?.(config);
  });
}

export function tool_request(
  fixture: Awaited<ReturnType<typeof rich_fixture>>,
  model = "responses",
) {
  const command = test_request(fixture.host, model);
  if (!command.invocation) throw new Error("fixture_invocation_missing");
  command.invocation.tools.push(
    create(ToolDefinitionSchema, {
      name: "propose_factor",
      description: "Propose a window",
      inputSchema: clone(JsonSchemaSchema, TEST_SCHEMA),
      strict: true,
    }),
  );
  command.invocation.toolChoice = {
    $typeName: "loop.v1.ToolChoice",
    mode: ToolChoiceMode.AUTO,
    namedTool: "",
  };
  return command;
}

export function tool_reply(model: string, args = '{"window":20}') {
  const id = `${model}-fixture-20260901`;
  if (model === "claude")
    return {
      id: "m1",
      type: "message",
      role: "assistant",
      model: id,
      stop_reason: "tool_use",
      stop_sequence: null,
      content: [
        { type: "tool_use", id: "call_1", name: "propose_factor", input: JSON.parse(args) },
      ],
      usage: {
        input_tokens: 9,
        output_tokens: 5,
        cache_read_input_tokens: 3,
        cache_creation_input_tokens: 0,
      },
    };
  if (model === "chat")
    return {
      id: "c1",
      object: "chat.completion",
      model: id,
      created: 1,
      choices: [
        {
          index: 0,
          finish_reason: "tool_calls",
          message: {
            role: "assistant",
            content: null,
            refusal: null,
            tool_calls: [
              {
                type: "function",
                id: "call_1",
                function: { name: "propose_factor", arguments: args },
              },
            ],
          },
        },
      ],
      usage: { prompt_tokens: 12, completion_tokens: 5 },
    };
  return {
    id: "r1",
    object: "response",
    model: id,
    status: "completed",
    error: null,
    output: [
      {
        type: "function_call",
        id: "fc1",
        call_id: "call_1",
        name: "propose_factor",
        arguments: args,
        status: "completed",
      },
    ],
    usage: { input_tokens: 12, output_tokens: 5 },
  };
}

export function sse_bytes(events: readonly unknown[], terminal = false): Uint8Array[] {
  const text =
    events
      .map((event) => {
        const name =
          typeof event === "object" && event !== null && "type" in event
            ? `event: ${event.type}\n`
            : "";
        return `${name}data: ${JSON.stringify(event)}\n\n`;
      })
      .join("") + (terminal ? "data: [DONE]\n\n" : "");
  const bytes = Buffer.from(text);
  const chunks: Uint8Array[] = [];
  for (let index = 0; index < bytes.length; index += 47)
    chunks.push(bytes.subarray(index, index + 47));
  return chunks;
}

export function text_events(model: string, text = "研究 idea") {
  const id = `${model}-fixture-20260901`;
  if (model === "claude")
    return [
      {
        type: "message_start",
        message: {
          id: "m1",
          model: id,
          role: "assistant",
          type: "message",
          content: [],
          stop_reason: null,
          stop_sequence: null,
          usage: { input_tokens: 12, output_tokens: 0 },
        },
      },
      { type: "content_block_start", index: 0, content_block: { type: "text", text: "" } },
      { type: "content_block_delta", index: 0, delta: { type: "text_delta", text } },
      { type: "content_block_stop", index: 0 },
      {
        type: "message_delta",
        delta: { stop_reason: "end_turn", stop_sequence: null },
        usage: { output_tokens: 5 },
      },
      { type: "message_stop" },
    ];
  if (model === "chat")
    return [
      {
        id: "c1",
        object: "chat.completion.chunk",
        model: id,
        choices: [{ index: 0, delta: { role: "assistant", content: text }, finish_reason: null }],
      },
      {
        id: "c1",
        object: "chat.completion.chunk",
        model: id,
        choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
      },
      {
        id: "c1",
        object: "chat.completion.chunk",
        model: id,
        choices: [],
        usage: { prompt_tokens: 12, completion_tokens: 5 },
      },
    ];
  const part = { type: "output_text", text, annotations: [] };
  const item = {
    type: "message",
    id: "m1",
    role: "assistant",
    status: "completed",
    content: [part],
  };
  return [
    { type: "response.created", response: { id: "r1", model: id } },
    {
      type: "response.output_item.added",
      output_index: 0,
      item: { ...item, status: "in_progress", content: [] },
    },
    {
      type: "response.content_part.added",
      output_index: 0,
      item_id: "m1",
      content_index: 0,
      part: { ...part, text: "" },
    },
    {
      type: "response.output_text.delta",
      output_index: 0,
      item_id: "m1",
      content_index: 0,
      delta: text,
    },
    { type: "response.output_text.done", output_index: 0, item_id: "m1", content_index: 0, text },
    { type: "response.content_part.done", output_index: 0, item_id: "m1", content_index: 0, part },
    { type: "response.output_item.done", output_index: 0, item },
    {
      type: "response.completed",
      response: {
        id: "r1",
        model: id,
        status: "completed",
        error: null,
        output: [item],
        usage: { input_tokens: 12, output_tokens: 5 },
      },
    },
  ].map((event, sequence_number) => ({ ...event, sequence_number }));
}

export function tool_events(model: string, count = 1) {
  const id = `${model}-fixture-20260901`;
  const parts = ['{"window":', "20}"];
  const events: unknown[] = [];
  if (model === "claude") {
    events.push({
      type: "message_start",
      message: {
        id: "m1",
        model: id,
        type: "message",
        role: "assistant",
        content: [],
        stop_reason: null,
        stop_sequence: null,
        usage: { input_tokens: 12, output_tokens: 0 },
      },
    });
    for (let index = 0; index < count; index++)
      events.push({
        type: "content_block_start",
        index,
        content_block: { type: "tool_use", id: `call_${index}`, name: "propose_factor", input: {} },
      });
    for (const partial_json of parts)
      for (let index = 0; index < count; index++)
        events.push({
          type: "content_block_delta",
          index,
          delta: { type: "input_json_delta", partial_json },
        });
    for (let index = 0; index < count; index++) events.push({ type: "content_block_stop", index });
    events.push(
      {
        type: "message_delta",
        delta: { stop_reason: "tool_use", stop_sequence: null },
        usage: { output_tokens: 5 },
      },
      { type: "message_stop" },
    );
    return events;
  }
  if (model === "chat") {
    events.push({
      id: "c1",
      model: id,
      choices: [
        {
          index: 0,
          delta: {
            role: "assistant",
            tool_calls: Array.from({ length: count }, (_, index) => ({
              index,
              id: `call_${index}`,
              type: "function",
              function: { name: "propose_factor", arguments: "" },
            })),
          },
          finish_reason: null,
        },
      ],
    });
    for (const fragment of parts)
      for (let index = 0; index < count; index++)
        events.push({
          id: "c1",
          model: id,
          choices: [
            {
              index: 0,
              delta: { tool_calls: [{ index, function: { arguments: fragment } }] },
              finish_reason: null,
            },
          ],
        });
    events.push(
      { id: "c1", model: id, choices: [{ index: 0, delta: {}, finish_reason: "tool_calls" }] },
      { id: "c1", model: id, choices: [], usage: { prompt_tokens: 12, completion_tokens: 5 } },
    );
    return events;
  }
  events.push({ type: "response.created", response: { id: "r1", model: id } });
  const output = Array.from({ length: count }, (_, index) => ({
    type: "function_call",
    id: `fc${index}`,
    call_id: `call_${index}`,
    name: "propose_factor",
    arguments: parts.join(""),
    status: "completed",
  }));
  output.forEach((item, output_index) => {
    events.push({
      type: "response.output_item.added",
      output_index,
      item: { ...item, status: "in_progress", arguments: "" },
    });
  });
  for (const delta of parts)
    output.forEach((item, output_index) => {
      events.push({
        type: "response.function_call_arguments.delta",
        output_index,
        item_id: item.id,
        delta,
      });
    });
  output.forEach((item, output_index) => {
    events.push(
      {
        type: "response.function_call_arguments.done",
        output_index,
        item_id: item.id,
        arguments: item.arguments,
      },
      { type: "response.output_item.done", output_index, item },
    );
  });
  events.push({
    type: "response.completed",
    response: {
      id: "r1",
      model: id,
      status: "completed",
      error: null,
      output,
      usage: { input_tokens: 12, output_tokens: 5 },
    },
  });
  return events.map((event, sequence_number) => ({
    ...(event as Record<string, unknown>),
    sequence_number,
  }));
}

export function reasoning_events(model: string) {
  const text = JSON.parse(JSON.stringify(text_events(model))) as Record<string, unknown>[];
  const first = text.shift();
  if (!first) throw new Error("missing_stream_start");
  if (model === "claude")
    return [
      first,
      {
        type: "content_block_start",
        index: 0,
        content_block: { type: "thinking", thinking: "", signature: "" },
      },
      {
        type: "content_block_delta",
        index: 0,
        delta: { type: "thinking_delta", thinking: "public summary" },
      },
      {
        type: "content_block_delta",
        index: 0,
        delta: { type: "signature_delta", signature: "private-" },
      },
      {
        type: "content_block_delta",
        index: 0,
        delta: { type: "signature_delta", signature: "signature" },
      },
      { type: "content_block_stop", index: 0 },
      ...text.map((event) =>
        typeof event.index === "number" ? { ...event, index: event.index + 1 } : event,
      ),
    ];
  const item = {
    type: "reasoning",
    id: "rs1",
    summary: [{ type: "summary_text", text: "public summary" }],
    encrypted_content: "private-ciphertext",
  };
  const end = text.pop();
  const response = end?.response as { output: unknown[] };
  response.output.unshift(item);
  return [
    first,
    {
      type: "response.output_item.added",
      output_index: 0,
      item: { type: "reasoning", id: "rs1", summary: [] },
    },
    {
      type: "response.reasoning_summary_part.added",
      output_index: 0,
      item_id: "rs1",
      summary_index: 0,
      part: { type: "summary_text", text: "" },
    },
    {
      type: "response.reasoning_summary_text.delta",
      output_index: 0,
      item_id: "rs1",
      summary_index: 0,
      delta: "public summary",
    },
    {
      type: "response.reasoning_summary_text.done",
      output_index: 0,
      item_id: "rs1",
      summary_index: 0,
      text: "public summary",
    },
    {
      type: "response.reasoning_summary_part.done",
      output_index: 0,
      item_id: "rs1",
      summary_index: 0,
      part: item.summary[0],
    },
    { type: "response.output_item.done", output_index: 0, item },
    ...text.map((event) =>
      typeof event.output_index === "number"
        ? { ...event, output_index: event.output_index + 1 }
        : event,
    ),
    end,
  ].map((event, sequence_number) => ({ ...event, sequence_number }));
}
