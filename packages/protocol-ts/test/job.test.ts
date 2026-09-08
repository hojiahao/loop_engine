import { readFileSync } from "node:fs";
import { create } from "@bufbuild/protobuf";
import { DurationSchema, TimestampSchema } from "@bufbuild/protobuf/wkt";
import { describe, expect, it } from "vitest";

import {
  ArtifactRefSchema,
  ArtifactSchemaReferenceSchema,
} from "../src/generated/loop/v1/artifact_pb.js";
import { BacktestSpecSchema } from "../src/generated/loop/v1/backtest_pb.js";
import {
  ActorIdSchema,
  ActorKind,
  ActorSchema,
  ArtifactIdSchema,
  BacktestIdSchema,
  CausationIdSchema,
  CivilDateSchema,
  CorrelationIdSchema,
  ErrorCategory,
  ErrorDetailSchema,
  ExactDecimalSchema,
  FactorExpressionIdSchema,
  FactorSpecIdSchema,
  HoldoutEvaluationPlanIdSchema,
  HoldoutGrantIdSchema,
  HoldoutPeriodIdSchema,
  IdempotencyKeySchema,
  JobBatchIdSchema,
  JobIdSchema,
  LeaseIdSchema,
  ModelIdSchema,
  ModelResolutionIdSchema,
  MoneySchema,
  PolicyIdSchema,
  PolicyReferenceSchema,
  ProtocolLimitsSchema,
  ProtocolSelectionSnapshotSchema,
  ProviderIdSchema,
  RunIdSchema,
  ServiceErrorSchema,
  Sha256DigestSchema,
  SnapshotIdSchema,
} from "../src/generated/loop/v1/common_pb.js";
import { SampleRole, SampleWindowSchema } from "../src/generated/loop/v1/data_pb.js";
import { DevelopmentDatasetReferenceSchema } from "../src/generated/loop/v1/development_data_pb.js";
import {
  CanonicalFactorAstSchema,
  FactorAstNodeSchema,
  FactorAstSchema,
  FactorDirection,
  FactorSpecSchema,
  FieldReferenceSchema,
  FrozenResearchPolicyReferenceSchema,
} from "../src/generated/loop/v1/factor_pb.js";
import { HoldoutGrantReferenceSchema } from "../src/generated/loop/v1/holdout_pb.js";
import {
  ArtifactJobInputSchema,
  BacktestJobInputSchema,
  BudgetExhaustionSchema,
  DiscoveryJobInputSchema,
  FactorEvaluationJobInputSchema,
  FactorRejectionCode,
  FactorRejectionSchema,
  HoldoutBacktestJobInputSchema,
  InfrastructureFailureSchema,
  type JobBudget,
  JobBudgetSchema,
  JobCancellationSchema,
  JobKind,
  JobLeaseSchema,
  JobOutcomeSchema,
  type JobRecord,
  JobRecordSchema,
  JobSpecificationSchema,
  JobState,
  JobSuccessSchema,
  ReconciliationJobInputSchema,
} from "../src/generated/loop/v1/job_pb.js";
import {
  ModelCapabilitiesSchema,
  ModelPricingSchema,
  ModelProtocolFamily,
  ModelResolutionSnapshotSchema,
} from "../src/generated/loop/v1/model_pb.js";
import {
  ResearchProvenanceFingerprintSchema,
  ReturnDefinition,
} from "../src/generated/loop/v1/research_common_pb.js";
import {
  canonicalProtocolSelectionBytes,
  factorSpecIdentitySha256,
  type JobValidationCode,
  JobValidationError,
  protocolSelectionSha256,
  validateHoldoutBacktestPlanEntryBinding,
  validateJobRecord,
  validateJobSpecification,
} from "../src/job.js";
import { validateJobWireDispatchCandidate } from "../src/runtime-validation.js";

const FACTOR_ID = "sha256:3a4f28e6e918ec379af280264bf314eaac843a0aded6fefcdcdd06cf925b895a";
const EXPRESSION_ID = "sha256:2b93fad0265af4e02df2b2dce69d3b5a221bfd553aacaa52bb652666374d7cb3";
const OTHER_FACTOR_ID = `sha256:${"b".repeat(64)}`;
const vectorText = readFileSync(
  new URL("../../../tests/contracts/job_record_vectors.tsv", import.meta.url),
  "utf8",
);
const protocolGoldenText = readFileSync(
  new URL("../../../tests/contracts/protocol_selection_golden.tsv", import.meta.url),
  "utf8",
);
const protocolNegativeText = readFileSync(
  new URL("../../../tests/contracts/protocol_selection_negative.tsv", import.meta.url),
  "utf8",
);
const holdoutBindingText = readFileSync(
  new URL("../../../tests/contracts/holdout_job_binding_vectors.tsv", import.meta.url),
  "utf8",
);
const holdoutGrantLifetimeText = readFileSync(
  new URL("../../../tests/contracts/holdout_grant_lifetime_vectors.tsv", import.meta.url),
  "utf8",
);
const holdoutGolden = JSON.parse(
  readFileSync(
    new URL("../../../tests/contracts/holdout_identity_golden.json", import.meta.url),
    "utf8",
  ),
) as HoldoutGoldenFixture;

