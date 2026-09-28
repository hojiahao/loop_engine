import { mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";

import { model_snapshot, plugin_digest, read_private, validate_deployment } from "../src/config.js";
import { decimal_units } from "../src/host.js";
import { canonical_json, hex_digest } from "../src/identity.js";
import { test_config } from "./fixture.js";

const directories: string[] = [];
afterEach(async () => {
  for (const path of directories.splice(0)) await rm(path, { recursive: true, force: true });
});

describe("provider deployment pins", () => {
  it("binds installed host bytes and locked dependencies", async () => {
    const digest = await plugin_digest();
    expect(digest).toHaveLength(32);
    expect(hex_digest(await plugin_digest())).toBe(hex_digest(digest));
  });

  it("changes model identity when policy-relevant catalog bytes change", () => {
    const config = test_config("/test-fixture");
    const model = config.models[0];
    if (!model) throw new Error("fixture_model_missing");
    const first = model_snapshot(config, model, new Uint8Array(32));
    model.input_usd = "2";
    const second = model_snapshot(config, model, new Uint8Array(32));
    expect(second.resolutionId).not.toEqual(first.resolutionId);
    expect(second.snapshotSha256).not.toEqual(first.snapshotSha256);
    expect(first.capabilities?.supportsStreaming).toBe(false);
    expect(first.capabilities?.supportsTools).toBe(false);
  });

  it("rejects duplicate credentials identities and unknown model ACL entries", () => {
    const config = test_config("/test-fixture");
    const principal = config.principals[0];
    if (!principal) throw new Error("fixture_principal_missing");
    config.principals.push(principal);
    expect(() => validate_deployment(config)).toThrow("invalid_provider_deployment");
    config.principals.pop();
    principal.model_ids.push("unregistered");
    expect(() => validate_deployment(config)).toThrow("invalid_provider_deployment");
  });

  it("refuses public or symlinked private configuration", async () => {
    const directory = await mkdtemp(join(tmpdir(), "loop-provider-config-"));
    directories.push(directory);
    const path = join(directory, "config.json");
    await writeFile(path, "{}", { mode: 0o644 });
    await expect(read_private(path)).rejects.toThrow("invalid_private_file");
    const link = join(directory, "link.json");
    await symlink(path, link);
    await expect(read_private(link)).rejects.toThrow();
  });

  it("uses exact integer money without binary rounding", () => {
    expect(decimal_units("0.000000001")).toBe(1n);
    expect(decimal_units("12.34")).toBe(12_340_000_000n);
    for (const value of ["NaN", "-1", "1e6", "0.00", "0.0000000001", "1000000"])
      expect(() => decimal_units(value)).toThrow();
  });

  it("canonicalizes key order and rejects lossy JSON identities", () => {
    expect(canonical_json({ z: [1, true], a: "text" })).toBe('{"a":"text","z":[1,true]}');
    for (const value of [NaN, Infinity, 1.2, undefined, "\ud800"])
      expect(() => canonical_json(value)).toThrow();
  });
});
