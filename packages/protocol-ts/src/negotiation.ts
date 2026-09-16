import { timingSafeEqual } from "node:crypto";
import { create } from "@bufbuild/protobuf";

import {
  type ProtocolInfo,
  type ProtocolLimits,
  ProtocolLimitsSchema,
  type ProtocolSelectionSnapshot,
} from "./generated/loop/v1/common_pb.js";
import { protocol_selection_sha256 } from "./job.js";

const MAX_PACKAGES = 16;
const MAX_FEATURES = 256;
const MAX_NAME_BYTES = 128;
const MAX_BUILD_VERSION_BYTES = 128;
const MAX_UNARY_BYTES = 4_194_304n;
const MAX_STREAM_EVENT_BYTES = 1_048_576n;
const MAX_CANONICAL_AST_BYTES = 262_144n;
const MAX_AST_NODES = 4_096;
const MAX_AST_DEPTH = 64;
const MAX_PAGE_RECORDS = 500;
const MAX_IDENTITY_BYTES = 128;
const MAX_ARTIFACT_URI_BYTES = 2_048;
const encoder = new TextEncoder();

export type ProtocolNegotiationCode =
  | "invalid_protocol_info"
  | "invalid_requirement"
  | "unsupported_major"
  | "unsupported_required_feature"
  | "unsupported_feature"
  | "incompatible_limits"
  | "invalid_selection"
  | "unavailable_build"
  | "unavailable_descriptor";

export class ProtocolNegotiationError extends Error {
  public override readonly name = "ProtocolNegotiationError";

  public constructor(public readonly code: ProtocolNegotiationCode) {
    super(code);
  }
}

export interface ProtocolBuildIdentity {
  readonly buildVersion: string;
  readonly buildSha256: Uint8Array;
}

export interface NegotiatedProtocol {
  readonly selectedPackage: string;
  readonly enabledFeatures: readonly string[];
  readonly effectiveLimits: ProtocolLimits;
  readonly localBuild: ProtocolBuildIdentity;
  readonly peerBuild: ProtocolBuildIdentity;
}

/** Validate the closed availability projection advertised by one deployed peer. */
export function validate_protocol_info(info: ProtocolInfo): void {
  if (
    info.supportedPackages.length < 1 ||
    info.supportedPackages.length > MAX_PACKAGES ||
    !is_sorted_unique(info.supportedPackages) ||
    info.supportedPackages.some((value) => !is_protocol_package(value)) ||
    info.features.length > MAX_FEATURES ||
    !is_sorted_unique(info.features) ||
    info.features.some((value) => !is_protocol_feature(value)) ||
    !is_valid_limits(info.limits) ||
    !is_build_version(info.buildVersion) ||
    info.buildSha256?.value.byteLength !== 32
  ) {
    throw new ProtocolNegotiationError("invalid_protocol_info");
  }
}

/** Negotiate one explicitly required service major before mutable or paid work. */
export function negotiate_protocol_availability(
  local: ProtocolInfo,
  peer: ProtocolInfo,
  requiredPackage: string,
  requiredFeatures: readonly string[],
): Readonly<NegotiatedProtocol> {
  validate_protocol_info(local);
  validate_protocol_info(peer);
  validate_requirement(requiredPackage, requiredFeatures);
  if (
    !local.supportedPackages.includes(requiredPackage) ||
    !peer.supportedPackages.includes(requiredPackage)
  ) {
    throw new ProtocolNegotiationError("unsupported_major");
  }
  const peerFeatures = new Set(peer.features);
  const enabledFeatures = local.features.filter((feature) => peerFeatures.has(feature));
  if (requiredFeatures.some((required) => !enabledFeatures.includes(required))) {
    throw new ProtocolNegotiationError("unsupported_required_feature");
  }

  return Object.freeze({
    selectedPackage: requiredPackage,
    enabledFeatures: Object.freeze(enabledFeatures),
    effectiveLimits: minimum_limits(
      require_limits(local.limits, "invalid_protocol_info"),
      require_limits(peer.limits, "invalid_protocol_info"),
    ),
    localBuild: protocol_info_build(local),
    peerBuild: protocol_info_build(peer),
  });
}

