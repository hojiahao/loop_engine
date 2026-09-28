import { createHash } from "node:crypto";

/** Canonical bounded JSON for provider-owned pins and request fingerprints. */
export function canonical_json(value: unknown, depth = 0): string {
  if (depth > 32) throw new Error("invalid_provider_json");
  if (value === null || typeof value === "boolean") return JSON.stringify(value);
  if (typeof value === "string") {
    if (!value.isWellFormed()) throw new Error("invalid_provider_json");
    return JSON.stringify(value);
  }
  if (typeof value === "number" && Number.isSafeInteger(value)) return JSON.stringify(value);
  if (Array.isArray(value)) {
    return `[${value.map((item) => canonical_json(item, depth + 1)).join(",")}]`;
  }
  if (typeof value === "object" && value !== null) {
    return `{${Object.keys(value)
      .sort()
      .map(
        (key) =>
          `${canonical_json(key, depth + 1)}:${canonical_json(Reflect.get(value, key), depth + 1)}`,
      )
      .join(",")}}`;
  }
  throw new Error("invalid_provider_json");
}

export function digest_json(domain: string, value: unknown): Uint8Array {
  return createHash("sha256").update(domain).update("\0").update(canonical_json(value)).digest();
}

export function hex_digest(bytes: Uint8Array): string {
  return Buffer.from(bytes).toString("hex");
}