interface HoldoutGoldenFixture {
  readonly trusted_backtest_schema_sha256: string;
  readonly backtest_artifacts: readonly {
    readonly sha256: string;
    readonly content: string;
  }[];
  readonly periods: readonly {
    readonly canonical_json: string;
    readonly canonical_sha256: string;
    readonly holdout_period_id: string;
  }[];
  readonly plans: readonly {
    readonly canonical_json: string;
    readonly plan_sha256: string;
    readonly holdout_evaluation_plan_id: string;
  }[];
}

interface Vector {
  readonly name: string;
  readonly expected: "accept" | JobValidationCode;
  readonly kind: string;
  readonly input: string;
  readonly state: string;
  readonly lease: string;
  readonly outcome: string;
  readonly attempt: number;
  readonly mutation: string;
}

describe("JobRecord structural validation", () => {
  it("executes every row in the shared fail-closed matrix", () => {
    const shared = vectors();
    expect(shared).toHaveLength(109);
    for (const vector of shared) {
      const value = record(vector);
      if (vector.expected === "accept") {
        expect(validateJobRecord(value), vector.name).toMatchObject({
          kind: value.specification?.kind,
          state: value.state,
        });
      } else {
        try {
          validateJobRecord(value);
          throw new Error(`expected ${vector.name} to fail`);
        } catch (error) {
          expect(error, vector.name).toBeInstanceOf(JobValidationError);
          expect((error as JobValidationError).code, vector.name).toBe(vector.expected);
        }
      }
    }
  });
});

describe("ProtocolSelection canonical identity", () => {
  it("matches the shared producer golden", () => {
    const fields = protocolGoldenText.trimEnd().split("\n")[1]?.split("\t");
    expect(fields).toHaveLength(3);
    const selection = protocolSelection();
    expect(new TextDecoder().decode(canonicalProtocolSelectionBytes(selection))).toBe(fields?.[1]);
    expect(Buffer.from(protocolSelectionSha256(selection)).toString("hex")).toBe(fields?.[2]);
  });

  it("rejects every shared malformed selection", () => {
    for (const line of protocolNegativeText.trimEnd().split("\n").slice(1)) {
      const [name, expected, mutation] = line.split("\t");
      const specification = validSpecification({
        name: name ?? "selection",
        expected: (expected ?? "invalid_protocol_selection") as Vector["expected"],
        kind: "discovery",
        input: "discovery",
        state: "queued",
        lease: "absent",
        outcome: "absent",
        attempt: 0,
        mutation: "none",
      });
      const selection = specification.protocolSelection;
      if (selection === undefined) throw new Error("missing selection fixture");
      switch (mutation) {
        case "digest_mismatch":
          if (selection.selectionSha256) selection.selectionSha256.value[0] ^= 0xff;
          break;
        case "unsorted_features":
          selection.enabledFeatures.reverse();
          break;
        case "duplicate_features":
          selection.enabledFeatures[1] = selection.enabledFeatures[0] as string;
          break;
        case "zero_limit":
          if (selection.effectiveLimits) selection.effectiveLimits.maximumAstNodes = 0;
          break;
        case "future_timestamp":
          selection.selectedAt = timestamp(6);
          selection.selectionSha256 = digestFromBytes(protocolSelectionSha256(selection));
          break;
        case "invalid_package":
          selection.selectedPackage = "Loop.v1";
          break;
        case "zero_package_version":
          selection.selectedPackage = "loop.v0";
          break;
        case "leading_zero_package_version":
          selection.selectedPackage = "loop.v01";
          break;
        default:
          throw new Error(`unknown selection mutation ${mutation}`);
      }
      expect(() => validateJobSpecification(specification), name).toThrowError(
        expect.objectContaining({ code: expected }),
      );
    }
  });
});

