import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { ModelRoute } from "../src/config.js";
import { discover_models } from "../src/model-discovery.js";
import { TEST_SECRET, test_config } from "./fixture.js";
import { metadata_fixture } from "./metadata-fixture.js";

let fixture: Awaited<ReturnType<typeof metadata_fixture>>;
beforeEach(async () => {
  fixture = await metadata_fixture();
});
afterEach(async () => fixture?.close());

function discovery_model(plugin: ModelRoute["plugin"]): ModelRoute {
  const seed = test_config("/unused").models[0];
  if (!seed) throw new Error("missing_seed");
  return { ...seed, plugin };
}

async function discover(plugin: ModelRoute["plugin"], signal = new AbortController().signal) {
  return discover_models(
    discovery_model(plugin),
    { LOOP_LLM_TEST: TEST_SECRET },
    signal,
    fixture.fetcher,
  );
}

describe("official model inventory", () => {
  it.each(["openai_chat", "openai_responses"] as const)(
    "lists %s without inventing capabilities or generation verification",
    async (plugin) => {
      fixture.state.pages = [{ data: [{ id: "z-model" }, { id: "a-model" }] }];
      const inventory = await discover(plugin);
      expect(inventory.models).toEqual([
        { id: "a-model", availability: "active" },
        { id: "z-model", availability: "active" },
      ]);
      expect(inventory.sha256).toMatch(/^[a-f0-9]{64}$/);
      expect(fixture.origins).toEqual(["https://api.openai.com"]);
      expect(fixture.requests[0]?.headers.authorization).toBe(`Bearer ${TEST_SECRET}`);
      expect(fixture.requests[0]?.method).toBe("GET");
      expect(fixture.requests[0]?.url.pathname).toBe("/v1/models");
    },
  );

  it("separates retirement and announced deprecation", async () => {
    fixture.state.pages = [
      {
        data: [
          { id: "retired", shutdown_date: "2020-01-01" },
          { id: "deprecated", shutdown_date: "2099-01-01" },
        ],
      },
    ];
    expect((await discover("openai_chat")).models.map((model) => model.availability)).toEqual([
      "deprecated",
      "retired",
    ]);
  });

  it("uses Anthropic cursors and reported capability flags", async () => {
    fixture.state.pages = [
      {
        data: [
          {
            id: "claude-a",
            max_input_tokens: 4096,
            max_tokens: 256,
            capabilities: { image_input: { supported: true }, thinking: { supported: false } },
          },
        ],
        has_more: true,
        last_id: "claude-a",
      },
      { data: [{ id: "claude-b" }], has_more: false },
    ];
    const inventory = await discover("anthropic");
    expect(inventory.models[0]).toMatchObject({
      id: "claude-a",
      input_tokens: 4096,
      output_tokens: 256,
      vision: true,
      reasoning: false,
    });
    expect(fixture.requests[1]?.url.searchParams.get("after_id")).toBe("claude-a");
    expect(fixture.requests[0]?.headers["x-api-key"]).toBe(TEST_SECRET);
    expect(fixture.requests[0]?.headers["anthropic-version"]).toBe("2023-06-01");
    expect(fixture.requests[0]?.headers.authorization).toBeUndefined();
  });

  it.each(["google_generate", "google_interactions"] as const)(
    "filters non-generation models for %s",
    async (plugin) => {
      fixture.state.pages = [
        {
          models: [{ name: "models/embed", supportedGenerationMethods: ["embedContent"] }],
          nextPageToken: "second",
        },
        {
          models: [
            {
              name: "models/gemini-fixture",
              supportedGenerationMethods: ["generateContent"],
              inputTokenLimit: 4096,
              outputTokenLimit: 256,
              thinking: true,
            },
          ],
        },
      ];
      expect((await discover(plugin)).models).toEqual([
        {
          id: "gemini-fixture",
          availability: "active",
          input_tokens: 4096,
          output_tokens: 256,
          reasoning: true,
        },
      ]);
      expect(fixture.requests[1]?.url.searchParams.get("pageToken")).toBe("second");
      expect(fixture.requests[0]?.headers["x-goog-api-key"]).toBe(TEST_SECRET);
      expect(fixture.requests[0]?.url.searchParams.has("key")).toBe(false);
    },
  );

  it("uses Cohere chat inventory and native page tokens", async () => {
    fixture.state.pages = [
      {
        models: [{ name: "embedding", endpoints: ["embed"], is_deprecated: false }],
        next_page_token: "second",
      },
      {
        models: [
          {
            name: "command-fixture",
            endpoints: ["chat"],
            is_deprecated: true,
            context_length: 4096,
          },
        ],
      },
    ];
    expect((await discover("cohere")).models).toEqual([
      {
        id: "command-fixture",
        availability: "deprecated",
        input_tokens: 4096,
      },
    ]);
    expect(fixture.requests[0]?.url.searchParams.get("endpoint")).toBe("chat");
    expect(fixture.requests[1]?.url.searchParams.get("page_token")).toBe("second");
  });

  it.each([
    "ollama",
    "vllm",
    "sglang",
    "llamacpp",
    "lmstudio",
    "nim",
    "openai_compatible",
  ] as const)("uses the configured anonymous %s model list", async (plugin) => {
    const model = discovery_model(plugin);
    model.compatible = {
      base_url: "http://127.0.0.1:11434/v1",
      wire: "chat",
      auth: "none",
      strict_tools: false,
      output_limit: "max_tokens",
      thinking: "none",
      reasoning_field: "none",
      stream_usage: "separate",
    };
    delete model.secret_env;
    fixture.state.pages = [{ data: [{ id: "local-model" }] }];
    expect(
      (await discover_models(model, {}, new AbortController().signal, fixture.fetcher)).models,
    ).toHaveLength(1);
    expect(fixture.requests[0]?.headers.authorization).toBeUndefined();
    expect(fixture.requests[0]?.headers["x-api-key"]).toBeUndefined();
  });

  it.each([
    "bedrock_converse",
    "azure_chat",
    "vertex_generate",
    "deepseek",
    "litellm",
    "portkey",
    "openrouter",
  ] as const)("reports unsupported %s discovery before dispatch", async (plugin) => {
    await expect(discover(plugin)).rejects.toThrow("provider_discovery_unavailable");
    expect(fixture.requests).toHaveLength(0);
  });

  it.each([
    [{ data: [{ id: "duplicate" }, { id: "duplicate" }] }, "provider_discovery_limit"],
    [{ data: [{ id: "first" }], has_more: true }, "provider_discovery_pagination"],
    [{ data: [{ id: "bad model name" }] }, "provider_dependency_failed"],
  ])("rejects incomplete or ambiguous inventory %j", async (body, message) => {
    fixture.state.pages = [body];
    await expect(discover("openai_chat")).rejects.toThrow(String(message));
  });

  it("rejects a cycling cursor without an unbounded scan", async () => {
    fixture.state.pages = [
      { data: [{ id: "one" }], has_more: true, last_id: "same" },
      { data: [{ id: "two" }], has_more: true, last_id: "same" },
    ];
    await expect(discover("anthropic")).rejects.toThrow("provider_discovery_cycle");
    expect(fixture.requests).toHaveLength(2);
  });

  it("bounds pagination to eight pages", async () => {
    fixture.state.pages = Array.from({ length: 9 }, (_, index) => ({
      data: [{ id: `model-${index}` }],
      has_more: true,
      last_id: `cursor-${index}`,
    }));
    await expect(discover("anthropic")).rejects.toThrow("provider_discovery_limit");
    expect(fixture.requests).toHaveLength(8);
  });

  it.each([
    [401, "provider_credentials_denied"],
    [429, "provider_rate_limited"],
    [503, "provider_dependency_failed"],
  ])("redacts HTTP %i failures without retrying", async (status, message) => {
    fixture.state.status = Number(status);
    fixture.state.pages = [{ error: TEST_SECRET }];
    await expect(discover("openai_chat")).rejects.toThrow(String(message));
    expect(fixture.requests).toHaveLength(1);
  });

  it("does not follow redirects with a supplier key", async () => {
    fixture.state.status = 302;
    await expect(discover("openai_chat")).rejects.toThrow("provider_dependency_failed");
    expect(fixture.requests).toHaveLength(1);
  });

  it("cancels a stalled list request", async () => {
    fixture.state.stall = true;
    await expect(discover("openai_chat", AbortSignal.timeout(100))).rejects.toThrow(
      "provider_cancelled",
    );
  });

  it("bounds decoded response bytes", async () => {
    fixture.state.raw = JSON.stringify({ padding: "x".repeat(524_288) });
    await expect(discover("openai_chat")).rejects.toThrow("provider_response_too_large");
  });

  it("denies missing credentials before a request", async () => {
    await expect(
      discover_models(
        discovery_model("anthropic"),
        {},
        new AbortController().signal,
        fixture.fetcher,
      ),
    ).rejects.toThrow("provider_credentials_missing");
    expect(fixture.requests).toHaveLength(0);
  });
});
