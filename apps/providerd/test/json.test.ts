import { create } from "@bufbuild/protobuf";
import { JsonSchemaSchema } from "@loop-engine/protocol/provider";
import { describe, expect, it } from "vitest";
import {
  check_document,
  compile_schema,
  type JsonValue,
  json_bytes,
  json_digest,
  make_document,
  parse_json,
} from "../src/json.js";
import { TEST_SCHEMA } from "./rich-fixture.js";

describe("bounded JSON documents", () => {
  it("uses the RFC 8785 number and property ordering profile", () => {
    const value = parse_json('{"z":1e30,"a":[333333333.33333329,4.50,2e-3,1e-27]}');
    expect(Buffer.from(json_bytes(value)).toString()).toBe(
      '{"a":[333333333.3333333,4.5,0.002,1e-27],"z":1e+30}',
    );
  });
  it.each([
    '{"a":1,"a":2}',
    '{"a":1,"\\u0061":2}',
    '{"x":1,}',
    '{/*comment*/"x":1}',
    "1e999",
    '"\\ud800"',
    "\ufeff{}",
  ])("rejects ambiguous JSON %s", (text) => {
    expect(() => parse_json(text)).toThrow();
  });
  it("bounds depth before parsing and rejects malformed UTF-8", () => {
    expect(() => parse_json(`${"[".repeat(100_000)}0${"]".repeat(100_000)}`)).toThrow(
      "json_complexity_exceeded",
    );
    expect(() => parse_json(Uint8Array.of(0xff))).toThrow();
  });
  it("rejects a byte-order mark in encoded documents", () => {
    const bytes = new TextEncoder().encode('\ufeff{"window":20}');
    expect(() => parse_json(bytes)).toThrow("invalid_json_document");
    const schema = compile_schema(TEST_SCHEMA);
    const document = make_document('{"window":20}', schema);
    document.utf8Json = bytes;
    expect(() => check_document(document, schema)).toThrow("invalid_json_document");
  });
  it("binds document bytes to a registered schema and canonical digest", () => {
    const schema = compile_schema(TEST_SCHEMA);
    const document = make_document('{ "window": 20 }', schema);
    expect(check_document(document, schema)).toBe('{ "window": 20 }');
    if (document.canonicalSha256) document.canonicalSha256.value = new Uint8Array(32);
    expect(() => check_document(document, schema)).toThrow("document_digest_mismatch");
    expect(() => make_document('{"window":0}', schema)).toThrow("model_schema_mismatch");
  });
  it("rejects unsupported regular expressions and remote schema references", () => {
    const cases: JsonValue[] = [
      { type: "string", pattern: ".*" },
      { $ref: "https://invalid.example/schema" },
    ];
    for (const value of cases) {
      const bytes = json_bytes(value);
      expect(() =>
        compile_schema(
          create(JsonSchemaSchema, {
            schemaId: "unsafe",
            schemaVersion: 1,
            canonicalJson: bytes,
            schemaSha256: { value: json_digest(bytes) },
          }),
        ),
      ).toThrow("unsupported_json_schema");
    }
  });
});