describe("holdout job resolved plan binding", () => {
  it("executes every shared plan-entry binding vector", () => {
    for (const line of holdoutBindingText.trimEnd().split("\n").slice(1)) {
      const [name, expected, mutation] = line.split("\t");
      const fixture = holdoutBindingFixture();
      switch (mutation) {
        case "none":
          break;
        case "plan_identity": {
          const identity = create(HoldoutEvaluationPlanIdSchema, { value: digestId(99) });
          fixture.input.holdoutEvaluationPlanId = identity;
          if (fixture.input.consumedGrant) {
            fixture.input.consumedGrant.holdoutEvaluationPlanId = identity;
          }
          break;
        }
        case "entry_factor":
          if (fixture.input.frozenBacktestSpec) {
            fixture.input.frozenBacktestSpec.factorSpecId = factorId(OTHER_FACTOR_ID);
          }
          break;
        case "entry_budget":
          if (fixture.input.budget) fixture.input.budget.maximumSteps = 41;
          break;
        case "entry_artifact":
          if (fixture.input.frozenBacktestSpec) {
            fixture.input.frozenBacktestSpec.canonicalSpecSha256 = digest(99);
          }
          break;
        case "canonical_plan_tampered":
          fixture.canonicalPlanBytes = appendSpace(fixture.canonicalPlanBytes);
          break;
        case "canonical_period_tampered":
          fixture.canonicalPeriodBytes = appendSpace(fixture.canonicalPeriodBytes);
          break;
        default:
          throw new Error(`unknown holdout mutation ${mutation}`);
      }
      if (expected === "accept") {
        expect(
          () =>
            validateHoldoutBacktestPlanEntryBinding(
              fixture.input,
              timestamp(5),
              fixture.canonicalPeriodBytes,
              fixture.canonicalPlanBytes,
              fixture.trustedBacktestSchemaSha256,
              fixture.resolvedBacktestArtifacts,
            ),
          name,
        ).not.toThrow();
      } else {
        expect(
          () =>
            validateHoldoutBacktestPlanEntryBinding(
              fixture.input,
              timestamp(5),
              fixture.canonicalPeriodBytes,
              fixture.canonicalPlanBytes,
              fixture.trustedBacktestSchemaSha256,
              fixture.resolvedBacktestArtifacts,
            ),
          name,
        ).toThrowError(expect.objectContaining({ code: expected }));
      }
    }
  });

  it("enforces shared grant lifetime boundaries in the binder and wire candidate", () => {
    const rows = holdoutGrantLifetimeText.trimEnd().split("\n").slice(1);
    expect(rows).toHaveLength(6);
    for (const line of rows) {
      const fields = line.split("\t");
      expect(fields, line).toHaveLength(5);
      const [name, expectedBinder, expectedWire, seconds, nanos] = fields;
      if (
        name === undefined ||
        expectedBinder === undefined ||
        expectedWire === undefined ||
        seconds === undefined ||
        nanos === undefined
      ) {
        throw new Error(`invalid grant lifetime vector: ${line}`);
      }
      const submittedAt = timestamp(Number(seconds), Number(nanos));
      const fixture = holdoutBindingFixture();
      const bind = () =>
        validateHoldoutBacktestPlanEntryBinding(
          fixture.input,
          submittedAt,
          fixture.canonicalPeriodBytes,
          fixture.canonicalPlanBytes,
          fixture.trustedBacktestSchemaSha256,
          fixture.resolvedBacktestArtifacts,
        );
      if (expectedBinder === "accept") {
        expect(bind, `${name} binder`).not.toThrow();
      } else {
        expect(bind, `${name} binder`).toThrowError(
          expect.objectContaining({ code: expectedBinder }),
        );
      }

      const specification = validSpecification({
        name,
        expected: "accept",
        kind: "holdout_backtest",
        input: "holdout_backtest",
        state: "queued",
        lease: "absent",
        outcome: "absent",
        attempt: 0,
        mutation: "none",
      });
      specification.submittedAt = submittedAt;
      const validateWireCandidate = () =>
        validateJobWireDispatchCandidate(specification, new Set([JobKind.HOLDOUT_BACKTEST]));
      if (expectedWire === "accept") {
        expect(validateWireCandidate(), name).toBe(JobKind.HOLDOUT_BACKTEST);
      } else {
        expect(validateWireCandidate, `${name} wire candidate`).toThrowError(
          expect.objectContaining({ code: expectedWire }),
        );
      }
    }
  });
});

function vectors(): Vector[] {
  return vectorText
    .trimEnd()
    .split("\n")
    .slice(1)
    .map((line) => {
      const values = line.split("\t");
      if (values.length !== 9) throw new Error(`invalid job vector: ${line}`);
      const [
        name,
        expected,
        kindName,
        inputName,
        stateName,
        lease,
        outcomeName,
        attempt,
        mutation,
      ] = values;
      if (
        [
          name,
          expected,
          kindName,
          inputName,
          stateName,
          lease,
          outcomeName,
          attempt,
          mutation,
        ].some((value) => value === undefined)
      ) {
        throw new Error(`incomplete job vector: ${line}`);
      }
      return {
        name: name as string,
        expected: expected as Vector["expected"],
        kind: kindName as string,
        input: inputName as string,
        state: stateName as string,
        lease: lease as string,
        outcome: outcomeName as string,
        attempt: Number.parseInt(attempt as string, 10),
        mutation: mutation as string,
      };
    });
}

function record(vector: Vector): JobRecord {
  const specification = vector.kind === "missing" ? undefined : validSpecification(vector);
  const enforcedBudget = specificationBudget(specification);
  const value = create(JobRecordSchema, {
    specification,
    state: state(vector.state),
    revision: 1n,
    attempt: vector.attempt,
    activeLease: vector.lease === "present" ? validLease() : undefined,
    outcome: outcome(vector.outcome, vector.attempt, enforcedBudget),
    updatedAt: timestamp(20),
  });
  mutate(value, vector.mutation);
  return value;
}

function validSpecification(vector: Vector) {
  const selection = protocolSelection();
  selection.selectionSha256 = digestFromBytes(protocolSelectionSha256(selection));
  return create(JobSpecificationSchema, {
    jobId: create(JobIdSchema, { value: "job.01" }),
    runId: create(RunIdSchema, { value: "run.01" }),
    kind: kind(vector.kind),
    input: input(vector.input),
    submittedAt: timestamp(5),
    submittedBy: actor(),
    idempotencyKey: create(IdempotencyKeySchema, { value: "idem.01" }),
    correlationId: create(CorrelationIdSchema, { value: "corr.01" }),
    causationId: create(CausationIdSchema, { value: "cause.01" }),
    protocolSelection: selection,
  });
}

