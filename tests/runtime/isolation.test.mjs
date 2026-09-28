import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const compose = fileURLToPath(new URL("../../infra/compose/runtime-workers.yaml", import.meta.url));
const python = `
import hashlib, os, pathlib, socket, sys
role, protected, host_view, expected = sys.argv[1:]
assert os.getuid() == 65532
status = pathlib.Path('/proc/self/status').read_text()
assert 'NoNewPrivs:\\t1' in status
assert 'CapEff:\\t0000000000000000' in status
for path in [protected, host_view, '/var/run/docker.sock', '/run/secrets/loopd-database-url']:
    try:
        pathlib.Path(path).read_bytes()
    except OSError:
        pass
    else:
        raise AssertionError('unexpected source or control-plane access')
try:
    pathlib.Path('/etc/loop-write-test').write_text('forbidden')
except OSError:
    pass
else:
    raise AssertionError('writable root filesystem')
assert not any('DATABASE' in name or 'CAPABILITY' in name or 'API_KEY' in name for name in os.environ)
if role in ('research', 'holdout'):
    payload = pathlib.Path('/data/payload')
    assert hashlib.sha256(payload.read_bytes()).hexdigest() == expected
    try:
        payload.write_text('forbidden')
    except OSError:
        pass
    else:
        raise AssertionError('writable data mount')
else:
    assert not pathlib.Path('/data').exists()
probe = socket.socket()
probe.settimeout(0.2)
assert probe.connect_ex(('192.0.2.1', 443)) != 0
print('isolated ' + role)
`;

test("runtime roles cannot escape their read-only leaf namespace", (t) => {
  const directory = mkdtempSync(join(tmpdir(), "loop-runtime-isolation-"));
  const project = `loop-isolation-${randomUUID()}`;
  const view = join(directory, "view");
  const protectedStore = join(directory, "protected");
  mkdirSync(view, { mode: 0o755 });
  mkdirSync(protectedStore, { mode: 0o700 });
  writeFileSync(join(view, "payload"), "authorized synthetic bytes", { mode: 0o444 });
  writeFileSync(join(protectedStore, "secret"), "never available to discovery", { mode: 0o600 });
  const env = { ...process.env, LOOP_WORKER_VIEW: view };
  const base = ["compose", "--project-name", project, "-f", compose];
  t.after(() => {
    const cleanup = spawnSync("docker", [...base, "--profile", "*", "down"], {
      env,
      timeout: 30_000,
      encoding: "utf8",
    });
    chmodSync(view, 0o755);
    rmSync(directory, { recursive: true, force: true });
    assert.equal(cleanup.status, 0, cleanup.stderr);
  });
  const config = spawnSync("docker", [...base, "--profile", "*", "config", "--format", "json"], {
    env,
    encoding: "utf8",
    timeout: 15_000,
  });
  assert.equal(config.status, 0, config.stderr);
  const services = JSON.parse(config.stdout).services;
  for (const [role, service] of Object.entries(services)) {
    assert.match(service.image, /@sha256:[a-f0-9]{64}$/);
    assert.equal(service.network_mode, "none");
    assert.equal(service.read_only, true);
    assert.deepEqual(service.cap_drop, ["ALL"]);
    assert.equal(service.user, "65532:65532");
    assert.deepEqual(
      (service.volumes ?? []).map((mount) => mount.target),
      role === "research" || role === "holdout" ? ["/data"] : [],
    );
    const result = spawnSync(
      "docker",
      [
        ...base,
        "--profile",
        role,
        "run",
        "--rm",
        "--no-deps",
        role,
        "-c",
        python,
        role,
        join(protectedStore, "secret"),
        join(view, "payload"),
        "250de46952250ff305e75d702ca9c7eeb4e53c9ddd04126b216d07b7608d4374",
      ],
      { env, encoding: "utf8", timeout: 120_000 },
    );
    assert.equal(result.status, 0, `${role}: ${result.stderr}`);
    assert.equal(result.stdout.trim(), `isolated ${role}`);
  }
  assert.equal(readFileSync(join(view, "payload"), "utf8"), "authorized synthetic bytes");
});
