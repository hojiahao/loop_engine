import { type ChildProcess, execFile, spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { type AddressInfo, createServer } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { fromJson } from "@bufbuild/protobuf";
import { createClient } from "@connectrpc/connect";
import { createGrpcTransport } from "@connectrpc/connect-node";
import {
  ModelResolutionSnapshotSchema,
  PolicyReferenceSchema,
  ProviderService,
} from "@loop-engine/protocol/provider";
import { afterEach, describe, expect, it } from "vitest";
import { validate_deployment } from "../src/config.js";
import { ProviderHost } from "../src/host.js";
import { seed_sources } from "./catalog-fixture.js";
import { test_certificates, test_config, test_reply, test_request } from "./fixture.js";
import { metadata_fixture } from "./metadata-fixture.js";

const executable = fileURLToPath(new URL("../dist/index.js", import.meta.url));
const exec_file = promisify(execFile);
let directory: string;
let fixture: Awaited<ReturnType<typeof metadata_fixture>>;
afterEach(async () => {
  await fixture?.close();
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

function wait_event(child: ChildProcess, event: string): Promise<Record<string, unknown>> {
  return new Promise((resolve, reject) => {
    const output = child.stdout;
    if (!output) return reject(new Error("missing_output"));
    let bytes = "";
    const timer = setTimeout(() => finish(new Error("cli_event_timeout")), 10_000);
    function finish(error?: Error, value?: Record<string, unknown>) {
      clearTimeout(timer);
      output?.off("data", read_event);
      child.off("exit", exited);
      if (error) reject(error);
      else resolve(value ?? {});
    }
    function exited() {
      finish(new Error("cli_exited"));
    }
    function read_event(chunk: Buffer) {
      bytes += chunk.toString("utf8");
      const lines = bytes.split("\n");
      bytes = lines.pop() ?? "";
      for (const line of lines) {
        const value = JSON.parse(line) as Record<string, unknown>;
        if (value.event === event) finish(undefined, value);
      }
    }
    child.once("exit", exited);
    output.on("data", read_event);
  });
}

describe("installed catalog lifecycle", () => {
  it("publishes, describes, reloads and restarts with immutable invocation pins", async () => {
    directory = await mkdtemp(join(tmpdir(), "loop-catalog-cli-"));
    fixture = await metadata_fixture();
    const config = test_config(directory);
    const model = config.models[0];
    const principal = config.principals[0];
    if (!model || !principal) throw new Error("missing_fixture");
    principal.certificate_sha256 = test_certificates(directory);
    principal.model_ids = [model.id];
    model.plugin = "ollama";
    model.input_token_limit = 128;
    delete model.secret_env;
    model.compatible = {
      base_url: `${fixture.base_url}/v1`,
      wire: "chat",
      auth: "none",
      strict_tools: false,
      output_limit: "max_tokens",
      thinking: "none",
      reasoning_field: "none",
      stream_usage: "separate",
    };
    config.models = [model];
    config.port = await free_port();
    config.catalog = {
      directory: join(directory, "catalog"),
      sources: join(directory, "sources.json"),
      maximum_generations: 64,
      trusted_keys: [],
    };
    validate_deployment(config);
    const sources = seed_sources(config);
    const path = join(directory, "deployment.json");
    await writeFile(path, JSON.stringify(config), { mode: 0o600 });
    await writeFile(config.catalog.sources, JSON.stringify(sources), { mode: 0o600 });
    const environment = {
      ...process.env,
      PROVIDERD_DEPLOYMENT: path,
      PROVIDERD_PORT: String(await free_port()),
    };
    async function cli(argument: string) {
      const result = await exec_file(process.execPath, [executable, argument], {
        env: environment,
        timeout: 10_000,
      });
      expect(result.stderr).toBe("");
      return JSON.parse(result.stdout);
    }
    expect((await cli("--catalog-refresh")).revision).toBe(1);
    const pins = await cli("--describe");
    expect(pins.catalog.models[0].verification).toBe("contract_verified");
    expect(fixture.requests).toHaveLength(0);
    const static_host = new ProviderHost({ ...config, catalog: undefined }, new Uint8Array(32), {});
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
    function request(selected = pins) {
      const command = test_request(static_host);
      if (!command.invocation) throw new Error("missing_request");
      command.invocation.model = fromJson(
        ModelResolutionSnapshotSchema,
        selected.models[0].snapshot,
      );
      command.invocation.requestPolicy = fromJson(PolicyReferenceSchema, selected.request_policy);
      return command;
    }
    fixture.state.pages = Array.from({ length: 3 }, () =>
      test_reply("/v1/chat/completions", { model: model.model }),
    );
    let child = spawn(process.execPath, [executable], {
      env: environment,
      stdio: ["ignore", "pipe", "pipe"],
    });
    try {
      await wait_event(child, "listening");
      expect(
        (await client.invokeModel(request(), { timeoutMs: 4500 })).response?.content,
      ).not.toHaveLength(0);
      sources.revision++;
      sources.issued_at = new Date().toISOString();
      sources.overrides[0] = { route_id: model.id, input_usd: "10", availability: "active" };
      await writeFile(config.catalog.sources, JSON.stringify(sources), { mode: 0o600 });
      expect((await cli("--catalog-refresh")).revision).toBe(2);
      const reloaded = wait_event(child, "catalog_reloaded");
      child.kill("SIGHUP");
      expect((await reloaded).revision).toBe(2);
      const next = await cli("--describe");
      expect(next.models[0].snapshot.resolutionId).not.toEqual(
        pins.models[0].snapshot.resolutionId,
      );
      await expect(client.invokeModel(request(next), { timeoutMs: 4500 })).rejects.toThrow(
        "provider_budget_denied",
      );
      expect(
        (await client.invokeModel(request(), { timeoutMs: 4500 })).response?.resolutionId?.value,
      ).toBe(pins.models[0].snapshot.resolutionId.value);
      const exited = once(child, "exit");
      child.kill("SIGTERM");
      expect((await exited)[0]).toBe(0);
      child = spawn(process.execPath, [executable], {
        env: environment,
        stdio: ["ignore", "pipe", "pipe"],
      });
      await wait_event(child, "listening");
      expect(
        (await client.invokeModel(request(), { timeoutMs: 4500 })).response?.resolutionId?.value,
      ).toBe(pins.models[0].snapshot.resolutionId.value);
      expect(fixture.requests).toHaveLength(3);
      expect(fixture.requests.every((entry) => !entry.headers.authorization)).toBe(true);
    } finally {
      if (child.exitCode === null && child.signalCode === null) {
        const exited = once(child, "exit");
        child.kill("SIGKILL");
        await exited;
      }
    }
  }, 60_000);
});
