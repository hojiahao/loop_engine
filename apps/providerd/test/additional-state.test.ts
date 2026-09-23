import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { create } from "@bufbuild/protobuf";
import { timestampNow } from "@bufbuild/protobuf/wkt";
import { Code, ConnectError } from "@connectrpc/connect";
import {
  ArtifactRefSchema,
  ImageDetail,
  ModelMessageSchema,
  ModelRole,
  StreamModelRequestSchema,
  StructuredOutputDefinitionSchema,
  ToolDefinitionSchema,
  ToolResultStatus,
} from "@loop-engine/protocol/provider";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { model_snapshot, validate_deployment } from "../src/config.js";
import { hex_digest } from "../src/identity.js";
import { json_digest } from "../src/json.js";
import { actor_directory, prompt_schema } from "../src/private-state.js";
import {
  ADDITIONAL,
  type Additional,
  additional_fixture,
  additional_reply,
  thinking_config,
} from "./additional-fixture.js";
import { test_request } from "./fixture.js";
import { sse_bytes, TEST_SCHEMA, tool_request } from "./rich-fixture.js";

let directory: string;
let fixture: Awaited<ReturnType<typeof additional_fixture>>;
beforeAll(async () => {
  directory = await mkdtemp(join(tmpdir(), "loop-provider-state-"));
  fixture = await additional_fixture(directory, thinking_config);
});
afterAll(async () => {
  await fixture?.close();
  if (directory) await rm(directory, { recursive: true, force: true });
});
beforeEach(() => {
  fixture.requests.length = 0;
  fixture.state.reply = undefined;
  fixture.state.events = undefined;
});
function request(plugin: Additional) {
  const command = test_request(fixture.host, plugin);
  if (plugin === "cohere") {
    if (!command.invocation?.budget?.maximumCost?.amount) throw new Error("missing_budget");
    command.invocation.budget.maximumOutputTokens = 1536n;
    command.invocation.budget.maximumCost.amount.value = "0.01";
  }
  return command;
}
async function artifact(media: string, bytes: Buffer) {
  const root = fixture.config.prompts;
  if (!root) throw new Error("missing_prompts");
  const digest = json_digest(bytes);
  const id = hex_digest(digest);
  const view = join(root, actor_directory(fixture.principal.actor_id));
  await mkdir(view, { recursive: true, mode: 0o700 });
  await writeFile(join(view, id), bytes, { mode: 0o600 });
  return create(ArtifactRefSchema, {
    artifactId: { value: id },
    uri: `loop-prompt://sha256/${id}`,
    sha256: { value: digest },
    schema: {
      name: "loop.prompt-artifact",
      version: 1,
      schemaSha256: { value: prompt_schema(media) },
    },
    mediaType: media,
    byteSize: BigInt(bytes.length),
    createdAt: timestampNow(),
  });
}
function thinking_events(plugin: Additional) {
  if (plugin === "cohere")
    return [
      { type: "message-start", id: "c1", delta: { message: { role: "assistant" } } },
      {
        type: "content-start",
        index: 0,
        delta: { message: { content: { type: "thinking", thinking: "" } } },
      },
      { type: "content-delta", index: 0, delta: { message: { content: { thinking: "Summary" } } } },
      { type: "content-end", index: 0 },
      {
        type: "content-start",
        index: 1,
        delta: { message: { content: { type: "text", text: "idea" } } },
      },
      { type: "content-end", index: 1 },
      {
        type: "message-end",
        delta: {
          finish_reason: "COMPLETE",
          usage: { tokens: { input_tokens: 12, output_tokens: 8 } },
        },
      },
    ];
  if (plugin === "google_interactions")
    return [
      {
        event_type: "interaction.created",
        interaction: { id: "i1", model: `${plugin}-fixture-20260901`, status: "in_progress" },
      },
      { event_type: "step.start", index: 0, step: { type: "thought" } },
      {
        event_type: "step.delta",
        index: 0,
        delta: { type: "thought_summary", content: { type: "text", text: "Summary" } },
      },
      {
        event_type: "step.delta",
        index: 0,
        delta: { type: "thought_signature", signature: "signed-thinking" },
      },
      { event_type: "step.stop", index: 0 },
      { event_type: "step.start", index: 1, step: { type: "model_output", content: [] } },
      { event_type: "step.delta", index: 1, delta: { type: "text", text: "idea" } },
      { event_type: "step.stop", index: 1 },
      {
        event_type: "interaction.completed",
        interaction: {
          id: "i1",
          status: "completed",
          usage: {
            total_input_tokens: 12,
            total_output_tokens: 5,
            total_thought_tokens: 3,
            total_tokens: 20,
          },
        },
      },
    ];
  return [
    {
      responseId: "g1",
      modelVersion: `${plugin}-fixture-20260901`,
      candidates: [{ content: { role: "model", parts: [{ thought: true, text: "Summary" }] } }],
    },
    {
      responseId: "g1",
      modelVersion: `${plugin}-fixture-20260901`,
      candidates: [
        {
          content: {
            role: "model",
            parts: [{ text: "idea", thoughtSignature: "signed-thinking" }],
          },
          finishReason: "STOP",
        },
      ],
      usageMetadata: {
        promptTokenCount: 12,
        candidatesTokenCount: 5,
        thoughtsTokenCount: 3,
        totalTokenCount: 20,
      },
    },
  ];
}

