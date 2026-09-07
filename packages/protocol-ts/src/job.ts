import { createHash, timingSafeEqual } from "node:crypto";
import type { Timestamp } from "@bufbuild/protobuf/wkt";

import { validateArtifactRef } from "./artifact.js";
import type { BacktestSpec } from "./generated/loop/v1/backtest_pb.js";
import {
  ActorKind,
  ErrorCategory,
  type PolicyReference,
  type ProtocolLimits,
  type ProtocolSelectionSnapshot,
  type Sha256Digest,
} from "./generated/loop/v1/common_pb.js";
import { SampleRole } from "./generated/loop/v1/data_pb.js";
import type { DevelopmentDatasetReference } from "./generated/loop/v1/development_data_pb.js";
import {
  type FactorAst,
  type FactorAstNode,
  FactorDirection,
  type FactorSpec,
} from "./generated/loop/v1/factor_pb.js";
import {
  FactorRejectionCode,
  type HoldoutBacktestJobInput,
  type JobBudget,
  JobKind,
  type JobRecord,
  type JobSpecification,
  JobState,
} from "./generated/loop/v1/job_pb.js";
import { ModelProtocolFamily, type ModelResolutionSnapshot } from "./generated/loop/v1/model_pb.js";
import {
  type ResearchProvenanceFingerprint,
  ReturnDefinition,
} from "./generated/loop/v1/research_common_pb.js";
import {
  type CanonicalHoldoutEvaluationPlan,
  type HoldoutJobBudget,
  parseCanonicalHoldoutEvaluationPlan,
  verifyHoldoutPeriodIdentity,
} from "./holdout-identity.js";

const MAX_ID_BYTES = 128;
const MAX_REASON_BYTES = 2_048;
const MAX_ERROR_CODE_BYTES = 128;
const MAX_ERROR_MESSAGE_BYTES = 2_048;
const MAX_ERROR_FIELD_PATH_BYTES = 512;
const MAX_ERROR_DETAILS = 32;
const MAX_OUTCOME_ARTIFACTS = 64;
const MAX_DATASET_SNAPSHOTS = 128;
const MAX_PROTOCOL_FEATURES = 256;
const MAX_PROTOCOL_NAME_BYTES = 128;
const MAX_BUILD_VERSION_BYTES = 128;
const MAX_ACTOR_DISPLAY_NAME_BYTES = 256;
const MAX_AUTHENTICATED_SUBJECT_BYTES = 512;
const MAX_JOB_STEPS = 1_000_000;
const MAX_JOB_TOKENS = 1_000_000_000_000n;
const MAX_JOB_WALL_TIME_SECONDS = 604_800n;
const MAX_HOLDOUT_ENTRIES = 4_096;
const MAX_CANDIDATES = 1_000_000;
const MAX_PROTOCOL_UNARY_BYTES = 4_194_304n;
const MAX_PROTOCOL_STREAM_EVENT_BYTES = 1_048_576n;
const MAX_PROTOCOL_CANONICAL_AST_BYTES = 262_144n;
const MAX_PROTOCOL_AST_NODES = 4_096;
const MAX_PROTOCOL_AST_DEPTH = 64;
const MAX_PROTOCOL_AST_DIRECT_ARGUMENTS = 1_024;
const MAX_PROTOCOL_PAGE_RECORDS = 500;
const MAX_PROTOCOL_IDENTITY_BYTES = 128;
const MAX_PROTOCOL_ARTIFACT_URI_BYTES = 2_048;
const MIN_TIMESTAMP_SECONDS = -62_135_596_800n;
const MAX_TIMESTAMP_SECONDS = 253_402_300_799n;
const encoder = new TextEncoder();
const PROTOCOL_SELECTION_DOMAIN = encoder.encode("loop.protocol-selection/v1");
const FACTOR_AST_DOMAIN = encoder.encode("loop.factor-ast/v1");
const FACTOR_SPEC_DOMAIN = encoder.encode("loop.factor-spec/v1");

export type JobValidationCode =
  | "missing_field"
  | "unknown_enum"
  | "kind_input_mismatch"
  | "state_lease_mismatch"
  | "state_outcome_mismatch"
  | "rejection_not_allowed"
  | "lease_job_mismatch"
  | "invalid_attempt"
  | "invalid_revision"
  | "invalid_identity"
  | "factor_identity_mismatch"
  | "invalid_lease"
  | "invalid_terminal_payload"
  | "collection_limit"
  | "invalid_envelope"
  | "invalid_budget"
  | "invalid_protocol_selection"
  | "invalid_input"
  | "invalid_provenance"
  | "binding_mismatch";

export class JobValidationError extends Error {
  public override readonly name = "JobValidationError";

  public constructor(
    public readonly code: JobValidationCode,
    public readonly field: string,
  ) {
    super(`${field} failed job validation (${code})`);
  }
}

export interface ValidatedJobSpecificationShape {
  readonly kind: JobKind;
}

export interface ValidatedJobShape extends ValidatedJobSpecificationShape {
  readonly state: JobState;
}

/** Classify only the protocol v1 kind/input matrix. */
export function validateJobSpecificationShape(
  specification: JobSpecification,
): Readonly<ValidatedJobSpecificationShape> {
  const kind = validateKind(specification.kind);
  const inputCase = specification.input.case;
  if (inputCase === undefined) fail("missing_field", "specification.input");
  const expectedInput: Record<JobKind, string | undefined> = {
    [JobKind.UNSPECIFIED]: undefined,
    [JobKind.DISCOVERY]: "discovery",
    [JobKind.FACTOR_EVALUATION]: "factorEvaluation",
    [JobKind.BACKTEST]: "backtest",
    [JobKind.INDEPENDENT_RECONCILIATION]: "reconciliation",
    [JobKind.REPORT]: "artifact",
    [JobKind.PROSPECTIVE_OBSERVATION]: "artifact",
    [JobKind.HOLDOUT_BACKTEST]: "holdoutBacktest",
  };
  if (expectedInput[kind] !== inputCase) fail("kind_input_mismatch", "specification.input");
  return Object.freeze({ kind });
}

/**
 * Validate a v1 wire envelope and every binding provable from its inline fields.
 * The wire boundary proves an attached AST and canonical JSON encode one exact
 * tree. Registry resolution, AST typing, and normalization proof remain the
 * factor domain binder's responsibility. Development dataset references are
 * opaque identities here; Phase 4/5 server-owned snapshot registry and
 * capability resolution must verify their roles before persistence or dispatch.
 */
export function validateJobSpecification(
  specification: JobSpecification,
): Readonly<ValidatedJobSpecificationShape> {
  const shape = validateJobSpecificationShape(specification);
  const submittedAt = validateJobEnvelope(specification);
  validateJobInput(specification, submittedAt);
  return shape;
}

function validateJobEnvelope(specification: JobSpecification): Timestamp {
  requireTokenId(specification.jobId?.value, "specification.job_id");
  requireTokenId(specification.runId?.value, "specification.run_id");
  const submittedAt = requireTimestamp(specification.submittedAt, "specification.submitted_at");
  validateActor(specification.submittedBy, "specification.submitted_by");
  requireTokenId(specification.idempotencyKey?.value, "specification.idempotency_key");
  requireTokenId(specification.correlationId?.value, "specification.correlation_id");
  requireTokenId(specification.causationId?.value, "specification.causation_id");
  const selection = specification.protocolSelection;
  if (selection === undefined) fail("missing_field", "specification.protocol_selection");
  validateProtocolSelection(selection, submittedAt);
  return submittedAt;
}