/** Prove a pinned selection remains executable from local immutable content. */
export function validate_selection_availability(
  selection: ProtocolSelectionSnapshot,
  local: ProtocolInfo,
  retainedBuilds: readonly ProtocolBuildIdentity[],
  availableSchemaDescriptors: readonly Uint8Array[],
  requiredPackage: string,
  requiredFeatures: readonly string[],
): void {
  validate_protocol_info(local);
  validate_requirement(requiredPackage, requiredFeatures);
  let computed: Uint8Array;
  try {
    computed = protocol_selection_sha256(selection);
  } catch {
    throw new ProtocolNegotiationError("invalid_selection");
  }
  if (
    selection.selectionSha256?.value.byteLength !== 32 ||
    !equal_bytes(selection.selectionSha256.value, computed)
  ) {
    throw new ProtocolNegotiationError("invalid_selection");
  }
  if (
    selection.selectedPackage !== requiredPackage ||
    !local.supportedPackages.includes(selection.selectedPackage)
  ) {
    throw new ProtocolNegotiationError("unsupported_major");
  }
  if (requiredFeatures.some((required) => !selection.enabledFeatures.includes(required))) {
    throw new ProtocolNegotiationError("unsupported_required_feature");
  }
  if (selection.enabledFeatures.some((feature) => !local.features.includes(feature))) {
    throw new ProtocolNegotiationError("unsupported_feature");
  }
  const selectedLimits = require_limits(selection.effectiveLimits, "invalid_selection");
  const localLimits = require_limits(local.limits, "invalid_protocol_info");
  if (!limits_fit(selectedLimits, localLimits)) {
    throw new ProtocolNegotiationError("incompatible_limits");
  }

  const serverBuild = selection_build(
    selection.serverBuildVersion,
    selection.serverBuildSha256?.value,
  );
  const clientBuild = selection_build(
    selection.clientBuildVersion,
    selection.clientBuildSha256?.value,
  );
  if (
    !build_is_available(serverBuild, local, retainedBuilds) ||
    !build_is_available(clientBuild, local, retainedBuilds)
  ) {
    throw new ProtocolNegotiationError("unavailable_build");
  }
  const descriptor = selection.schemaDescriptorSha256?.value;
  if (
    descriptor?.byteLength !== 32 ||
    !availableSchemaDescriptors.some((available) => equal_bytes(available, descriptor))
  ) {
    throw new ProtocolNegotiationError(
      descriptor?.byteLength === 32 ? "unavailable_descriptor" : "invalid_selection",
    );
  }
}

function validate_requirement(requiredPackage: string, requiredFeatures: readonly string[]): void {
  if (
    !is_protocol_package(requiredPackage) ||
    requiredFeatures.length > MAX_FEATURES ||
    !is_sorted_unique(requiredFeatures) ||
    requiredFeatures.some((feature) => !is_protocol_feature(feature))
  ) {
    throw new ProtocolNegotiationError("invalid_requirement");
  }
}

function protocol_info_build(info: ProtocolInfo): ProtocolBuildIdentity {
  const digest = info.buildSha256?.value;
  if (digest?.byteLength !== 32) throw new ProtocolNegotiationError("invalid_protocol_info");
  return Object.freeze({ buildVersion: info.buildVersion, buildSha256: digest.slice() });
}

function selection_build(version: string, digest: Uint8Array | undefined): ProtocolBuildIdentity {
  if (digest?.byteLength !== 32) throw new ProtocolNegotiationError("invalid_selection");
  return { buildVersion: version, buildSha256: digest };
}

function build_is_available(
  requested: ProtocolBuildIdentity,
  local: ProtocolInfo,
  retained: readonly ProtocolBuildIdentity[],
): boolean {
  const current = protocol_info_build(local);
  return (
    build_identity_equal(requested, current) ||
    retained.some((available) => build_identity_equal(requested, available))
  );
}

function build_identity_equal(left: ProtocolBuildIdentity, right: ProtocolBuildIdentity): boolean {
  return (
    left.buildVersion === right.buildVersion && equal_bytes(left.buildSha256, right.buildSha256)
  );
}

