import type { Timestamp } from "@bufbuild/protobuf/wkt";

import { DiscoveryJobStatus } from "./generated/loop/discovery/v1/service_pb.js";
import { ResearchJobStatus } from "./generated/loop/research/v1/service_pb.js";
import {
  RoundDisposition,
  RunStage,
  RunStatus,
  type RunView,
} from "./generated/loop/runs/v1/service_pb.js";
import type { Money } from "./generated/loop/v1/common_pb.js";

const MAX_SIGNED = 9_223_372_036_854_775_807n;
const MAX_WALL = 2_592_000n;

/** Version selection is explicit and must be checked before any side effect. */
export function validate_workflow(version: number): void {
  if (version !== 0 && version !== 2 && version !== 3) invalid();
}

/** Shape checks only: transport authority and historical spend are server gates. */
export function validate_view(view: RunView): void {
  identity(view.runId?.value);
  if (
    ![
      RunStatus.ACTIVE,
      RunStatus.COMPLETED,
      RunStatus.BUDGET_EXHAUSTED,
      RunStatus.INFRASTRUCTURE_FAILED,
      RunStatus.DEADLINE_EXCEEDED,
      RunStatus.SEARCH_EXHAUSTED,
    ].includes(view.status)
  )
    invalid();
  if (view.revision <= 0n || view.revision > MAX_SIGNED) invalid();
  if (
    !Number.isInteger(view.maximumRounds) ||
    view.maximumRounds < 1 ||
    view.maximumRounds > 64 ||
    !Number.isInteger(view.completedRounds) ||
    view.completedRounds < 0 ||
    view.completedRounds > view.maximumRounds ||
    (view.status === RunStatus.COMPLETED && view.completedRounds !== view.maximumRounds) ||
    (view.status === RunStatus.ACTIVE && view.completedRounds === view.maximumRounds)
  )
    invalid();
  const budget = view.budget;
  if (
    budget === undefined ||
    budget.maximumSteps <= 0n ||
    budget.maximumSteps > MAX_SIGNED ||
    budget.maximumInputTokens <= 0n ||
    budget.maximumInputTokens > MAX_SIGNED ||
    budget.maximumOutputTokens <= 0n ||
    budget.maximumOutputTokens > MAX_SIGNED
  )
    invalid();
  const wall = budget.maximumWallTime;
  if (
    wall === undefined ||
    wall.seconds < 0n ||
    wall.seconds > MAX_WALL ||
    !Number.isInteger(wall.nanos) ||
    wall.nanos < 0 ||
    wall.nanos >= 1_000_000_000 ||
    wall.nanos % 1_000_000 !== 0 ||
    (wall.seconds === 0n && wall.nanos === 0) ||
    (wall.seconds === MAX_WALL && wall.nanos !== 0)
  )
    invalid();
  if (
    view.reservedSteps <= 0n ||
    view.reservedSteps > budget.maximumSteps ||
    view.reservedInputTokens <= 0n ||
    view.reservedInputTokens > budget.maximumInputTokens ||
    view.reservedOutputTokens <= 0n ||
    view.reservedOutputTokens > budget.maximumOutputTokens ||
    usd_nanos(view.reservedCost) === 0n ||
    usd_nanos(budget.maximumCost) === 0n ||
    usd_nanos(view.reservedCost) > usd_nanos(budget.maximumCost)
  )
    invalid();
  const submitted = timestamp(view.submittedAt);
  if (
    timestamp(view.updatedAt) < submitted ||
    timestamp(view.deadline) <= submitted ||
    timestamp(view.deadline) - submitted !== wall.seconds * 1_000_000_000n + BigInt(wall.nanos)
  )
    invalid();
  if (view.adaptive !== undefined) validate_adaptive(view);
  else if (view.status === RunStatus.SEARCH_EXHAUSTED) invalid();
  if (view.research !== undefined) {
    validate_research(view);
    return;
  }
  const child = view.currentJob;
  if (child === undefined) invalid();
  identity(child.jobId?.value);
  if (
    !Number.isInteger(child.status) ||
    child.status < DiscoveryJobStatus.QUEUED ||
    child.status > DiscoveryJobStatus.PAUSED ||
    child.revision <= 0n ||
    child.revision > MAX_SIGNED ||
    (view.status === RunStatus.COMPLETED && child.status !== DiscoveryJobStatus.SUCCEEDED)
  )
    invalid();
  const childSubmitted = timestamp(child.submittedAt);
  if (childSubmitted < submitted || timestamp(child.updatedAt) < childSubmitted) invalid();
}

