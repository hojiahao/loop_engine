import { readFileSync } from "node:fs";
import { create } from "@bufbuild/protobuf";
import { TimestampSchema } from "@bufbuild/protobuf/wkt";
import { describe, expect, it } from "vitest";

import {
  type ArtifactValidationCode,
  ArtifactValidationError,
  validateArtifactRef,
} from "../src/artifact.js";
import {
  ArtifactRefSchema,
  ArtifactSchemaReferenceSchema,
} from "../src/generated/loop/v1/artifact_pb.js";
import { ArtifactIdSchema, Sha256DigestSchema } from "../src/generated/loop/v1/common_pb.js";

interface Vector {
  readonly name: string;
  readonly expected: "accept" | ArtifactValidationCode;
  readonly uri: string;
  readonly digest: Uint8Array;
  readonly artifactId: string;
  readonly createdAt: string;
}

const vectorText = readFileSync(
  new URL("../../../tests/contracts/artifact_ref_vectors.tsv", import.meta.url),
  "utf8",
);

describe("ArtifactRef validation", () => {
  it("matches the shared fail-closed vectors", () => {
    for (const vector of vectors()) {
      const reference = create(ArtifactRefSchema, {
        artifactId: create(ArtifactIdSchema, { value: vector.artifactId }),
        uri: vector.uri,
        sha256: create(Sha256DigestSchema, { value: vector.digest }),
        schema: create(ArtifactSchemaReferenceSchema, {
          name: "table.factor_values",
          version: 1,
          schemaSha256: create(Sha256DigestSchema, { value: new Uint8Array(32).fill(2) }),
        }),
        mediaType: "application/vnd.apache.parquet",
        byteSize: 42n,
        rowCount: 1n,
        createdAt: vectorTimestamp(vector.createdAt),
      });

      if (vector.expected === "accept") {
        const createdAt = reference.createdAt;
        expect(createdAt, vector.name).toBeDefined();
        expect(validateArtifactRef(reference), vector.name).toMatchObject({
          artifactId: vector.artifactId,
          uri: vector.uri,
          createdAtSeconds: createdAt?.seconds,
          createdAtNanos: createdAt?.nanos,
        });
      } else {
        try {
          validateArtifactRef(reference);
          throw new Error(`expected ${vector.name} to fail`);
        } catch (error) {
          expect(error, vector.name).toBeInstanceOf(ArtifactValidationError);
          expect((error as ArtifactValidationError).code, vector.name).toBe(vector.expected);
        }
      }
    }
  });

  it("rejects an injected inline payload before domain construction", () => {
    const digest = new Uint8Array(32);
    const reference = create(ArtifactRefSchema, {
      artifactId: create(ArtifactIdSchema, { value: `sha256:${"00".repeat(32)}` }),
      uri: `artifact://sha256/${"00".repeat(32)}`,
      sha256: create(Sha256DigestSchema, { value: digest }),
      schema: create(ArtifactSchemaReferenceSchema, {
        name: "table.factor_values",
        version: 1,
        schemaSha256: create(Sha256DigestSchema, { value: new Uint8Array(32) }),
      }),
      mediaType: "application/vnd.apache.parquet",
      createdAt: create(TimestampSchema, { seconds: 1n }),
    });
    const injected = reference as typeof reference & { inlineBytes: Uint8Array };
    injected.inlineBytes = new Uint8Array([1]);

    expect(() => validateArtifactRef(injected)).toThrow(ArtifactValidationError);
  });
});

function vectors(): Vector[] {
  return vectorText
    .split("\n")
    .filter((line) => line !== "" && !line.startsWith("#"))
    .map((line) => {
      const [name, expected, rawUri, rawDigest, rawArtifactId, createdAt] = line.split("\t");
      if (
        name === undefined ||
        expected === undefined ||
        rawUri === undefined ||
        rawDigest === undefined ||
        rawArtifactId === undefined ||
        createdAt === undefined
      ) {
        throw new Error("invalid shared artifact vector");
      }
      const digestHex = token(rawDigest, "");
      const digest = decodeHex(digestHex);
      const uri = token(rawUri, digestHex.length === 64 ? digestHex : "");
      const artifactId = rawArtifactId === "@matching" ? `sha256:${digestHex}` : rawArtifactId;
      return { name, expected: expected as Vector["expected"], uri, digest, artifactId, createdAt };
    });
}

function vectorTimestamp(value: string) {
  switch (value) {
    case "valid":
      return create(TimestampSchema, { seconds: 1n });
    case "missing":
      return undefined;
    case "zero":
      return create(TimestampSchema);
    case "before_minimum":
      return create(TimestampSchema, { seconds: -62_135_596_801n });
    case "after_maximum":
      return create(TimestampSchema, { seconds: 253_402_300_800n });
    case "negative_nanos":
      return create(TimestampSchema, { seconds: 1n, nanos: -1 });
    case "nanos_overflow":
      return create(TimestampSchema, { seconds: 1n, nanos: 1_000_000_000 });
    default:
      throw new Error(`unknown created_at vector ${value}`);
  }
}

function token(value: string, digest: string): string {
  return value
    .replaceAll("@digest", digest)
    .replaceAll("@zero32", "00".repeat(32))
    .replaceAll("@zero31", "00".repeat(31))
    .replaceAll("@one32", "11".repeat(32))
    .replaceAll("@upperdigest", "AA".repeat(32))
    .replaceAll("@overlong", `artifact://sha256/${"0".repeat(2_100)}`);
}

function decodeHex(value: string): Uint8Array {
  if (value.length % 2 !== 0) {
    throw new Error("invalid fixture hex");
  }
  return Uint8Array.from(
    Array.from({ length: value.length / 2 }, (_, index) =>
      Number.parseInt(value.slice(index * 2, index * 2 + 2), 16),
    ),
  );
}
