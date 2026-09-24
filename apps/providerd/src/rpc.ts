import { createHash } from "node:crypto";
import type { IncomingMessage } from "node:http";
import { createSecureServer, type Http2ServerRequest } from "node:http2";
import { TLSSocket } from "node:tls";
import { create } from "@bufbuild/protobuf";
import {
  Code,
  type ConnectRouter,
  createContextKey,
  createContextValues,
  type HandlerContext,
} from "@connectrpc/connect";
import { connectNodeAdapter } from "@connectrpc/connect-node";
import {
  ErrorCategory,
  type InvokeModelRequest,
  InvokeModelRequestSchema,
  ProviderService,
  type StreamModelRequest,
} from "@loop-engine/protocol/provider";

import { type Principal, read_private } from "./config.js";
import { ProviderError, rpc_error } from "./errors.js";
import type { ProviderHost } from "./host.js";

const principal_key = createContextKey<Principal | undefined>(undefined);

/** Optional private listener. Its health sibling never acquires model routes. */
export async function create_provider_rpc(host: ProviderHost, shutdown: AbortSignal) {
  const [ca, cert, key] = await Promise.all([
    read_private(host.config.tls.ca),
    read_private(host.config.tls.certificate),
    read_private(host.config.tls.key),
  ]);
  function transport_values(request: IncomingMessage | Http2ServerRequest) {
    const socket = request.socket;
    let principal: Principal | undefined;
    if (socket instanceof TLSSocket && socket.authorized) {
      const certificate = socket.getPeerCertificate();
      if (certificate.raw) {
        const digest = createHash("sha256").update(certificate.raw).digest("hex");
        principal = host.config.principals.find((entry) => entry.certificate_sha256 === digest);
      }
    }
    return createContextValues().set(principal_key, principal);
  }
  function request_gate(context: HandlerContext) {
    if (!context.values.get(principal_key))
      throw rpc_error(
        new ProviderError(
          "provider_identity_denied",
          Code.Unauthenticated,
          ErrorCategory.AUTHENTICATION,
        ),
      );
    if (context.timeoutMs() === undefined)
      throw rpc_error(new ProviderError("provider_deadline_required"));
    for (const name of context.requestHeader.keys()) {
      if (
        name.includes("holdout") ||
        name.includes("capability") ||
        name.startsWith("x-forwarded-") ||
        name === "forwarded" ||
        name === "authorization" ||
        name === "x-loop-actor"
      ) {
        throw rpc_error(
          new ProviderError(
            "provider_metadata_denied",
            Code.PermissionDenied,
            ErrorCategory.AUTHORIZATION,
          ),
        );
      }
    }
  }
  async function invoke_model(request: InvokeModelRequest, context: HandlerContext) {
    try {
      const principal = context.values.get(principal_key);
      if (!principal)
        throw new ProviderError(
          "provider_identity_denied",
          Code.Unauthenticated,
          ErrorCategory.AUTHENTICATION,
        );
      return { response: await host.invoke(request, principal, context.signal) };
    } catch (error) {
      throw rpc_error(error);
    }
  }
  async function* stream_model(request: StreamModelRequest, context: HandlerContext) {
    try {
      const principal = context.values.get(principal_key);
      if (!principal)
        throw new ProviderError(
          "provider_identity_denied",
          Code.Unauthenticated,
          ErrorCategory.AUTHENTICATION,
        );
      const command = create(InvokeModelRequestSchema, {
        context: request.context,
        invocation: request.invocation,
      });
      for await (const event of host.stream(command, principal, context.signal)) yield { event };
    } catch (error) {
      throw rpc_error(error);
    }
  }
  function routes(router: ConnectRouter) {
    router.rpc(ProviderService.method.invokeModel, invoke_model);
    router.rpc(ProviderService.method.streamModel, stream_model);
  }
  const adapter = connectNodeAdapter({
    routes,
    contextValues: transport_values,
    requestGate: request_gate,
    grpc: true,
    connect: false,
    grpcWeb: false,
    readMaxBytes: 524_288,
    writeMaxBytes: 524_288,
    acceptCompression: [],
    maxTimeoutMs: host.config.policy.wall_time_ms,
    shutdownSignal: shutdown,
  });
  const server = createSecureServer(
    {
      ca,
      cert,
      key,
      requestCert: true,
      rejectUnauthorized: true,
      minVersion: "TLSv1.3",
      allowHTTP1: false,
      settings: { maxConcurrentStreams: host.config.policy.concurrency, maxHeaderListSize: 16_384 },
    },
    adapter,
  );
  server.on("session", (session) => {
    session.setTimeout(host.config.policy.wall_time_ms + 1000, () => session.destroy());
    const stop_session = () => session.close();
    shutdown.addEventListener("abort", stop_session, { once: true });
    session.once("close", () => shutdown.removeEventListener("abort", stop_session));
  });
  return server;
}