function validateProtocolSelection(
  selection: ProtocolSelectionSnapshot,
  submittedAt: Timestamp,
): void {
  const expected = protocolSelectionSha256(selection);
  const actual = requireDigest(
    selection.selectionSha256,
    "specification.protocol_selection.selection_sha256",
  );
  if (!equalDigest(expected, actual)) {
    fail("invalid_protocol_selection", "specification.protocol_selection.selection_sha256");
  }
  const selectedAt = requireTimestamp(
    selection.selectedAt,
    "specification.protocol_selection.selected_at",
  );
  if (compareTimestamp(selectedAt, submittedAt) > 0) {
    fail("invalid_protocol_selection", "specification.protocol_selection.selected_at");
  }
}

/** Emit the normative canonical v1 protocol-selection document. */
export function canonicalProtocolSelectionBytes(selection: ProtocolSelectionSnapshot): Uint8Array {
  if (!isProtocolPackage(selection.selectedPackage)) {
    fail("invalid_protocol_selection", "specification.protocol_selection.selected_package");
  }
  if (
    selection.enabledFeatures.length > MAX_PROTOCOL_FEATURES ||
    selection.enabledFeatures.some((feature) => !isProtocolFeature(feature)) ||
    selection.enabledFeatures.some(
      (feature, index) => index > 0 && (selection.enabledFeatures[index - 1] as string) >= feature,
    )
  ) {
    fail("invalid_protocol_selection", "specification.protocol_selection.enabled_features");
  }
  const limits = selection.effectiveLimits;
  if (limits === undefined) {
    fail("missing_field", "specification.protocol_selection.effective_limits");
  }
  validateProtocolLimits(limits);
  if (
    !isBuildVersion(selection.serverBuildVersion) ||
    !isBuildVersion(selection.clientBuildVersion)
  ) {
    fail("invalid_protocol_selection", "specification.protocol_selection.build_version");
  }
  const serverBuild = requireDigest(
    selection.serverBuildSha256,
    "specification.protocol_selection.server_build_sha256",
  );
  const descriptor = requireDigest(
    selection.schemaDescriptorSha256,
    "specification.protocol_selection.schema_descriptor_sha256",
  );
  const clientBuild = requireDigest(
    selection.clientBuildSha256,
    "specification.protocol_selection.client_build_sha256",
  );
  const selectedAt = requireTimestamp(
    selection.selectedAt,
    "specification.protocol_selection.selected_at",
  );
  const features = selection.enabledFeatures.map((feature) => `"${feature}"`).join(",");
  const canonical =
    `{"schema":"loop.protocol-selection/v1","selected_package":"${selection.selectedPackage}",` +
    `"enabled_features":[${features}],"effective_limits":{` +
    `"maximum_unary_bytes":"${limits.maximumUnaryBytes}",` +
    `"maximum_stream_event_bytes":"${limits.maximumStreamEventBytes}",` +
    `"maximum_canonical_ast_bytes":"${limits.maximumCanonicalAstBytes}",` +
    `"maximum_ast_nodes":"${limits.maximumAstNodes}",` +
    `"maximum_ast_depth":"${limits.maximumAstDepth}",` +
    `"maximum_page_records":"${limits.maximumPageRecords}",` +
    `"maximum_identity_bytes":"${limits.maximumIdentityBytes}",` +
    `"maximum_artifact_uri_bytes":"${limits.maximumArtifactUriBytes}"},` +
    `"server_build_version":"${selection.serverBuildVersion}",` +
    `"server_build_sha256":"${encodeDigest(serverBuild)}",` +
    `"schema_descriptor_sha256":"${encodeDigest(descriptor)}",` +
    `"selected_at":{"seconds":"${selectedAt.seconds}","nanos":"${selectedAt.nanos}"},` +
    `"client_build_version":"${selection.clientBuildVersion}",` +
    `"client_build_sha256":"${encodeDigest(clientBuild)}"}`;
  return encoder.encode(canonical);
}

/** Compute the domain-separated digest claimed by `selection_sha256`. */
export function protocolSelectionSha256(selection: ProtocolSelectionSnapshot): Uint8Array {
  return domainDigest(PROTOCOL_SELECTION_DOMAIN, canonicalProtocolSelectionBytes(selection));
}

function validateProtocolLimits(limits: ProtocolLimits): void {
  const valid =
    limits.maximumUnaryBytes >= 1n &&
    limits.maximumUnaryBytes <= MAX_PROTOCOL_UNARY_BYTES &&
    limits.maximumStreamEventBytes >= 1n &&
    limits.maximumStreamEventBytes <= MAX_PROTOCOL_STREAM_EVENT_BYTES &&
    limits.maximumCanonicalAstBytes >= 1n &&
    limits.maximumCanonicalAstBytes <= MAX_PROTOCOL_CANONICAL_AST_BYTES &&
    limits.maximumAstNodes >= 1 &&
    limits.maximumAstNodes <= MAX_PROTOCOL_AST_NODES &&
    limits.maximumAstDepth >= 1 &&
    limits.maximumAstDepth <= MAX_PROTOCOL_AST_DEPTH &&
    limits.maximumPageRecords >= 1 &&
    limits.maximumPageRecords <= MAX_PROTOCOL_PAGE_RECORDS &&
    limits.maximumIdentityBytes >= 1 &&
    limits.maximumIdentityBytes <= MAX_PROTOCOL_IDENTITY_BYTES &&
    limits.maximumArtifactUriBytes >= 1 &&
    limits.maximumArtifactUriBytes <= MAX_PROTOCOL_ARTIFACT_URI_BYTES &&
    limits.maximumCanonicalAstBytes <= limits.maximumUnaryBytes &&
    BigInt(limits.maximumIdentityBytes) <= limits.maximumUnaryBytes &&
    BigInt(limits.maximumArtifactUriBytes) <= limits.maximumUnaryBytes;
  if (!valid) {
    fail("invalid_protocol_selection", "specification.protocol_selection.effective_limits");
  }
}

function validateJobInput(specification: JobSpecification, submittedAt: Timestamp): void {
  const input = specification.input;
  switch (input.case) {
    case "discovery":
      validateDevelopmentDataset(input.value.dataset);
      validatePolicy(input.value.researchPolicy, "specification.input.discovery.research_policy");
      validateModelResolution(
        input.value.makerModel,
        "specification.input.discovery.maker_model",
        submittedAt,
      );
      validateModelResolution(
        input.value.checkerModel,
        "specification.input.discovery.checker_model",
        submittedAt,
      );
      validateBudget(input.value.budget, "specification.input.discovery.budget");
      if (input.value.maximumCandidates < 1 || input.value.maximumCandidates > MAX_CANDIDATES) {
        fail("invalid_input", "specification.input.discovery.maximum_candidates");
      }
      return;
    case "factorEvaluation":
      if (input.value.factor === undefined) {
        fail("missing_field", "specification.input.factor_evaluation.factor");
      }
      validateFactorSpecIdentityEnvelope(input.value.factor);
      validateDevelopmentDataset(input.value.dataset);
      validateBudget(input.value.budget, "specification.input.factor_evaluation.budget");
      return;
    case "backtest":
      validateBudget(input.value.budget, "specification.input.backtest.budget");
      requireSha256Id(
        input.value.factorSpecId?.value,
        "specification.input.backtest.factor_spec_id",
      );
      validateDevelopmentDataset(input.value.dataset);
      validateSimpleReturn(
        input.value.returnDefinition,
        "specification.input.backtest.return_definition",
      );
      validateProvenance(input.value.provenance, "specification.input.backtest.provenance");
      requireDigest(
        input.value.deterministicSeed,
        "specification.input.backtest.deterministic_seed",
      );
      return;
    case "reconciliation": {
      const primary = requireTokenId(
        input.value.primaryBacktestId?.value,
        "specification.input.reconciliation.primary_backtest_id",
      );
      const independent = requireTokenId(
        input.value.independentBacktestId?.value,
        "specification.input.reconciliation.independent_backtest_id",
      );
      if (primary === independent) {
        fail("binding_mismatch", "specification.input.reconciliation.backtest_ids");
      }
      validatePolicy(
        input.value.reconciliationPolicy,
        "specification.input.reconciliation.reconciliation_policy",
      );
      validateBudget(input.value.budget, "specification.input.reconciliation.budget");
      return;
    }
    case "holdoutBacktest":
      validateHoldoutBacktestInput(input.value, submittedAt);
      return;
    case "artifact":
      if (input.value.input === undefined) {
        fail("missing_field", "specification.input.artifact.input");
      }
      try {
        validateArtifactRef(input.value.input);
      } catch {
        fail("invalid_input", "specification.input.artifact.input");
      }
      validatePolicy(input.value.policy, "specification.input.artifact.policy");
      validateBudget(input.value.budget, "specification.input.artifact.budget");
      return;
    case undefined:
      fail("missing_field", "specification.input");
  }
}

