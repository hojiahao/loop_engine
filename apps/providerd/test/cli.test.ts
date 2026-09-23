import { execFile, spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import type { AddressInfo } from "node:net";
import { createServer } from "node:net";
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

import { ProviderHost } from "../src/host.js";
import { TEST_SECRET, test_certificates, test_config, test_request } from "./fixture.js";

const executable = fileURLToPath(new URL("../dist/index.js", import.meta.url));
const exec_file = promisify(execFile);
const directories: string[] = [];
afterEach(async () => {
  for (const directory of directories.splice(0))
    await rm(directory, { recursive: true, force: true });
});

async function configuration() {
  const directory = await mkdtemp(join(tmpdir(), "loop-provider-cli-"));
  directories.push(directory);
  const config = test_config(directory);
  const path = join(directory, "deployment.json");
  await writeFile(path, JSON.stringify(config), { mode: 0o600 });
  return { directory, config, path };
}

async function free_port() {
  const server = createServer();
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  return { server, port: (server.address() as AddressInfo).port };
}

describe("installed provider executable", () => {
  // The runner deadline includes the child's existing 10-second timeout and cleanup.
  it("prints exact non-secret pins without enabling a listener or calling a model", async () => {
    const fixture = await configuration();
    const result = await exec_file(process.execPath, [executable, "--describe"], {
      env: { ...process.env, PROVIDERD_DEPLOYMENT: fixture.path, LOOP_LLM_TEST: TEST_SECRET },
      timeout: 10_000,
    });
    const pins = JSON.parse(result.stdout);
    expect(pins.models).toHaveLength(3);
    expect(pins.models[0].snapshot.providerPluginSha256.value).toBeTruthy();
    expect(result.stdout + result.stderr).not.toContain(TEST_SECRET);
    expect(result.stderr).toBe("");
  }, 15_000);

  it("redacts invalid deployment values on startup", async () => {
    const fixture = await configuration();
    await writeFile(fixture.path, JSON.stringify({ secret: TEST_SECRET }));
    await expect(
      exec_file(process.execPath, [executable], {
        env: { ...process.env, PROVIDERD_DEPLOYMENT: fixture.path },
        timeout: 10_000,
      }),
    ).rejects.toMatchObject({ stderr: "provider_start_failed\n", stdout: "" });
  }, 15_000);

  it("serves health and authenticated RPC from the compiled entry point", async () => {
    const fixture = await configuration();
    const principal = fixture.config.principals[0];
    if (!principal) throw new Error("fixture_principal_missing");
    principal.certificate_sha256 = test_certificates(fixture.directory);
    const listeners = await Promise.all([free_port(), free_port()]);
    const health_port = listeners[0]?.port;
    const rpc_port = listeners[1]?.port;
    if (!health_port || !rpc_port) throw new Error("fixture_port_missing");
    fixture.config.port = rpc_port;
    await writeFile(fixture.path, JSON.stringify(fixture.config));
    await Promise.all(
      listeners.map(({ server }) => new Promise<void>((resolve) => server.close(() => resolve()))),
    );
    const environment = {
      ...process.env,
      PROVIDERD_DEPLOYMENT: fixture.path,
      PROVIDERD_PORT: String(health_port),
      LOOP_LLM_TEST: "",
    };
    const pins = JSON.parse(
      (
        await exec_file(process.execPath, [executable, "--describe"], {
          env: environment,
          timeout: 10_000,
        })
      ).stdout,
    );
    const child = spawn(process.execPath, [executable], {
      env: environment,
      stdio: ["ignore", "pipe", "pipe"],
    });
    const exited = once(child, "exit");
    try {
      await Promise.race([
        once(child.stdout, "data"),
        exited.then(() => {
          throw new Error("provider_start_failed");
        }),
      ]);
      expect((await fetch(`http://127.0.0.1:${health_port}/healthz`)).status).toBe(200);
      const client = createClient(
        ProviderService,
        createGrpcTransport({
          baseUrl: `https://localhost:${rpc_port}`,
          idleConnectionTimeoutMs: 100,
          nodeOptions: {
            ca: await readFile(fixture.config.tls.ca),
            cert: await readFile(join(fixture.directory, "client.pem")),
            key: await readFile(join(fixture.directory, "client.key")),
          },
        }),
      );
      const request = test_request(new ProviderHost(fixture.config, new Uint8Array(32), {}));
      if (!request.invocation) throw new Error("fixture_request_missing");
      request.invocation.model = fromJson(ModelResolutionSnapshotSchema, pins.models[0].snapshot);
      request.invocation.requestPolicy = fromJson(PolicyReferenceSchema, pins.request_policy);
      await expect(client.invokeModel(request, { timeoutMs: 4500 })).rejects.toMatchObject({
        rawMessage: "provider_credentials_missing",
      });
      child.kill("SIGTERM");
      expect((await exited)[0]).toBe(0);
    } finally {
      child.kill("SIGKILL");
    }
  }, 15_000);
});
