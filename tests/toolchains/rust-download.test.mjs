import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const script = fileURLToPath(new URL("../../scripts/download-rust-component.sh", import.meta.url));
const archive = "dist/2026-02-12/cargo-1.93.1-x86_64-unknown-linux-gnu.tar.xz";
const content = "pinned Rust component fixture";
const digest = createHash("sha256").update(content).digest("hex");

function fixture(t) {
  const directory = mkdtempSync(join(tmpdir(), "loop-rust-download-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const curl = join(directory, "curl");
  copyFileSync(new URL("fixtures/curl.sh", import.meta.url), curl);
  chmodSync(curl, 0o755);
  const output = join(directory, "component with spaces.tar.xz");
  const log = join(directory, "requests.log");
  return {
    directory,
    output,
    run(mode, mirror = "sjtug", hash = digest, path = archive) {
      return spawnSync("bash", [script, mirror, path, hash, output], {
        encoding: "utf8",
        timeout: 5_000,
        env: {
          ...process.env,
          PATH: `${directory}:${process.env.PATH}`,
          LOOP_TEST_DOWNLOAD_LOG: log,
          LOOP_TEST_DOWNLOAD_MODE: mode,
        },
      });
    },
    requests() {
      return existsSync(log) ? readFileSync(log, "utf8").trim().split("\n") : [];
    },
    assertClean() {
      assert.deepEqual(
        readdirSync(directory).filter((name) => name.includes(".part.")),
        [],
      );
    },
  };
}

test("primary failure falls back to verified official bytes", (t) => {
  const f = fixture(t);
  const result = f.run("fallback");
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(f.requests(), [
    `https://mirrors.sjtug.sjtu.edu.cn/rust-static/${archive}`,
    `https://static.rust-lang.org/${archive}`,
  ]);
  assert.equal(readFileSync(f.output, "utf8"), content);
  f.assertClean();
});

test("official primary has a distinct secondary source", (t) => {
  const f = fixture(t);
  assert.equal(f.run("secondary", "official").status, 0);
  assert.deepEqual(f.requests(), [
    `https://static.rust-lang.org/${archive}`,
    `https://rsproxy.cn/${archive}`,
  ]);
  f.assertClean();
});

test("valid cache avoids every network request", (t) => {
  const f = fixture(t);
  writeFileSync(f.output, content);
  assert.equal(f.run("unavailable").status, 0);
  assert.deepEqual(f.requests(), []);
  f.assertClean();
});

test("checksum mismatch fails before fallback or replacement", (t) => {
  const f = fixture(t);
  writeFileSync(f.output, "old cache");
  const result = f.run("corrupt");
  assert.equal(result.status, 1);
  assert.match(result.stderr, /checksum mismatch/);
  assert.equal(f.requests().length, 1);
  assert.equal(readFileSync(f.output, "utf8"), "old cache");
  f.assertClean();
});

test("all transports fail within a finite source list", (t) => {
  const f = fixture(t);
  assert.equal(f.run("unavailable").status, 1);
  assert.equal(f.requests().length, 3);
  assert.equal(new Set(f.requests()).size, 3);
  assert.equal(existsSync(f.output), false);
  f.assertClean();
});

test("transport cancellation removes partial bytes", (t) => {
  const f = fixture(t);
  assert.equal(f.run("interrupt").status, 143);
  assert.equal(existsSync(f.output), false);
  f.assertClean();
});

for (const [mirror, hash, path] of [
  ["https://untrusted.example", digest, archive],
  ["official", "00", archive],
  ["official", digest, "dist/../../unexpected"],
]) {
  test(`invalid download arguments are rejected: ${mirror} ${path} ${hash.length}`, (t) => {
    const f = fixture(t);
    assert.equal(f.run("success", mirror, hash, path).status, 2);
    assert.deepEqual(f.requests(), []);
    f.assertClean();
  });
}

test("output symlinks cannot redirect a download", (t) => {
  const f = fixture(t);
  const target = join(f.directory, "unrelated");
  writeFileSync(target, "preserve");
  symlinkSync(target, f.output);
  assert.equal(f.run("success").status, 2);
  assert.equal(readFileSync(target, "utf8"), "preserve");
  assert.deepEqual(f.requests(), []);
});
