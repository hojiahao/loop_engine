import { readFileSync } from "node:fs";
import { create, fromBinary, toJson } from "@bufbuild/protobuf";
import { InvokeModelRequestSchema, JsonSchemaSchema } from "@loop-engine/protocol/provider";
import { expect, test } from "vitest";

import { digest_json, hex_digest } from "../src/identity.js";
import { compile_schema, json_bytes, json_digest, parse_json } from "../src/json.js";

test("Rust journal fingerprint matches the Provider protobuf JSON projection", () => {
  const fixture = JSON.parse(
    readFileSync(
      new URL("../../../fixtures/contracts/protocol/v1/harness_invocation.json", import.meta.url),
      "utf8",
    ),
  ) as { request_base64: string; request_sha256: string };
  const request = fromBinary(
    InvokeModelRequestSchema,
    Buffer.from(fixture.request_base64, "base64"),
  );
  expect(
    hex_digest(
      digest_json("loop.provider-invocation/v1", toJson(InvokeModelRequestSchema, request)),
    ),
  ).toBe(fixture.request_sha256);
});

const tool_fixtures = JSON.parse(
  readFileSync(
    new URL("../../../fixtures/contracts/protocol/v1/harness_tools.json", import.meta.url),
    "utf8",
  ),
) as { name: string; request_base64: string; request_sha256: string }[];

test.each(tool_fixtures)("Rust $name fingerprint matches typed Provider history", (fixture) => {
  const request = fromBinary(
    InvokeModelRequestSchema,
    Buffer.from(fixture.request_base64, "base64"),
  );
  expect(
    hex_digest(
      digest_json("loop.provider-invocation/v1", toJson(InvokeModelRequestSchema, request)),
    ),
  ).toBe(fixture.request_sha256);
});

test.each([
  ["research-describe", "loop.research-describe/v1"],
  ["research-description", "loop.research-description/v1"],
])("deployed %s schema compiles under Provider restrictions", (file, schema_id) => {
  const canonical_json = json_bytes(
    parse_json(readFileSync(new URL(`../../../config/schemas/${file}.v1.json`, import.meta.url))),
  );
  const schema = create(JsonSchemaSchema, {
    schemaId: schema_id,
    schemaVersion: 1,
    canonicalJson: canonical_json,
    schemaSha256: { value: json_digest(canonical_json) },
  });
  expect(() => compile_schema(schema)).not.toThrow();
});