function validate_research(view: RunView): void {
  const research = view.research;
  if (research === undefined) invalid();
  if (
    view.currentJob !== undefined ||
    view.revision > BigInt(view.maximumRounds * 3 + 1) ||
    !Number.isInteger(research.round) ||
    research.round < 1 ||
    research.round > view.maximumRounds ||
    research.round < view.completedRounds ||
    research.round > view.completedRounds + 1 ||
    (view.status === RunStatus.ACTIVE && research.round !== view.completedRounds + 1) ||
    ([RunStatus.COMPLETED, RunStatus.SEARCH_EXHAUSTED].includes(view.status) &&
      research.round !== view.completedRounds) ||
    research.bindingSha256?.value.length !== 32 ||
    ![RunStage.DISCOVERY, RunStage.EVALUATION, RunStage.PORTFOLIO, RunStage.PERTURBATION].includes(
      research.stage,
    ) ||
    ![
      RoundDisposition.UNSPECIFIED,
      RoundDisposition.EVALUATED,
      RoundDisposition.DUPLICATE,
      RoundDisposition.PREVIOUSLY_REJECTED,
      RoundDisposition.INSUFFICIENT_COVERAGE,
      RoundDisposition.EXHAUSTED,
    ].includes(research.disposition)
  )
    invalid();
  const perturbation = research.perturbationJobId !== undefined;
  if (
    (perturbation &&
      (view.adaptive === undefined ||
        research.discoveryJobId !== undefined ||
        research.round === 1)) ||
    (!perturbation && research.discoveryJobId === undefined)
  )
    invalid();
  const proposalId = perturbation
    ? research.perturbationJobId?.value
    : research.discoveryJobId?.value;
  identity(proposalId);
  const proposalStage = [RunStage.DISCOVERY, RunStage.PERTURBATION].includes(research.stage);
  if (
    (research.stage === RunStage.PERTURBATION && !perturbation) ||
    (research.stage === RunStage.DISCOVERY && perturbation)
  )
    invalid();
  const filtered = [RoundDisposition.DUPLICATE, RoundDisposition.PREVIOUSLY_REJECTED].includes(
    research.disposition,
  );
  if (!proposalStage || filtered) {
    factor_id(research.expressionId?.value);
    factor_id(research.factorSpecId?.value);
  } else if (research.expressionId !== undefined || research.factorSpecId !== undefined) invalid();
  if (proposalStage) {
    if (
      research.evaluationJobId !== undefined ||
      (!filtered &&
        ![RoundDisposition.UNSPECIFIED, RoundDisposition.EXHAUSTED].includes(research.disposition))
    )
      invalid();
  } else {
    identity(research.evaluationJobId?.value);
    if (research.evaluationJobId?.value === proposalId) invalid();
  }
  if (
    (filtered && !proposalStage) ||
    (research.disposition === RoundDisposition.INSUFFICIENT_COVERAGE &&
      research.stage !== RunStage.EVALUATION) ||
    (research.disposition === RoundDisposition.EVALUATED &&
      research.stage !== RunStage.PORTFOLIO) ||
    (view.status === RunStatus.ACTIVE && research.disposition !== RoundDisposition.UNSPECIFIED) ||
    (view.status === RunStatus.COMPLETED &&
      research.disposition === RoundDisposition.UNSPECIFIED) ||
    (research.disposition === RoundDisposition.EXHAUSTED &&
      (view.status !== RunStatus.SEARCH_EXHAUSTED || research.stage !== RunStage.PERTURBATION)) ||
    (view.status === RunStatus.SEARCH_EXHAUSTED &&
      research.disposition !== RoundDisposition.EXHAUSTED)
  )
    invalid();
  const current = research.currentChild;
  if (current.case === undefined) invalid();
  const child = current.value;
  const discovery = current.case === "discovery";
  if (
    (research.stage === RunStage.DISCOVERY) !== discovery ||
    !Number.isInteger(child.status) ||
    child.status < 1 ||
    child.status > (discovery ? DiscoveryJobStatus.PAUSED : ResearchJobStatus.BUDGET_EXHAUSTED) ||
    (discovery && child.jobId?.value !== research.discoveryJobId?.value) ||
    (research.stage === RunStage.EVALUATION &&
      child.jobId?.value !== research.evaluationJobId?.value) ||
    (research.stage === RunStage.PERTURBATION &&
      child.jobId?.value !== research.perturbationJobId?.value) ||
    (research.stage === RunStage.PORTFOLIO &&
      (child.jobId?.value === proposalId ||
        child.jobId?.value === research.evaluationJobId?.value)) ||
    child.revision <= 0n ||
    child.revision > MAX_SIGNED ||
    (research.disposition !== RoundDisposition.UNSPECIFIED &&
      child.status !== (discovery ? DiscoveryJobStatus.SUCCEEDED : ResearchJobStatus.SUCCEEDED))
  )
    invalid();
  identity(child.jobId?.value);
  const submitted = timestamp(child.submittedAt);
  if (submitted < timestamp(view.submittedAt) || timestamp(child.updatedAt) < submitted) invalid();
}