function validateDevelopmentDataset(dataset: DevelopmentDatasetReference | undefined): void {
  if (dataset === undefined) fail("missing_field", "specification.input.dataset");
  if (dataset.snapshotIds.length < 1 || dataset.snapshotIds.length > MAX_DATASET_SNAPSHOTS) {
    fail("invalid_input", "specification.input.dataset.snapshot_ids");
  }
  let previous: string | undefined;
  for (const snapshot of dataset.snapshotIds) {
    const value = requireTokenId(snapshot.value, "specification.input.dataset.snapshot_ids");
    if (previous !== undefined && previous >= value) {
      fail("invalid_input", "specification.input.dataset.snapshot_ids");
    }
    previous = value;
  }
  requireDigest(dataset.manifestSha256, "specification.input.dataset.manifest_sha256");
}

function validatePolicy(policy: PolicyReference | undefined, field: string): PolicyReference {
  if (policy === undefined) fail("missing_field", field);
  const policyId = policy.policyId?.value;
  if (policyId === undefined) fail("missing_field", field);
  if (!/^[a-z][a-z0-9_.-]{0,127}$/.test(policyId)) fail("invalid_input", field);
  if (!/^[1-9][0-9]{0,19}$/.test(policy.revision)) fail("invalid_input", field);
  try {
    if (BigInt(policy.revision) > 18_446_744_073_709_551_615n) fail("invalid_input", field);
  } catch {
    fail("invalid_input", field);
  }
  requireDigest(policy.sha256, field);
  return policy;
}

/**
 * Emit the canonical FactorSpec identity document after input-local identity
 * validation. This proves the attached wire AST is the exact tree encoded by
 * canonical JSON; registry semantics remain mandatory at the domain binder.
 */
export function canonicalFactorSpecIdentityBytes(factor: FactorSpec): Uint8Array {
  const factorExpressionId = requireSha256Id(
    factor.expressionId?.value,
    "specification.input.factor_evaluation.factor.expression_id",
  );
  const expression = factor.expression;
  if (expression === undefined) {
    fail("missing_field", "specification.input.factor_evaluation.factor.expression");
  }
  const attachedExpressionId = requireSha256Id(
    expression.expressionId?.value,
    "specification.input.factor_evaluation.factor.expression.expression_id",
  );
  if (!equalTextConstantTime(factorExpressionId, attachedExpressionId)) {
    fail(
      "binding_mismatch",
      "specification.input.factor_evaluation.factor.expression.expression_id",
    );
  }
  if (
    expression.canonicalizationProfile !== "loop.factor-ast/v1" ||
    expression.canonicalJson.byteLength === 0 ||
    BigInt(expression.canonicalJson.byteLength) > MAX_PROTOCOL_CANONICAL_AST_BYTES
  ) {
    fail("invalid_input", "specification.input.factor_evaluation.factor.expression");
  }
  const ast = expression.ast;
  if (
    ast === undefined ||
    ast.schemaVersion !== 1 ||
    ast.root === undefined ||
    ast.root.node.case === undefined
  ) {
    fail("invalid_input", "specification.input.factor_evaluation.factor.expression.ast");
  }
  const attachedAst = canonicalWireFactorAstBytes(ast);
  if (!equalDigest(attachedAst, expression.canonicalJson)) {
    fail("binding_mismatch", "specification.input.factor_evaluation.factor.expression.ast");
  }
  const computedExpressionId = encodeDigest(
    domainDigest(FACTOR_AST_DOMAIN, expression.canonicalJson),
  );
  if (!equalTextConstantTime(factorExpressionId, computedExpressionId)) {
    fail(
      "binding_mismatch",
      "specification.input.factor_evaluation.factor.expression.canonical_json",
    );
  }

  const registry = requireDigest(
    factor.operatorRegistrySha256,
    "specification.input.factor_evaluation.factor.operator_registry_sha256",
  );
  let direction: string;
  switch (factor.direction) {
    case FactorDirection.HIGHER_IS_BETTER:
      direction = "higher_is_better";
      break;
    case FactorDirection.LOWER_IS_BETTER:
      direction = "lower_is_better";
      break;
    case FactorDirection.UNSPECIFIED:
      throw new JobValidationError(
        "invalid_input",
        "specification.input.factor_evaluation.factor.direction",
      );
    default:
      throw new JobValidationError(
        "unknown_enum",
        "specification.input.factor_evaluation.factor.direction",
      );
  }
  const policies = factor.frozenPolicy;
  if (policies === undefined) {
    fail("missing_field", "specification.input.factor_evaluation.factor.frozen_policy");
  }
  const canonical =
    `{"schema":"loop.factor-spec/v1","expression_id":"${factorExpressionId}",` +
    `"operator_registry_sha256":"${encodeDigest(registry)}","direction":"${direction}"` +
    writeFactorPolicy("universe_policy", policies.universePolicy) +
    writeFactorPolicy("data_policy", policies.dataPolicy) +
    writeFactorPolicy("calendar_policy", policies.calendarPolicy) +
    writeFactorPolicy("preprocess_policy", policies.preprocessPolicy) +
    writeFactorPolicy("neutralization_policy", policies.neutralizationPolicy) +
    writeFactorPolicy("portfolio_policy", policies.portfolioPolicy) +
    writeFactorPolicy("execution_policy", policies.executionPolicy) +
    writeFactorPolicy("cost_policy", policies.costPolicy) +
    writeFactorPolicy("evaluation_policy", policies.evaluationPolicy) +
    "}";
  return encoder.encode(canonical);
}

function canonicalWireFactorAstBytes(ast: FactorAst): Uint8Array {
  if (ast.schemaVersion !== 1 || ast.root === undefined) {
    fail("invalid_input", "specification.input.factor_evaluation.factor.expression.ast");
  }
  const state = { nodes: 0 };
  const canonical = writeWireFactorAstNode(ast.root, 1, state);
  const bytes = encoder.encode(canonical);
  if (BigInt(bytes.byteLength) > MAX_PROTOCOL_CANONICAL_AST_BYTES) {
    fail("invalid_input", "specification.input.factor_evaluation.factor.expression.ast");
  }
  return bytes;
}