function mutate(record: JobRecord, mutation: string): void {
  const specification = record.specification;
  const lease = record.activeLease;
  switch (mutation) {
    case "none":
      return;
    case "zero_revision":
      record.revision = 0n;
      return;
    case "missing_spec_job_id":
      if (specification) specification.jobId = undefined;
      return;
    case "malformed_spec_job_id":
      if (specification) specification.jobId = create(JobIdSchema, { value: "bad id" });
      return;
    case "lease_job_mismatch":
      if (lease) lease.jobId = create(JobIdSchema, { value: "job.02" });
      return;
    case "lease_future_revision":
      if (lease) lease.acquiredRevision = 2n;
      return;
    case "lease_zero_revision":
      if (lease) lease.acquiredRevision = 0n;
      return;
    case "lease_missing_id":
      if (lease) lease.leaseId = undefined;
      return;
    case "lease_missing_job_id":
      if (lease) lease.jobId = undefined;
      return;
    case "lease_missing_owner":
      if (lease) lease.owner = undefined;
      return;
    case "lease_unknown_owner_kind":
      if (lease?.owner) lease.owner.kind = 999 as ActorKind;
      return;
    case "lease_missing_timestamp":
      if (lease) lease.heartbeatAt = undefined;
      return;
    case "lease_invalid_order":
      if (lease) lease.heartbeatAt = timestamp(30);
      return;
    case "factor_input_id_missing":
      setInputFactorId(record, undefined);
      return;
    case "factor_input_id_malformed":
      setInputFactorId(record, "SHA256:bad");
      return;
    case "rejection_id_missing":
      rejection(record).factorSpecId = undefined;
      return;
    case "rejection_id_malformed":
      rejection(record).factorSpecId = factorId("SHA256:bad");
      return;
    case "rejection_id_mismatch":
      rejection(record).factorSpecId = factorId(OTHER_FACTOR_ID);
      return;
    case "rejection_unspecified_code":
      rejection(record).code = FactorRejectionCode.UNSPECIFIED;
      return;
    case "rejection_unknown_code":
      rejection(record).code = 999 as FactorRejectionCode;
      return;
    case "rejection_missing_reason":
      rejection(record).reason = " ";
      return;
    case "rejection_control_reason":
      rejection(record).reason = "forged\nrecord";
      return;
    case "rejection_missing_timestamp":
      rejection(record).rejectedAt = undefined;
      return;
    case "rejection_invalid_timestamp":
      rejection(record).rejectedAt = create(TimestampSchema, { nanos: 1_000_000_000 });
      return;
    case "rejection_too_many_evidence":
      rejection(record).evidence = Array.from({ length: 65 }, () => create(ArtifactRefSchema));
      return;
    case "success_too_many_outputs":
      success(record).outputs = Array.from({ length: 65 }, () => create(ArtifactRefSchema));
      return;
    case "infra_missing_error":
      infrastructure(record).error = undefined;
      return;
    case "infra_unspecified_category":
      if (infrastructure(record).error)
        infrastructure(record).error.category = ErrorCategory.UNSPECIFIED;
      return;
    case "infra_unknown_category":
      if (infrastructure(record).error)
        infrastructure(record).error.category = 999 as ErrorCategory;
      return;
    case "infra_missing_code":
      if (infrastructure(record).error) infrastructure(record).error.code = "";
      return;
    case "infra_missing_message":
      if (infrastructure(record).error) infrastructure(record).error.message = "";
      return;
    case "infra_attempt_mismatch":
      infrastructure(record).attempt = 1;
      return;
    case "infra_missing_timestamp":
      infrastructure(record).failedAt = undefined;
      return;
    case "infra_too_many_details":
      if (infrastructure(record).error)
        infrastructure(record).error.details = Array.from({ length: 33 }, () =>
          create(ErrorDetailSchema),
        );
      return;
    case "infra_invalid_detail":
      infrastructure(record).error?.details.push(create(ErrorDetailSchema));
      return;
    case "cancellation_missing_reason":
      cancellation(record).reason = "";
      return;
    case "cancellation_missing_actor":
      cancellation(record).cancelledBy = undefined;
      return;
    case "cancellation_missing_timestamp":
      cancellation(record).cancelledAt = undefined;
      return;
    case "budget_missing_limit":
      budget(record).exhaustedLimit = "";
      return;
    case "budget_missing_object":
      budget(record).enforcedBudget = undefined;
      return;
    case "budget_missing_timestamp":
      budget(record).exhaustedAt = undefined;
      return;
    case "lease_predates_submission":
      if (lease) lease.issuedAt = timestamp(4);
      return;
    case "lease_expired_at_update":
      if (lease) lease.expiresAt = timestamp(20);
      return;
    case "submitted_actor_empty_subject":
      if (specification?.submittedBy) specification.submittedBy.authenticatedSubject = "";
      return;
    case "submitted_actor_control_subject":
      if (specification?.submittedBy)
        specification.submittedBy.authenticatedSubject = "subject\nforged";
      return;
    case "policy_revision_zero":
      discoveryPolicy(record).revision = "0";
      return;
    case "policy_revision_leading_zero":
      discoveryPolicy(record).revision = "01";
      return;
    case "policy_revision_sign":
      discoveryPolicy(record).revision = "+1";
      return;
    case "factor_expression_hash_mismatch": {
      const expression = factorSpec(record).expression;
      if (expression) {
        const mutated = new Uint8Array(expression.canonicalJson.byteLength + 1);
        mutated.set(expression.canonicalJson);
        mutated[mutated.length - 1] = 32;
        expression.canonicalJson = mutated;
      }
      return;
    }
    case "factor_ast_canonical_mismatch": {
      const root = factorSpec(record).expression?.ast?.root;
      if (root?.node.case !== "field") throw new Error("fixture root must be a field");
      root.node.value.field = "market.open";
      return;
    }
    case "factor_spec_hash_mismatch":
      factorSpec(record).factorSpecId = factorId(OTHER_FACTOR_ID);
      return;
    case "factor_unspecified_direction":
      factorSpec(record).direction = FactorDirection.UNSPECIFIED;
      return;
    case "protocol_selection_digest_mismatch": {
      const value = specification?.protocolSelection?.selectionSha256?.value;
      if (value) value[0] = (value[0] ?? 0) ^ 0xff;
      return;
    }
    case "holdout_plan_digest_mismatch": {
      if (specification?.input.case === "holdoutBacktest") {
        const value = specification.input.value.evaluationPlanSha256?.value;
        if (value) value[0] = (value[0] ?? 0) ^ 0xff;
      }
      return;
    }
    case "holdout_unknown_sample_role":
      if (specification?.input.case === "holdoutBacktest") {
        const sample = specification.input.value.frozenBacktestSpec?.sample;
        if (sample) sample.role = 999 as SampleRole;
      }
      return;
    case "holdout_unknown_return_definition":
      if (specification?.input.case === "holdoutBacktest") {
        const frozen = specification.input.value.frozenBacktestSpec;
        if (frozen) frozen.returnDefinition = 999 as ReturnDefinition;
      }
      return;
    case "model_invalid_plugin_version":
      if (specification?.input.case === "discovery" && specification.input.value.makerModel) {
        specification.input.value.makerModel.providerPluginVersion = "1 bad";
      }
      return;
    case "dataset_snapshot_id_malformed":
      if (specification?.input.case === "discovery") {
        const snapshot = specification.input.value.dataset?.snapshotIds[0];
        if (snapshot) snapshot.value = "bad id";
      }
      return;
    case "factor_ast_empty_root": {
      const root = factorSpec(record).expression?.ast?.root;
      if (root) root.node = { case: undefined };
      return;
    }
    case "holdout_submission_before_grant_issue":
      if (
        specification?.input.case === "holdoutBacktest" &&
        specification.input.value.consumedGrant
      ) {
        specification.input.value.consumedGrant.issuedAt = timestamp(6);
      }
      return;
    case "holdout_submission_at_grant_expiry":
      if (
        specification?.input.case === "holdoutBacktest" &&
        specification.input.value.consumedGrant
      ) {
        specification.input.value.consumedGrant.expiresAt = timestamp(5);
      }
      return;
    case "holdout_submission_after_grant_expiry":
      if (
        specification?.input.case === "holdoutBacktest" &&
        specification.input.value.consumedGrant
      ) {
        specification.submittedAt = timestamp(6);
        specification.input.value.consumedGrant.expiresAt = timestamp(5);
      }
      return;
    default:
      throw new Error(`unknown mutation ${mutation}`);
  }
}

