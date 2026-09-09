import type { ResearchProvenanceFingerprint } from "./generated/loop/v1/research_common_pb.js";

export const PROVENANCE_COMPONENTS = Object.freeze([
  "source_code",
  "operator_registry",
  "configuration",
  "data_manifest",
  "trading_calendar",
  "environment",
] as const);

export type ProvenanceComponent = (typeof PROVENANCE_COMPONENTS)[number];

export type ProvenanceErrorCode =
  | "invalid_digest"
  | "recording_mismatch"
  | "stale"
  | "unresolved_current";

/** Integrity/freshness failure, never deterministic factor rejection. */
export class ProvenanceError extends Error {
  readonly changed: readonly ProvenanceComponent[];

  constructor(
    readonly code: ProvenanceErrorCode,
    changed: readonly ProvenanceComponent[] = [],
  ) {
    super(`research provenance failed: ${code}: ${changed.join(",")}`);
    this.name = "ProvenanceError";
    this.changed = Object.freeze([...changed]);
  }
}

/** Validated, immutable copies; later DTO mutation cannot alter this snapshot. */
export class ProvenanceSnapshot {
  private readonly digests: readonly string[];

  private constructor(digests: string[]) {
    this.digests = Object.freeze(digests);
    Object.freeze(this);
  }

  static fromWire(value: ResearchProvenanceFingerprint): ProvenanceSnapshot {
    const fields = [
      value.sourceCodeSha256,
      value.operatorRegistrySha256,
      value.configurationSha256,
      value.dataManifestSha256,
      value.tradingCalendarSha256,
      value.environmentSha256,
    ];
    const digests = PROVENANCE_COMPONENTS.map((component, index) => {
      const bytes = fields[index]?.value;
      if (!(bytes instanceof Uint8Array) || bytes.byteLength !== 32) {
        throw new ProvenanceError("invalid_digest", [component]);
      }
      return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
    });
    return new ProvenanceSnapshot(digests);
  }

  /** Every changed field in stable protocol order. */
  differences(other: ProvenanceSnapshot): readonly ProvenanceComponent[] {
    return Object.freeze(
      PROVENANCE_COMPONENTS.filter((_, index) => this.digests[index] !== other.digests[index]),
    );
  }
}

/** Metadata freshness only, not a data capability or factor admission. */
export type ProvenanceAssessment =
  | { readonly status: "current"; readonly changed: readonly [] }
  | { readonly status: "stale"; readonly changed: readonly ProvenanceComponent[] }
  | { readonly status: "unresolved"; readonly changed: readonly [] };

/**
 * Verify original-run integrity before freshness; never rewrite historical evidence.
 * The owner must resolve factor, backtest, sample, seed, frozen inputs and current
 * context independently. Matching caller metadata is not authority or execution proof.
 */
export function assessProvenance(
  recorded: ProvenanceSnapshot,
  frozen: ProvenanceSnapshot,
  current: ProvenanceSnapshot | undefined,
): ProvenanceAssessment {
  const mismatch = recorded.differences(frozen);
  if (mismatch.length !== 0) throw new ProvenanceError("recording_mismatch", mismatch);
  if (current === undefined) {
    return Object.freeze({ status: "unresolved", changed: Object.freeze([] as const) });
  }
  const changed = recorded.differences(current);
  return changed.length === 0
    ? Object.freeze({ status: "current", changed: Object.freeze([] as const) })
    : Object.freeze({ status: "stale", changed });
}

/** Refuse stale/unresolved metrics at a current-result consumption boundary. */
export function requireCurrentProvenance(assessment: ProvenanceAssessment): void {
  if (assessment.status === "stale") throw new ProvenanceError("stale", assessment.changed);
  if (assessment.status !== "current") throw new ProvenanceError("unresolved_current");
}