function writeWireFactorAstNode(
  node: FactorAstNode,
  depth: number,
  state: { nodes: number },
): string {
  if (depth > MAX_PROTOCOL_AST_DEPTH || ++state.nodes > MAX_PROTOCOL_AST_NODES) {
    fail("invalid_input", "specification.input.factor_evaluation.factor.expression.ast");
  }

  switch (node.node.case) {
    case "field": {
      const field = requireFactorIdentifier(node.node.value.field);
      return `{"node":"field","field":"${field}"}`;
    }
    case "literal":
      switch (node.node.value.value.case) {
        case "decimal": {
          const decimal = node.node.value.value.value.value;
          if (decimal === "-0" || !/^-?(?:0|[1-9][0-9]*)(?:\.[0-9]*[1-9])?$/.test(decimal)) {
            fail(
              "invalid_input",
              "specification.input.factor_evaluation.factor.expression.ast.literal.decimal",
            );
          }
          return `{"node":"decimal","value":"${decimal}"}`;
        }
        case "boolean":
          return `{"node":"boolean","value":${node.node.value.value.value ? "true" : "false"}}`;
        case "enumeration": {
          const enumeration = node.node.value.value.value;
          const enumType = requireFactorIdentifier(enumeration.enumType);
          const value = requireFactorIdentifier(enumeration.value);
          return `{"node":"enum","enum_type":"${enumType}","value":"${value}"}`;
        }
        case undefined:
          fail(
            "missing_field",
            "specification.input.factor_evaluation.factor.expression.ast.literal",
          );
      }
      break;
    case "call": {
      const call = node.node.value;
      if (call.operator === undefined) {
        fail(
          "missing_field",
          "specification.input.factor_evaluation.factor.expression.ast.call.operator",
        );
      }
      const operator = requireFactorIdentifier(call.operator.operator);
      const operatorVersion = call.operator.operatorVersion;
      if (
        !/^[1-9][0-9]{0,19}$/.test(operatorVersion) ||
        BigInt(operatorVersion) > 18_446_744_073_709_551_615n
      ) {
        fail(
          "invalid_input",
          "specification.input.factor_evaluation.factor.expression.ast.call.operator_version",
        );
      }
      if (call.arguments.length > MAX_PROTOCOL_AST_DIRECT_ARGUMENTS) {
        fail(
          "invalid_input",
          "specification.input.factor_evaluation.factor.expression.ast.call.arguments",
        );
      }
      const argumentsJson = call.arguments
        .map((argument) => writeWireFactorAstNode(argument, depth + 1, state))
        .join(",");
      return (
        `{"node":"call","operator":"${operator}",` +
        `"operator_version":"${operatorVersion}","arguments":[${argumentsJson}]}`
      );
    }
    case undefined:
      fail("missing_field", "specification.input.factor_evaluation.factor.expression.ast.node");
  }
  fail("invalid_input", "specification.input.factor_evaluation.factor.expression.ast");
}

function requireFactorIdentifier(value: string): string {
  if (
    encoder.encode(value).byteLength > 128 ||
    !/^[a-z][a-z0-9_]*(?:\.[a-z][a-z0-9_]*)*$/.test(value)
  ) {
    fail("invalid_input", "specification.input.factor_evaluation.factor.expression.ast");
  }
  return value;
}

/** Compute the domain-separated digest claimed by an inline `factor_spec_id`. */
export function factorSpecIdentitySha256(factor: FactorSpec): Uint8Array {
  return domainDigest(FACTOR_SPEC_DOMAIN, canonicalFactorSpecIdentityBytes(factor));
}

/** Validate both content-addressed identities carried by an inline factor. */
export function validateFactorSpecIdentityEnvelope(factor: FactorSpec): void {
  const claimed = requireSha256Id(
    factor.factorSpecId?.value,
    "specification.input.factor_evaluation.factor.factor_spec_id",
  );
  const computed = encodeDigest(factorSpecIdentitySha256(factor));
  if (!equalTextConstantTime(claimed, computed)) {
    fail("binding_mismatch", "specification.input.factor_evaluation.factor.factor_spec_id");
  }
}

function writeFactorPolicy(name: string, reference: PolicyReference | undefined): string {
  const policy = validatePolicy(
    reference,
    "specification.input.factor_evaluation.factor.frozen_policy",
  );
  const digest = requireDigest(
    policy.sha256,
    "specification.input.factor_evaluation.factor.frozen_policy",
  );
  return (
    `,"${name}":{"policy_id":"${policy.policyId?.value}",` +
    `"revision":"${policy.revision}","sha256":"${encodeDigest(digest)}"}`
  );
}

function validateModelResolution(
  model: ModelResolutionSnapshot | undefined,
  field: string,
  submittedAt: Timestamp,
): void {
  if (model === undefined) fail("missing_field", field);
  requireTokenId(model.resolutionId?.value, field);
  requireTokenId(model.providerId?.value, field);
  requireTokenId(model.modelId?.value, field);
  if (
    !isBoundedText(model.requestedAlias, MAX_PROTOCOL_NAME_BYTES) ||
    !isBoundedText(model.providerPluginName, MAX_PROTOCOL_NAME_BYTES) ||
    !isBuildVersion(model.providerPluginVersion)
  ) {
    fail("invalid_input", field);
  }
  if (
    model.protocolFamily < ModelProtocolFamily.OPENAI_RESPONSES ||
    model.protocolFamily > ModelProtocolFamily.COHERE_V2_CHAT
  ) {
    fail("unknown_enum", field);
  }
  const capabilities = model.capabilities;
  if (capabilities === undefined) fail("missing_field", field);
  if (
    capabilities.contextWindowTokens < 1n ||
    capabilities.contextWindowTokens > MAX_JOB_TOKENS ||
    capabilities.maximumOutputTokens < 1n ||
    capabilities.maximumOutputTokens > capabilities.contextWindowTokens
  ) {
    fail("invalid_input", field);
  }
  const pricing = model.pricing;
  if (pricing === undefined) fail("missing_field", field);
  validateMoney(pricing.inputPerMillionTokens, field);
  validateMoney(pricing.outputPerMillionTokens, field);
  validateMoney(pricing.cachedInputPerMillionTokens, field);
  requireDigest(model.capabilitySha256, field);
  requireDigest(model.catalogSha256, field);
  requireDigest(model.providerPluginSha256, field);
  requireDigest(model.snapshotSha256, field);
  const resolvedAt = requireTimestamp(model.resolvedAt, field);
  if (compareTimestamp(resolvedAt, submittedAt) > 0) fail("invalid_input", field);
}

function validateBudget(budget: JobBudget | undefined, field: string): void {
  if (budget === undefined) fail("missing_field", field);
  if (
    budget.maximumSteps < 1 ||
    budget.maximumSteps > MAX_JOB_STEPS ||
    budget.maximumInputTokens > MAX_JOB_TOKENS ||
    budget.maximumOutputTokens > MAX_JOB_TOKENS
  ) {
    fail("invalid_budget", field);
  }
  validateMoney(budget.maximumCost, field);
  const duration = budget.maximumWallTime;
  if (duration === undefined) fail("missing_field", field);
  if (
    duration.seconds < 0n ||
    duration.seconds > MAX_JOB_WALL_TIME_SECONDS ||
    duration.nanos < 0 ||
    duration.nanos >= 1_000_000_000 ||
    (duration.seconds === 0n && duration.nanos === 0) ||
    (duration.seconds === MAX_JOB_WALL_TIME_SECONDS && duration.nanos !== 0)
  ) {
    fail("invalid_budget", field);
  }
}

function validateMoney(
  money:
    | {
        readonly amount?: { readonly value: string };
        readonly currencyCode: string;
      }
    | undefined,
  field: string,
): void {
  if (money === undefined || money.amount === undefined) fail("missing_field", field);
  if (!/^[A-Z]{3}$/.test(money.currencyCode) || !isNormalizedCost(money.amount.value)) {
    fail("invalid_budget", field);
  }
}

function isNormalizedCost(amount: string): boolean {
  const match = /^(0|[1-9][0-9]*)(?:\.([0-9]*[1-9]))?$/.exec(amount);
  if (match === null) return false;
  const integer = match[1] as string;
  const fraction = match[2];
  if ((fraction?.length ?? 0) > 9) return false;
  const significant =
    integer === "0"
      ? Math.max((fraction ?? "").replace(/^0+/, "").length, 1)
      : integer.length + (fraction?.length ?? 0);
  return (
    significant <= 18 &&
    BigInt(integer) <= 1_000_000n &&
    !(integer === "1000000" && fraction !== undefined)
  );
}

function validateProvenance(
  provenance: ResearchProvenanceFingerprint | undefined,
  field: string,
): void {
  if (provenance === undefined) fail("missing_field", field);
  const digests = [
    provenance.sourceCodeSha256,
    provenance.operatorRegistrySha256,
    provenance.configurationSha256,
    provenance.dataManifestSha256,
    provenance.tradingCalendarSha256,
    provenance.environmentSha256,
  ];
  if (digests.some((digest) => digest?.value.byteLength !== 32)) {
    fail("invalid_provenance", field);
  }
}