function input(value: string): ReturnType<typeof create<typeof JobSpecificationSchema>>["input"] {
  switch (value) {
    case "missing":
      return { case: undefined };
    case "discovery":
      return {
        case: "discovery",
        value: create(DiscoveryJobInputSchema, {
          dataset: developmentDataset(),
          researchPolicy: policy("policy.research"),
          makerModel: model("resolution.maker"),
          checkerModel: model("resolution.checker"),
          budget: validBudget(),
          maximumCandidates: 40,
        }),
      };
    case "factor_evaluation":
      return {
        case: "factorEvaluation",
        value: create(FactorEvaluationJobInputSchema, {
          factor: validFactorSpec(),
          dataset: developmentDataset(),
          budget: validBudget(),
        }),
      };
    case "backtest":
      return {
        case: "backtest",
        value: create(BacktestJobInputSchema, {
          budget: validBudget(),
          factorSpecId: factorId(FACTOR_ID),
          dataset: developmentDataset(),
          returnDefinition: ReturnDefinition.SIMPLE_NAV_RETURN,
          provenance: provenance(),
          deterministicSeed: digest(9),
        }),
      };
    case "reconciliation":
      return {
        case: "reconciliation",
        value: create(ReconciliationJobInputSchema, {
          primaryBacktestId: create(BacktestIdSchema, { value: "backtest.primary" }),
          independentBacktestId: create(BacktestIdSchema, { value: "backtest.independent" }),
          reconciliationPolicy: policy("policy.reconciliation"),
          budget: validBudget(),
        }),
      };
    case "holdout_backtest":
      return {
        case: "holdoutBacktest",
        value: holdoutInput(),
      };
    case "artifact":
      return {
        case: "artifact",
        value: create(ArtifactJobInputSchema, {
          input: artifact(),
          policy: policy("policy.artifact"),
          budget: validBudget(),
        }),
      };
    default:
      throw new Error(`unknown fixture input ${value}`);
  }
}

