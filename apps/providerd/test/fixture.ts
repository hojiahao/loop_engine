import { execFileSync } from "node:child_process";
import { createHash, randomUUID, X509Certificate } from "node:crypto";
import { once } from "node:events";
import { chmodSync, readFileSync } from "node:fs";
import { createServer } from "node:http";
import type { AddressInfo } from "node:net";
import { join } from "node:path";
import { create } from "@bufbuild/protobuf";
import { timestampNow } from "@bufbuild/protobuf/wkt";
import { createClient } from "@connectrpc/connect";
import { createGrpcTransport } from "@connectrpc/connect-node";
import {
  ActorKind,
  InvokeModelRequestSchema,
  ModelRole,
  ProviderService,
} from "@loop-engine/protocol/provider";

import { type Deployment, validate_deployment } from "../src/config.js";
import { ProviderHost } from "../src/host.js";
import { open_journal } from "../src/journal.js";
import { create_provider_rpc } from "../src/rpc.js";

export const TEST_SECRET = "fixture-secret-never-in-errors";

export function test_config(directory: string): Deployment {
  return validate_deployment({
    schema: "loop.provider-deployment/v1",
    resolved_at: "2026-09-01T00:00:00Z",
    port: 8091,
    tls: {
      ca: join(directory, "ca.pem"),
      certificate: join(directory, "server.pem"),
      key: join(directory, "server.key"),
    },
    journal: join(directory, "journal"),
    principals: [
      {
        certificate_sha256: "a".repeat(64),
        actor_id: "loopd-test",
        actor_kind: "service",
        model_ids: ["responses", "chat", "claude"],
      },
    ],
    policy: {
      id: "native-text",
      revision: "1",
      input_tokens: 1024,
      output_tokens: 256,
      maximum_usd: "1",
      wall_time_ms: 5000,
      concurrency: 4,
    },
    models: ["responses", "chat", "claude"].map((id) => ({
      id,
      plugin: id === "responses" ? "openai_responses" : id === "chat" ? "openai_chat" : "anthropic",
      model: `${id}-fixture-20260901`,
      alias: id,
      context_tokens: 4096,
      output_tokens: 256,
      input_usd: "1",
      output_usd: "2",
      cached_usd: "0.1",
      secret_env: "LOOP_LLM_TEST",
    })),
  });
}

export function test_request(host: ProviderHost, id = "responses") {
  const selected = host.models.find((model) => model.route.id === id);
  if (!selected) throw new Error("fixture_model_missing");
  const request_id = randomUUID();
  return create(InvokeModelRequestSchema, {
    context: {
      requestId: { value: request_id },
      correlationId: { value: "contract-test" },
      idempotencyKey: { value: randomUUID() },
      actor: { actorId: { value: "loopd-test" }, kind: ActorKind.SERVICE },
      requestedAt: timestampNow(),
    },
    invocation: {
      requestId: { value: request_id },
      model: selected.snapshot,
      requestPolicy: host.policy,
      messages: [
        {
          role: ModelRole.SYSTEM,
          content: [{ content: { case: "text", value: { text: "Propose a research idea." } } }],
        },
        {
          role: ModelRole.USER,
          content: [
            {
              content: {
                case: "text",
                value: { text: "Use only the supplied development context." },
              },
            },
          ],
        },
      ],
      budget: {
        maximumInputTokens: 128n,
        maximumOutputTokens: 64n,
        maximumCost: { currencyCode: "USD", amount: { value: "0.001" } },
        maximumWallTime: { seconds: 4n },
      },
    },
  });
}

export function test_reply(path: string, body: Record<string, unknown>): unknown {
  if (path.endsWith("input_tokens") || path.endsWith("count_tokens"))
    return { input_tokens: 12, object: "response.input_tokens" };
  if (path.endsWith("/chat/completions"))
    return {
      id: "chat-fixture",
      object: "chat.completion",
      created: 1,
      model: body.model,
      choices: [
        {
          index: 0,
          finish_reason: "stop",
          message: { role: "assistant", content: "diagnostic idea", refusal: null },
        },
      ],
      usage: {
        prompt_tokens: 12,
        completion_tokens: 5,
        total_tokens: 17,
        prompt_tokens_details: { cached_tokens: 3 },
      },
    };
  if (path.endsWith("/messages"))
    return {
      id: "msg-fixture",
      type: "message",
      role: "assistant",
      model: body.model,
      stop_reason: "end_turn",
      stop_sequence: null,
      content: [{ type: "text", text: "diagnostic idea" }],
      usage: {
        input_tokens: 9,
        output_tokens: 5,
        cache_read_input_tokens: 3,
        cache_creation_input_tokens: 0,
      },
    };
  return {
    id: "resp-fixture",
    object: "response",
    model: body.model,
    status: "completed",
    error: null,
    output: [
      {
        type: "message",
        id: "msg-fixture",
        role: "assistant",
        status: "completed",
        content: [{ type: "output_text", text: "diagnostic idea", annotations: [] }],
      },
    ],
    usage: {
      input_tokens: 12,
      output_tokens: 5,
      total_tokens: 17,
      input_tokens_details: { cached_tokens: 3 },
      output_tokens_details: { reasoning_tokens: 0 },
    },
  };
}