function validateHoldoutBacktestInput(
  input: HoldoutBacktestJobInput,
  submittedAt: Timestamp,
): void {
  const grant = input.consumedGrant;
  if (grant === undefined) {
    fail("missing_field", "specification.input.holdout_backtest.consumed_grant");
  }
  const [grantIssuedAt, grantExpiresAt] = validateHoldoutGrant(grant);
  if (
    compareTimestamp(submittedAt, grantIssuedAt) < 0 ||
    compareTimestamp(submittedAt, grantExpiresAt) >= 0
  ) {
    fail("invalid_input", "specification.input.holdout_backtest.consumed_grant.validity_window");
  }
  if (input.consumedGrantRevision === 0n) {
    fail("invalid_input", "specification.input.holdout_backtest.consumed_grant_revision");
  }
  const batchId = requireTokenId(
    input.jobBatchId?.value,
    "specification.input.holdout_backtest.job_batch_id",
  );
  if (batchId === grant.holdoutGrantId?.value) {
    fail("binding_mismatch", "specification.input.holdout_backtest.job_batch_id");
  }
  const planId = requireSha256Id(
    input.holdoutEvaluationPlanId?.value,
    "specification.input.holdout_backtest.holdout_evaluation_plan_id",
  );
  const grantPlanId = requireSha256Id(
    grant.holdoutEvaluationPlanId?.value,
    "specification.input.holdout_backtest.consumed_grant.holdout_evaluation_plan_id",
  );
  if (planId !== grantPlanId) {
    fail("binding_mismatch", "specification.input.holdout_backtest.holdout_evaluation_plan_id");
  }
  const planDigest = requireDigest(
    input.evaluationPlanSha256,
    "specification.input.holdout_backtest.evaluation_plan_sha256",
  );
  const grantPlanDigest = requireDigest(
    grant.evaluationPlanSha256,
    "specification.input.holdout_backtest.consumed_grant.evaluation_plan_sha256",
  );
  if (!equalDigest(planDigest, grantPlanDigest)) {
    fail("binding_mismatch", "specification.input.holdout_backtest.evaluation_plan_sha256");
  }
  if (
    input.evaluationPlanEntryIndex < 1 ||
    input.evaluationPlanEntryIndex > grant.evaluationPlanEntryCount
  ) {
    fail("invalid_input", "specification.input.holdout_backtest.evaluation_plan_entry_index");
  }
  if (input.frozenBacktestSpec === undefined) {
    fail("missing_field", "specification.input.holdout_backtest.frozen_backtest_spec");
  }
  validateFrozenBacktestSpec(input.frozenBacktestSpec, grant.issuedAt);
  validateBudget(input.budget, "specification.input.holdout_backtest.budget");
}

/**
 * Bind a holdout job to freshly verified canonical period and plan bytes.
 * Both documents are reparsed on every call, so mutable or caller-constructed
 * canonical DTOs cannot cross this boundary. The bytes, trusted schema digest,
 * and artifact map must come from a server-owned resolver, never the request or
 * worker. This binds only factor ID, budget, and the canonical-spec digest. The
 * Phase 7 owning parser must use the exact referenced artifact bytes to derive
 * or validate and bind every frozen BacktestSpec field, including sample,
 * snapshots, return definition, provenance, and seed. Persisted grant
 * resolution and runtime authorization remain external Phase 4 gates.
 */
export function validateHoldoutBacktestPlanEntryBinding(
  input: HoldoutBacktestJobInput,
  submittedAt: Timestamp,
  canonicalPeriodBytes: Uint8Array | string,
  canonicalPlanBytes: Uint8Array | string,
  trustedBacktestSchemaSha256: Uint8Array,
  resolvedBacktestArtifacts: ReadonlyMap<string, Uint8Array>,
): void {
  validateHoldoutBacktestInput(input, requireTimestamp(submittedAt, "specification.submitted_at"));
  const grant = input.consumedGrant;
  const frozen = input.frozenBacktestSpec;
  const budget = input.budget;
  if (grant === undefined || frozen === undefined || budget === undefined) {
    fail("missing_field", "specification.input.holdout_backtest");
  }
  const planId = requireSha256Id(
    input.holdoutEvaluationPlanId?.value,
    "specification.input.holdout_backtest.holdout_evaluation_plan_id",
  );
  const planDigest = requireDigest(
    input.evaluationPlanSha256,
    "specification.input.holdout_backtest.evaluation_plan_sha256",
  );
  const periodId = requireSha256Id(
    grant.holdoutPeriodId?.value,
    "specification.input.holdout_backtest.consumed_grant.holdout_period_id",
  );
  const periodDigest = requireDigest(
    grant.canonicalPeriodSha256,
    "specification.input.holdout_backtest.consumed_grant.canonical_period_sha256",
  );
  let plan: CanonicalHoldoutEvaluationPlan;
  try {
    const period = verifyHoldoutPeriodIdentity(canonicalPeriodBytes, periodId, periodDigest);
    plan = parseCanonicalHoldoutEvaluationPlan(
      canonicalPlanBytes,
      period,
      trustedBacktestSchemaSha256,
      resolvedBacktestArtifacts,
    );
  } catch {
    fail("binding_mismatch", "specification.input.holdout_backtest.resolved_plan");
  }
  if (
    !equalTextConstantTime(planId, plan.holdoutEvaluationPlanId) ||
    !equalDigest(planDigest, plan.planSha256) ||
    !equalTextConstantTime(periodId, plan.value.holdout_period_id) ||
    !equalTextConstantTime(encodeDigest(periodDigest), plan.value.canonical_period_sha256) ||
    grant.evaluationPlanEntryCount !== plan.value.entries.length
  ) {
    fail("binding_mismatch", "specification.input.holdout_backtest.resolved_plan");
  }
  const entry = plan.value.entries.find(
    (candidate) => candidate.entry_index === input.evaluationPlanEntryIndex.toString(),
  );
  if (entry === undefined) {
    fail("binding_mismatch", "specification.input.holdout_backtest.evaluation_plan_entry_index");
  }
  const factorId = requireSha256Id(
    frozen.factorSpecId?.value,
    "specification.input.holdout_backtest.frozen_backtest_spec.factor_spec_id",
  );
  const canonicalSpec = requireDigest(
    frozen.canonicalSpecSha256,
    "specification.input.holdout_backtest.frozen_backtest_spec.canonical_spec_sha256",
  );
  if (
    !equalTextConstantTime(factorId, entry.factor_spec_id) ||
    !equalTextConstantTime(encodeDigest(canonicalSpec), entry.backtest_spec_artifact.sha256) ||
    !holdoutBudgetMatchesPlan(budget, entry.job_budget)
  ) {
    fail("binding_mismatch", "specification.input.holdout_backtest.resolved_plan_entry");
  }
}

function holdoutBudgetMatchesPlan(budget: JobBudget, expected: HoldoutJobBudget): boolean {
  const cost = budget.maximumCost;
  const wall = budget.maximumWallTime;
  if (cost?.amount === undefined || wall === undefined) return false;
  const wallNanoseconds = wall.seconds * 1_000_000_000n + BigInt(wall.nanos);
  return (
    budget.maximumSteps.toString() === expected.maximum_steps &&
    budget.maximumInputTokens.toString() === expected.maximum_input_tokens &&
    budget.maximumOutputTokens.toString() === expected.maximum_output_tokens &&
    cost.amount.value === expected.maximum_cost.amount &&
    cost.currencyCode === expected.maximum_cost.currency_code &&
    wallNanoseconds.toString() === expected.maximum_wall_time_ns
  );
}