function minimum_limits(left: ProtocolLimits, right: ProtocolLimits): ProtocolLimits {
  return create(ProtocolLimitsSchema, {
    maximumUnaryBytes:
      left.maximumUnaryBytes < right.maximumUnaryBytes
        ? left.maximumUnaryBytes
        : right.maximumUnaryBytes,
    maximumStreamEventBytes:
      left.maximumStreamEventBytes < right.maximumStreamEventBytes
        ? left.maximumStreamEventBytes
        : right.maximumStreamEventBytes,
    maximumCanonicalAstBytes:
      left.maximumCanonicalAstBytes < right.maximumCanonicalAstBytes
        ? left.maximumCanonicalAstBytes
        : right.maximumCanonicalAstBytes,
    maximumAstNodes: Math.min(left.maximumAstNodes, right.maximumAstNodes),
    maximumAstDepth: Math.min(left.maximumAstDepth, right.maximumAstDepth),
    maximumPageRecords: Math.min(left.maximumPageRecords, right.maximumPageRecords),
    maximumIdentityBytes: Math.min(left.maximumIdentityBytes, right.maximumIdentityBytes),
    maximumArtifactUriBytes: Math.min(left.maximumArtifactUriBytes, right.maximumArtifactUriBytes),
  });
}

function limits_fit(selected: ProtocolLimits, local: ProtocolLimits): boolean {
  return (
    selected.maximumUnaryBytes <= local.maximumUnaryBytes &&
    selected.maximumStreamEventBytes <= local.maximumStreamEventBytes &&
    selected.maximumCanonicalAstBytes <= local.maximumCanonicalAstBytes &&
    selected.maximumAstNodes <= local.maximumAstNodes &&
    selected.maximumAstDepth <= local.maximumAstDepth &&
    selected.maximumPageRecords <= local.maximumPageRecords &&
    selected.maximumIdentityBytes <= local.maximumIdentityBytes &&
    selected.maximumArtifactUriBytes <= local.maximumArtifactUriBytes
  );
}

function is_valid_limits(limits: ProtocolLimits | undefined): boolean {
  return (
    limits !== undefined &&
    limits.maximumUnaryBytes >= 1n &&
    limits.maximumUnaryBytes <= MAX_UNARY_BYTES &&
    limits.maximumStreamEventBytes >= 1n &&
    limits.maximumStreamEventBytes <= MAX_STREAM_EVENT_BYTES &&
    limits.maximumCanonicalAstBytes >= 1n &&
    limits.maximumCanonicalAstBytes <= MAX_CANONICAL_AST_BYTES &&
    limits.maximumAstNodes >= 1 &&
    limits.maximumAstNodes <= MAX_AST_NODES &&
    limits.maximumAstDepth >= 1 &&
    limits.maximumAstDepth <= MAX_AST_DEPTH &&
    limits.maximumPageRecords >= 1 &&
    limits.maximumPageRecords <= MAX_PAGE_RECORDS &&
    limits.maximumIdentityBytes >= 1 &&
    limits.maximumIdentityBytes <= MAX_IDENTITY_BYTES &&
    limits.maximumArtifactUriBytes >= 1 &&
    limits.maximumArtifactUriBytes <= MAX_ARTIFACT_URI_BYTES &&
    limits.maximumCanonicalAstBytes <= limits.maximumUnaryBytes &&
    BigInt(limits.maximumIdentityBytes) <= limits.maximumUnaryBytes &&
    BigInt(limits.maximumArtifactUriBytes) <= limits.maximumUnaryBytes
  );
}

function require_limits(
  limits: ProtocolLimits | undefined,
  code: "invalid_protocol_info" | "invalid_selection",
): ProtocolLimits {
  if (limits === undefined) throw new ProtocolNegotiationError(code);
  return limits;
}

function is_sorted_unique(values: readonly string[]): boolean {
  return values.every((value, index) => index === 0 || (values[index - 1] as string) < value);
}

function is_protocol_package(value: string): boolean {
  return (
    encoder.encode(value).byteLength <= MAX_NAME_BYTES &&
    /^[a-z][a-z0-9_]*(?:\.[a-z][a-z0-9_]*)*\.v[1-9][0-9]*$/.test(value)
  );
}

function is_protocol_feature(value: string): boolean {
  return (
    encoder.encode(value).byteLength <= MAX_NAME_BYTES &&
    /^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)+$/.test(value)
  );
}

function is_build_version(value: string): boolean {
  return (
    encoder.encode(value).byteLength <= MAX_BUILD_VERSION_BYTES &&
    /^[A-Za-z0-9][A-Za-z0-9.+_-]*$/.test(value)
  );
}

function equal_bytes(left: Uint8Array, right: Uint8Array): boolean {
  return left.byteLength === right.byteLength && timingSafeEqual(left, right);
}
