import { type ChildProcess, execFile, spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { once } from "node:events";
import { access, mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { type AddressInfo, createServer } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { clone, create, toJson } from "@bufbuild/protobuf";
import { timestampNow } from "@bufbuild/protobuf/wkt";
import { Code, createClient } from "@connectrpc/connect";
import { createGrpcTransport } from "@connectrpc/connect-node";
import {
  InvocationState,
  type InvokeModelRequest,
  InvokeModelRequestSchema,
  LookupInvocationRequestSchema,
  LookupInvocationResponseSchema,
  ProviderService,
  StreamModelRequestSchema,
} from "@loop-engine/protocol/provider";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { build_catalog } from "../src/catalog-merge.js";
import { catalog_bytes, load_catalog } from "../src/catalog-store.js";
import type { Deployment } from "../src/config.js";
import { digest_json } from "../src/identity.js";
import { finish_invocation, open_journal } from "../src/journal.js";
import { seed_sources } from "./catalog-fixture.js";
import { TEST_SECRET, test_fixture, test_request } from "./fixture.js";

const executable = fileURLToPath(new URL("../dist/index.js", import.meta.url));
const exec_file = promisify(execFile);
let directory: string;
let fixture: Awaited<ReturnType<typeof test_fixture>> | undefined;

beforeEach(async () => {
  directory = await mkdtemp(join(tmpdir(), "loop-recovery-cli-"));
  fixture = await test_fixture(directory);
});

afterEach(async () => {
  await fixture?.close();
  fixture = undefined;
  if (directory) await rm(directory, { recursive: true, force: true });
});

async function free_port() {
  const server = createServer();
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const port = (server.address() as AddressInfo).port;
  await new Promise<void>((resolve) => server.close(() => resolve()));
  return port;
}

function wait_ready(child: ChildProcess): Promise<Record<string, unknown>> {
  return new Promise((resolve, reject) => {
    const output = child.stdout;
    if (!output) return reject(new Error("missing_output"));
    let bytes = "";
    const timer = setTimeout(() => finish(new Error("recovery_start_timeout")), 10_000);
    function finish(error?: Error, value?: Record<string, unknown>) {
      clearTimeout(timer);
      output?.off("data", read_event);
      child.off("exit", exited);
      child.off("error", failed);
      if (error) reject(error);
      else resolve(value ?? {});
    }
    function failed() {
      finish(new Error("recovery_process_failed"));
    }
    function exited() {
      finish(new Error("recovery_start_failed"));
    }
    function read_event(chunk: Buffer) {
      bytes += chunk.toString("utf8");
      const lines = bytes.split("\n");
      bytes = lines.pop() ?? "";
      try {
        for (const line of lines) {
          const value = JSON.parse(line) as Record<string, unknown>;
          if (value.event === "listening") finish(undefined, value);
        }
      } catch {
        finish(new Error("invalid_recovery_event"));
      }
    }
    child.once("exit", exited);
    child.once("error", failed);
    output.on("data", read_event);
  });
}

async function stop(child: ChildProcess) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const exited = once(child, "exit");
  child.kill("SIGKILL");
  await exited;
}

function lookup_request(original: InvokeModelRequest) {
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

async function configuration() {
  if (!fixture) throw new Error("missing_fixture");
  const config = structuredClone(fixture.config);
  config.port = await free_port();
  const path = join(directory, "deployment.json");
  const environment = {
    PATH: process.env.PATH,
    PROVIDERD_DEPLOYMENT: path,
    PROVIDERD_PORT: String(await free_port()),
  };
  return { config, path, environment };
}

async function expired_catalog(config: Deployment) {
  const options = config.catalog;
  if (!options) throw new Error("missing_catalog");
  await open_journal(options.directory);
  const sources = seed_sources(config);
  await writeFile(options.sources, JSON.stringify(sources), { mode: 0o600 });
  const generation = await build_catalog(
    config,
    new Uint8Array(32).fill(1),
    [],
    {},
    AbortSignal.timeout(5000),
  );
  // The fixture represents a formerly valid immutable generation whose window
  // has ended. The regular loader must parse it but refuse activating it now.
  const past = Date.now() - 120_000;
  generation.resolved_at = new Date(past).toISOString();
  generation.expires_at = new Date(past + 60_000).toISOString();
  for (const source of generation.sources)
    if (Date.parse(source.issued_at) > past) source.issued_at = new Date(past - 1000).toISOString();
  await finish_invocation(
    options.directory,
    { result_path: join(options.directory, "0001.result") },
    catalog_bytes(generation),
  );
  const history = await load_catalog(config);
  expect(history).toHaveLength(1);
  expect(Date.parse(history[0]?.expires_at ?? "")).toBeLessThan(Date.now());
}

describe("installed read-only recovery mode", () => {
  it.each(["missing", "expired"])(
    "recovers completed work with a %s catalog and no supplier credentials",
    async (catalog) => {
      if (!fixture) throw new Error("missing_fixture");
      const original = test_request(fixture.host);
      const first = await fixture.client().invokeModel(original, { timeoutMs: 4500 });
      const { config, path, environment } = await configuration();
      config.catalog = {
        directory: join(directory, "catalog"),
        sources: join(directory, "sources.json"),
        maximum_generations: 64,
        trusted_keys: [],
      };
      if (catalog === "expired") await expired_catalog(config);
      await writeFile(path, JSON.stringify(config), { mode: 0o600 });
      await expect(
        exec_file(process.execPath, [executable], { env: environment, timeout: 10_000 }),
      ).rejects.toMatchObject({ stderr: "provider_start_failed\n", stdout: "" });
      const before = await readdir(config.journal);
      const child = spawn(process.execPath, [executable, "--recover-only"], {
        env: environment,
        stdio: ["ignore", "pipe", "pipe"],
      });
      let stderr = "";
      child.stderr.on("data", (chunk: Buffer) => {
        stderr += chunk.toString("utf8");
      });
      try {
        expect(await wait_ready(child)).toMatchObject({
          model_rpc_configured: true,
          recovery_only: true,
        });
        const client = createClient(
          ProviderService,
          createGrpcTransport({
            baseUrl: `https://localhost:${config.port}`,
            idleConnectionTimeoutMs: 100,
            nodeOptions: {
              ca: await readFile(config.tls.ca),
              cert: await readFile(join(directory, "client.pem")),
              key: await readFile(join(directory, "client.key")),
            },
          }),
        );
        const result = await client.lookupInvocation(lookup_request(original), { timeoutMs: 4500 });
        expect(result.state).toBe(InvocationState.COMPLETED);
        expect(result.response).toEqual(first.response);
        await expect(client.invokeModel(original, { timeoutMs: 4500 })).rejects.toMatchObject({
          code: Code.FailedPrecondition,
          rawMessage: "provider_recovery_only",
        });
        const stream = client.streamModel(
          create(StreamModelRequestSchema, {
            context: original.context,
            invocation: original.invocation,
          }),
          { timeoutMs: 4500 },
        );
        await expect(async () => {
          for await (const _event of stream) throw new Error("unexpected_stream_event");
        }).rejects.toMatchObject({ rawMessage: "provider_recovery_only" });
        expect(await readdir(config.journal)).toEqual(before);
        expect(fixture.requests).toHaveLength(2);
        expect(stderr).toBe("");
        expect(JSON.stringify(toJson(LookupInvocationResponseSchema, result))).not.toContain(
          TEST_SECRET,
        );
        const exited = once(child, "exit");
        child.kill("SIGTERM");
        expect((await exited)[0]).toBe(0);
      } finally {
        await stop(child);
      }
    },
    30_000,
  );

  it("refuses missing journal storage without manufacturing an empty replacement", async () => {
    const { config, path, environment } = await configuration();
    config.journal = join(directory, "missing-journal");
    await writeFile(path, JSON.stringify(config), { mode: 0o600 });
    await expect(
      exec_file(process.execPath, [executable, "--recover-only"], {
        env: environment,
        timeout: 10_000,
      }),
    ).rejects.toMatchObject({ stderr: "provider_start_failed\n", stdout: "" });
    await expect(access(config.journal)).rejects.toMatchObject({ code: "ENOENT" });
  }, 15_000);

  it.each([
    ["--describe", "--recover-only"],
    ["--recover-only", "--describe"],
    ["--catalog-refresh", "--recover-only"],
  ])(
    "rejects contradictory modes %j",
    async (...arguments_) => {
      const { config, path, environment } = await configuration();
      await writeFile(path, JSON.stringify(config), { mode: 0o600 });
      await expect(
        exec_file(process.execPath, [executable, ...arguments_], {
          env: environment,
          timeout: 10_000,
        }),
      ).rejects.toMatchObject({ stderr: "provider_start_failed\n", stdout: "" });
    },
    15_000,
  );
});
