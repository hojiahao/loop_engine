import { readFileSync } from "node:fs";
import { create } from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";

import { Sha256DigestSchema } from "../src/generated/loop/v1/common_pb.js";
import { ResearchProvenanceFingerprintSchema } from "../src/generated/loop/v1/research_common_pb.js";
import {
  assessProvenance,
  PROVENANCE_COMPONENTS,
  ProvenanceError,
  ProvenanceSnapshot,
  requireCurrentProvenance,
} from "../src/provenance.js";

const wireFields = [
  "sourceCodeSha256",
  "operatorRegistrySha256",
  "configurationSha256",
  "dataManifestSha256",
  "tradingCalendarSha256",
  "environmentSha256",
] as const;

function fingerprint(mask = "000000") {
  expect(mask).toMatch(/^[01]{6}$/);
  const value = create(ResearchProvenanceFingerprintSchema);
  for (const [index, field] of wireFields.entries()) {
    const bytes = new Uint8Array(32).fill(index + 1);
    if (mask[index] === "1") bytes[0] = (bytes[0] ?? 0) ^ 0xff;
    value[field] = create(Sha256DigestSchema, { value: bytes });
  }
  return value;
}

const vectors = readFileSync(
  new URL("../../../tests/contracts/provenance_vectors.tsv", import.meta.url),
  "ascii",
)
  .trim()
  .split("\n")
  .slice(1)
  .map((line) => line.split("\t"));

describe("research provenance", () => {
  it.each(vectors)(
    "shared vector %s",
    (_, recordedMask, frozenMask, currentMask, status, changed) => {
      const recorded = ProvenanceSnapshot.fromWire(fingerprint(recordedMask));
      const frozen = ProvenanceSnapshot.fromWire(fingerprint(frozenMask));
      const current =
        currentMask === "-" ? undefined : ProvenanceSnapshot.fromWire(fingerprint(currentMask));
      try {
        const assessment = assessProvenance(recorded, frozen, current);
        expect(assessment.status).toBe(status);
        expect(assessment.changed.join(",") || "-").toBe(changed);
      } catch (error) {
        if (!(error instanceof ProvenanceError)) throw error;
        expect(error.code).toBe(status);
        expect(error.changed.join(",") || "-").toBe(changed);
      }
    },
  );

  for (const [index, component] of PROVENANCE_COMPONENTS.entries()) {
    const field = wireFields[index];
    if (field === undefined) throw new Error("fixture component has no wire field");
    it.each([undefined, 0, 31, 33, 1024])(`${field} rejects size %s`, (size) => {
      const value = fingerprint();
      value[field] =
        size === undefined
          ? undefined
          : create(Sha256DigestSchema, { value: new Uint8Array(size) });
      expect(() => ProvenanceSnapshot.fromWire(value)).toThrowError(
        new ProvenanceError("invalid_digest", [component]),
      );
    });
  }

  it("detaches immutable snapshots from wire bytes", () => {
    const wire = fingerprint();
    const snapshot = ProvenanceSnapshot.fromWire(wire);
    wire.sourceCodeSha256?.value.fill(255);
    expect(snapshot.differences(ProvenanceSnapshot.fromWire(fingerprint()))).toEqual([]);
    expect(snapshot.differences(ProvenanceSnapshot.fromWire(wire))).toEqual(["source_code"]);
    expect(Object.isFrozen(snapshot)).toBe(true);
  });

  it.each([
    ["000000", "current"],
    ["000010", "stale"],
    ["-", "unresolved_current"],
  ])("only current metrics pass the gate: %s", (mask, status) => {
    const frozen = ProvenanceSnapshot.fromWire(fingerprint());
    const current = mask === "-" ? undefined : ProvenanceSnapshot.fromWire(fingerprint(mask));
    const assessment = assessProvenance(frozen, frozen, current);
    expect(Object.isFrozen(assessment)).toBe(true);
    expect(Object.isFrozen(assessment.changed)).toBe(true);
    if (status === "current") requireCurrentProvenance(assessment);
    else
      expect(() => requireCurrentProvenance(assessment)).toThrowError(
        new ProvenanceError(
          status === "stale" ? "stale" : "unresolved_current",
          assessment.changed,
        ),
      );
  });
});
