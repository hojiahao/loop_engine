import { writeFileSync } from "node:fs";
import { create, toBinary } from "@bufbuild/protobuf";

import {
  ProtocolInfoSchema,
  ProtocolLimitsSchema,
  Sha256DigestSchema,
} from "../generated/loop/v1/common_pb.js";

const [outputPath, unexpected] = process.argv.slice(2);
if (outputPath === undefined || unexpected !== undefined) {
  process.stderr.write("usage: generate-wire-fixture <output-file>\n");
  process.exitCode = 2;
} else {
  const message = create(ProtocolInfoSchema, {
    supportedPackages: [
      "loop.audit.v1",
      "loop.discovery.v1",
      "loop.holdout.v1",
      "loop.jobs.v1",
      "loop.protocol.v1",
      "loop.provider.v1",
      "loop.research.v1",
      "loop.v1",
    ],
    features: ["artifacts.by-reference.v1", "factors.canonical-json.v1"],
    limits: create(ProtocolLimitsSchema, {
      maximumUnaryBytes: 4_194_304n,
      maximumStreamEventBytes: 1_048_576n,
      maximumCanonicalAstBytes: 262_144n,
      maximumAstNodes: 4_096,
      maximumAstDepth: 64,
      maximumPageRecords: 500,
      maximumIdentityBytes: 128,
      maximumArtifactUriBytes: 2_048,
    }),
    buildVersion: "0.2.0-alpha.1+wire-fixture.1",
    buildSha256: create(Sha256DigestSchema, {
      value: Uint8Array.from({ length: 32 }, (_, index) => index),
    }),
  });
  writeFileSync(outputPath, toBinary(ProtocolInfoSchema, message));
}