function validateHoldoutGrant(
  grant: NonNullable<HoldoutBacktestJobInput["consumedGrant"]>,
): readonly [Timestamp, Timestamp] {
  requireTokenId(
    grant.holdoutGrantId?.value,
    "specification.input.holdout_backtest.consumed_grant.holdout_grant_id",
  );
  const periodId = requireSha256Id(
    grant.holdoutPeriodId?.value,
    "specification.input.holdout_backtest.consumed_grant.holdout_period_id",
  );
  requireDigest(
    grant.freezeManifestSha256,
    "specification.input.holdout_backtest.consumed_grant.freeze_manifest_sha256",
  );
  const issuedAt = requireTimestamp(
    grant.issuedAt,
    "specification.input.holdout_backtest.consumed_grant.issued_at",
  );
  const expiresAt = requireTimestamp(
    grant.expiresAt,
    "specification.input.holdout_backtest.consumed_grant.expires_at",
  );
  if (compareTimestamp(issuedAt, expiresAt) >= 0) {
    fail("invalid_input", "specification.input.holdout_backtest.consumed_grant.expires_at");
  }
  requireSha256Id(
    grant.holdoutEvaluationPlanId?.value,
    "specification.input.holdout_backtest.consumed_grant.holdout_evaluation_plan_id",
  );
  requireDigest(
    grant.evaluationPlanSha256,
    "specification.input.holdout_backtest.consumed_grant.evaluation_plan_sha256",
  );
  if (grant.evaluationPlanEntryCount < 1 || grant.evaluationPlanEntryCount > MAX_HOLDOUT_ENTRIES) {
    fail(
      "invalid_input",
      "specification.input.holdout_backtest.consumed_grant.evaluation_plan_entry_count",
    );
  }
  const periodDigest = requireDigest(
    grant.canonicalPeriodSha256,
    "specification.input.holdout_backtest.consumed_grant.canonical_period_sha256",
  );
  if (periodId !== encodeDigest(periodDigest)) {
    fail(
      "binding_mismatch",
      "specification.input.holdout_backtest.consumed_grant.canonical_period_sha256",
    );
  }
  return [issuedAt, expiresAt];
}

function validateFrozenBacktestSpec(
  backtest: BacktestSpec,
  grantIssuedAt: Timestamp | undefined,
): void {
  requireTokenId(
    backtest.backtestId?.value,
    "specification.input.holdout_backtest.frozen_backtest_spec.backtest_id",
  );
  if (backtest.schemaVersion !== 1) {
    fail(
      "invalid_input",
      "specification.input.holdout_backtest.frozen_backtest_spec.schema_version",
    );
  }
  requireSha256Id(
    backtest.factorSpecId?.value,
    "specification.input.holdout_backtest.frozen_backtest_spec.factor_spec_id",
  );
  if (
    backtest.snapshotIds.length < 1 ||
    backtest.snapshotIds.length > MAX_DATASET_SNAPSHOTS ||
    backtest.snapshotIds.some((snapshot) => !/^sha256:[0-9a-f]{64}$/.test(snapshot.value)) ||
    backtest.snapshotIds.some(
      (snapshot, index) =>
        index > 0 && (backtest.snapshotIds[index - 1]?.value ?? "") >= snapshot.value,
    )
  ) {
    fail("invalid_input", "specification.input.holdout_backtest.frozen_backtest_spec.snapshot_ids");
  }
  validateLockedSample(backtest);
  validateSimpleReturn(
    backtest.returnDefinition,
    "specification.input.holdout_backtest.frozen_backtest_spec.return_definition",
  );
  validateProvenance(
    backtest.provenance,
    "specification.input.holdout_backtest.frozen_backtest_spec.provenance",
  );
  requireDigest(
    backtest.canonicalSpecSha256,
    "specification.input.holdout_backtest.frozen_backtest_spec.canonical_spec_sha256",
  );
  requireDigest(
    backtest.deterministicSeed,
    "specification.input.holdout_backtest.frozen_backtest_spec.deterministic_seed",
  );
  const createdAt = requireTimestamp(
    backtest.createdAt,
    "specification.input.holdout_backtest.frozen_backtest_spec.created_at",
  );
  const issuedAt = requireTimestamp(
    grantIssuedAt,
    "specification.input.holdout_backtest.consumed_grant.issued_at",
  );
  if (compareTimestamp(createdAt, issuedAt) > 0) {
    fail(
      "binding_mismatch",
      "specification.input.holdout_backtest.frozen_backtest_spec.created_at",
    );
  }
}

function validateLockedSample(backtest: BacktestSpec): void {
  const sample = backtest.sample;
  if (sample === undefined) {
    fail("missing_field", "specification.input.holdout_backtest.frozen_backtest_spec.sample");
  }
  if (
    !Number.isInteger(sample.role) ||
    sample.role < SampleRole.UNSPECIFIED ||
    sample.role > SampleRole.PROSPECTIVE_OBSERVATION
  ) {
    fail("unknown_enum", "specification.input.holdout_backtest.frozen_backtest_spec.sample.role");
  }
  if (
    sample.role !== SampleRole.FIRST_LOCKED_CONFIRMATION &&
    sample.role !== SampleRole.SECOND_LOCKED_HISTORICAL_HOLDOUT
  ) {
    fail("invalid_input", "specification.input.holdout_backtest.frozen_backtest_spec.sample.role");
  }
  const start = validateCivilDate(
    sample.startInclusive,
    "specification.input.holdout_backtest.frozen_backtest_spec.sample.start_inclusive",
  );
  const end = validateCivilDate(
    sample.endInclusive,
    "specification.input.holdout_backtest.frozen_backtest_spec.sample.end_inclusive",
  );
  if (start > end) {
    fail("invalid_input", "specification.input.holdout_backtest.frozen_backtest_spec.sample");
  }
}

