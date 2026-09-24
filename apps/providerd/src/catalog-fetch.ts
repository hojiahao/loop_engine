import { createPublicKey, verify } from "node:crypto";
import { catalog_bytes } from "./catalog-store.js";
import {
  type CatalogDocument,
  type CatalogOptions,
  type CatalogSources,
  catalog_envelope,
} from "./catalog-types.js";
import { compatible_url } from "./compatible-config.js";
import { ProviderError } from "./errors.js";
import { parse_json } from "./json.js";
import { bounded_fetch } from "./native.js";

export function check_validity(
  value: { issued_at: string; expires_at: string },
  now: number,
): void {
  const issued = new Date(value.issued_at).getTime();
  const expiry = new Date(value.expires_at).getTime();
  if (
    !Number.isFinite(issued) ||
    !Number.isFinite(expiry) ||
    issued > now ||
    expiry <= now ||
    issued >= expiry
  )
    throw new ProviderError("provider_catalog_expired");
}

/** The key ID selects administrative trust; model input never selects a key. */
export function catalog_signing(value: CatalogDocument): Uint8Array {
  return Buffer.concat([Buffer.from("loop.model-catalog-signature/v1\0"), catalog_bytes(value)]);
}

export function verify_catalog(
  value: unknown,
  source: CatalogSources["remotes"][number],
  options: CatalogOptions,
  now: number,
): CatalogDocument {
  const envelope = catalog_envelope.parse(value);
  const trusted = options.trusted_keys.filter((key) => key.id === source.key_id);
  if (
    trusted.length !== 1 ||
    !trusted[0] ||
    envelope.key_id !== source.key_id ||
    envelope.payload.source_id !== source.source_id
  )
    throw new ProviderError("provider_catalog_untrusted");
  const key = createPublicKey(trusted[0].public_key);
  const signature = Buffer.from(envelope.signature, "base64");
  if (
    key.asymmetricKeyType !== "ed25519" ||
    signature.toString("base64") !== envelope.signature ||
    !verify(null, catalog_signing(envelope.payload), key, signature)
  )
    throw new ProviderError("provider_catalog_signature");
  check_validity(envelope.payload, now);
  return envelope.payload;
}

/** Signed metadata is fetched without provider credentials and with no redirects. */
export async function fetch_catalog(
  source: CatalogSources["remotes"][number],
  options: CatalogOptions,
  signal: AbortSignal,
  fetcher: typeof fetch = fetch,
): Promise<CatalogDocument> {
  if (!compatible_url(source.url) || new URL(source.url).protocol !== "https:")
    throw new ProviderError("provider_catalog_destination");
  const response = await bounded_fetch(fetcher)(source.url, {
    method: "GET",
    headers: { accept: "application/json" },
    redirect: "error",
    signal: AbortSignal.any([signal, AbortSignal.timeout(5000)]),
  });
  if (
    !response.ok ||
    response.headers.get("content-type")?.split(";")[0]?.trim() !== "application/json"
  )
    throw new ProviderError("provider_catalog_fetch_failed");
  return verify_catalog(parse_json(await response.text(), 524_288), source, options, Date.now());
}
