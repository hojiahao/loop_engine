import { Code, ConnectError } from "@connectrpc/connect";
import { ErrorCategory, ServiceErrorSchema } from "@loop-engine/protocol/provider";

export class ProviderError extends Error {
  constructor(
    readonly code: string,
    readonly status = Code.FailedPrecondition,
    readonly category = ErrorCategory.VALIDATION,
  ) {
    super(code);
    this.name = "ProviderError";
  }
}

/** Only allowlisted codes cross RPC; SDK messages/headers can contain secrets. */
export function rpc_error(error: unknown): ConnectError {
  const known =
    error instanceof ProviderError
      ? error
      : new ProviderError("provider_internal", Code.Internal, ErrorCategory.INTERNAL);
  return new ConnectError(known.code, known.status, undefined, [
    {
      desc: ServiceErrorSchema,
      value: {
        code: known.code,
        category: known.category,
        message: known.code,
        retryable: false,
        details: [],
      },
    },
  ]);
}

export function native_error(error: unknown, signal: AbortSignal): ProviderError {
  if (error instanceof ProviderError)
    return error.category === ErrorCategory.VALIDATION
      ? new ProviderError(error.code, Code.DataLoss, ErrorCategory.DEPENDENCY)
      : error;
  if (signal.aborted)
    return new ProviderError("provider_cancelled", Code.Canceled, ErrorCategory.CANCELLED);
  const status =
    typeof error === "object" && error !== null
      ? "status" in error
        ? error.status
        : "statusCode" in error
          ? error.statusCode
          : "$metadata" in error &&
              typeof error.$metadata === "object" &&
              error.$metadata !== null &&
              "httpStatusCode" in error.$metadata
            ? error.$metadata.httpStatusCode
            : undefined
      : undefined;
  if (status === 429)
    return new ProviderError(
      "provider_rate_limited",
      Code.ResourceExhausted,
      ErrorCategory.RATE_LIMIT,
    );
  if (status === 401 || status === 403)
    return new ProviderError(
      "provider_credentials_denied",
      Code.Unavailable,
      ErrorCategory.DEPENDENCY,
    );
  return new ProviderError(
    "provider_dependency_failed",
    Code.Unavailable,
    ErrorCategory.DEPENDENCY,
  );
}
