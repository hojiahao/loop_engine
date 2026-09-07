import { readFileSync } from "node:fs";
import { create, fromBinary, toBinary } from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";

import { GetProtocolInfoRequestSchema } from "../src/generated/loop/protocol/v1/service_pb.js";
import {
  ProtocolInfoSchema,
  type ProtocolLimits,
  ProtocolLimitsSchema,
  Sha256DigestSchema,
} from "../src/generated/loop/v1/common_pb.js";

const FIXTURE_DIRECTORY = new URL("../../../fixtures/contracts/protocol/v1/", import.meta.url);
const SUPPORTED_PACKAGES = [
  "loop.audit.v1",
  "loop.discovery.v1",
  "loop.holdout.v1",
  "loop.jobs.v1",
  "loop.protocol.v1",
  "loop.provider.v1",
  "loop.research.v1",
  "loop.v1",
];
const FEATURES = ["artifacts.by-reference.v1", "factors.canonical-json.v1"];
const PRODUCER_FIXTURES = [
  "protocol_info_v1.binpb",
  "protocol_info_v1.rust.binpb",
  "protocol_info_v1.typescript.binpb",
];
const EXPECTED_LIMITS: Omit<ProtocolLimits, "$typeName"> = {
  maximumUnaryBytes: 4_194_304n,
  maximumStreamEventBytes: 1_048_576n,
  maximumCanonicalAstBytes: 262_144n,
  maximumAstNodes: 4_096,
  maximumAstDepth: 64,
  maximumPageRecords: 500,
  maximumIdentityBytes: 128,
  maximumArtifactUriBytes: 2_048,
};

function fixture(name: string): Uint8Array {
  return readFileSync(new URL(name, FIXTURE_DIRECTORY));
}

function expectProjection(wire: Uint8Array): void {
  const decoded = fromBinary(ProtocolInfoSchema, wire);
  expect(decoded.supportedPackages).toEqual(SUPPORTED_PACKAGES);
  expect(decoded.features).toEqual(FEATURES);
  expect(decoded.limits).toMatchObject(EXPECTED_LIMITS);
  expect(decoded.buildVersion).toBe("0.2.0-alpha.1+wire-fixture.1");
  expect(Buffer.from(decoded.buildSha256?.value ?? []).toString("hex")).toBe(
    "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
  );

  const decodedAgain = fromBinary(ProtocolInfoSchema, toBinary(ProtocolInfoSchema, decoded));
  expect(decodedAgain.supportedPackages).toEqual(SUPPORTED_PACKAGES);
  expect(decodedAgain.features).toEqual(FEATURES);
  expect(decodedAgain.limits).toMatchObject(EXPECTED_LIMITS);
  expect(decodedAgain.buildVersion).toBe("0.2.0-alpha.1+wire-fixture.1");
  expect(Buffer.from(decodedAgain.buildSha256?.value ?? []).toString("hex")).toBe(
    "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
  );
}

describe("generated Protobuf bindings", () => {
  it("imports the core and protocol service descriptors", () => {
    expect(ProtocolInfoSchema.typeName).toBe("loop.v1.ProtocolInfo");
    expect(GetProtocolInfoRequestSchema.typeName).toBe("loop.protocol.v1.GetProtocolInfoRequest");
  });
});

describe("ProtocolInfo wire compatibility", () => {
  it("keeps the TypeScript fixture equal to current native encoder output", () => {
    const expected = create(ProtocolInfoSchema, {
      supportedPackages: SUPPORTED_PACKAGES,
      features: FEATURES,
      limits: create(ProtocolLimitsSchema, EXPECTED_LIMITS),
      buildVersion: "0.2.0-alpha.1+wire-fixture.1",
      buildSha256: create(Sha256DigestSchema, {
        value: Uint8Array.from({ length: 32 }, (_, index) => index),
      }),
    });
    expect(Buffer.from(fixture("protocol_info_v1.typescript.binpb"))).toEqual(
      Buffer.from(toBinary(ProtocolInfoSchema, expected)),
    );
  });

  it("decodes and re-encodes every producer fixture semantically", () => {
    for (const name of PRODUCER_FIXTURES) {
      expectProjection(fixture(name));
    }
  });

  it("tolerates an additive unknown field as an old reader", () => {
    expectProjection(fixture("protocol_info_v1_unknown_field.binpb"));
    // Unknown-field preservation and byte equality are deliberately not part
    // of this contract. A lossless forwarder retains the original envelope.
  });
});
