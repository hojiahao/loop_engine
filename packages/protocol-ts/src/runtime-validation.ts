import { fromBinary } from "@bufbuild/protobuf";

import { type ServiceError, ServiceErrorSchema } from "./generated/loop/v1/common_pb.js";
import type { JobKind, JobSpecification } from "./generated/loop/v1/job_pb.js";
import { JobValidationError, validateJobSpecification, validateServiceError } from "./job.js";

export const SERVICE_ERROR_TYPE_URL = "type.googleapis.com/loop.v1.ServiceError";

export type RuntimeValidationCode =
  | "invalid_status"
  | "unexpected_response_body"
  | "unexpected_detail"
  | "unsupported_enum"
  | "unsupported_oneof"
  | "unsupported_kind"
  | "incompatible_variant"
  | "invalid_specification"
  | "malformed_detail"
  | "invalid_service_error";

export class RuntimeValidationError extends Error {
  public readonly code: RuntimeValidationCode;

  public constructor(code: RuntimeValidationCode) {
    super(code);
    this.name = "RuntimeValidationError";
    this.code = code;
  }
}

export interface RichStatusDetail {
  readonly typeUrl: string;
  readonly value: Uint8Array;
}

export function validateOperationalFailure(
  grpcStatusCode: number,
  responseBody: Uint8Array,
  details: readonly RichStatusDetail[],
): ServiceError {
  if (!Number.isInteger(grpcStatusCode) || grpcStatusCode < 1 || grpcStatusCode > 16) {
    throw new RuntimeValidationError("invalid_status");
  }
  if (responseBody.byteLength !== 0) {
    throw new RuntimeValidationError("unexpected_response_body");
  }
  if (details.length !== 1) {
    throw new RuntimeValidationError("unexpected_detail");
  }
  const detail = details[0];
  if (detail === undefined || detail.typeUrl !== SERVICE_ERROR_TYPE_URL) {
    throw new RuntimeValidationError("unexpected_detail");
  }

  let serviceError: ServiceError;
  try {
    serviceError = fromBinary(ServiceErrorSchema, detail.value);
  } catch {
    throw new RuntimeValidationError("malformed_detail");
  }
  try {
    validateServiceError(serviceError);
  } catch (error) {
    if (error instanceof JobValidationError && error.code === "unknown_enum") {
      throw new RuntimeValidationError("unsupported_enum");
    }
    throw new RuntimeValidationError("invalid_service_error");
  }
  return serviceError;
}

/**
 * Validate only a necessary wire-level dispatch candidate.
 *
 * A returned kind does not select or authorize a handler. FactorSpec registry
 * binding, holdout plan-entry and Phase 7 owning-BacktestSpec parsing, artifact
 * availability, server-owned dataset snapshot/capability resolution, and
 * runtime authorization remain mandatory external gates.
 */
export function validateJobWireDispatchCandidate(
  specification: JobSpecification,
  enabledJobKinds: ReadonlySet<JobKind>,
): JobKind {
  let kind: JobKind;
  try {
    kind = validateJobSpecification(specification).kind;
  } catch (error) {
    if (!(error instanceof JobValidationError)) throw error;
    if (error.code === "unknown_enum") throw new RuntimeValidationError("unsupported_enum");
    if (error.code === "missing_field" && error.field === "specification.input") {
      throw new RuntimeValidationError("unsupported_oneof");
    }
    if (error.code === "kind_input_mismatch") {
      throw new RuntimeValidationError("incompatible_variant");
    }
    throw new RuntimeValidationError("invalid_specification");
  }
  if (!enabledJobKinds.has(kind)) {
    throw new RuntimeValidationError("unsupported_kind");
  }
  return kind;
}
