import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";

import { claim_invocation, finish_invocation, open_journal } from "../src/journal.js";

const directories: string[] = [];
const claim = {
  schema: "loop.provider-claim/v1" as const,
  actor: "test-owner",
  request_sha256: "a".repeat(64),
  reserved_nano_usd: "100",
};
const worker = fileURLToPath(new URL("./journal-worker.mjs", import.meta.url));
afterEach(async () => {
  for (const directory of directories.splice(0))
    await rm(directory, { recursive: true, force: true });
});

async function test_directory() {
  const directory = await mkdtemp(join(tmpdir(), "loop-provider-journal-"));
  directories.push(directory);
  await open_journal(directory);
  return directory;
}

function run_worker(directory: string, key: string, mode = "race") {
  const child = spawn(process.execPath, [worker, directory, key, mode], {
    stdio: ["ignore", "pipe", "pipe"],
  });
  let output = "";
  let error = "";
  child.stdout.on("data", (chunk) => {
    output += String(chunk);
  });
  child.stderr.on("data", (chunk) => {
    error += String(chunk);
  });
  const completed = once(child, "exit").then(([code, signal]) => ({
    code,
    signal,
    output: output.trim(),
    error,
  }));
  return { child, completed };
}

describe("private invocation journal", () => {
  it("requires an existing administrative parent before creating journal state", async () => {
    const directory = await test_directory();
    await expect(open_journal(join(directory, "missing", "journal"))).rejects.toMatchObject({
      code: "journal_unavailable",
    });
    expect(await readdir(directory)).toEqual([]);
  });

  it.each([2, 4, 8])(
    "permits one new dispatch among %i independent processes",
    async (writers) => {
      const directory = await test_directory();
      const processes = Array.from({ length: writers }, () => run_worker(directory, "same-key"));
      try {
        const results = await Promise.all(processes.map((process) => process.completed));
        expect(results.every((result) => result.code === 0)).toBe(true);
        expect(results.filter((result) => result.output === "claimed")).toHaveLength(1);
        expect(results.filter((result) => result.output === "invocation_ambiguous")).toHaveLength(
          writers - 1,
        );
      } finally {
        for (const process of processes) process.child.kill("SIGKILL");
      }
    },
    15_000,
  );

  it.each(["claimed", "completed"])("retains the %s boundary after kill/restart", async (mode) => {
    const directory = await test_directory();
    const process = run_worker(directory, "crash-key", mode);
    try {
      await once(process.child.stdout, "data");
      process.child.kill("SIGKILL");
      expect((await process.completed).signal).toBe("SIGKILL");
      const restarted = await run_worker(directory, "crash-key").completed;
      expect(restarted.code).toBe(0);
      expect(restarted.output).toBe(mode === "completed" ? "cached" : "invocation_ambiguous");
    } finally {
      process.child.kill("SIGKILL");
    }
  });

  it("preserves the first result instead of overwriting it", async () => {
    const directory = await test_directory();
    const slot = await claim_invocation(directory, "immutable", claim);
    await finish_invocation(directory, slot, new Uint8Array([1, 2, 3]));
    await expect(finish_invocation(directory, slot, new Uint8Array([9]))).rejects.toThrow();
    expect(
      Array.from((await claim_invocation(directory, "immutable", claim)).cached ?? []),
    ).toEqual([1, 2, 3]);
    expect((await readdir(directory)).some((name) => name.startsWith(".pending-"))).toBe(false);
  });

  it("refuses corrupt response bytes without authorizing a resend", async () => {
    const directory = await test_directory();
    const slot = await claim_invocation(directory, "corrupt", claim);
    await finish_invocation(directory, slot, new Uint8Array([1]));
    const record = JSON.parse(await readFile(slot.result_path, "utf8"));
    record.bytes = "Ag==";
    await writeFile(slot.result_path, JSON.stringify(record));
    await expect(claim_invocation(directory, "corrupt", claim)).rejects.toMatchObject({
      code: "invocation_ambiguous",
    });
  });
});