function validateCivilDate(
  date: { readonly year: number; readonly month: number; readonly day: number } | undefined,
  field: string,
): number {
  if (date === undefined) fail("missing_field", field);
  const leap = date.year % 4 === 0 && (date.year % 100 !== 0 || date.year % 400 === 0);
  const monthDays = [0, 31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
  const maximumDay = monthDays[date.month];
  if (
    !Number.isInteger(date.year) ||
    date.year < 1 ||
    date.year > 9_999 ||
    maximumDay === undefined ||
    date.day < 1 ||
    date.day > maximumDay
  ) {
    fail("invalid_input", field);
  }
  return date.year * 10_000 + date.month * 100 + date.day;
}

function validateSimpleReturn(value: ReturnDefinition, field: string): void {
  if (
    !Number.isInteger(value) ||
    value < ReturnDefinition.UNSPECIFIED ||
    value > ReturnDefinition.SIMPLE_NAV_RETURN
  ) {
    fail("unknown_enum", field);
  }
  if (value !== ReturnDefinition.SIMPLE_NAV_RETURN) fail("invalid_input", field);
}

/** Validate cross-field invariants before a wire DTO enters domain state. */
export function validateJobRecord(record: JobRecord): Readonly<ValidatedJobShape> {
  const specification = record.specification;
  if (specification === undefined) fail("missing_field", "specification");
  if (record.revision === 0n) fail("invalid_revision", "revision");
  const { kind } = validateJobSpecification(specification);
  const specificationJobId = requireTokenId(specification.jobId?.value, "specification.job_id");
  const submittedAt = requireTimestamp(specification.submittedAt, "specification.submitted_at");
  const updatedAt = requireTimestamp(record.updatedAt, "updated_at");
  if (compareTimestamp(updatedAt, submittedAt) < 0) fail("invalid_envelope", "updated_at");

  const state = validateState(record.state);
  const hasLease = record.activeLease !== undefined;
  const isActive = state === JobState.LEASED || state === JobState.RUNNING;
  const isTerminal = isTerminalState(state);
  if (hasLease !== isActive) fail("state_lease_mismatch", "active_lease");
  if (state === JobState.QUEUED) {
    if (record.attempt !== 0) fail("invalid_attempt", "attempt");
  } else if ((isActive || isTerminal) && record.attempt === 0) {
    fail("invalid_attempt", "attempt");
  }

  const outcomeCase = record.outcome?.outcome.case;
  if (record.outcome !== undefined && outcomeCase === undefined) {
    fail("missing_field", "outcome.outcome");
  }
  if (state === JobState.QUEUED || isActive) {
    if (outcomeCase !== undefined) fail("state_outcome_mismatch", "outcome");
  } else {
    const expectedOutcome: Partial<Record<JobState, string>> = {
      [JobState.SUCCEEDED]: "success",
      [JobState.FACTOR_REJECTED]: "factorRejection",
      [JobState.INFRASTRUCTURE_FAILED]: "infrastructureFailure",
      [JobState.CANCELLED]: "cancellation",
      [JobState.BUDGET_EXHAUSTED]: "budgetExhaustion",
    };
    if (expectedOutcome[state] !== outcomeCase) fail("state_outcome_mismatch", "outcome");
  }

  const outcome = record.outcome?.outcome;
  switch (outcome?.case) {
    case "success":
      validateArtifacts(outcome.value.outputs, "outcome.success.outputs");
      break;
    case "factorRejection": {
      if (
        kind !== JobKind.FACTOR_EVALUATION &&
        kind !== JobKind.BACKTEST &&
        kind !== JobKind.HOLDOUT_BACKTEST
      ) {
        fail("rejection_not_allowed", "outcome.factor_rejection");
      }
      const expected = factorIdFromInput(specification);
      const actual = requireSha256Id(
        outcome.value.factorSpecId?.value,
        "outcome.factor_rejection.factor_spec_id",
      );
      if (expected !== actual) {
        fail("factor_identity_mismatch", "outcome.factor_rejection.factor_spec_id");
      }
      if (!isFactorRejectionCode(outcome.value.code)) {
        fail("unknown_enum", "outcome.factor_rejection.code");
      }
      if (!isBoundedText(outcome.value.reason, MAX_REASON_BYTES)) {
        fail("invalid_terminal_payload", "outcome.factor_rejection.reason");
      }
      validateEventTimestamp(
        requireTimestamp(outcome.value.rejectedAt, "outcome.factor_rejection.rejected_at"),
        submittedAt,
        updatedAt,
        "outcome.factor_rejection.rejected_at",
      );
      validateArtifacts(outcome.value.evidence, "outcome.factor_rejection.evidence");
      break;
    }
    case "infrastructureFailure": {
      const failure = outcome.value;
      const serviceError = failure.error;
      if (serviceError === undefined) fail("missing_field", "outcome.infrastructure_failure.error");
      validateServiceError(serviceError);
      if (failure.attempt === 0 || failure.attempt !== record.attempt) {
        fail("invalid_terminal_payload", "outcome.infrastructure_failure.attempt");
      }
      validateEventTimestamp(
        requireTimestamp(failure.failedAt, "outcome.infrastructure_failure.failed_at"),
        submittedAt,
        updatedAt,
        "outcome.infrastructure_failure.failed_at",
      );
      break;
    }
    case "cancellation":
      if (!isBoundedText(outcome.value.reason, MAX_REASON_BYTES)) {
        fail("invalid_terminal_payload", "outcome.cancellation.reason");
      }
      validateActor(outcome.value.cancelledBy, "outcome.cancellation.cancelled_by");
      validateEventTimestamp(
        requireTimestamp(outcome.value.cancelledAt, "outcome.cancellation.cancelled_at"),
        submittedAt,
        updatedAt,
        "outcome.cancellation.cancelled_at",
      );
      break;
    case "budgetExhaustion":
      if (!isBoundedText(outcome.value.exhaustedLimit, MAX_ERROR_CODE_BYTES)) {
        fail("invalid_terminal_payload", "outcome.budget_exhaustion.exhausted_limit");
      }
      validateBudget(outcome.value.enforcedBudget, "outcome.budget_exhaustion.enforced_budget");
      if (!budgetsEqual(outcome.value.enforcedBudget, jobBudget(specification))) {
        fail("binding_mismatch", "outcome.budget_exhaustion.enforced_budget");
      }
      validateEventTimestamp(
        requireTimestamp(outcome.value.exhaustedAt, "outcome.budget_exhaustion.exhausted_at"),
        submittedAt,
        updatedAt,
        "outcome.budget_exhaustion.exhausted_at",
      );
      break;
    case undefined:
      break;
  }

  const lease = record.activeLease;
  if (lease !== undefined) {
    requireTokenId(lease.leaseId?.value, "active_lease.lease_id");
    const leaseJobId = requireTokenId(lease.jobId?.value, "active_lease.job_id");
    if (specificationJobId !== leaseJobId) fail("lease_job_mismatch", "active_lease.job_id");
    validateActor(lease.owner, "active_lease.owner");
    if (lease.acquiredRevision === 0n || lease.acquiredRevision > record.revision) {
      fail("invalid_revision", "active_lease.acquired_revision");
    }
    const issued = requireTimestamp(lease.issuedAt, "active_lease.issued_at");
    const heartbeat = requireTimestamp(lease.heartbeatAt, "active_lease.heartbeat_at");
    const expires = requireTimestamp(lease.expiresAt, "active_lease.expires_at");
    if (
      compareTimestamp(issued, submittedAt) < 0 ||
      compareTimestamp(issued, heartbeat) > 0 ||
      compareTimestamp(heartbeat, updatedAt) > 0 ||
      compareTimestamp(updatedAt, expires) >= 0
    ) {
      fail("invalid_lease", "active_lease.timestamps");
    }
  }

  return Object.freeze({ kind, state });
}

function validateEventTimestamp(
  eventAt: Timestamp,
  submittedAt: Timestamp,
  updatedAt: Timestamp,
  field: string,
): void {
  if (compareTimestamp(eventAt, submittedAt) < 0 || compareTimestamp(eventAt, updatedAt) > 0) {
    fail("invalid_envelope", field);
  }
}

function jobBudget(specification: JobSpecification): JobBudget {
  const input = specification.input;
  const budget = input.case === undefined ? undefined : input.value.budget;
  if (budget === undefined) fail("missing_field", "specification.input.budget");
  return budget;
}

function budgetsEqual(left: JobBudget | undefined, right: JobBudget): boolean {
  return (
    left !== undefined &&
    left.maximumSteps === right.maximumSteps &&
    left.maximumInputTokens === right.maximumInputTokens &&
    left.maximumOutputTokens === right.maximumOutputTokens &&
    left.maximumCost?.amount?.value === right.maximumCost?.amount?.value &&
    left.maximumCost?.currencyCode === right.maximumCost?.currencyCode &&
    left.maximumWallTime?.seconds === right.maximumWallTime?.seconds &&
    left.maximumWallTime?.nanos === right.maximumWallTime?.nanos
  );
}

function factorIdFromInput(specification: JobSpecification): string {
  let identity: string | undefined;
  switch (specification.input.case) {
    case "factorEvaluation":
      identity = specification.input.value.factor?.factorSpecId?.value;
      break;
    case "backtest":
      identity = specification.input.value.factorSpecId?.value;
      break;
    case "holdoutBacktest":
      identity = specification.input.value.frozenBacktestSpec?.factorSpecId?.value;
      break;
  }
  return requireSha256Id(identity, "specification.input.factor_spec_id");
}

function validateKind(kind: JobKind): JobKind {
  switch (kind) {
    case JobKind.DISCOVERY:
    case JobKind.FACTOR_EVALUATION:
    case JobKind.BACKTEST:
    case JobKind.INDEPENDENT_RECONCILIATION:
    case JobKind.REPORT:
    case JobKind.PROSPECTIVE_OBSERVATION:
    case JobKind.HOLDOUT_BACKTEST:
      return kind;
    default:
      fail("unknown_enum", "specification.kind");
  }
}

function validateState(state: JobState): JobState {
  switch (state) {
    case JobState.QUEUED:
    case JobState.LEASED:
    case JobState.RUNNING:
    case JobState.SUCCEEDED:
    case JobState.FACTOR_REJECTED:
    case JobState.INFRASTRUCTURE_FAILED:
    case JobState.CANCELLED:
    case JobState.BUDGET_EXHAUSTED:
      return state;
    default:
      fail("unknown_enum", "state");
  }
}

function isTerminalState(state: JobState): boolean {
  return (
    state === JobState.SUCCEEDED ||
    state === JobState.FACTOR_REJECTED ||
    state === JobState.INFRASTRUCTURE_FAILED ||
    state === JobState.CANCELLED ||
    state === JobState.BUDGET_EXHAUSTED
  );
}

function isFactorRejectionCode(code: FactorRejectionCode): boolean {
  return (
    code === FactorRejectionCode.DUPLICATE ||
    code === FactorRejectionCode.PREVIOUSLY_FAILED ||
    code === FactorRejectionCode.INSUFFICIENT_COVERAGE ||
    code === FactorRejectionCode.DETERMINISTIC_FILTER ||
    code === FactorRejectionCode.PERFORMANCE ||
    code === FactorRejectionCode.CORRELATION ||
    code === FactorRejectionCode.SEMANTIC_REVIEW ||
    code === FactorRejectionCode.POLICY
  );
}

function isErrorCategory(category: ErrorCategory): boolean {
  return category >= ErrorCategory.VALIDATION && category <= ErrorCategory.BUDGET_EXHAUSTED;
}

export function validateServiceError(serviceError: {
  readonly category: ErrorCategory;
  readonly code: string;
  readonly message: string;
  readonly details: readonly {
    readonly fieldPath: string;
    readonly code: string;
    readonly message: string;
  }[];
}): void {
  if (!isErrorCategory(serviceError.category)) {
    fail("unknown_enum", "outcome.infrastructure_failure.error.category");
  }
  if (
    !isStableErrorCode(serviceError.code) ||
    !isBoundedText(serviceError.message, MAX_ERROR_MESSAGE_BYTES)
  ) {
    fail("invalid_terminal_payload", "outcome.infrastructure_failure.error");
  }
  if (serviceError.details.length > MAX_ERROR_DETAILS) {
    fail("collection_limit", "outcome.infrastructure_failure.error.details");
  }
  for (const detail of serviceError.details) {
    if (
      !isBoundedFieldPath(detail.fieldPath) ||
      !isStableErrorCode(detail.code) ||
      !isBoundedText(detail.message, MAX_ERROR_MESSAGE_BYTES)
    ) {
      fail("invalid_terminal_payload", "outcome.infrastructure_failure.error.details");
    }
  }
}

function validateActor(
  actor:
    | {
        readonly actorId?: { readonly value: string };
        readonly kind: ActorKind;
        readonly displayName: string;
        readonly authenticatedSubject: string;
      }
    | undefined,
  field: string,
): void {
  if (actor === undefined) fail("missing_field", field);
  requireTokenId(actor.actorId?.value, field);
  if (actor.kind < ActorKind.HUMAN || actor.kind > ActorKind.SCHEDULER) {
    fail("unknown_enum", field);
  }
  if (
    (actor.displayName !== "" && !isBoundedText(actor.displayName, MAX_ACTOR_DISPLAY_NAME_BYTES)) ||
    !isBoundedText(actor.authenticatedSubject, MAX_AUTHENTICATED_SUBJECT_BYTES)
  ) {
    fail("invalid_envelope", field);
  }
}

function validateArtifacts(
  artifacts: readonly Parameters<typeof validateArtifactRef>[0][],
  field: string,
): void {
  if (artifacts.length > MAX_OUTCOME_ARTIFACTS) fail("collection_limit", field);
  for (const artifact of artifacts) {
    try {
      validateArtifactRef(artifact);
    } catch {
      fail("invalid_terminal_payload", field);
    }
  }
}

function requireSha256Id(value: string | undefined, field: string): string {
  if (value === undefined) fail("missing_field", field);
  if (!/^sha256:[0-9a-f]{64}$/.test(value)) fail("invalid_identity", field);
  return value;
}

function requireDigest(value: Sha256Digest | undefined, field: string): Uint8Array {
  if (value === undefined) fail("missing_field", field);
  if (value.value.byteLength !== 32) fail("invalid_identity", field);
  return value.value;
}

function encodeDigest(value: Uint8Array): string {
  return `sha256:${Buffer.from(value).toString("hex")}`;
}

function domainDigest(domain: Uint8Array, canonical: Uint8Array): Uint8Array {
  return createHash("sha256").update(domain).update(Uint8Array.of(0)).update(canonical).digest();
}

function equalTextConstantTime(left: string, right: string): boolean {
  const leftBytes = encoder.encode(left);
  const rightBytes = encoder.encode(right);
  return leftBytes.byteLength === rightBytes.byteLength && timingSafeEqual(leftBytes, rightBytes);
}

function equalDigest(left: Uint8Array, right: Uint8Array): boolean {
  return left.byteLength === right.byteLength && timingSafeEqual(left, right);
}

function isProtocolPackage(value: string): boolean {
  return (
    encoder.encode(value).byteLength <= MAX_PROTOCOL_NAME_BYTES &&
    /^[a-z][a-z0-9_]*(?:\.[a-z][a-z0-9_]*)*\.v[1-9][0-9]*$/.test(value)
  );
}

function isProtocolFeature(value: string): boolean {
  return (
    encoder.encode(value).byteLength <= MAX_PROTOCOL_NAME_BYTES &&
    /^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)+$/.test(value)
  );
}

