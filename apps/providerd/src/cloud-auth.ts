import { createRequire } from "node:module";
import { Code } from "@connectrpc/connect";
import { ErrorCategory } from "@loop-engine/protocol/provider";
import { ProviderError } from "./errors.js";

export type CloudToken = (signal: AbortSignal) => Promise<string>;

/** Injection is owned by the host constructor, never by an RPC/configuration. */
export interface CloudIdentity {
  readonly azure?: CloudToken;
  readonly google?: CloudToken;
}

/** Some credential chains cannot cancel discovery. Never dispatch after expiry,
 * and consume late rejection without exposing credential SDK messages. */
export async function bounded_identity<T>(
  lookup: () => Promise<T>,
  signal: AbortSignal,
): Promise<T> {
  signal.throwIfAborted();
  let stop: (() => void) | undefined;
  try {
    const cancelled = new Promise<never>((_resolve, reject) => {
      stop = () =>
        reject(
          new ProviderError("cloud_identity_cancelled", Code.Canceled, ErrorCategory.CANCELLED),
        );
      signal.addEventListener("abort", stop, { once: true });
    });
    const value = await Promise.race([Promise.resolve().then(lookup), cancelled]);
    signal.throwIfAborted();
    return value;
  } catch {
    throw new ProviderError(
      signal.aborted ? "cloud_identity_cancelled" : "cloud_identity_unavailable",
      signal.aborted ? Code.Canceled : Code.Unavailable,
      signal.aborted ? ErrorCategory.CANCELLED : ErrorCategory.DEPENDENCY,
    );
  } finally {
    if (stop) signal.removeEventListener("abort", stop);
  }
}

export async function cloud_token(resolve: CloudToken, signal: AbortSignal): Promise<string> {
  const value = await bounded_identity(() => resolve(signal), signal);
  if (!/^[\x21-\x7e]{1,16384}$/.test(value))
    throw new ProviderError(
      "cloud_identity_unavailable",
      Code.Unavailable,
      ErrorCategory.DEPENDENCY,
    );
  return value;
}

export function azure_identity(): CloudToken {
  const { DefaultAzureCredential } = createRequire(import.meta.url)(
    "@azure/identity",
  ) as typeof import("@azure/identity");
  const credential = new DefaultAzureCredential({
    processTimeoutInMs: 1000,
    retryOptions: { maxRetries: 0 },
  });
  return async (signal) => {
    const token = await credential.getToken("https://ai.azure.com/.default", {
      abortSignal: signal,
    });
    if (!token || token.expiresOnTimestamp <= Date.now()) throw new Error("cloud_token_expired");
    return token.token;
  };
}

export function google_identity(): CloudToken {
  const { GoogleAuth } = createRequire(import.meta.url)(
    "google-auth-library",
  ) as typeof import("google-auth-library");
  const auth = new GoogleAuth({
    scopes: ["https://www.googleapis.com/auth/cloud-platform"],
    clientOptions: { transporterOptions: { timeout: 5000, retry: false } },
  });
  return async () => {
    const value = await auth.getAccessToken();
    if (!value) throw new Error("cloud_token_missing");
    return value;
  };
}
