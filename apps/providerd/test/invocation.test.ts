import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { clone, create } from "@bufbuild/protobuf";
import { Code, ConnectError } from "@connectrpc/connect";
import {
  InvokeModelRequestSchema,
  ModelFinishReason,
  ServiceErrorSchema,
  StreamModelRequestSchema,
} from "@loop-engine/protocol/provider";
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { ProviderHost } from "../src/host.js";
import { TEST_SECRET, test_fixture, test_reply, test_request } from "./fixture.js";

let directory: string;
let fixture: Awaited<ReturnType<typeof test_fixture>>;
beforeAll(async () => {
  directory = await mkdtemp(join(tmpdir(), "loop-provider-contract-"));
  fixture = await test_fixture(directory);
});
afterAll(async () => {
  await fixture?.close();
  if (directory) await rm(directory, { recursive: true, force: true });
});
beforeEach(() => {
  fixture.requests.length = 0;
  fixture.state.status = 200;
  fixture.state.delay = 0;
  fixture.state.count = undefined;
  fixture.state.body = undefined;
  fixture.state.reply = undefined;
  fixture.state.on_request = undefined;
});

function request_copy(model = "responses") {
  return clone(InvokeModelRequestSchema, test_request(fixture.host, model));
}

async function denied(request: ReturnType<typeof test_request>, code: string) {
  try {
    await fixture.client().invokeModel(request, { timeoutMs: 4500 });
    throw new Error("unexpected_success");
  } catch (error) {
    expect(error).toBeInstanceOf(ConnectError);
    const failure = ConnectError.from(error);
    expect(failure.rawMessage).toBe(code);
    expect(failure.findDetails(ServiceErrorSchema).map((detail) => detail.code)).toEqual([code]);
    expect(JSON.stringify(failure)).not.toContain(TEST_SECRET);
  }
}