function outcome(value: string, attempt: number, enforcedBudget: JobBudget | undefined) {
  switch (value) {
    case "absent":
      return undefined;
    case "empty":
      return create(JobOutcomeSchema);
    case "success":
      return create(JobOutcomeSchema, {
        outcome: { case: "success", value: create(JobSuccessSchema) },
      });
    case "factor_rejection":
      return create(JobOutcomeSchema, {
        outcome: {
          case: "factorRejection",
          value: create(FactorRejectionSchema, {
            factorSpecId: factorId(FACTOR_ID),
            code: FactorRejectionCode.PERFORMANCE,
            reason: "fails frozen performance threshold",
            rejectedAt: timestamp(20),
          }),
        },
      });
    case "infrastructure_failure":
      return create(JobOutcomeSchema, {
        outcome: {
          case: "infrastructureFailure",
          value: create(InfrastructureFailureSchema, {
            error: create(ServiceErrorSchema, {
              category: ErrorCategory.DEPENDENCY,
              code: "dataset_unavailable",
              message: "dataset unavailable",
            }),
            attempt,
            failedAt: timestamp(20),
          }),
        },
      });
    case "cancellation":
      return create(JobOutcomeSchema, {
        outcome: {
          case: "cancellation",
          value: create(JobCancellationSchema, {
            reason: "cancelled by operator",
            cancelledBy: actor(),
            cancelledAt: timestamp(20),
          }),
        },
      });
    case "budget_exhaustion":
      return create(JobOutcomeSchema, {
        outcome: {
          case: "budgetExhaustion",
          value: create(BudgetExhaustionSchema, {
            exhaustedLimit: "maximum_steps",
            enforcedBudget,
            exhaustedAt: timestamp(20),
          }),
        },
      });
    default:
      throw new Error(`unknown fixture outcome ${value}`);
  }
}

function validBudget(): JobBudget {
  return create(JobBudgetSchema, {
    maximumSteps: 40,
    maximumInputTokens: 100_000n,
    maximumOutputTokens: 20_000n,
    maximumCost: money("12.5"),
    maximumWallTime: create(DurationSchema, { seconds: 3_600n }),
  });
}

function money(amount: string) {
  return create(MoneySchema, {
    amount: create(ExactDecimalSchema, { value: amount }),
    currencyCode: "USD",
  });
}

function digest(byte: number) {
  return create(Sha256DigestSchema, { value: new Uint8Array(32).fill(byte) });
}

function digestFromBytes(value: Uint8Array) {
  return create(Sha256DigestSchema, { value });
}

function digestId(byte: number): string {
  return `sha256:${byte.toString(16).padStart(2, "0").repeat(32)}`;
}

function policy(value: string) {
  return create(PolicyReferenceSchema, {
    policyId: create(PolicyIdSchema, { value }),
    revision: "1",
    sha256: digest(6),
  });
}

function validFactorSpec() {
  const expressionId = create(FactorExpressionIdSchema, { value: EXPRESSION_ID });
  const factor = create(FactorSpecSchema, {
    expressionId,
    expression: create(CanonicalFactorAstSchema, {
      expressionId,
      ast: create(FactorAstSchema, {
        schemaVersion: 1,
        root: create(FactorAstNodeSchema, {
          node: {
            case: "field",
            value: create(FieldReferenceSchema, { field: "market.close" }),
          },
        }),
      }),
      canonicalizationProfile: "loop.factor-ast/v1",
      canonicalJson: new TextEncoder().encode('{"node":"field","field":"market.close"}'),
    }),
    direction: FactorDirection.HIGHER_IS_BETTER,
    frozenPolicy: create(FrozenResearchPolicyReferenceSchema, {
      universePolicy: policy("policy.universe"),
      dataPolicy: policy("policy.data"),
      calendarPolicy: policy("policy.calendar"),
      preprocessPolicy: policy("policy.preprocess"),
      neutralizationPolicy: policy("policy.neutralization"),
      portfolioPolicy: policy("policy.portfolio"),
      executionPolicy: policy("policy.execution"),
      costPolicy: policy("policy.cost"),
      evaluationPolicy: policy("policy.evaluation"),
    }),
    operatorRegistrySha256: digest(2),
  });
  expect(`sha256:${Buffer.from(factorSpecIdentitySha256(factor)).toString("hex")}`).toBe(FACTOR_ID);
  factor.factorSpecId = factorId(FACTOR_ID);
  return factor;
}

function developmentDataset() {
  return create(DevelopmentDatasetReferenceSchema, {
    snapshotIds: [create(SnapshotIdSchema, { value: "snapshot.dev.01" })],
    manifestSha256: digest(7),
  });
}

function provenance() {
  return create(ResearchProvenanceFingerprintSchema, {
    sourceCodeSha256: digest(1),
    operatorRegistrySha256: digest(2),
    configurationSha256: digest(3),
    dataManifestSha256: digest(4),
    tradingCalendarSha256: digest(5),
    environmentSha256: digest(6),
  });
}

function model(resolution: string) {
  return create(ModelResolutionSnapshotSchema, {
    resolutionId: create(ModelResolutionIdSchema, { value: resolution }),
    providerId: create(ProviderIdSchema, { value: "provider.fixture" }),
    modelId: create(ModelIdSchema, { value: "model.fixture" }),
    requestedAlias: "fixture-model",
    protocolFamily: ModelProtocolFamily.OPENAI_RESPONSES,
    capabilities: create(ModelCapabilitiesSchema, {
      contextWindowTokens: 100_000n,
      maximumOutputTokens: 20_000n,
    }),
    pricing: create(ModelPricingSchema, {
      inputPerMillionTokens: money("1"),
      outputPerMillionTokens: money("2"),
      cachedInputPerMillionTokens: money("0.5"),
    }),
    capabilitySha256: digest(11),
    catalogSha256: digest(12),
    resolvedAt: timestamp(1),
    providerPluginName: "fixture-provider",
    providerPluginVersion: "1.0.0",
    providerPluginSha256: digest(13),
    snapshotSha256: digest(14),
  });
}