function validate_adaptive(view: RunView): void {
  const adaptive = view.adaptive;
  if (adaptive === undefined || view.research === undefined) invalid();
  const policy = adaptive.feedbackPolicy;
  if (
    policy === undefined ||
    policy.policyId === undefined ||
    /^[a-z][a-z0-9_.-]{0,127}$/.exec(policy.policyId.value)?.[0] !== policy.policyId.value ||
    /^[1-9][0-9]{0,19}$/.exec(policy.revision)?.[0] !== policy.revision ||
    BigInt(policy.revision) > 18_446_744_073_709_551_615n ||
    policy.sha256?.value.length !== 32 ||
    adaptive.feedbackSha256?.value.length !== 32 ||
    adaptive.modelContextSha256?.value.length !== 32 ||
    !Number.isInteger(adaptive.sourceCount) ||
    adaptive.sourceCount < 0 ||
    adaptive.sourceCount > 16 ||
    adaptive.sourceCount > view.completedRounds ||
    (adaptive.familySha256 === undefined) !== (adaptive.stateSha256 === undefined) ||
    adaptive.randomDraws < 0n ||
    adaptive.randomDraws > BigInt(view.maximumRounds * 32) ||
    (adaptive.familySha256 === undefined &&
      (adaptive.randomDraws !== 0n || view.research.perturbationJobId !== undefined)) ||
    [adaptive.familySha256, adaptive.stateSha256].some(
      (digest) => digest !== undefined && digest.value.length !== 32,
    )
  )
    invalid();
}

function factor_id(value: string | undefined): void {
  if (value === undefined || /^sha256:[a-f0-9]{64}$/.exec(value)?.[0] !== value) invalid();
}

function identity(value: string | undefined): void {
  if (value === undefined || /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/.exec(value)?.[0] !== value)
    invalid();
}

function timestamp(value: Timestamp | undefined): bigint {
  if (
    value === undefined ||
    value.seconds < -62_135_596_800n ||
    value.seconds > 253_402_300_799n ||
    !Number.isInteger(value.nanos) ||
    value.nanos < 0 ||
    value.nanos >= 1_000_000_000
  )
    invalid();
  return value.seconds * 1_000_000_000n + BigInt(value.nanos);
}

function usd_nanos(value: Money | undefined): bigint {
  const amount = value?.amount?.value;
  if (
    value?.currencyCode !== "USD" ||
    amount === undefined ||
    /^(0|[1-9][0-9]{0,5})(\.[0-9]{0,8}[1-9])?$/.exec(amount)?.[0] !== amount
  )
    invalid();
  const [whole, fraction = ""] = amount.split(".");
  return BigInt(whole ?? "0") * 1_000_000_000n + BigInt(fraction.padEnd(9, "0"));
}

function invalid(): never {
  throw new Error("invalid_run_view");
}
