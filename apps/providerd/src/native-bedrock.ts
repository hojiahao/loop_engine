import { createRequire } from "node:module";
import { bedrock_parameters, bedrock_result } from "./bedrock-content.js";
import { bedrock_stream } from "./bedrock-stream.js";
import { bedrock_transport } from "./bedrock-transport.js";
import { bounded_identity } from "./cloud-auth.js";
import type { ModelRoute } from "./config.js";
import { native_error, ProviderError } from "./errors.js";
import { input_ceiling, type NativePlugin } from "./native.js";
import { stream_invalid } from "./stream.js";

function bedrock_error(error: unknown, signal: AbortSignal): ProviderError {
  if (
    error instanceof Error &&
    ["ThrottlingException", "ModelNotReadyException"].includes(error.name)
  )
    return native_error({ status: 429 }, signal);
  return native_error(error, signal);
}

export function bedrock_plugin(
  model: ModelRoute,
  secrets: Readonly<Record<string, string | undefined>>,
  fetcher: typeof fetch,
): NativePlugin {
  const cloud = model.cloud;
  if (cloud?.kind !== "bedrock") throw new ProviderError("invalid_cloud_route");
  const { region } = cloud;
  const { BedrockRuntimeClient, ConverseCommand, ConverseStreamCommand } = createRequire(
    import.meta.url,
  )("@aws-sdk/client-bedrock-runtime") as typeof import("@aws-sdk/client-bedrock-runtime");
  const { defaultProvider } = createRequire(import.meta.url)(
    "@aws-sdk/credential-provider-node",
  ) as typeof import("@aws-sdk/credential-provider-node");
  const references = cloud.credentials;
  const lookup = references
    ? async () => {
        const accessKeyId = secrets[references.access_key_env];
        const secretAccessKey = secrets[references.secret_key_env];
        const sessionToken = references.session_token_env
          ? secrets[references.session_token_env]
          : undefined;
        if (
          !accessKeyId ||
          !secretAccessKey ||
          (references.session_token_env && !sessionToken) ||
          !/^[A-Za-z0-9]{16,128}$/.test(accessKeyId) ||
          !/^[\x21-\x7e]{1,4096}$/.test(secretAccessKey) ||
          (sessionToken !== undefined && !/^[\x21-\x7e]{1,16384}$/.test(sessionToken))
        )
          throw new Error("cloud_credentials_missing");
        return { accessKeyId, secretAccessKey, sessionToken };
      }
    : defaultProvider({
        timeout: 1000,
        maxRetries: 0,
        clientConfig: { region: cloud.region, maxAttempts: 1 },
      });
  const origin = `https://bedrock-runtime.${cloud.region}.amazonaws.com`;
  async function client(signal: AbortSignal) {
    const credentials = await bounded_identity(lookup, signal);
    if (
      "expiration" in credentials &&
      credentials.expiration &&
      credentials.expiration <= new Date()
    )
      throw new ProviderError("cloud_credentials_expired");
    signal.throwIfAborted();
    return new BedrockRuntimeClient({
      region,
      endpoint: origin,
      credentials,
      maxAttempts: 1,
      authSchemePreference: ["sigv4"],
      requestHandler: bedrock_transport(origin, fetcher),
    });
  }
  return {
    async count_input(input, signal) {
      signal.throwIfAborted();
      bedrock_parameters(input);
      return input_ceiling(input);
    },
    async invoke(input, signal) {
      let sdk: Awaited<ReturnType<typeof client>> | undefined;
      try {
        sdk = await client(signal);
        return bedrock_result(
          await sdk.send(new ConverseCommand(bedrock_parameters(input)), { abortSignal: signal }),
          input,
        );
      } catch (error) {
        throw bedrock_error(error, signal);
      } finally {
        sdk?.destroy();
      }
    },
    async *stream(input, signal) {
      let sdk: Awaited<ReturnType<typeof client>> | undefined;
      try {
        sdk = await client(signal);
        const reply = await sdk.send(new ConverseStreamCommand(bedrock_parameters(input)), {
          abortSignal: signal,
        });
        if (!reply.stream) stream_invalid();
        yield* bedrock_stream(reply.stream, input);
      } catch (error) {
        throw bedrock_error(error, signal);
      } finally {
        sdk?.destroy();
      }
    },
  };
}
