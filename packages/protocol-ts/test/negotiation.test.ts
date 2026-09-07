import { readFileSync } from "node:fs";
import { create } from "@bufbuild/protobuf";
import { TimestampSchema } from "@bufbuild/protobuf/wkt";
import { describe, expect, it } from "vitest";

import {
  type ProtocolInfo,
  ProtocolInfoSchema,
  type ProtocolLimits,
  ProtocolLimitsSchema,
  ProtocolSelectionSnapshotSchema,
  Sha256DigestSchema,
} from "../src/generated/loop/v1/common_pb.js";
import { protocolSelectionSha256 } from "../src/job.js";
import {
  negotiateProtocolAvailability,
  type ProtocolBuildIdentity,
  ProtocolNegotiationError,
  validateProtocolSelectionAvailability,
} from "../src/negotiation.js";

const REQUIRED_PACKAGE = "loop.research.v1";
const REQUIRED_FEATURES = ["jobs.envelope.v1"] as const;
const vectorText = readFileSync(
  new URL("../../../tests/contracts/protocol_negotiation_vectors.tsv", import.meta.url),
  "utf8",
);

interface Vector {
  readonly name: string;
  readonly operation: string;
  readonly expected: string;
  readonly mutation: string;
}

const vectors: readonly Vector[] = vectorText
  .trimEnd()
  .split("\n")
  .slice(1)
  .map((line) => {
    const [name, operation, expected, mutation] = line.split("\t");
    if (
      name === undefined ||
      operation === undefined ||
      expected === undefined ||
      mutation === undefined
    ) {
      throw new Error("malformed shared protocol negotiation vector");
    }
    return { name, operation, expected, mutation };
  });

describe("protocol availability negotiation", () => {
  it("fails closed for every shared negotiation and selection vector", () => {
    expect(vectors).toHaveLength(15);
    for (const vector of vectors) {
      const result =
        vector.operation === "negotiate"
          ? runNegotiation(vector.mutation)
          : runSelectionValidation(vector.mutation);
      expect(result, vector.name).toBe(vector.expected);
    }
  });
});

function runNegotiation(mutation: string): string {
  const local = protocolInfo("client.1", 0x11, true);
  const peer = protocolInfo("server.1", 0x22, false);
  switch (mutation) {
    case "none":
      break;
    case "local_remove_package":
      local.supportedPackages.splice(0, 1);
      break;
    case "peer_remove_package":
      peer.supportedPackages.splice(0, 1);
      break;
    case "local_remove_required_feature":
      local.features.splice(local.features.indexOf(REQUIRED_FEATURES[0]), 1);
      break;
    case "peer_remove_required_feature":
      peer.features.splice(peer.features.indexOf(REQUIRED_FEATURES[0]), 1);
      break;
    case "peer_unsorted_packages":
      peer.supportedPackages.reverse();
      break;
    default:
      throw new Error(`unknown negotiation mutation ${mutation}`);
  }
  try {
    const negotiated = negotiateProtocolAvailability(
      local,
      peer,
      REQUIRED_PACKAGE,
      REQUIRED_FEATURES,
    );
    expect(negotiated.enabledFeatures).toEqual(["factors.canonical-json.v1", "jobs.envelope.v1"]);
    expect(negotiated.effectiveLimits).toEqual(peer.limits);
    return "accept";
  } catch (error) {
    if (error instanceof ProtocolNegotiationError) return error.code;
    throw error;
  }
}

