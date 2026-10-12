import { createHash, randomUUID } from "node:crypto";
import { once } from "node:events";
import { readFileSync } from "node:fs";
import {
  chmod,
  mkdtemp,
  readdir,
  readFile,
  rm,
  symlink,
  unlink,
  writeFile,
} from "node:fs/promises";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { clone, create, fromBinary, toBinary, toJson } from "@bufbuild/protobuf";
import { timestampFromMs, timestampNow } from "@bufbuild/protobuf/wkt";
import { Code, createClient } from "@connectrpc/connect";
import { createGrpcTransport } from "@connectrpc/connect-node";
import {
  ContentBlockSchema,
  InvocationState,
  type InvokeModelRequest,
  InvokeModelRequestSchema,
  type LookupInvocationRequest,
  LookupInvocationRequestSchema,
  ModelFinishReason,
  ModelResponseSchema,
  ProviderService,
  StructuredOutputDefinitionSchema,
} from "@loop-engine/protocol/provider";
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { Deployment } from "../src/config.js";
import { ProviderHost } from "../src/host.js";
import { digest_json } from "../src/identity.js";
import { decimal_units } from "../src/pricing.js";
import { create_provider_rpc } from "../src/rpc.js";
import { TEST_SECRET, test_reply, test_request } from "./fixture.js";
import { rich_fixture, TEST_SCHEMA, tool_reply, tool_request } from "./rich-fixture.js";

let directory: string;
let fixture: Awaited<ReturnType<typeof rich_fixture>>;

beforeAll(async () => {
  directory = await mkdtemp(join(tmpdir(), "loop-provider-lookup-"));
  fixture = await rich_fixture(directory, (config) => {
    const route = config.models.find((model) => model.id === "responses");
    if (!route) throw new Error("missing_route");
    route.reasoning = "medium";
  });
});

afterAll(async () => {
  await fixture?.close();
  if (directory) await rm(directory, { recursive: true, force: true });
});

beforeEach(() => {
  fixture.requests.length = 0;
  fixture.state.status = 200;
  fixture.state.body = undefined;
  fixture.state.reply = undefined;
});

function lookup_request(original: InvokeModelRequest): LookupInvocationRequest {
  const context = clone(InvokeModelRequestSchema, original).context;
  if (!context) throw new Error("missing_context");
  context.requestId = { $typeName: "loop.v1.RequestId", value: randomUUID() };
  context.idempotencyKey = { $typeName: "loop.v1.IdempotencyKey", value: randomUUID() };
  context.requestedAt = timestampNow();
  return create(LookupInvocationRequestSchema, {
    context,
    originalRequestId: original.context?.requestId,
    originalIdempotencyKey: original.context?.idempotencyKey,
    requestSha256: {
      value: digest_json("loop.provider-invocation/v1", toJson(InvokeModelRequestSchema, original)),
    },
  });
}

function receipt_path(original: InvokeModelRequest, extension: "claim" | "result"): string {
  const actor = original.context?.actor?.actorId?.value;
  const key = original.context?.idempotencyKey?.value;
  if (!actor || !key) throw new Error("missing_identity");
  const id = createHash("sha256").update(actor).update("\0").update(key).digest("hex");
  return join(fixture.config.journal, `${id}.${extension}`);
}

async function invoke(original: InvokeModelRequest) {
  return fixture.client().invokeModel(original, { timeoutMs: 4500 });
}

async function lookup(request: LookupInvocationRequest) {
  return fixture.client().lookupInvocation(request, { timeoutMs: 4500 });
}

async function reader(config: Deployment) {
  // Recovery deliberately has no model credentials. No vendor path may run.
  const host = new ProviderHost(config, new Uint8Array(32).fill(2), {}, fixture.fetcher);
  const shutdown = new AbortController();
  const server = await create_provider_rpc(host, shutdown.signal);
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const client = createClient(
    ProviderService,
    createGrpcTransport({
      baseUrl: `https://localhost:${(server.address() as AddressInfo).port}`,
      idleConnectionTimeoutMs: 100,
      nodeOptions: {
        ca: readFileSync(config.tls.ca),
        cert: readFileSync(join(directory, "client.pem")),
        key: readFileSync(join(directory, "client.key")),
        minVersion: "TLSv1.3",
      },
    }),
  );
  async function close() {
    shutdown.abort();
    await new Promise<void>((resolve) => server.close(() => resolve()));
  }
  return { host, client, close };
}

