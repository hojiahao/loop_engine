import { createHash } from "node:crypto";
import { create } from "@bufbuild/protobuf";
import {
  type JsonDocument,
  JsonDocumentSchema,
  type JsonSchema,
} from "@loop-engine/protocol/provider";
import { Ajv, type ValidateFunction } from "ajv";
import canonicalize from "canonicalize";
import { createScanner, type Node, type ParseError, parseTree } from "jsonc-parser";

import { ProviderError } from "./errors.js";

export type JsonValue =
  | null
  | boolean
  | number
  | string
  | JsonValue[]
  | { [key: string]: JsonValue };
export interface RegisteredSchema {
  readonly reference: JsonSchema;
  readonly value: Record<string, JsonValue>;
  readonly validate: ValidateFunction;
}

/** Strict JSON with bounded depth, duplicate-key rejection and valid Unicode. */
export function parse_json(bytes: Uint8Array | string, maximum = 262_144, nesting = 32): JsonValue {
  const text =
    typeof bytes === "string"
      ? bytes
      : new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(bytes);
  if (!text.isWellFormed() || Buffer.byteLength(text) > maximum || text.charCodeAt(0) === 0xfeff)
    throw new ProviderError("invalid_json_document");
  // Check nesting before the recursive syntax-tree parser allocates its stack.
  const scanner = createScanner(text, true);
  let depth = 0;
  for (;;) {
    scanner.scan();
    if (!scanner.getTokenLength()) break;
    const token = text.slice(
      scanner.getTokenOffset(),
      scanner.getTokenOffset() + scanner.getTokenLength(),
    );
    if (token === "{" || token === "[") {
      if (++depth > nesting) throw new ProviderError("json_complexity_exceeded");
    } else if (token === "}" || token === "]") depth--;
  }
  const errors: ParseError[] = [];
  const root = parseTree(text, errors, {
    disallowComments: true,
    allowTrailingComma: false,
    allowEmptyContent: false,
  });
  if (!root || errors.length) throw new ProviderError("invalid_json_document");
  let nodes = 0;
  function read_node(node: Node, level: number): JsonValue {
    if (++nodes > 16_384 || level > nesting) throw new ProviderError("json_complexity_exceeded");
    if (node.type === "object") {
      const value: Record<string, JsonValue> = Object.create(null);
      for (const child of node.children ?? []) {
        const key = child.children?.[0]?.value as unknown;
        const item = child.children?.[1];
        if (typeof key !== "string" || !key.isWellFormed() || Object.hasOwn(value, key) || !item)
          throw new ProviderError("invalid_json_document");
        value[key] = read_node(item, level + 1);
      }
      return value;
    }
    if (node.type === "array")
      return (node.children ?? []).map((child) => read_node(child, level + 1));
    const value: unknown = node.value;
    if (
      value === null ||
      typeof value === "boolean" ||
      (typeof value === "number" && Number.isFinite(value)) ||
      (typeof value === "string" && value.isWellFormed())
    )
      return value;
    throw new ProviderError("invalid_json_document");
  }
  return read_node(root, 0);
}

export function json_bytes(value: JsonValue): Uint8Array {
  const text = canonicalize(value);
  if (typeof text !== "string") throw new ProviderError("invalid_json_document");
  return new TextEncoder().encode(text);
}

export function json_digest(bytes: Uint8Array): Uint8Array {
  return createHash("sha256").update(bytes).digest();
}

/** Only deployment-pinned schema bytes reach this compiler. No remote loading. */
export function compile_schema(reference: JsonSchema): RegisteredSchema {
  const value = parse_json(reference.canonicalJson);
  if (
    value === null ||
    typeof value !== "object" ||
    Array.isArray(value) ||
    reference.canonicalJson.length > 65_536 ||
    !Buffer.from(json_bytes(value)).equals(reference.canonicalJson) ||
    !Buffer.from(json_digest(reference.canonicalJson)).equals(
      reference.schemaSha256?.value ?? new Uint8Array(),
    )
  )
    throw new ProviderError("invalid_registered_schema");
  // Regexes, remote references and asynchronous extensions are not part of this profile.
  function check_schema(node: JsonValue): void {
    if (node === null || typeof node !== "object") return;
    if (Array.isArray(node)) {
      for (const child of node) check_schema(child);
      return;
    }
    for (const [key, item] of Object.entries(node)) {
      if (
        ["pattern", "patternProperties", "format", "$async", "$data"].includes(key) ||
        (key === "$ref" && (typeof item !== "string" || !item.startsWith("#")))
      )
        throw new ProviderError("unsupported_json_schema");
      if (
        ["properties", "$defs", "definitions"].includes(key) &&
        item &&
        typeof item === "object" &&
        !Array.isArray(item)
      ) {
        for (const child of Object.values(item)) check_schema(child);
      } else if (
        [
          "items",
          "additionalProperties",
          "anyOf",
          "oneOf",
          "allOf",
          "not",
          "if",
          "then",
          "else",
        ].includes(key)
      )
        check_schema(item);
    }
  }
  check_schema(value);
  try {
    const ajv = new Ajv({
      strict: true,
      strictRequired: true,
      allErrors: false,
      ownProperties: true,
      allowUnionTypes: true,
      coerceTypes: false,
      useDefaults: false,
      removeAdditional: false,
    });
    return { reference, value, validate: ajv.compile(value) };
  } catch {
    throw new ProviderError("invalid_registered_schema");
  }
}

export function make_document(text: string, schema: RegisteredSchema): JsonDocument {
  const value = parse_json(text);
  if (!schema.validate(value)) throw new ProviderError("model_schema_mismatch");
  return create(JsonDocumentSchema, {
    utf8Json: new TextEncoder().encode(text),
    canonicalSha256: { value: json_digest(json_bytes(value)) },
    schemaId: schema.reference.schemaId,
    schemaSha256: schema.reference.schemaSha256,
  });
}

export function check_document(
  document: JsonDocument | undefined,
  schema: RegisteredSchema,
): string {
  if (
    !document ||
    document.schemaId !== schema.reference.schemaId ||
    !Buffer.from(document.schemaSha256?.value ?? []).equals(
      schema.reference.schemaSha256?.value ?? new Uint8Array(),
    )
  )
    throw new ProviderError("document_schema_denied");
  const text = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(document.utf8Json);
  const checked = make_document(text, schema);
  if (
    !Buffer.from(checked.canonicalSha256?.value ?? []).equals(
      document.canonicalSha256?.value ?? new Uint8Array(),
    )
  )
    throw new ProviderError("document_digest_mismatch");
  return text;
}