export function test_certificates(directory: string) {
  function openssl(args: string[]) {
    execFileSync("openssl", args, { cwd: directory, stdio: "ignore", timeout: 10_000 });
  }
  openssl([
    "req",
    "-x509",
    "-newkey",
    "ec",
    "-pkeyopt",
    "ec_paramgen_curve:prime256v1",
    "-nodes",
    "-keyout",
    "ca.key",
    "-out",
    "ca.pem",
    "-subj",
    "/CN=Loop-provider-test",
    "-days",
    "1",
    "-addext",
    "basicConstraints=critical,CA:TRUE",
    "-addext",
    "keyUsage=critical,keyCertSign,cRLSign",
  ]);
  for (const name of ["server", "client", "unknown"]) {
    openssl([
      "req",
      "-new",
      "-newkey",
      "ec",
      "-pkeyopt",
      "ec_paramgen_curve:prime256v1",
      "-nodes",
      "-keyout",
      `${name}.key`,
      "-out",
      `${name}.csr`,
      "-subj",
      "/CN=localhost",
      "-addext",
      "subjectAltName=DNS:localhost",
    ]);
    openssl([
      "x509",
      "-req",
      "-in",
      `${name}.csr`,
      "-CA",
      "ca.pem",
      "-CAkey",
      "ca.key",
      "-CAcreateserial",
      "-out",
      `${name}.pem`,
      "-days",
      "1",
      "-copy_extensions",
      "copy",
    ]);
    chmodSync(join(directory, `${name}.key`), 0o600);
    chmodSync(join(directory, `${name}.pem`), 0o600);
  }
  chmodSync(join(directory, "ca.pem"), 0o600);
  return createHash("sha256")
    .update(new X509Certificate(readFileSync(join(directory, "client.pem"))).raw)
    .digest("hex");
}

export async function test_fixture(directory: string) {
  const requests: {
    path: string;
    body: Record<string, unknown>;
    authorization?: string;
    key?: string;
    version?: string;
  }[] = [];
  const state = {
    status: 200,
    delay: 0,
    count: undefined as number | undefined,
    body: undefined as unknown,
    reply: undefined as unknown,
    on_request: undefined as (() => void) | undefined,
  };
  const vendor = createServer(async (request, response) => {
    const chunks: Buffer[] = [];
    for await (const chunk of request) chunks.push(Buffer.from(chunk));
    const body = JSON.parse(Buffer.concat(chunks).toString("utf8")) as Record<string, unknown>;
    const path = request.url ?? "";
    requests.push({
      path,
      body,
      authorization: request.headers.authorization,
      key: request.headers["x-api-key"] as string | undefined,
      version: request.headers["anthropic-version"] as string | undefined,
    });
    state.on_request?.();
    if (state.delay) await new Promise((resolve) => setTimeout(resolve, state.delay));
    response.writeHead(state.status, {
      "content-type": "application/json",
      location: "/redirected",
    });
    const counting = path.endsWith("count_tokens") || path.endsWith("input_tokens");
    response.end(
      JSON.stringify(
        state.body ??
          (counting
            ? state.count !== undefined
              ? { input_tokens: state.count }
              : test_reply(path, body)
            : (state.reply ?? test_reply(path, body))),
      ),
    );
  });
  vendor.listen(0, "127.0.0.1");
  await once(vendor, "listening");
  const address = vendor.address() as AddressInfo;
  const seen_urls: string[] = [];
  const fetcher: typeof fetch = async (input, init) => {
    const url = new URL(input instanceof Request ? input.url : String(input));
    seen_urls.push(url.origin);
    return fetch(`http://127.0.0.1:${address.port}${url.pathname}`, init);
  };
  const config = test_config(directory);
  const digest = test_certificates(directory);
  const principal = config.principals[0];
  if (!principal) throw new Error("fixture_principal_missing");
  principal.certificate_sha256 = digest;
  await open_journal(config.journal);
  const host = new ProviderHost(
    config,
    new Uint8Array(32).fill(1),
    { LOOP_LLM_TEST: TEST_SECRET },
    fetcher,
  );
  const shutdown = new AbortController();
  const rpc = await create_provider_rpc(host, shutdown.signal);
  rpc.listen(0, "127.0.0.1");
  await once(rpc, "listening");
  const rpc_address = rpc.address() as AddressInfo;
  function client(name = "client") {
    return createClient(
      ProviderService,
      createGrpcTransport({
        baseUrl: `https://localhost:${rpc_address.port}`,
        idleConnectionTimeoutMs: 100,
        nodeOptions: {
          ca: readFileSync(config.tls.ca),
          ...(name
            ? {
                cert: readFileSync(join(directory, `${name}.pem`)),
                key: readFileSync(join(directory, `${name}.key`)),
              }
            : {}),
          minVersion: "TLSv1.3",
        },
      }),
    );
  }
  async function close() {
    shutdown.abort();
    await Promise.all([
      new Promise<void>((resolve) => rpc.close(() => resolve())),
      new Promise<void>((resolve) => {
        vendor.closeAllConnections();
        vendor.close(() => resolve());
      }),
    ]);
  }
  return { host, client, requests, state, seen_urls, config, principal, close, fetcher };
}
