import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { chownSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../../", import.meta.url));
const compose = join(root, "infra/compose/provider.yaml");
const node_image =
  "m.daocloud.io/docker.io/library/node:24.17.0-bookworm-slim@sha256:862263c612aa437e3037674b85419622a9d93bff80aa1eee5398dfe686375532";

function private_file(path, value) {
  writeFileSync(path, value, { mode: 0o600 });
  set_owner(path);
}

function set_owner(path) {
  if (process.getuid() === 0) chownSync(path, 65532, 65532);
  else {
    const result = spawnSync("sudo", ["-n", "chown", "65532:65532", path], {
      encoding: "utf8",
      timeout: 5000,
    });
    assert.equal(result.status, 0, result.stderr);
  }
}

function certificates(directory) {
  function openssl(args) {
    const result = spawnSync("openssl", args, {
      cwd: directory,
      encoding: "utf8",
      timeout: 10_000,
    });
    assert.equal(result.status, 0, result.stderr);
    return result.stdout;
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
    "/CN=Loop-isolation-fixture",
    "-days",
    "1",
    "-addext",
    "basicConstraints=critical,CA:TRUE",
    "-addext",
    "keyUsage=critical,keyCertSign,cRLSign",
  ]);
  for (const name of ["server", "client", "supplier"]) {
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
      `subjectAltName=DNS:${name === "supplier" ? "api.openai.com" : "localhost,DNS:provider"}`,
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
  }
  const result = spawnSync(
    "openssl",
    ["x509", "-in", join(directory, "client.pem"), "-outform", "DER"],
    { timeout: 10_000 },
  );
  assert.equal(result.status, 0);
  return createHash("sha256").update(result.stdout).digest("hex");
}

