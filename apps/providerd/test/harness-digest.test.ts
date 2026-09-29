import { readFileSync } from "node:fs";
import { fromBinary, toJson } from "@bufbuild/protobuf";
import { InvokeModelRequestSchema } from "@loop-engine/protocol/provider";
import { expect, test } from "vitest";

import { digest_json, hex_digest } from "../src/identity.js";

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
