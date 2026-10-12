import type { Timestamp } from "@bufbuild/protobuf/wkt";

import { DiscoveryJobStatus } from "./generated/loop/discovery/v1/service_pb.js";
import { RunStatus, type RunView } from "./generated/loop/runs/v1/service_pb.js";
import type { Money } from "./generated/loop/v1/common_pb.js";

const MAX_SIGNED = 9_223_372_036_854_775_807n;
const MAX_WALL = 2_592_000n;

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
