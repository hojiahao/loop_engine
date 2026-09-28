import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { build_catalog } from "../src/catalog-merge.js";
import { catalog_digest, load_catalog, publish_catalog } from "../src/catalog-store.js";
import type { Deployment } from "../src/config.js";
import { open_journal } from "../src/journal.js";
import { CATALOG_PLUGIN, CATALOG_SECRETS, seed_sources } from "./catalog-fixture.js";
import { test_config } from "./fixture.js";

let directory: string;
let config: Deployment;
const worker = fileURLToPath(new URL("./catalog-worker.mjs", import.meta.url));
beforeEach(async () => {
  directory = await mkdtemp(join(tmpdir(), "loop-catalog-process-"));
  config = test_config(directory);
  config.catalog = {
    directory: join(directory, "catalog"),
    sources: join(directory, "sources.json"),
    maximum_generations: 64,
    trusted_keys: [],
  };
  await open_journal(config.catalog.directory);
  await writeFile(config.catalog.sources, JSON.stringify(seed_sources(config)), { mode: 0o600 });
  const generation = await build_catalog(
    config,
    CATALOG_PLUGIN,
    [],
    CATALOG_SECRETS,
    new AbortController().signal,
  );
  await writeFile(join(directory, "candidate.json"), JSON.stringify(generation), { mode: 0o600 });
  await writeFile(join(directory, "deployment.json"), JSON.stringify(config), { mode: 0o600 });
});
afterEach(async () => {
  if (directory) await rm(directory, { recursive: true, force: true });
});

function run_worker(mode = "race") {
  const child = spawn(
    process.execPath,
    [worker, join(directory, "deployment.json"), join(directory, "candidate.json"), mode],
    {
      stdio: ["ignore", "pipe", "pipe", "ipc"],
    },
  );
  let output = "";
  let error = "";
  child.stdout?.on("data", (chunk) => {
    output += String(chunk);
  });
  child.stderr?.on("data", (chunk) => {
    error += String(chunk);
  });
  const completed = once(child, "exit").then(([code, signal]) => ({
    code,
    signal,
    output: output.trim(),
    error,
  }));
  const ready = Promise.race([
    once(child, "message").then(([message]) => {
      if (message !== "ready") throw new Error("worker_not_ready");
    }),
    completed.then(() => {
      throw new Error("worker_exited");
    }),
  ]);
  return { child, completed, ready };
}

describe("durable catalog publication", () => {
  it.each([2, 4, 8])(
    "serializes %i independent OS publishers",
    async (writers) => {
      const processes = Array.from({ length: writers }, () => run_worker());
      try {
        await Promise.all(processes.map((process) => process.ready));
        for (const process of processes) process.child.send("publish");
        const results = await Promise.all(processes.map((process) => process.completed));
        expect(results.every((result) => result.code === 0 && result.error === "")).toBe(true);
        expect(results.filter((result) => result.output === "published")).toHaveLength(1);
        expect(
          results.filter((result) =>
            ["provider_catalog_conflict", "provider_catalog_publish_failed"].includes(
              result.output,
            ),
          ),
        ).toHaveLength(writers - 1);
        expect(await load_catalog(config)).toHaveLength(1);
        expect(await readdir(join(directory, "catalog"))).toEqual(["0001.result"]);
      } finally {
        for (const process of processes) process.child.kill("SIGKILL");
      }
    },
    20_000,
  );

  it.each(["prepared", "completed"])(
    "recovers after killing a %s publisher",
    async (mode) => {
      const process = run_worker(mode);
      try {
        await process.ready;
        const boundary = once(process.child, "message");
        process.child.send("publish");
        expect((await boundary)[0]).toBe(mode);
        process.child.kill("SIGKILL");
        expect((await process.completed).signal).toBe("SIGKILL");
        const before = await load_catalog(config);
        expect(before).toHaveLength(mode === "completed" ? 1 : 0);
        const restarted = run_worker();
        try {
          await restarted.ready;
          restarted.child.send("publish");
          const result = await restarted.completed;
          expect(result.code).toBe(0);
          expect(result.output).toBe(
            mode === "completed" ? "provider_catalog_conflict" : "published",
          );
        } finally {
          restarted.child.kill("SIGKILL");
        }
        const recovered = await load_catalog(config);
        expect(recovered).toHaveLength(1);
        if (before[0] && recovered[0])
          expect(catalog_digest(recovered[0])).toBe(catalog_digest(before[0]));
      } finally {
        process.child.kill("SIGKILL");
      }
    },
    20_000,
  );

  it("bounds retained generations without silently removing old pins", async () => {
    if (!config.catalog) throw new Error("missing_catalog");
    config.catalog.maximum_generations = 2;
    const first = JSON.parse(await readFile(join(directory, "candidate.json"), "utf8"));
    await publish_catalog(config, first);
    const second = { ...first, revision: 2, previous_sha256: catalog_digest(first) };
    await publish_catalog(config, second);
    await expect(
      publish_catalog(config, { ...second, revision: 3, previous_sha256: catalog_digest(second) }),
    ).rejects.toThrow("provider_catalog_conflict");
    expect(await load_catalog(config)).toHaveLength(2);
  });
});