function protocolSelection() {
  return create(ProtocolSelectionSnapshotSchema, {
    selectedPackage: "loop.v1",
    enabledFeatures: ["jobs.envelope.v1", "jobs.kind-input.v1"],
    effectiveLimits: create(ProtocolLimitsSchema, {
      maximumUnaryBytes: 4_194_304n,
      maximumStreamEventBytes: 1_048_576n,
      maximumCanonicalAstBytes: 262_144n,
      maximumAstNodes: 4_096,
      maximumAstDepth: 64,
      maximumPageRecords: 500,
      maximumIdentityBytes: 128,
      maximumArtifactUriBytes: 2_048,
    }),
    serverBuildVersion: "0.2.0-alpha.1",
    serverBuildSha256: digest(21),
    schemaDescriptorSha256: digest(22),
    selectedAt: timestamp(2),
    clientBuildVersion: "0.2.0-alpha.1",
    clientBuildSha256: digest(23),
  });
}

function artifact() {
  const id = digestId(24);
  return create(ArtifactRefSchema, {
    artifactId: create(ArtifactIdSchema, { value: id }),
    uri: `artifact://sha256/${id.slice(7)}`,
    sha256: digest(24),
    schema: create(ArtifactSchemaReferenceSchema, {
      name: "loop.report",
      version: 1,
      schemaSha256: digest(25),
    }),
    mediaType: "application/json",
    byteSize: 1n,
    createdAt: timestamp(1),
  });
}

function holdoutInput() {
  const planId = digestId(32);
  return create(HoldoutBacktestJobInputSchema, {
    consumedGrant: create(HoldoutGrantReferenceSchema, {
      holdoutGrantId: create(HoldoutGrantIdSchema, { value: "grant.01" }),
      holdoutPeriodId: create(HoldoutPeriodIdSchema, { value: digestId(31) }),
      freezeManifestSha256: digest(33),
      issuedAt: timestamp(4),
      expiresAt: timestamp(100),
      holdoutEvaluationPlanId: create(HoldoutEvaluationPlanIdSchema, { value: planId }),
      evaluationPlanSha256: digest(34),
      evaluationPlanEntryCount: 2,
      canonicalPeriodSha256: digest(31),
    }),
    consumedGrantRevision: 1n,
    frozenBacktestSpec: create(BacktestSpecSchema, {
      backtestId: create(BacktestIdSchema, { value: "backtest.holdout.01" }),
      schemaVersion: 1,
      factorSpecId: factorId(FACTOR_ID),
      snapshotIds: [create(SnapshotIdSchema, { value: digestId(35) })],
      sample: create(SampleWindowSchema, {
        role: SampleRole.SECOND_LOCKED_HISTORICAL_HOLDOUT,
        startInclusive: create(CivilDateSchema, { year: 2025, month: 1, day: 1 }),
        endInclusive: create(CivilDateSchema, { year: 2026, month: 8, day: 31 }),
      }),
      returnDefinition: ReturnDefinition.SIMPLE_NAV_RETURN,
      provenance: provenance(),
      canonicalSpecSha256: digest(36),
      createdAt: timestamp(3),
      deterministicSeed: digest(37),
    }),
    budget: validBudget(),
    jobBatchId: create(JobBatchIdSchema, { value: "batch.01" }),
    holdoutEvaluationPlanId: create(HoldoutEvaluationPlanIdSchema, { value: planId }),
    evaluationPlanSha256: digest(34),
    evaluationPlanEntryIndex: 1,
  });
}

function holdoutBindingFixture() {
  const period = holdoutGolden.periods[0];
  const plan = holdoutGolden.plans[0];
  if (period === undefined || plan === undefined) throw new Error("missing holdout golden fixture");
  const planValue = JSON.parse(plan.canonical_json) as {
    entries: Array<{
      factor_spec_id: string;
      backtest_spec_artifact: { sha256: string };
    }>;
  };
  const firstEntry = planValue.entries[0];
  if (firstEntry === undefined) throw new Error("missing holdout plan entry");
  const input = holdoutInput();
  const grant = input.consumedGrant;
  const frozen = input.frozenBacktestSpec;
  if (grant === undefined || frozen === undefined) throw new Error("incomplete holdout input");
  grant.holdoutPeriodId = create(HoldoutPeriodIdSchema, { value: period.holdout_period_id });
  grant.canonicalPeriodSha256 = digestFromBytes(rawDigest(period.canonical_sha256));
  grant.holdoutEvaluationPlanId = create(HoldoutEvaluationPlanIdSchema, {
    value: plan.holdout_evaluation_plan_id,
  });
  grant.evaluationPlanSha256 = digestFromBytes(rawDigest(plan.plan_sha256));
  grant.evaluationPlanEntryCount = 2;
  input.holdoutEvaluationPlanId = create(HoldoutEvaluationPlanIdSchema, {
    value: plan.holdout_evaluation_plan_id,
  });
  input.evaluationPlanSha256 = digestFromBytes(rawDigest(plan.plan_sha256));
  frozen.factorSpecId = factorId(firstEntry.factor_spec_id);
  frozen.canonicalSpecSha256 = digestFromBytes(rawDigest(firstEntry.backtest_spec_artifact.sha256));
  if (frozen.sample) {
    frozen.sample.role = SampleRole.FIRST_LOCKED_CONFIRMATION;
    frozen.sample.startInclusive = create(CivilDateSchema, { year: 2021, month: 1, day: 1 });
    frozen.sample.endInclusive = create(CivilDateSchema, { year: 2024, month: 12, day: 31 });
  }
  const encoder = new TextEncoder();
  return {
    input,
    canonicalPeriodBytes: encoder.encode(period.canonical_json),
    canonicalPlanBytes: encoder.encode(plan.canonical_json),
    trustedBacktestSchemaSha256: rawDigest(holdoutGolden.trusted_backtest_schema_sha256),
    resolvedBacktestArtifacts: new Map(
      holdoutGolden.backtest_artifacts.map((artifact) => [
        artifact.sha256,
        encoder.encode(artifact.content),
      ]),
    ),
  };
}