function runSelectionValidation(mutation: string): string {
  const local = protocolInfo("client.1", 0x11, true);
  const selection = protocolSelection();
  let recomputeDigest = true;
  switch (mutation) {
    case "none":
      break;
    case "selection_package":
      selection.selectedPackage = "loop.research.v2";
      break;
    case "selection_remove_required_feature":
      selection.enabledFeatures.splice(1, 1);
      break;
    case "selection_add_unsupported_feature":
      selection.enabledFeatures.push("streams.sequence.v1");
      break;
    case "selection_limit_exceeds_local":
      requireLimits(selection.effectiveLimits).maximumUnaryBytes = 3_145_728n;
      break;
    case "selection_server_build":
      selection.serverBuildVersion = "server.2";
      break;
    case "selection_client_build":
      selection.clientBuildSha256 = digest(0x44);
      break;
    case "selection_descriptor":
      selection.schemaDescriptorSha256 = digest(0x55);
      break;
    case "selection_digest": {
      const claimed = selection.selectionSha256?.value;
      if (claimed === undefined) throw new Error("fixture selection digest required");
      claimed[0] = (claimed[0] ?? 0) ^ 0xff;
      recomputeDigest = false;
      break;
    }
    default:
      throw new Error(`unknown selection mutation ${mutation}`);
  }
  if (recomputeDigest) selection.selectionSha256 = digestBytes(protocolSelectionSha256(selection));

  const retainedBuilds: readonly ProtocolBuildIdentity[] = [
    { buildVersion: "server.1", buildSha256: new Uint8Array(32).fill(0x22) },
  ];
  try {
    validateProtocolSelectionAvailability(
      selection,
      local,
      retainedBuilds,
      [new Uint8Array(32).fill(0x33)],
      REQUIRED_PACKAGE,
      REQUIRED_FEATURES,
    );
    return "accept";
  } catch (error) {
    if (error instanceof ProtocolNegotiationError) return error.code;
    throw error;
  }
}

function protocolInfo(buildVersion: string, buildByte: number, local: boolean): ProtocolInfo {
  return create(ProtocolInfoSchema, {
    supportedPackages: ["loop.research.v1", "loop.v1"],
    features: local
      ? ["artifacts.by-reference.v1", "factors.canonical-json.v1", "jobs.envelope.v1"]
      : ["factors.canonical-json.v1", "jobs.envelope.v1", "streams.sequence.v1"],
    limits: local
      ? limits(2_097_152n, 524_288n, 131_072n, 2_048, 32, 250, 128, 1_024)
      : limits(1_048_576n, 262_144n, 65_536n, 1_024, 16, 100, 64, 512),
    buildVersion,
    buildSha256: digest(buildByte),
  });
}

function protocolSelection() {
  const selection = create(ProtocolSelectionSnapshotSchema, {
    selectedPackage: REQUIRED_PACKAGE,
    enabledFeatures: ["factors.canonical-json.v1", "jobs.envelope.v1"],
    effectiveLimits: limits(1_048_576n, 262_144n, 65_536n, 1_024, 16, 100, 64, 512),
    serverBuildVersion: "server.1",
    serverBuildSha256: digest(0x22),
    schemaDescriptorSha256: digest(0x33),
    selectedAt: create(TimestampSchema, { seconds: 2n, nanos: 0 }),
    clientBuildVersion: "client.1",
    clientBuildSha256: digest(0x11),
  });
  selection.selectionSha256 = digestBytes(protocolSelectionSha256(selection));
  return selection;
}

function limits(
  unary: bigint,
  stream: bigint,
  ast: bigint,
  nodes: number,
  depth: number,
  page: number,
  identity: number,
  uri: number,
): ProtocolLimits {
  return create(ProtocolLimitsSchema, {
    maximumUnaryBytes: unary,
    maximumStreamEventBytes: stream,
    maximumCanonicalAstBytes: ast,
    maximumAstNodes: nodes,
    maximumAstDepth: depth,
    maximumPageRecords: page,
    maximumIdentityBytes: identity,
    maximumArtifactUriBytes: uri,
  });
}

function requireLimits(value: ProtocolLimits | undefined): ProtocolLimits {
  if (value === undefined) throw new Error("fixture limits required");
  return value;
}

function digest(byte: number) {
  return create(Sha256DigestSchema, { value: new Uint8Array(32).fill(byte) });
}

function digestBytes(value: Uint8Array) {
  return create(Sha256DigestSchema, { value });
}