describe("read-only authenticated invocation recovery", () => {
  it("observes absence without writing a claim or contacting a supplier", async () => {
    const request = lookup_request(test_request(fixture.host));
    const before = await readdir(fixture.config.journal);
    expect(await lookup(request)).toMatchObject({ state: InvocationState.ABSENT });
    expect(await readdir(fixture.config.journal)).toEqual(before);
    expect(fixture.requests).toHaveLength(0);
    const result = await lookup(request);
    expect(result.response).toBeUndefined();
    expect(result.reservedCost).toBeUndefined();
  });

  it("returns the immutable completed response and exact reservation", async () => {
    const original = test_request(fixture.host);
    const first = await invoke(original);
    const before = await readFile(receipt_path(original, "result"));
    const result = await lookup(lookup_request(original));
    expect(result.state).toBe(InvocationState.COMPLETED);
    expect(result.response).toEqual(first.response);
    expect(result.reservedCost?.currencyCode).toBe("USD");
    expect(decimal_units(result.reservedCost?.amount?.value ?? "-1")).toBe(256_000n);
    expect(await readFile(receipt_path(original, "result"))).toEqual(before);
    expect(fixture.requests).toHaveLength(2);
  });

  it("preserves historical length termination when recovering a completed receipt", async () => {
    const original = test_request(fixture.host);
    fixture.state.reply = {
      ...(test_reply("/responses", { model: "responses-fixture-20260901" }) as object),
      status: "incomplete",
      incomplete_details: { reason: "max_output_tokens" },
    };
    const first = await invoke(original);
    expect(first.response?.finishReason).toBe(ModelFinishReason.LENGTH);
    const result = await lookup(lookup_request(original));
    expect(result.state).toBe(InvocationState.COMPLETED);
    expect(result.response).toEqual(first.response);
    expect(fixture.requests).toHaveLength(2);
  });

  it.each(["toolCall", "structuredOutput", "reasoning"])(
    "recovers valid historical %s content without interpreting or executing it",
    async (kind) => {
      const original = kind === "toolCall" ? tool_request(fixture) : test_request(fixture.host);
      if (!original.invocation) throw new Error("missing_invocation");
      const reply = test_reply("/responses", {
        model: "responses-fixture-20260901",
      }) as { output: unknown[] };
      if (kind === "toolCall") fixture.state.reply = tool_reply("responses");
      else if (kind === "structuredOutput") {
        original.invocation.structuredOutput = create(StructuredOutputDefinitionSchema, {
          name: "factor_window",
          jsonSchema: TEST_SCHEMA,
          strict: true,
        });
        reply.output = [
          {
            type: "message",
            id: "msg-fixture",
            role: "assistant",
            status: "completed",
            content: [{ type: "output_text", text: '{"window":20}', annotations: [] }],
          },
        ];
        fixture.state.reply = reply;
      } else {
        reply.output.unshift({
          type: "reasoning",
          id: "rs1",
          encrypted_content: "fixture-encrypted-reasoning",
          summary: [{ type: "summary_text", text: "historical reasoning summary" }],
        });
        fixture.state.reply = reply;
      }
      const first = await invoke(original);
      expect(first.response?.content[0]?.content.case).toBe(kind);
      const result = await lookup(lookup_request(original));
      expect(result.state).toBe(InvocationState.COMPLETED);
      expect(result.response).toEqual(first.response);
      expect(fixture.requests).toHaveLength(2);
    },
  );

  it("recovers after restart without the original route, plugin pins or secret", async () => {
    const original = test_request(fixture.host);
    const first = await invoke(original);
    const config = structuredClone(fixture.config);
    config.models = config.models.filter((model) => model.id !== "responses");
    for (const principal of config.principals)
      principal.model_ids = principal.model_ids.filter((id) => id !== "responses");
    const restarted = await reader(config);
    try {
      const result = await restarted.client.lookupInvocation(lookup_request(original), {
        timeoutMs: 4500,
      });
      expect(result.state).toBe(InvocationState.COMPLETED);
      expect(result.response).toEqual(first.response);
      expect(fixture.requests).toHaveLength(2);
    } finally {
      await restarted.close();
    }
  });

  it("accepts a fresh query after the original command's five-minute window", async () => {
    const original = test_request(fixture.host);
    const first = await invoke(original);
    const restarted = await reader(structuredClone(fixture.config));
    const later = Date.now() + 301_000;
    const clock = vi.spyOn(Date, "now").mockReturnValue(later);
    try {
      const request = lookup_request(original);
      if (request.context) request.context.requestedAt = timestampFromMs(later);
      const result = await restarted.client.lookupInvocation(request, { timeoutMs: 4500 });
      expect(result.response).toEqual(first.response);
      expect(fixture.requests).toHaveLength(2);
    } finally {
      clock.mockRestore();
      await restarted.close();
    }
  });

  it("reads receipts even after the generation request window is exhausted", async () => {
    const config = structuredClone(fixture.config);
    config.policy.rate.requests = 1;
    const principal = config.principals[0];
    if (!principal) throw new Error("missing_principal");
    const host = new ProviderHost(
      config,
      new Uint8Array(32).fill(1),
      { LOOP_LLM_TEST: TEST_SECRET },
      fixture.fetcher,
    );
    const original = test_request(host);
    const first = await host.invoke(original, principal, new AbortController().signal);
    await expect(
      host.invoke(test_request(host), principal, new AbortController().signal),
    ).rejects.toMatchObject({ code: "provider_rate_capacity" });
    for (let attempt = 0; attempt < 2; attempt++) {
      const result = await host.lookup(
        lookup_request(original),
        principal,
        new AbortController().signal,
      );
      expect(result.response).toEqual(first);
    }
    expect(fixture.requests).toHaveLength(2);
  });

  it("preserves an uncertain outbound attempt without sending another request", async () => {
    const original = test_request(fixture.host);
    fixture.state.status = 429;
    fixture.state.body = { error: { message: TEST_SECRET } };
    await expect(invoke(original)).rejects.toMatchObject({ code: Code.ResourceExhausted });
    const result = await lookup(lookup_request(original));
    expect(result.state).toBe(InvocationState.AMBIGUOUS);
    expect(result.response).toBeUndefined();
    expect(decimal_units(result.reservedCost?.amount?.value ?? "-1")).toBe(256_000n);
    expect(fixture.requests).toHaveLength(1);
    expect(JSON.stringify(result)).not.toContain(TEST_SECRET);
  });

  it.each(["", '{"schema":'])("keeps an interrupted claim %j ambiguous", async (partial) => {
    const original = test_request(fixture.host);
    await writeFile(receipt_path(original, "claim"), partial, { mode: 0o600 });
    const result = await lookup(lookup_request(original));
    expect(result.state).toBe(InvocationState.AMBIGUOUS);
    expect(result.reservedCost).toBeUndefined();
    expect(result.response).toBeUndefined();
    expect(await readFile(receipt_path(original, "claim"), "utf8")).toBe(partial);
    expect(fixture.requests).toHaveLength(0);
  });

  it("denies a different request digest for the same actor and key", async () => {
    const original = test_request(fixture.host);
    await invoke(original);
    const request = lookup_request(original);
    if (request.requestSha256) request.requestSha256.value = new Uint8Array(32).fill(9);
    await expect(lookup(request)).rejects.toMatchObject({
      code: Code.FailedPrecondition,
      rawMessage: "invocation_conflict",
    });
    expect(fixture.requests).toHaveLength(2);
  });

  it("denies a different original request ID despite the matching digest", async () => {
    const original = test_request(fixture.host);
    await invoke(original);
    const request = lookup_request(original);
    if (request.originalRequestId) request.originalRequestId.value = randomUUID();
    await expect(lookup(request)).rejects.toMatchObject({
      code: Code.FailedPrecondition,
      rawMessage: "invocation_conflict",
    });
  });

  it.each(["requestId", "idempotencyKey"] as const)(
    "requires a fresh query %s distinct from the original command",
    async (field) => {
      const original = test_request(fixture.host);
      const request = lookup_request(original);
      if (!request.context || !original.context) throw new Error("missing_context");
      if (field === "requestId") request.context.requestId = original.context.requestId;
      else request.context.idempotencyKey = original.context.idempotencyKey;
      await expect(lookup(request)).rejects.toMatchObject({
        rawMessage: "invalid_request_context",
      });
      expect(fixture.requests).toHaveLength(0);
    },
  );

  it.each(["invalid-json", "checksum", "extra-field"])(
    "rejects completed evidence corrupted by %s",
    async (kind) => {
      const original = test_request(fixture.host);
      await invoke(original);
      const path = receipt_path(original, "result");
      const record = JSON.parse(await readFile(path, "utf8")) as Record<string, unknown>;
      if (kind === "checksum") record.sha256 = "0".repeat(64);
      if (kind === "extra-field") record.untrusted = TEST_SECRET;
      await writeFile(path, kind === "invalid-json" ? "{" : JSON.stringify(record));
      await expect(lookup(lookup_request(original))).rejects.toMatchObject({
        code: Code.DataLoss,
        rawMessage: "provider_receipt_corrupt",
      });
      expect(fixture.requests).toHaveLength(2);
    },
  );

  it("does not treat a completed result with a missing claim as absence", async () => {
    const original = test_request(fixture.host);
    await invoke(original);
    await unlink(receipt_path(original, "claim"));
    await expect(lookup(lookup_request(original))).rejects.toMatchObject({
      code: Code.DataLoss,
      rawMessage: "provider_receipt_corrupt",
    });
  });

  it("rejects invalid response bytes even with a valid envelope checksum", async () => {
    const original = test_request(fixture.host);
    await invoke(original);
    const content = Buffer.from([0xff]);
    await writeFile(
      receipt_path(original, "result"),
      JSON.stringify({
        bytes: content.toString("base64"),
        sha256: createHash("sha256").update(content).digest("hex"),
      }),
    );
    await expect(lookup(lookup_request(original))).rejects.toMatchObject({
      code: Code.DataLoss,
      rawMessage: "provider_receipt_corrupt",
    });
  });

  it.each(["content", "finish", "usage"])(
    "rejects completed protobuf evidence with invalid %s semantics",
    async (kind) => {
      const original = test_request(fixture.host);
      await invoke(original);
      const path = receipt_path(original, "result");
      const record = JSON.parse(await readFile(path, "utf8")) as { bytes: string };
      const response = fromBinary(ModelResponseSchema, Buffer.from(record.bytes, "base64"));
      if (kind === "content") response.content = [];
      if (kind === "finish") response.finishReason = 0;
      if (kind === "usage" && response.usage) response.usage.cachedInputTokens = 1000n;
      const content = toBinary(ModelResponseSchema, response);
      await writeFile(
        path,
        JSON.stringify({
          bytes: Buffer.from(content).toString("base64"),
          sha256: createHash("sha256").update(content).digest("hex"),
        }),
      );
      await expect(lookup(lookup_request(original))).rejects.toMatchObject({
        code: Code.DataLoss,
        rawMessage: "provider_receipt_corrupt",
      });
      expect(fixture.requests).toHaveLength(2);
    },
  );

  it("rejects a parseable but invalid claim instead of treating it as interrupted", async () => {
    const original = test_request(fixture.host);
    await writeFile(receipt_path(original, "claim"), JSON.stringify({ actor: "wrong" }), {
      mode: 0o600,
    });
    await expect(lookup(lookup_request(original))).rejects.toMatchObject({
      code: Code.DataLoss,
      rawMessage: "provider_receipt_corrupt",
    });
    expect(fixture.requests).toHaveLength(0);
  });

  it.each(["toolCall", "structuredOutput"] as const)(
    "rejects malformed nested %s data inside checksummed protobuf evidence",
    async (kind) => {
      const original = test_request(fixture.host);
      const first = await invoke(original);
      if (!first.response) throw new Error("missing_response");
      const response = clone(ModelResponseSchema, first.response);
      response.content = [
        kind === "toolCall"
          ? create(ContentBlockSchema, {
              content: {
                case: "toolCall",
                value: { toolCallId: "call_1", toolName: "propose_factor" },
              },
            })
          : create(ContentBlockSchema, {
              content: {
                case: "structuredOutput",
                value: {
                  output: {
                    schemaId: TEST_SCHEMA.schemaId,
                    schemaSha256: TEST_SCHEMA.schemaSha256,
                    utf8Json: Buffer.from('{"window":20}'),
                    canonicalSha256: { value: new Uint8Array(32) },
                  },
                },
              },
            }),
      ];
      if (kind === "toolCall") response.finishReason = ModelFinishReason.TOOL_CALL;
      const content = toBinary(ModelResponseSchema, response);
      await writeFile(
        receipt_path(original, "result"),
        JSON.stringify({
          bytes: Buffer.from(content).toString("base64"),
          sha256: createHash("sha256").update(content).digest("hex"),
        }),
      );
      await expect(lookup(lookup_request(original))).rejects.toMatchObject({
        code: Code.DataLoss,
        rawMessage: "provider_receipt_corrupt",
      });
      expect(fixture.requests).toHaveLength(2);
    },
  );

  it.each(["symlink", "public-mode"])("rejects %s claim storage", async (kind) => {
    const original = test_request(fixture.host);
    await invoke(original);
    const path = receipt_path(original, "claim");
    if (kind === "public-mode") await chmod(path, 0o644);
    else {
      await unlink(path);
      await symlink(receipt_path(original, "result"), path);
    }
    await expect(lookup(lookup_request(original))).rejects.toMatchObject({
      code: Code.DataLoss,
      rawMessage: "provider_receipt_corrupt",
    });
  });

  it.each(["missing", "symlink"])("fails closed on a %s journal directory", async (kind) => {
    const config = structuredClone(fixture.config);
    config.journal = join(directory, `journal-${kind}`);
    if (kind === "symlink") await symlink(fixture.config.journal, config.journal);
    const restarted = await reader(config);
    try {
      await expect(
        restarted.client.lookupInvocation(lookup_request(test_request(fixture.host)), {
          timeoutMs: 4500,
        }),
      ).rejects.toMatchObject({ rawMessage: "journal_unavailable" });
      expect(fixture.requests).toHaveLength(0);
    } finally {
      await restarted.close();
    }
  });

  it("does not expose another actor's result even with its digest and key", async () => {
    const original = test_request(fixture.host);
    await invoke(original);
    const config = structuredClone(fixture.config);
    const principal = config.principals[0];
    if (!principal) throw new Error("missing_principal");
    principal.actor_id = "another-service";
    const restarted = await reader(config);
    try {
      const request = lookup_request(original);
      if (request.context?.actor?.actorId) request.context.actor.actorId.value = principal.actor_id;
      const result = await restarted.client.lookupInvocation(request, { timeoutMs: 4500 });
      expect(result.state).toBe(InvocationState.ABSENT);
      expect(result.response).toBeUndefined();
      expect(result.reservedCost).toBeUndefined();
      expect(fixture.requests).toHaveLength(2);
    } finally {
      await restarted.close();
    }
  });

  it("rejects unregistered certificates before disclosing state", async () => {
    await expect(
      fixture.client("unknown").lookupInvocation(lookup_request(test_request(fixture.host)), {
        timeoutMs: 4500,
      }),
    ).rejects.toMatchObject({ code: Code.Unauthenticated });
    expect(fixture.requests).toHaveLength(0);
  });

  it("rejects forged actor metadata on a valid certificate", async () => {
    const request = lookup_request(test_request(fixture.host));
    if (request.context?.actor?.actorId) request.context.actor.actorId.value = "another-actor";
    await expect(lookup(request)).rejects.toMatchObject({
      code: Code.PermissionDenied,
      rawMessage: "provider_actor_denied",
    });
  });

  it.each(["authorization", "x-loop-holdout-capability", "x-forwarded-for"])(
    "rejects forbidden %s metadata on lookup",
    async (header) => {
      await expect(
        fixture.client().lookupInvocation(lookup_request(test_request(fixture.host)), {
          timeoutMs: 4500,
          headers: { [header]: "untrusted" },
        }),
      ).rejects.toMatchObject({ code: Code.PermissionDenied });
    },
  );

  it("rejects expired query context independently of original command age", async () => {
    const request = lookup_request(test_request(fixture.host));
    if (request.context?.requestedAt) request.context.requestedAt.seconds -= 301n;
    await expect(lookup(request)).rejects.toMatchObject({ rawMessage: "invalid_request_context" });
    expect(fixture.requests).toHaveLength(0);
  });

  it.each([0, 31, 33])("rejects a %i-byte original request digest", async (length) => {
    const request = lookup_request(test_request(fixture.host));
    if (request.requestSha256) request.requestSha256.value = new Uint8Array(length);
    await expect(lookup(request)).rejects.toMatchObject({ rawMessage: "invalid_request_digest" });
    expect(fixture.requests).toHaveLength(0);
  });

  it("requires an RPC deadline for read-only work", async () => {
    await expect(
      fixture.client().lookupInvocation(lookup_request(test_request(fixture.host))),
    ).rejects.toMatchObject({ rawMessage: "provider_deadline_required" });
  });

  it("propagates cancellation without creating any journal files", async () => {
    const request = lookup_request(test_request(fixture.host));
    const before = await readdir(fixture.config.journal);
    const controller = new AbortController();
    controller.abort();
    await expect(
      fixture.client().lookupInvocation(request, { timeoutMs: 4500, signal: controller.signal }),
    ).rejects.toMatchObject({ code: Code.Canceled });
    await expect(
      fixture.host.lookup(request, fixture.principal, controller.signal),
    ).rejects.toMatchObject({ code: "provider_cancelled" });
    expect(await readdir(fixture.config.journal)).toEqual(before);
    expect(fixture.requests).toHaveLength(0);
  });
});