describe("additional native state and capabilities", () => {
  it.each(ADDITIONAL)("streams %s thinking with private continuation", async (plugin) => {
    fixture.state.events = sse_bytes(thinking_events(plugin));
    const command = request(plugin);
    const events = [];
    for await (const result of fixture.client().streamModel(
      create(StreamModelRequestSchema, {
        context: command.context,
        invocation: command.invocation,
      }),
      { timeoutMs: 4500 },
    ))
      events.push(result.event);
    expect(events.at(-1)?.event).toMatchObject({
      case: "completed",
      value: {
        response: {
          content: [
            {
              content: {
                case: "reasoning",
                value: {
                  text: "Summary",
                  continuation: {
                    providerId: { value: plugin === "cohere" ? "cohere" : "google" },
                  },
                },
              },
            },
            { content: { case: "text", value: { text: "idea" } } },
          ],
          usage: {
            inputTokens: 12n,
            outputTokens: 8n,
            reasoningTokens: plugin === "cohere" ? 0n : 3n,
          },
        },
      },
    });
    expect(
      JSON.stringify(events, (_key, value) =>
        typeof value === "bigint" ? value.toString() : value,
      ),
    ).not.toContain("signed-thinking");
  });

  it("keeps Cohere tool plans after thinking content", async () => {
    fixture.state.events = sse_bytes([
      ...thinking_events("cohere").slice(0, 4),
      { type: "tool-plan-delta", delta: { message: { tool_plan: "Propose a window." } } },
      {
        type: "tool-call-start",
        index: 0,
        delta: {
          message: {
            tool_calls: {
              type: "function",
              id: "call_1",
              function: { name: "propose_factor", arguments: "" },
            },
          },
        },
      },
      {
        type: "tool-call-delta",
        index: 0,
        delta: { message: { tool_calls: { function: { arguments: '{"window":20}' } } } },
      },
      { type: "tool-call-end", index: 0 },
      {
        type: "message-end",
        delta: {
          finish_reason: "TOOL_CALL",
          usage: { tokens: { input_tokens: 12, output_tokens: 8 } },
        },
      },
    ]);
    const command = tool_request(fixture, "cohere");
    if (!command.invocation) throw new Error("missing_invocation");
    command.invocation.budget = request("cohere").invocation?.budget;
    const events = [];
    for await (const result of fixture.client().streamModel(
      create(StreamModelRequestSchema, {
        context: command.context,
        invocation: command.invocation,
      }),
      { timeoutMs: 4500 },
    ))
      events.push(result.event);
    expect(events.at(-1)?.event).toMatchObject({
      case: "completed",
      value: {
        response: {
          content: [
            { content: { case: "reasoning", value: { text: "Summary" } } },
            { content: { case: "text", value: { text: "Propose a window." } } },
            { content: { case: "toolCall", value: { toolCallId: "call_1" } } },
          ],
        },
      },
    });
  });

  it.each(ADDITIONAL)("sends verified private image bytes to %s", async (plugin) => {
    const bytes = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
    const reference = await artifact("image/png", bytes);
    const command = request(plugin);
    command.invocation?.messages.at(-1)?.content.push({
      $typeName: "loop.v1.ContentBlock",
      content: {
        case: "image",
        value: {
          $typeName: "loop.v1.ImageContent",
          artifact: reference,
          detail: ImageDetail.AUTO,
        },
      },
    });
    fixture.state.reply = additional_reply(plugin, false, plugin !== "cohere");
    await fixture.client().invokeModel(command, { timeoutMs: 4500 });
    expect(JSON.stringify(fixture.requests.at(-1)?.body)).toContain(bytes.toString("base64"));
    expect(JSON.stringify(fixture.requests.at(-1)?.body)).not.toContain("loop-prompt:");
  });

  it.each(["google_generate", "google_interactions"] as const)(
    "sends a private PDF through %s",
    async (plugin) => {
      const bytes = Buffer.from("%PDF-1.4\nfixture\n%%EOF");
      const reference = await artifact("application/pdf", bytes);
      const command = request(plugin);
      command.invocation?.messages.at(-1)?.content.push({
        $typeName: "loop.v1.ContentBlock",
        content: {
          case: "document",
          value: { $typeName: "loop.v1.DocumentContent", artifact: reference },
        },
      });
      fixture.state.reply = additional_reply(plugin, false, true);
      await fixture.client().invokeModel(command, { timeoutMs: 4500 });
      expect(JSON.stringify(fixture.requests.at(-1)?.body)).toContain(bytes.toString("base64"));
    },
  );

  it("binds signed Gemini parts to their visible tool call", async () => {
    fixture.state.reply = additional_reply("google_generate", true, true);
    const response = (
      await fixture
        .client()
        .invokeModel(tool_request(fixture, "google_generate"), { timeoutMs: 4500 })
    ).response;
    const call = response?.content.find((block) => block.content.case === "toolCall");
    if (call?.content.case !== "toolCall") throw new Error("missing_call");
    call.content.value.toolName = "another_factor";
    const next = tool_request(fixture, "google_generate");
    next.invocation?.tools.push(
      create(ToolDefinitionSchema, {
        name: "another_factor",
        description: "Another valid tool",
        strict: true,
        inputSchema: TEST_SCHEMA,
      }),
    );
    next.invocation?.messages.push(
      create(ModelMessageSchema, { role: ModelRole.ASSISTANT, content: response?.content }),
      create(ModelMessageSchema, {
        role: ModelRole.TOOL,
        content: [
          {
            content: {
              case: "toolResult",
              value: {
                toolCallId: call.content.value.toolCallId,
                status: ToolResultStatus.SUCCESS,
                result: { case: "text", value: { text: "done" } },
              },
            },
          },
        ],
      }),
    );
    const before = fixture.requests.length;
    await expect(fixture.client().invokeModel(next, { timeoutMs: 4500 })).rejects.toThrow(
      "google_continuation_denied",
    );
    expect(fixture.requests).toHaveLength(before);
  });

  it("replays signed Gemini JSON after canonicalization", async () => {
    const command = request("google_generate");
    if (!command.invocation) throw new Error("missing_invocation");
    command.invocation.structuredOutput = create(StructuredOutputDefinitionSchema, {
      name: "window",
      strict: true,
      jsonSchema: TEST_SCHEMA,
    });
    const original = '{ "window": 20 }';
    fixture.state.reply = JSON.parse(
      JSON.stringify(additional_reply("google_generate", false, true)).replace(
        "diagnostic idea",
        original.replaceAll('"', '\\"'),
      ),
    );
    const response = (await fixture.client().invokeModel(command, { timeoutMs: 4500 })).response;
    const output = response?.content.find((block) => block.content.case === "structuredOutput");
    if (output?.content.case !== "structuredOutput") throw new Error("missing_output");
    const document = output.content.value.output;
    if (!document) throw new Error("missing_document");
    expect(new TextDecoder().decode(document.utf8Json)).toBe(original);
    // A client may normalize JSON whitespace without changing its canonical digest.
    document.utf8Json = new TextEncoder().encode('{"window":20}');
    const next = request("google_generate");
    next.invocation?.messages.push(
      create(ModelMessageSchema, { role: ModelRole.ASSISTANT, content: response?.content }),
      create(ModelMessageSchema, {
        role: ModelRole.USER,
        content: [{ content: { case: "text", value: { text: "Continue." } } }],
      }),
    );
    fixture.state.reply = additional_reply("google_generate", false, true);
    await fixture.client().invokeModel(next, { timeoutMs: 4500 });
    expect(fixture.requests.at(-1)?.body).toMatchObject({
      contents: [
        { role: "user" },
        {
          role: "model",
          parts: [
            { thought: true, text: "Summary" },
            { text: original, thoughtSignature: "signed-thinking" },
          ],
        },
        { role: "user" },
      ],
    });
  });

  it("groups parallel Gemini tool results in one user turn", async () => {
    const parts = ["call_1", "call_2"].map((id) => ({
      functionCall: { id, name: "propose_factor", args: { window: 20 } },
      thoughtSignature: `signed-${id}`,
    }));
    fixture.state.reply = {
      responseId: "parallel-fixture",
      modelVersion: "google_generate-fixture-20260901",
      candidates: [{ content: { role: "model", parts }, finishReason: "STOP" }],
      usageMetadata: { promptTokenCount: 12, candidatesTokenCount: 5, totalTokenCount: 17 },
    };
    const response = (
      await fixture
        .client()
        .invokeModel(tool_request(fixture, "google_generate"), { timeoutMs: 4500 })
    ).response;
    const next = tool_request(fixture, "google_generate");
    next.invocation?.messages.push(
      create(ModelMessageSchema, { role: ModelRole.ASSISTANT, content: response?.content }),
      ...["call_1", "call_2"].map((id) =>
        create(ModelMessageSchema, {
          role: ModelRole.TOOL,
          content: [
            {
              content: {
                case: "toolResult",
                value: {
                  toolCallId: id,
                  status: ToolResultStatus.SUCCESS,
                  result: { case: "text", value: { text: `result-${id}` } },
                },
              },
            },
          ],
        }),
      ),
    );
    fixture.state.reply = additional_reply("google_generate", false, true);
    await fixture.client().invokeModel(next, { timeoutMs: 4500 });
    expect(fixture.requests.at(-1)?.body).toMatchObject({
      contents: [
        { role: "user" },
        { role: "model", parts },
        {
          role: "user",
          parts: ["call_1", "call_2"].map((id) => ({
            functionResponse: {
              id,
              name: "propose_factor",
              response: { output: `result-${id}` },
            },
          })),
        },
      ],
    });
  });

  const invalid_steps = [
    {
      type: "model_output",
      content: [
        {
          type: "text",
          text: "Unrequested grounding",
          annotations: [{ type: "url_citation", url: "https://example.invalid/citation" }],
        },
      ],
    },
    { type: "thought", signature: "signed-thinking", summary: [{ type: "text", text: null }] },
  ];
  it.each(invalid_steps)("rejects unsupported Interactions $type content", async (step) => {
    fixture.state.reply = {
      ...additional_reply("google_interactions", false, true),
      steps: [step],
    };
    await expect(
      fixture.client().invokeModel(request("google_interactions"), { timeoutMs: 4500 }),
    ).rejects.toThrow("invalid_google_output");
  });

  it.each(invalid_steps)("rejects invalid $type before a stream preview", async (step) => {
    fixture.state.events = sse_bytes([
      {
        event_type: "interaction.created",
        interaction: {
          id: "invalid-content",
          model: "google_interactions-fixture-20260901",
          status: "in_progress",
        },
      },
      { event_type: "step.start", index: 0, step },
    ]);
    const command = request("google_interactions");
    const events: string[] = [];
    try {
      for await (const result of fixture.client().streamModel(
        create(StreamModelRequestSchema, {
          context: command.context,
          invocation: command.invocation,
        }),
        { timeoutMs: 4500 },
      ))
        events.push(result.event?.event.case ?? "");
      throw new Error("unexpected_success");
    } catch (error) {
      expect(ConnectError.from(error).code).toBe(Code.DataLoss);
    }
    expect(events).toEqual(["started"]);
  });

  it.each(["cohere", "google_interactions"])(
    "requires and pins the %s vendor input ceiling",
    (plugin) => {
      const config = structuredClone(fixture.config);
      const model = config.models.find((entry) => entry.plugin === plugin);
      if (!model) throw new Error("missing_model");
      const old = model_snapshot(config, model, new Uint8Array(32));
      model.input_token_limit = 256;
      expect(model_snapshot(config, model, new Uint8Array(32)).snapshotSha256).not.toEqual(
        old.snapshotSha256,
      );
      model.input_token_limit = model.context_tokens;
      expect(() => validate_deployment(config)).not.toThrow();
      model.input_token_limit++;
      expect(() => validate_deployment(config)).toThrow("invalid_provider_deployment");
      delete model.input_token_limit;
      expect(() => validate_deployment(config)).toThrow("invalid_provider_deployment");
    },
  );

  it("denies Cohere PDF capability at deployment", () => {
    const config = structuredClone(fixture.config);
    const model = config.models.find((entry) => entry.plugin === "cohere");
    if (!model) throw new Error("missing_model");
    model.features.documents = true;
    expect(() => validate_deployment(config)).toThrow("invalid_provider_deployment");
  });
});