function isBuildVersion(value: string): boolean {
  return (
    encoder.encode(value).byteLength <= MAX_BUILD_VERSION_BYTES &&
    /^[A-Za-z0-9][A-Za-z0-9.+_-]*$/.test(value)
  );
}

function requireTokenId(value: string | undefined, field: string): string {
  if (value === undefined) fail("missing_field", field);
  if (
    encoder.encode(value).byteLength > MAX_ID_BYTES ||
    !/^[A-Za-z0-9][A-Za-z0-9._:-]*$/.test(value)
  ) {
    fail("invalid_identity", field);
  }
  return value;
}

function requireTimestamp(value: Timestamp | undefined, field: string): Timestamp {
  if (value === undefined) fail("missing_field", field);
  if (
    value.seconds < MIN_TIMESTAMP_SECONDS ||
    value.seconds > MAX_TIMESTAMP_SECONDS ||
    value.nanos < 0 ||
    value.nanos >= 1_000_000_000
  ) {
    fail("invalid_terminal_payload", field);
  }
  return value;
}

function compareTimestamp(left: Timestamp, right: Timestamp): number {
  if (left.seconds !== right.seconds) return left.seconds < right.seconds ? -1 : 1;
  return left.nanos === right.nanos ? 0 : left.nanos < right.nanos ? -1 : 1;
}

function isBoundedText(value: string, maximumBytes: number): boolean {
  return (
    value.trim() !== "" &&
    encoder.encode(value).byteLength <= maximumBytes &&
    !hasAsciiControl(value)
  );
}

function isStableErrorCode(value: string): boolean {
  return /^[a-z][a-z0-9_.-]{0,127}$/.test(value);
}

function isBoundedFieldPath(value: string): boolean {
  return (
    value.trim() !== "" &&
    encoder.encode(value).byteLength <= MAX_ERROR_FIELD_PATH_BYTES &&
    !hasAsciiControl(value)
  );
}

function hasAsciiControl(value: string): boolean {
  return [...value].some((character) => {
    const codePoint = character.codePointAt(0);
    return codePoint !== undefined && (codePoint < 32 || codePoint === 127);
  });
}

function fail(code: JobValidationCode, field: string): never {
  throw new JobValidationError(code, field);
}
