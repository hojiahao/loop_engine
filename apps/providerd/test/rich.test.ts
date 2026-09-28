import { mkdir, mkdtemp, readFile, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { clone, create } from "@bufbuild/protobuf";
import { timestampFromDate, timestampNow } from "@bufbuild/protobuf/wkt";
import {
  ArtifactRefSchema,
  ContentBlockSchema,
  ImageDetail,
  InvokeModelRequestSchema,
  ModelFinishReason,
  ModelMessageSchema,
  ModelRole,
  StructuredOutputDefinitionSchema,
  ToolResultStatus,
} from "@loop-engine/protocol/provider";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { ProviderHost } from "../src/host.js";
import { hex_digest } from "../src/identity.js";
import { json_digest } from "../src/json.js";
import { actor_directory, prompt_schema, save_continuation } from "../src/private-state.js";
import { TEST_SECRET, test_reply, test_request } from "./fixture.js";
import { rich_fixture, TEST_SCHEMA, tool_reply, tool_request } from "./rich-fixture.js";

let directory: string;
let fixture: Awaited<ReturnType<typeof rich_fixture>>;
beforeAll(async () => {
  directory = await mkdtemp(join(tmpdir(), "loop-provider-rich-"));
  fixture = await rich_fixture(directory);
});
afterAll(async () => {
  await fixture?.close();
  if (directory) await rm(directory, { recursive: true, force: true });
});
beforeEach(() => {
  fixture.requests.length = 0;
  fixture.state.reply = undefined;
  fixture.state.count = undefined;
});

async function invoke(command: ReturnType<typeof test_request>) {
  return (await fixture.client().invokeModel(command, { timeoutMs: 4500 })).response;
}

describe("native tools and schema output", () => {
  it.each(["responses", "chat", "claude"])(
    "round trips a %s tool result without executing it",
    async (model) => {
      fixture.state.reply = tool_reply(model);
      const command = tool_request(fixture, model);
      const response = await invoke(command);
      expect(response?.finishReason).toBe(ModelFinishReason.TOOL_CALL);
      const call = response?.content[0];
      expect(call?.content.case).toBe("toolCall");
      if (call?.content.case !== "toolCall") throw new Error("missing_call");
      expect(Buffer.from(call.content.value.arguments?.utf8Json ?? []).toString()).toBe(
        '{"window":20}',
      );
      const next = tool_request(fixture, model);
      next.invocation?.messages.push(
        create(ModelMessageSchema, { role: ModelRole.ASSISTANT, content: [call] }),
        create(ModelMessageSchema, {
          role: ModelRole.TOOL,
          content: [
            {
              content: {
                case: "toolResult",
                value: {
                  toolCallId: call.content.value.toolCallId,
                  status: ToolResultStatus.ERROR,
                  result: { case: "text", value: { text: "evaluation unavailable" } },
                },
              },
            },
          ],
        }),
      );
      fixture.state.reply = undefined;
      expect((await invoke(next))?.finishReason).toBe(ModelFinishReason.STOP);
      expect(JSON.stringify(fixture.requests.at(-1)?.body)).toContain("evaluation unavailable");
      expect(JSON.stringify(fixture.requests.at(-1)?.body)).toContain(
        model === "claude" ? '"is_error":true' : "error",
      );
      expect(fixture.requests).toHaveLength(4);
      expect(fixture.requests[0]?.body.tools).toHaveLength(1);
    },
  );

  it.each(["responses", "chat", "claude"])("validates %s structured output", async (model) => {
    const command = test_request(fixture.host, model);
    if (!command.invocation) throw new Error("missing_invocation");
    command.invocation.structuredOutput = create(StructuredOutputDefinitionSchema, {
      name: "factor_window",
      jsonSchema: TEST_SCHEMA,
      strict: true,
    });
    const reply = JSON.parse(
      JSON.stringify(
        test_reply(
          model === "chat" ? "/chat/completions" : model === "claude" ? "/messages" : "/responses",
          { model: `${model}-fixture-20260901` },
        ),
      ),
    );
    if (model === "claude") reply.content[0].text = '{"window":20}';
    else if (model === "chat") reply.choices[0].message.content = '{"window":20}';
    else reply.output[0].content[0].text = '{"window":20}';
    fixture.state.reply = reply;
    const result = await invoke(command);
    expect(result?.content[0]?.content.case).toBe("structuredOutput");
    const transport = fixture.requests.at(-1)?.body;
    expect(
      transport?.[
        model === "claude" ? "output_config" : model === "chat" ? "response_format" : "text"
      ],
    ).toBeDefined();
    expect(fixture.requests[0]?.body[model === "claude" ? "output_config" : "text"]).toBeDefined();
  });

  it.each(["responses", "chat", "claude"])(
    "rejects %s schema-invalid tool output",
    async (model) => {
      fixture.state.reply = tool_reply(model, '{"window":0}');
      await expect(invoke(tool_request(fixture, model))).rejects.toThrow("model_schema_mismatch");
    },
  );

  it("rejects unregistered or changed schemas before transport", async () => {
    const command = tool_request(fixture);
    const schema = command.invocation?.tools[0]?.inputSchema;
    if (!schema) throw new Error("missing_schema");
    const changed = clone(InvokeModelRequestSchema, command);
    const new_schema = changed.invocation?.tools[0]?.inputSchema;
    if (new_schema) new_schema.schemaId = "unregistered";
    await expect(invoke(changed)).rejects.toThrow("provider_schema_denied");
    const bytes = new TextEncoder().encode('{"type":"string"}');
    schema.canonicalJson = bytes;
    await expect(invoke(command)).rejects.toThrow("provider_schema_denied");
    expect(fixture.requests).toHaveLength(0);
  });

  it("rejects unmatched tool results before transport", async () => {
    const command = tool_request(fixture);
    command.invocation?.messages.push(
      create(ModelMessageSchema, {
        role: ModelRole.TOOL,
        content: [
          {
            content: {
              case: "toolResult",
              value: {
                toolCallId: "unknown",
                status: ToolResultStatus.SUCCESS,
                result: { case: "text", value: { text: "unexpected" } },
              },
            },
          },
        ],
      }),
    );
    await expect(invoke(command)).rejects.toThrow("unsupported_request_content");
    expect(fixture.requests).toHaveLength(0);
  });

  it("accounts for Claude cache writes without discounting its reserve", async () => {
    fixture.state.reply = {
      ...tool_reply("claude"),
      usage: {
        input_tokens: 9,
        output_tokens: 5,
        cache_read_input_tokens: 3,
        cache_creation_input_tokens: 7,
        cache_creation: { ephemeral_5m_input_tokens: 7, ephemeral_1h_input_tokens: 0 },
      },
    };
    const result = await invoke(tool_request(fixture, "claude"));
    expect(result?.usage).toMatchObject({
      inputTokens: 19n,
      cachedInputTokens: 3n,
      cacheCreationInputTokens: 7n,
    });
    expect(fixture.requests.at(-1)?.body.cache_control).toEqual({ type: "ephemeral", ttl: "5m" });
    fixture.requests.length = 0;
    const tight = tool_request(fixture, "claude");
    if (tight.invocation?.budget?.maximumCost?.amount)
      tight.invocation.budget.maximumCost.amount.value = "0.0003";
    await expect(invoke(tight)).rejects.toThrow("provider_budget_denied");
    expect(fixture.requests).toHaveLength(0);
  });

  it("rejects unpriced one-hour cache creation", async () => {
    fixture.state.reply = {
      ...tool_reply("claude"),
      usage: {
        input_tokens: 9,
        output_tokens: 5,
        cache_read_input_tokens: 0,
        cache_creation_input_tokens: 7,
        cache_creation: { ephemeral_5m_input_tokens: 0, ephemeral_1h_input_tokens: 7 },
      },
    };
    await expect(invoke(tool_request(fixture, "claude"))).rejects.toThrow(
      "invalid_anthropic_output",
    );
  });
});

async function prompt_artifact(media: string, bytes: Buffer) {
  const root = fixture.config.prompts;
  if (!root) throw new Error("missing_prompts");
  const view = join(root, actor_directory(fixture.principal.actor_id));
  await mkdir(view, { recursive: true, mode: 0o700 });
  const digest = hex_digest(json_digest(bytes));
  const path = join(view, digest);
  await writeFile(path, bytes, { mode: 0o600 });
  return {
    path,
    reference: create(ArtifactRefSchema, {
      artifactId: { value: digest },
      uri: `loop-prompt://sha256/${digest}`,
      sha256: { value: json_digest(bytes) },
      schema: {
        name: "loop.prompt-artifact",
        version: 1,
        schemaSha256: { value: prompt_schema(media) },
      },
      mediaType: media,
      byteSize: BigInt(bytes.length),
      createdAt: timestampNow(),
    }),
  };
}

describe("private prompt artifacts", () => {
  it("denies an artifact outside the authenticated actor's namespace", async () => {
    const bytes = Buffer.from("%PDF-1.4\nprivate prompt\n%%EOF");
    const { reference, path } = await prompt_artifact("application/pdf", bytes);
    const other = join(fixture.config.prompts ?? "", actor_directory("another-actor"));
    await mkdir(other, { recursive: true, mode: 0o700 });
    await writeFile(join(other, reference.artifactId?.value ?? ""), bytes, {
      mode: 0o600,
    });
    await rm(path);
    const command = test_request(fixture.host);
    command.invocation?.messages[1]?.content.push(
      create(ContentBlockSchema, {
        content: { case: "document", value: { artifact: reference } },
      }),
    );
    await expect(invoke(command)).rejects.toThrow("provider_private_state_denied");
    expect(fixture.requests).toHaveLength(0);
  });
  it.each(["responses", "chat", "claude"])(
    "resolves an authorized %s image by immutable reference",
    async (model) => {
      const png = Buffer.from(
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aO3sAAAAASUVORK5CYII=",
        "base64",
      );
      const { reference } = await prompt_artifact("image/png", png);
      const command = test_request(fixture.host, model);
      command.invocation?.messages[1]?.content.push(
        create(ContentBlockSchema, {
          content: { case: "image", value: { artifact: reference, detail: ImageDetail.AUTO } },
        }),
      );
      expect((await invoke(command))?.finishReason).toBe(ModelFinishReason.STOP);
      expect(JSON.stringify(fixture.requests[0]?.body)).toContain(png.toString("base64"));
      expect(JSON.stringify(fixture.requests[1]?.body)).not.toContain("loop-prompt");
    },
  );

  it("denies research paths even with a valid prompt digest", async () => {
    const { reference } = await prompt_artifact(
      "application/pdf",
      Buffer.from("%PDF-1.4\nfixture\n%%EOF"),
    );
    reference.uri = "file:///protected/research.parquet";
    const command = test_request(fixture.host);
    command.invocation?.messages[1]?.content.push(
      create(ContentBlockSchema, { content: { case: "document", value: { artifact: reference } } }),
    );
    await expect(invoke(command)).rejects.toThrow("provider_private_state_denied");
    expect(fixture.requests).toHaveLength(0);
  });

  it("denies changed and symlinked artifacts", async () => {
    const { reference, path } = await prompt_artifact(
      "application/pdf",
      Buffer.from("%PDF-1.4\nfixture-two\n%%EOF"),
    );
    const command = test_request(fixture.host);
    command.invocation?.messages[1]?.content.push(
      create(ContentBlockSchema, { content: { case: "document", value: { artifact: reference } } }),
    );
    await writeFile(path, "%PDF-changed");
    await expect(invoke(command)).rejects.toThrow("provider_private_state_denied");
    await rm(path);
    await symlink(join(directory, "window.json"), path);
    await expect(invoke(command)).rejects.toThrow("provider_private_state_denied");
    expect(fixture.requests).toHaveLength(0);
  });
});

describe("private reasoning continuation", () => {
  it.each(["actor", "model", "expired", "corrupted"])(
    "denies %s continuation misuse before a supplier request",
    async (mutation) => {
      const config = structuredClone(fixture.config);
      const route = config.models.find((entry) => entry.id === "responses");
      if (!route) throw new Error("missing_model");
      route.reasoning = "medium";
      const host = new ProviderHost(
        config,
        new Uint8Array(32).fill(1),
        { LOOP_LLM_TEST: TEST_SECRET },
        fixture.fetcher,
      );
      const principal = config.principals[0];
      const snapshot = host.models.find((entry) => entry.route.id === "responses")?.snapshot;
      if (!principal || !snapshot) throw new Error("missing_fixture_identity");
      const continuation = await save_continuation(
        config,
        mutation === "actor" ? "another-actor" : principal.actor_id,
        snapshot,
        "public summary",
        {
          type: "reasoning",
          id: "rs1",
          encrypted_content: "private-ciphertext",
          summary: [],
        },
      );
      if (mutation === "model")
        continuation.modelResolutionId = {
          $typeName: "loop.v1.ModelResolutionId",
          value: "another-model",
        };
      if (mutation === "expired") continuation.expiresAt = timestampFromDate(new Date(0));
      if (mutation === "corrupted")
        await writeFile(
          join(config.journal, `continuation-${continuation.providerContinuationId?.value}.result`),
          "{}",
        );
      const next = test_request(host);
      next.invocation?.messages.push(
        create(ModelMessageSchema, {
          role: ModelRole.ASSISTANT,
          content: [
            {
              content: {
                case: "reasoning",
                value: { text: "public summary", continuation },
              },
            },
          ],
        }),
        create(ModelMessageSchema, {
          role: ModelRole.USER,
          content: [
            {
              content: {
                case: "text",
                value: { text: "Continue" },
              },
            },
          ],
        }),
      );
      await expect(host.invoke(next, principal, new AbortController().signal)).rejects.toThrow(
        "provider_private_state_denied",
      );
      expect(fixture.requests).toHaveLength(0);
    },
  );
  it.each(["responses", "claude"])(
    "preserves %s state across restart and rejects altered references",
    async (model) => {
      const config = structuredClone(fixture.config);
      const route = config.models.find((entry) => entry.id === model);
      if (!route) throw new Error("missing_model");
      route.reasoning = model === "claude" ? "adaptive" : "medium";
      const host = new ProviderHost(
        config,
        new Uint8Array(32).fill(1),
        { LOOP_LLM_TEST: TEST_SECRET },
        fixture.fetcher,
      );
      const principal = config.principals[0];
      if (!principal) throw new Error("missing_principal");
      const state =
        model === "claude"
          ? {
              type: "thinking",
              thinking: "brief public summary",
              signature: "private-vendor-signature",
            }
          : {
              type: "reasoning",
              id: "rs1",
              summary: [{ type: "summary_text", text: "brief public summary" }],
              encrypted_content: "private-vendor-ciphertext",
            };
      const reply = JSON.parse(
        JSON.stringify(
          test_reply(model === "claude" ? "/messages" : "/responses", { model: route.model }),
        ),
      );
      if (model === "claude") reply.content.unshift(state);
      else reply.output.unshift(state);
      fixture.state.reply = reply;
      const result = await host.invoke(
        test_request(host, model),
        principal,
        new AbortController().signal,
      );
      const block = result.content[0];
      if (block?.content.case !== "reasoning") throw new Error("missing_reasoning");
      expect(
        JSON.stringify(result, (_key, value) =>
          typeof value === "bigint" ? value.toString() : value,
        ),
      ).not.toContain("private-vendor");
      const id = block.content.value.continuation?.providerContinuationId?.value;
      expect(
        await readFile(join(config.journal, `continuation-${id}.result`), "utf8"),
      ).not.toContain("brief public summary");
      const restarted = new ProviderHost(
        config,
        new Uint8Array(32).fill(1),
        { LOOP_LLM_TEST: TEST_SECRET },
        fixture.fetcher,
      );
      const next = test_request(restarted, model);
      next.invocation?.messages.push(
        create(ModelMessageSchema, { role: ModelRole.ASSISTANT, content: result.content }),
        create(ModelMessageSchema, {
          role: ModelRole.USER,
          content: [{ content: { case: "text", value: { text: "Continue" } } }],
        }),
      );
      fixture.state.reply = undefined;
      await restarted.invoke(next, principal, new AbortController().signal);
      expect(JSON.stringify(fixture.requests.at(-1)?.body)).toContain("private-vendor");
      const count = fixture.requests.length;
      block.content.value.text = "tampered summary";
      await expect(restarted.invoke(next, principal, new AbortController().signal)).rejects.toThrow(
        "provider_private_state_denied",
      );
      expect(fixture.requests).toHaveLength(count);
    },
  );
});