test("actual Provider serves approved TLS calls inside data and egress boundaries", {
  timeout: 180_000,
}, async (t) => {
  const directory = mkdtempSync(join(tmpdir(), "loop-provider-isolation-"));
  const project = `loop-provider-${randomUUID()}`;
  const config_dir = join(directory, "config");
  const state_dir = join(directory, "state");
  const supplier_dir = join(directory, "supplier");
  const client_dir = join(directory, "client");
  for (const path of [config_dir, state_dir, supplier_dir, client_dir]) {
    mkdirSync(path, { mode: 0o700 });
  }
  const override = join(directory, "compose.json");
  const env = {
    ...process.env,
    LOOP_PROVIDER_RELEASE: root,
    LOOP_PROVIDER_CONFIG: config_dir,
    LOOP_PROVIDER_STATE: state_dir,
    LOOP_PROVIDER_EGRESS: join(directory, "egress.json"),
    LOOP_PROVIDER_ENV: join(directory, "secrets.env"),
  };
  const base = ["compose", "--project-name", project, "-f", compose, "-f", override];
  function docker(args, timeout = 30_000, input) {
    const result = spawnSync("docker", [...base, ...args], {
      env,
      encoding: "utf8",
      timeout,
      input,
      maxBuffer: 1_048_576,
    });
    assert.equal(result.status, 0, result.stderr || result.error?.message);
    return result.stdout;
  }
  t.after(() => {
    const result = spawnSync("docker", [...base, "down", "--volumes", "--remove-orphans"], {
      env,
      encoding: "utf8",
      timeout: 30_000,
    });
    if (process.getuid() !== 0) {
      const restored = spawnSync(
        "sudo",
        ["-n", "chown", "-R", `${process.getuid()}:${process.getgid()}`, directory],
        { encoding: "utf8", timeout: 5000 },
      );
      assert.equal(restored.status, 0, restored.stderr);
    }
    rmSync(directory, { recursive: true, force: true });
    assert.equal(result.status, 0, result.stderr);
  });
  const fingerprint = certificates(directory);
  for (const name of ["ca.pem", "server.pem", "server.key"])
    private_file(join(config_dir, name), readFileSync(join(directory, name)));
  for (const name of ["supplier.pem", "supplier.key"])
    private_file(join(supplier_dir, name), readFileSync(join(directory, name)));
  for (const name of ["ca.pem", "client.pem", "client.key"])
    private_file(join(client_dir, name), readFileSync(join(directory, name)));
  const protected_path = join(directory, "protected-data");
  writeFileSync(protected_path, "never available to provider", { mode: 0o600 });
  writeFileSync(env.LOOP_PROVIDER_ENV, "LOOP_LLM_TEST=synthetic-provider-key\n", { mode: 0o600 });
  private_file(
    env.LOOP_PROVIDER_EGRESS,
    JSON.stringify({
      schema: "loop.provider-egress/v1",
      routes: [
        {
          host: "api.openai.com",
          port: 443,
          target_host: "supplier",
          target_port: 8443,
          allow_private: true,
        },
      ],
    }),
  );
  private_file(
    join(config_dir, "deployment.json"),
    JSON.stringify({
      schema: "loop.provider-deployment/v1",
      resolved_at: "2026-09-01T00:00:00Z",
      port: 8091,
      listen_address: "0.0.0.0",
      tls: {
        ca: "/run/provider/ca.pem",
        certificate: "/run/provider/server.pem",
        key: "/run/provider/server.key",
      },
      journal: "/var/lib/provider/journal",
      principals: [
        {
          certificate_sha256: fingerprint,
          actor_id: "loopd-fixture",
          actor_kind: "service",
          model_ids: ["maker"],
        },
      ],
      policy: {
        id: "isolated",
        revision: "1",
        input_tokens: 1024,
        output_tokens: 256,
        maximum_usd: "1",
        wall_time_ms: 5000,
        concurrency: 2,
      },
      models: [
        {
          id: "maker",
          alias: "maker",
          plugin: "openai_responses",
          model: "model-fixture-20260901",
          context_tokens: 4096,
          output_tokens: 256,
          input_usd: "1",
          output_usd: "2",
          cached_usd: "0.1",
          secret_env: "LOOP_LLM_TEST",
        },
      ],
    }),
  );
  writeFileSync(
    override,
    JSON.stringify({
      services: {
        provider: { environment: { NODE_EXTRA_CA_CERTS: "/run/provider/ca.pem" } },
        client: {
          image: node_image,
          user: "65532:65532",
          read_only: true,
          cap_drop: ["ALL"],
          security_opt: ["no-new-privileges:true"],
          profiles: ["acceptance"],
          networks: ["provider"],
          entrypoint: ["node", "/opt/client.mjs"],
          volumes: [
            ...[
              "apps/providerd/package.json",
              "apps/providerd/node_modules",
              "node_modules",
              "packages/protocol-ts/dist",
              "packages/protocol-ts/package.json",
              "packages/protocol-ts/node_modules",
            ].map((path) => ({
              type: "bind",
              source: join(root, path),
              target: `/opt/loop-engine/${path}`,
              read_only: true,
            })),
            {
              type: "bind",
              source: join(root, "tests/runtime/provider-client.mjs"),
              target: "/opt/client.mjs",
              read_only: true,
            },
            { type: "bind", source: client_dir, target: "/fixture", read_only: true },
          ],
        },
        supplier: {
          image: node_image,
          user: "65532:65532",
          read_only: true,
          cap_drop: ["ALL"],
          security_opt: ["no-new-privileges:true"],
          networks: ["outbound"],
          entrypoint: ["node", "/opt/supplier.mjs"],
          volumes: [
            {
              type: "bind",
              source: join(root, "tests/runtime/provider-supplier.mjs"),
              target: "/opt/supplier.mjs",
              read_only: true,
            },
            { type: "bind", source: supplier_dir, target: "/fixture", read_only: true },
          ],
        },
      },
    }),
  );
  for (const path of [config_dir, state_dir, supplier_dir, client_dir]) set_owner(path);
  const rendered = JSON.parse(docker(["config", "--format", "json"]));
  assert.equal(rendered.networks.provider.internal, true);
  assert.deepEqual(Object.keys(rendered.services.provider.networks), ["provider"]);
  assert.equal(rendered.services.provider.read_only, true);
  assert.deepEqual(rendered.services.provider.cap_drop, ["ALL"]);
  assert.equal(rendered.services.provider.user, "65532:65532");
  assert.ok(
    rendered.services.provider.volumes.every(
      (volume) => ![root, protected_path].includes(volume.source),
    ),
  );
  docker(["up", "--detach", "--wait", "--wait-timeout", "45"], 120_000);
  const pins = JSON.parse(
    docker(["exec", "-T", "provider", "node", "dist/index.js", "--describe"]),
  );
  try {
    assert.equal(
      docker(["run", "--rm", "--no-deps", "-T", "client"], 15_000, JSON.stringify(pins)).trim(),
      "isolated invocation",
    );
  } catch (error) {
    t.diagnostic(docker(["logs", "--no-log-prefix", "provider", "egress"]));
    throw error;
  }
  const supplier = docker(["ps", "-q", "supplier"]).trim();
  const inspected = spawnSync(
    "docker",
    ["inspect", "--format", "{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}", supplier],
    { encoding: "utf8", timeout: 10_000 },
  );
  assert.equal(inspected.status, 0);
  const supplier_ip = inspected.stdout.trim();
  assert.match(supplier_ip, /^[0-9.]+$/);
  const probe = `
import assert from 'node:assert/strict';
import {readFileSync,writeFileSync,existsSync} from 'node:fs';
import {createConnection} from 'node:net';
import {request} from 'node:http';
import {Resolver} from 'node:dns/promises';
assert.equal(process.getuid(),65532);
const status=readFileSync('/proc/self/status','utf8');
assert.match(status,/NoNewPrivs:\\s+1/); assert.match(status,/CapEff:\\s+0000000000000000/);
for(const path of [${JSON.stringify(protected_path)},'/data','/var/run/docker.sock','/run/secrets/loopd-database-url','/opt/loop-engine/docs','/opt/loop-engine/.git','/opt/loop-engine/python']) assert.equal(existsSync(path),false);
assert.throws(()=>writeFileSync('/etc/forbidden','no'));
assert.ok(!Object.keys(process.env).some(name=>/DATABASE|HOLDOUT|CAPABILITY/.test(name)));
await assert.rejects(new Resolver({timeout:1000,tries:1}).resolve4('example.com'));
await new Promise((resolve,reject)=>{const socket=createConnection({host:${JSON.stringify(supplier_ip)},port:8443});socket.once('connect',()=>{socket.destroy();reject(new Error('direct_egress'));});socket.once('error',()=>resolve());socket.setTimeout(500,()=>{socket.destroy();resolve();});});
await new Promise((resolve,reject)=>{const call=request({host:'egress',port:8080,method:'CONNECT',path:'unapproved.example:443'});call.once('connect',(response,socket)=>{socket.destroy();response.statusCode===403?resolve():reject(new Error('egress_authority'));});call.once('error',reject);call.setTimeout(1000,()=>call.destroy(new Error('probe_timeout')));call.end();});
console.log('provider isolated');
`;
  assert.equal(
    docker(["exec", "-T", "provider", "node", "--input-type=module", "-e", probe]).trim(),
    "provider isolated",
  );
  assert.equal(readFileSync(protected_path, "utf8"), "never available to provider");
});