function rawDigest(value: string): Uint8Array {
  return new Uint8Array(Buffer.from(value.slice(7), "hex"));
}

function appendSpace(value: Uint8Array): Uint8Array {
  const result = new Uint8Array(value.byteLength + 1);
  result.set(value);
  result[result.byteLength - 1] = 32;
  return result;
}

function specificationBudget(specification: ReturnType<typeof validSpecification> | undefined) {
  return specification?.input.case === undefined ? undefined : specification.input.value.budget;
}

function validLease() {
  return create(JobLeaseSchema, {
    leaseId: create(LeaseIdSchema, { value: "lease.01" }),
    jobId: create(JobIdSchema, { value: "job.01" }),
    owner: actor(),
    acquiredRevision: 1n,
    issuedAt: timestamp(10),
    heartbeatAt: timestamp(11),
    expiresAt: timestamp(30),
  });
}
function actor() {
  return create(ActorSchema, {
    actorId: create(ActorIdSchema, { value: "worker.01" }),
    kind: ActorKind.SERVICE,
    displayName: "Loop worker",
    authenticatedSubject: "service:loop-worker",
  });
}
function factorId(value: string) {
  return create(FactorSpecIdSchema, { value });
}
function timestamp(seconds: number, nanos = 0) {
  return create(TimestampSchema, { seconds: BigInt(seconds), nanos });
}

function setInputFactorId(record: JobRecord, value: string | undefined): void {
  const inputValue = record.specification?.input;
  const id = value === undefined ? undefined : factorId(value);
  if (inputValue?.case === "factorEvaluation" && inputValue.value.factor)
    inputValue.value.factor.factorSpecId = id;
  else if (inputValue?.case === "backtest") inputValue.value.factorSpecId = id;
  else if (inputValue?.case === "holdoutBacktest" && inputValue.value.frozenBacktestSpec)
    inputValue.value.frozenBacktestSpec.factorSpecId = id;
  else throw new Error("mutation requires factor input");
}

function rejection(record: JobRecord) {
  const outcomeValue = record.outcome?.outcome;
  if (outcomeValue?.case !== "factorRejection") throw new Error("expected rejection");
  return outcomeValue.value;
}
function infrastructure(record: JobRecord) {
  const outcomeValue = record.outcome?.outcome;
  if (outcomeValue?.case !== "infrastructureFailure") throw new Error("expected failure");
  return outcomeValue.value;
}
function success(record: JobRecord) {
  const outcomeValue = record.outcome?.outcome;
  if (outcomeValue?.case !== "success") throw new Error("expected success");
  return outcomeValue.value;
}
function cancellation(record: JobRecord) {
  const outcomeValue = record.outcome?.outcome;
  if (outcomeValue?.case !== "cancellation") throw new Error("expected cancellation");
  return outcomeValue.value;
}
function budget(record: JobRecord) {
  const outcomeValue = record.outcome?.outcome;
  if (outcomeValue?.case !== "budgetExhaustion") throw new Error("expected budget exhaustion");
  return outcomeValue.value;
}

function discoveryPolicy(record: JobRecord) {
  const input = record.specification?.input;
  if (input?.case !== "discovery" || input.value.researchPolicy === undefined) {
    throw new Error("mutation requires discovery input");
  }
  return input.value.researchPolicy;
}

function factorSpec(record: JobRecord) {
  const input = record.specification?.input;
  if (input?.case !== "factorEvaluation" || input.value.factor === undefined) {
    throw new Error("mutation requires factor-evaluation input");
  }
  return input.value.factor;
}

function kind(value: string): JobKind {
  const values: Record<string, JobKind> = {
    unspecified: JobKind.UNSPECIFIED,
    unknown: 999 as JobKind,
    discovery: JobKind.DISCOVERY,
    factor_evaluation: JobKind.FACTOR_EVALUATION,
    backtest: JobKind.BACKTEST,
    independent_reconciliation: JobKind.INDEPENDENT_RECONCILIATION,
    report: JobKind.REPORT,
    prospective_observation: JobKind.PROSPECTIVE_OBSERVATION,
    holdout_backtest: JobKind.HOLDOUT_BACKTEST,
  };
  const result = values[value];
  if (result === undefined) throw new Error(`unknown kind ${value}`);
  return result;
}
function state(value: string): JobState {
  const values: Record<string, JobState> = {
    unspecified: JobState.UNSPECIFIED,
    unknown: 999 as JobState,
    queued: JobState.QUEUED,
    leased: JobState.LEASED,
    running: JobState.RUNNING,
    succeeded: JobState.SUCCEEDED,
    factor_rejected: JobState.FACTOR_REJECTED,
    infrastructure_failed: JobState.INFRASTRUCTURE_FAILED,
    cancelled: JobState.CANCELLED,
    budget_exhausted: JobState.BUDGET_EXHAUSTED,
  };
  const result = values[value];
  if (result === undefined) throw new Error(`unknown state ${value}`);
  return result;
}
