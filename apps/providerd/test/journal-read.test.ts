import { createHash } from "node:crypto";
import type { FileHandle } from "node:fs/promises";
import {
  appendFile,
  mkdir,
  mkdtemp,
  readdir,
  rename,
  rm,
  symlink,
  truncate,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

const hooks = vi.hoisted(() => ({
  before_open: undefined as ((path: string) => void | Promise<void>) | undefined,
  after_open: undefined as ((handle: FileHandle, path: string) => void | Promise<void>) | undefined,
}));

// Real files and handles are used throughout. The hook only controls the exact
// interleaving between a completed stat and a concurrent writer or administrator.
vi.mock("node:fs/promises", async (import_original) => {
  const actual = await import_original<typeof import("node:fs/promises")>();
  return {
    ...actual,
    mkdir: vi.fn(actual.mkdir),
    open: vi.fn(async (...args: Parameters<typeof actual.open>) => {
      await hooks.before_open?.(String(args[0]));
      const handle = await actual.open(...args);
      await hooks.after_open?.(handle, String(args[0]));
      return handle;
    }),
  };
});

import {
  claim_invocation,
  finish_invocation,
  open_journal,
  read_invocation,
} from "../src/journal.js";

const directories: string[] = [];
const key = "recover-key";
const claim = {
  schema: "loop.provider-claim/v1" as const,
  actor: "journal-reader",
  request_sha256: "a".repeat(64),
  reserved_nano_usd: "100",
};

afterEach(async () => {
  hooks.before_open = undefined;
  hooks.after_open = undefined;
  vi.restoreAllMocks();
  for (const directory of directories.splice(0))
    await rm(directory, { recursive: true, force: true });
});

async function test_directory() {
  const parent = await mkdtemp(join(tmpdir(), "loop-journal-read-"));
  directories.push(parent);
  const directory = join(parent, "journal");
  await open_journal(directory);
  return { parent, directory };
}

function claim_path(directory: string) {
  const id = createHash("sha256").update(claim.actor).update("\0").update(key).digest("hex");
  return join(directory, `${id}.claim`);
}

function read(directory: string) {
  return read_invocation(directory, key, claim.actor, claim.request_sha256);
}

function after_stat(handle: FileHandle, action: () => Promise<unknown>) {
  const stat = handle.stat.bind(handle);
  vi.spyOn(handle, "stat").mockImplementationOnce(async () => {
    const info = await stat();
    await action();
    return info;
  });
}

describe("bounded journal observations", () => {
  it("returns the persisted response from the retained journal directory", async () => {
    const { directory } = await test_directory();
    const slot = await claim_invocation(directory, key, claim);
    const bytes = new Uint8Array([1, 2, 3]);
    await finish_invocation(directory, slot, bytes);
    expect(await read(directory)).toEqual({
      state: "completed",
      reserved: 100n,
      bytes: Buffer.from(bytes),
    });
  });

  it("recovers publication between a missing claim and the result read", async () => {
    const { directory } = await test_directory();
    const bytes = Buffer.from([1, 2, 3]);
    let claim_reads = 0;
    let result_reads = 0;
    hooks.before_open = async (opened) => {
      if (!opened.startsWith("/proc/self/fd/")) return;
      if (opened.endsWith(".claim")) claim_reads++;
      if (opened.endsWith(".result")) {
        result_reads++;
        expect(claim_reads).toBe(1);
        const slot = await claim_invocation(directory, key, claim);
        await finish_invocation(directory, slot, bytes);
      }
    };
    expect(await read(directory)).toEqual({ state: "completed", reserved: 100n, bytes });
    expect(claim_reads).toBe(2);
    expect(result_reads).toBe(1);
  });

  it.each(["", '{"schema":'])("retains partial claim %j as ambiguous", async (prefix) => {
    const { directory } = await test_directory();
    await writeFile(claim_path(directory), prefix, { mode: 0o600 });
    expect(await read(directory)).toEqual({ state: "ambiguous" });
  });

  it("keeps a claim growing after stat ambiguous until a later observation", async () => {
    const { directory } = await test_directory();
    const path = claim_path(directory);
    await writeFile(path, "", { mode: 0o600 });
    hooks.after_open = (handle, opened) => {
      if (opened.endsWith(".claim"))
        after_stat(handle, () => writeFile(path, JSON.stringify(claim)));
    };
    expect(await read(directory)).toEqual({ state: "ambiguous" });
    hooks.after_open = undefined;
    expect(await read(directory)).toEqual({ state: "ambiguous", reserved: 100n });
  });

  it("limits record allocation when a result grows far beyond its initial stat", async () => {
    const { directory } = await test_directory();
    const slot = await claim_invocation(directory, key, claim);
    await finish_invocation(directory, slot, new Uint8Array([1]));
    let initial = 0;
    const reads: Array<{ mock: { calls: unknown[][] } }> = [];
    hooks.after_open = (handle, opened) => {
      if (!opened.endsWith(".result")) return;
      reads.push(vi.spyOn(handle, "read"));
      const stat = handle.stat.bind(handle);
      vi.spyOn(handle, "stat").mockImplementationOnce(async () => {
        const info = await stat();
        initial = info.size;
        await truncate(slot.result_path, 16 * 1_048_576);
        return info;
      });
    };
    await expect(read(directory)).rejects.toMatchObject({ code: "provider_receipt_corrupt" });
    expect(initial).toBeGreaterThan(0);
    expect(reads).toHaveLength(1);
    expect(reads[0]?.mock.calls).toHaveLength(1);
    const buffer = reads[0]?.mock.calls[0]?.[0];
    expect(buffer).toBeInstanceOf(Buffer);
    expect((buffer as Buffer).length).toBe(initial + 1);
  });

  it("rejects growth of a published result even when its JSON remains valid", async () => {
    const { directory } = await test_directory();
    const slot = await claim_invocation(directory, key, claim);
    await finish_invocation(directory, slot, new Uint8Array([1]));
    hooks.after_open = (handle, opened) => {
      if (opened.endsWith(".result")) after_stat(handle, () => appendFile(slot.result_path, " "));
    };
    await expect(read(directory)).rejects.toMatchObject({ code: "provider_receipt_corrupt" });
  });

  it("rejects an oversized claim before allocating a read buffer", async () => {
    const { directory } = await test_directory();
    const path = claim_path(directory);
    await writeFile(path, "", { mode: 0o600 });
    await truncate(path, 1_048_577);
    const reads: Array<{ mock: { calls: unknown[][] } }> = [];
    hooks.after_open = (handle, opened) => {
      if (opened.endsWith(".claim")) reads.push(vi.spyOn(handle, "read"));
    };
    await expect(read(directory)).rejects.toMatchObject({ code: "provider_receipt_corrupt" });
    expect(reads).toHaveLength(1);
    expect(reads[0]?.mock.calls).toHaveLength(0);
  });

  it.each(["missing", "replacement", "symlink"])(
    "fails unavailable when validated journal storage becomes %s",
    async (mode) => {
      const { parent, directory } = await test_directory();
      let changed = false;
      hooks.after_open = (handle, opened) => {
        if (opened !== directory || changed) return;
        changed = true;
        after_stat(handle, async () => {
          const moved = join(parent, "retained");
          await rename(directory, moved);
          if (mode === "replacement") await mkdir(directory, { mode: 0o700 });
          if (mode === "symlink") await symlink(moved, directory);
        });
      };
      await expect(read(directory)).rejects.toMatchObject({ code: "journal_unavailable" });
    },
  );

  it("does not return completed evidence from a replaced directory", async () => {
    const { parent, directory } = await test_directory();
    const slot = await claim_invocation(directory, key, claim);
    await finish_invocation(directory, slot, new Uint8Array([1]));
    let changed = false;
    hooks.after_open = (handle, opened) => {
      if (!opened.endsWith(".result") || changed) return;
      changed = true;
      after_stat(handle, async () => {
        await rename(directory, join(parent, "retained"));
        await mkdir(directory, { mode: 0o700 });
      });
    };
    await expect(read(directory)).rejects.toMatchObject({ code: "journal_unavailable" });
  });
});

describe("existing journal validation", () => {
  it("validates existing storage without creating or syncing directories", async () => {
    const { directory } = await test_directory();
    vi.mocked(mkdir).mockClear();
    const syncs: Array<{ mock: { calls: unknown[][] } }> = [];
    hooks.after_open = (handle) => {
      syncs.push(vi.spyOn(handle, "sync"));
    };
    await open_journal(directory, false);
    expect(mkdir).not.toHaveBeenCalled();
    expect(syncs).toHaveLength(1);
    expect(syncs[0]?.mock.calls).toHaveLength(0);
    expect(await readdir(directory)).toEqual([]);
  });

  it("does not create missing recovery storage", async () => {
    const { parent } = await test_directory();
    await expect(open_journal(join(parent, "missing"), false)).rejects.toMatchObject({
      code: "journal_unavailable",
    });
    expect(await readdir(parent)).toEqual(["journal"]);
  });
});