describe("native mTLS invocation", () => {
  it.each(["responses", "chat", "claude"])(
    "executes %s with native authentication and normalized usage",
    async (model) => {
      const request = request_copy(model);
      const result = await fixture.client().invokeModel(request, { timeoutMs: 4500 });
      expect(result.response?.content[0]?.content).toMatchObject({
        case: "text",
        value: { text: "diagnostic idea" },
      });
      expect(result.response?.usage).toMatchObject({
        inputTokens: 12n,
        outputTokens: 5n,
        cachedInputTokens: 3n,
      });
      expect(result.response?.usage?.chargedCost).toBeUndefined();
      expect(fixture.requests).toHaveLength(2);
      const generated = fixture.requests[1];
      expect(generated?.body.model).toBe(`${model}-fixture-20260901`);
      expect(generated?.body.stream).toBe(false);
      if (model === "claude") {
        expect(generated?.key).toBe(TEST_SECRET);
        expect(generated?.version).toBe("2023-06-01");
        expect(generated?.body.max_tokens).toBe(64);
        expect(generated?.body.system).toEqual([
          { type: "text", text: "Propose a research idea." },
        ]);
        expect(generated?.body.messages).toEqual([
          { role: "user", content: "Use only the supplied development context." },
        ]);
      } else {
        expect(generated?.authorization).toBe(`Bearer ${TEST_SECRET}`);
        expect(generated?.body.store).toBe(false);
        expect(
          generated?.body[model === "chat" ? "max_completion_tokens" : "max_output_tokens"],
        ).toBe(64);
      }
    },
  );

  it("replays completed output without another supplier call, including restart", async () => {
    const request = request_copy();
    const first = await fixture.client().invokeModel(request, { timeoutMs: 4500 });
    const second = await fixture.client().invokeModel(request, { timeoutMs: 4500 });
    expect(second).toEqual(first);
    const restarted = new ProviderHost(
      fixture.config,
      new Uint8Array(32).fill(1),
      { LOOP_LLM_TEST: TEST_SECRET },
      fixture.fetcher,
    );
    expect(
      await restarted.invoke(request, fixture.principal, new AbortController().signal),
    ).toEqual(first.response);
    expect(fixture.requests).toHaveLength(2);
  });

  it("rejects changed content under an existing idempotency key", async () => {
    const request = request_copy();
    await fixture.client().invokeModel(request, { timeoutMs: 4500 });
    const block = request.invocation?.messages[1]?.content[0];
    if (block?.content.case === "text")
      block.content.value.text = "Different input under the same key.";
    await denied(request, "invocation_conflict");
    expect(fixture.requests).toHaveLength(2);
  });

  it("rejects a CA-signed certificate absent from the deployment registry", async () => {
    await expect(
      fixture.client("unknown").invokeModel(request_copy(), { timeoutMs: 4500 }),
    ).rejects.toMatchObject({ code: Code.Unauthenticated });
    expect(fixture.requests).toHaveLength(0);
  });

  it("rejects clients without a certificate before reading model input", async () => {
    await expect(
      fixture.client("").invokeModel(request_copy(), { timeoutMs: 1000 }),
    ).rejects.toBeInstanceOf(ConnectError);
    expect(fixture.requests).toHaveLength(0);
  });

  it("rejects forged Actor metadata on an authenticated connection", async () => {
    const request = request_copy();
    if (request.context?.actor?.actorId) request.context.actor.actorId.value = "another-actor";
    await denied(request, "provider_actor_denied");
    expect(fixture.requests).toHaveLength(0);
  });

  it.each(["x-forwarded-for", "x-loop-holdout-capability", "authorization"])(
    "rejects forbidden %s metadata",
    async (header) => {
      await expect(
        fixture
          .client()
          .invokeModel(request_copy(), { timeoutMs: 4500, headers: { [header]: "untrusted" } }),
      ).rejects.toMatchObject({ code: Code.PermissionDenied });
      expect(fixture.requests).toHaveLength(0);
    },
  );

  it("rejects missing RPC deadlines", async () => {
    await expect(fixture.client().invokeModel(request_copy())).rejects.toMatchObject({
      rawMessage: "provider_deadline_required",
    });
    expect(fixture.requests).toHaveLength(0);
  });

  it("rejects model identity and capability changes", async () => {
    const request = request_copy();
    if (request.invocation?.model?.capabilities)
      request.invocation.model.capabilities.supportsTools = true;
    await denied(request, "provider_pin_denied");
    expect(fixture.requests).toHaveLength(0);
  });

  it("rejects unknown request policy revisions", async () => {
    const request = request_copy();
    if (request.invocation?.requestPolicy) request.invocation.requestPolicy.revision = "2";
    await denied(request, "provider_pin_denied");
    expect(fixture.requests).toHaveLength(0);
  });

  it("rejects document blocks before resolving any artifact", async () => {
    const request = request_copy();
    const block = request.invocation?.messages[1]?.content[0];
    if (block)
      block.content = { case: "document", value: { $typeName: "loop.v1.DocumentContent" } };
    await denied(request, "unsupported_request_content");
    expect(fixture.requests).toHaveLength(0);
  });

  it("denies streaming explicitly until its separate delivery unit", async () => {
    const request = request_copy();
    const stream = create(StreamModelRequestSchema, {
      context: request.context,
      invocation: request.invocation,
    });
    const events = fixture.client().streamModel(stream, { timeoutMs: 4500 });
    await expect(async () => {
      for await (const _event of events) {
        throw new Error("unexpected_event");
      }
    }).rejects.toMatchObject({ code: Code.Unimplemented });
  });

  it("denies generation when token counting exceeds the frozen budget", async () => {
    fixture.state.count = 129;
    await denied(request_copy(), "provider_input_budget");
    expect(fixture.requests).toHaveLength(1);
  });

  it("denies insufficient money before token counting or generation", async () => {
    const request = request_copy();
    if (request.invocation?.budget?.maximumCost?.amount)
      request.invocation.budget.maximumCost.amount.value = "0.000001";
    await denied(request, "provider_budget_denied");
    expect(fixture.requests).toHaveLength(0);
  });

  it("does not retry 429 and does not echo upstream secrets", async () => {
    fixture.state.status = 429;
    fixture.state.body = { error: { message: TEST_SECRET, type: "rate_limit_error" } };
    const request = request_copy();
    await denied(request, "provider_rate_limited");
    await denied(request, "invocation_ambiguous");
    expect(fixture.requests).toHaveLength(1);
  });

  it("refuses redirects instead of forwarding credentials", async () => {
    fixture.state.status = 307;
    await denied(request_copy(), "provider_dependency_failed");
    expect(fixture.requests).toHaveLength(1);
  });

  it("limits the decoded upstream response size", async () => {
    fixture.state.body = { error: "x".repeat(524_289) };
    await denied(request_copy(), "provider_dependency_failed");
    expect(fixture.requests).toHaveLength(1);
  });

  it("honors the invocation deadline without starting generation", async () => {
    fixture.state.delay = 1000;
    const request = request_copy();
    if (request.invocation?.budget)
      request.invocation.budget.maximumWallTime = {
        $typeName: "google.protobuf.Duration",
        seconds: 0n,
        nanos: 500_000_000,
      };
    await denied(request, "provider_deadline");
    expect(fixture.requests.length).toBeLessThanOrEqual(1);
    expect(fixture.requests.every((entry) => entry.path.endsWith("input_tokens"))).toBe(true);
  });

  it("rejects a successful HTTP response missing required model usage", async () => {
    fixture.state.reply = {
      ...(test_reply("/responses", { model: "responses-fixture-20260901" }) as object),
      usage: null,
    };
    await denied(request_copy(), "invalid_openai_output");
    expect(fixture.requests).toHaveLength(2);
  });

  it("rejects usage exceeding the approved input allowance", async () => {
    fixture.state.reply = {
      ...(test_reply("/responses", { model: "responses-fixture-20260901" }) as object),
      usage: { input_tokens: 129, output_tokens: 5 },
    };
    await denied(request_copy(), "provider_usage_exceeded");
    expect(fixture.requests).toHaveLength(2);
  });

  it("rejects unrecognized output variants instead of silently discarding them", async () => {
    fixture.state.reply = {
      ...(test_reply("/responses", { model: "responses-fixture-20260901" }) as object),
      output: [{ type: "unknown_new_tool" }],
    };
    await denied(request_copy(), "invalid_openai_output");
  });

  it("propagates caller cancellation and fences an ambiguous retry", async () => {
    fixture.state.delay = 1000;
    const request = request_copy();
    const controller = new AbortController();
    // Cancel only after the supplier has observed the request: the journal claim
    // must already exist, even when TLS or fsync is slow on a shared CI host.
    fixture.state.on_request = () => controller.abort();
    const response = fixture
      .client()
      .invokeModel(request, { timeoutMs: 4500, signal: controller.signal });
    await expect(response).rejects.toMatchObject({ code: Code.Canceled });
    await denied(request, "invocation_ambiguous");
    expect(fixture.requests).toHaveLength(1);
  });

  it("rejects additional work while every invocation slot is occupied", async () => {
    fixture.state.delay = 50;
    const active = Array.from({ length: fixture.config.policy.concurrency }, () =>
      fixture.host.invoke(request_copy(), fixture.principal, new AbortController().signal),
    );
    const completed = Promise.allSettled(active);
    try {
      await expect(
        fixture.host.invoke(request_copy(), fixture.principal, new AbortController().signal),
      ).rejects.toMatchObject({ code: "provider_capacity" });
    } finally {
      expect((await completed).every((result) => result.status === "fulfilled")).toBe(true);
    }
    expect(fixture.requests).toHaveLength(2 * fixture.config.policy.concurrency);
  });

  it("rejects stale command timestamps before token counting", async () => {
    const request = request_copy();
    if (request.context?.requestedAt) request.context.requestedAt.seconds -= 301n;
    await denied(request, "invalid_request_context");
    expect(fixture.requests).toHaveLength(0);
  });

  it("fails closed when the wall clock moves backwards", async () => {
    const host = new ProviderHost(
      fixture.config,
      new Uint8Array(32).fill(1),
      { LOOP_LLM_TEST: TEST_SECRET },
      fixture.fetcher,
    );
    const now = Date.now();
    const clock = vi.spyOn(Date, "now").mockReturnValue(now);
    try {
      await host.invoke(request_copy(), fixture.principal, new AbortController().signal);
      clock.mockReturnValue(now - 1);
      await expect(
        host.invoke(request_copy(), fixture.principal, new AbortController().signal),
      ).rejects.toMatchObject({ code: "provider_clock_regressed" });
    } finally {
      clock.mockRestore();
    }
    expect(fixture.requests).toHaveLength(2);
  });

  it.each(["responses", "chat", "claude"])("preserves %s refusals", async (model) => {
    const common = { model: `${model}-fixture-20260901` };
    const refusal = "The requested content is unavailable.";
    fixture.state.reply =
      model === "claude"
        ? {
            ...(test_reply("/messages", common) as object),
            stop_reason: "refusal",
            content: [{ type: "text", text: refusal }],
          }
        : model === "chat"
          ? {
              ...(test_reply("/chat/completions", common) as object),
              choices: [
                { finish_reason: "stop", message: { role: "assistant", content: null, refusal } },
              ],
            }
          : {
              ...(test_reply("/responses", common) as object),
              output: [
                { type: "message", role: "assistant", content: [{ type: "refusal", refusal }] },
              ],
            };
    const result = await fixture.client().invokeModel(request_copy(model), { timeoutMs: 4500 });
    expect(result.response?.finishReason).toBe(ModelFinishReason.CONTENT_FILTER);
    expect(result.response?.content[0]?.content).toMatchObject({
      case: "refusal",
      value: { reason: refusal },
    });
  });

  it("preserves length termination instead of claiming complete output", async () => {
    fixture.state.reply = {
      ...(test_reply("/responses", { model: "responses-fixture-20260901" }) as object),
      status: "incomplete",
      incomplete_details: { reason: "max_output_tokens" },
    };
    const result = await fixture.client().invokeModel(request_copy(), { timeoutMs: 4500 });
    expect(result.response?.finishReason).toBe(ModelFinishReason.LENGTH);
  });

  it("reports missing credentials before any supplier request", async () => {
    const host = new ProviderHost(fixture.config, new Uint8Array(32).fill(1), {}, fixture.fetcher);
    await expect(
      host.invoke(request_copy(), fixture.principal, new AbortController().signal),
    ).rejects.toMatchObject({ code: "provider_credentials_missing" });
    expect(fixture.requests).toHaveLength(0);
  });
});
