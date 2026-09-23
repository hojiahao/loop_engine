import { create, equals, fromBinary, toBinary, toJson } from "@bufbuild/protobuf";
import { timestampDate, timestampNow } from "@bufbuild/protobuf/wkt";
import { Code } from "@connectrpc/connect";
import {
  ActorKind,
  ErrorCategory,
  type InvokeModelRequest,
  InvokeModelRequestSchema,
  ModelResolutionSnapshotSchema,
  type ModelResponse,
  ModelResponseSchema,
  type ModelStreamEvent,
  ModelStreamEventSchema,
  PolicyReferenceSchema,
} from "@loop-engine/protocol/provider";

import { type Deployment, model_snapshot, type Principal, request_policy } from "./config.js";
import { ProviderContent } from "./content.js";
import { ProviderError } from "./errors.js";
import { digest_json, hex_digest } from "./identity.js";
import { claim_invocation, finish_invocation, JournalError } from "./journal.js";
import { type NativePlugin, type NativeReply, response_finish, response_usage } from "./native.js";
import { anthropic_plugin } from "./native-anthropic.js";
import { cohere_plugin } from "./native-cohere.js";
import { google_plugin } from "./native-google.js";
import { openai_plugin } from "./native-openai.js";
import { StreamEvidence, stream_invalid } from "./stream.js";

const factories = {
  openai_responses: (secret: string, fetcher: typeof fetch) =>
    openai_plugin(secret, false, fetcher),
  openai_chat: (secret: string, fetcher: typeof fetch) => openai_plugin(secret, true, fetcher),
  anthropic: anthropic_plugin,
  google_generate: (secret: string, fetcher: typeof fetch) => google_plugin(secret, false, fetcher),
  google_interactions: (secret: string, fetcher: typeof fetch) =>
    google_plugin(secret, true, fetcher),
  cohere: cohere_plugin,
};

export function decimal_units(value: string): bigint {
  if (!/^(?:0|[1-9][0-9]{0,5})(?:\.[0-9]{0,8}[1-9])?$/.test(value))
    throw new ProviderError("invalid_money");
  const [whole = "0", fraction = ""] = value.split(".");
  return BigInt(whole) * 1_000_000_000n + BigInt(fraction.padEnd(9, "0"));
}

function validate_id(value: string | undefined): string {
  if (!value || !/^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$/.test(value))
    throw new ProviderError("invalid_request_identity");
  return value;
}

/** Native dispatch owns neither research state nor autonomous retry decisions. */
export class ProviderHost {
  private active = 0;
  private last_time = 0;
  private readonly plugins = new Map<string, NativePlugin>();
  private readonly content: ProviderContent;
  readonly models;
  readonly policy;

  constructor(
    readonly config: Deployment,
    plugin: Uint8Array,
    secrets: Readonly<Record<string, string | undefined>>,
    fetcher: typeof fetch = fetch,
  ) {
    this.content = new ProviderContent(config);
    this.models = config.models.map((model) => ({
      route: model,
      snapshot: model_snapshot(config, model, plugin),
    }));
    this.policy = request_policy(config);
    for (const model of config.models) {
      const secret = secrets[model.secret_env];
      if (secret && /^[\x21-\x7e]{1,4096}$/.test(secret))
        this.plugins.set(model.id, factories[model.plugin](secret, fetcher));
    }
  }

  async invoke(
    command: InvokeModelRequest,
    principal: Principal,
    signal: AbortSignal,
  ): Promise<ModelResponse> {
    for await (const event of this.execute(command, principal, signal, false)) {
      if (event.event.case === "completed" && event.event.value.response)
        return event.event.value.response;
    }
    return stream_invalid();
  }

  stream(command: InvokeModelRequest, principal: Principal, signal: AbortSignal) {
    return this.execute(command, principal, signal, true);
  }

  private async *execute(
    command: InvokeModelRequest,
    principal: Principal,
    signal: AbortSignal,
    streaming: boolean,
  ): AsyncGenerator<ModelStreamEvent> {
    const now = Date.now();
    if (now < this.last_time)
      throw new ProviderError("provider_clock_regressed", Code.Unavailable, ErrorCategory.INTERNAL);
    this.last_time = now;
    const context = command.context;
    const invocation = command.invocation;
    const actor_kind = principal.actor_kind === "service" ? ActorKind.SERVICE : ActorKind.AGENT;
    if (
      !this.config.principals.includes(principal) ||
      !context?.actor ||
      context.actor.actorId?.value !== principal.actor_id ||
      context.actor.kind !== actor_kind
    ) {
      throw new ProviderError(
        "provider_actor_denied",
        Code.PermissionDenied,
        ErrorCategory.AUTHORIZATION,
      );
    }
    const key = validate_id(context.idempotencyKey?.value);
    const id = validate_id(context.requestId?.value);
    validate_id(context.correlationId?.value);
    if (context.causationId) validate_id(context.causationId.value);
    if (
      !context.requestedAt ||
      context.requestedAt.seconds < 0n ||
      context.requestedAt.seconds > 253_402_300_799n ||
      context.requestedAt.nanos < 0 ||
      context.requestedAt.nanos >= 1_000_000_000 ||
      Math.abs(timestampDate(context.requestedAt).getTime() - now) > 300_000 ||
      !invocation ||
      invocation.requestId?.value !== id
    ) {
      throw new ProviderError("invalid_request_context");
    }
    const selected = this.models.find(
      (model) => model.snapshot.resolutionId?.value === invocation.model?.resolutionId?.value,
    );
    if (
      !selected ||
      !invocation.model ||
      !equals(ModelResolutionSnapshotSchema, invocation.model, selected.snapshot) ||
      !principal.model_ids.includes(selected.route.id) ||
      !invocation.requestPolicy ||
      !equals(PolicyReferenceSchema, invocation.requestPolicy, this.policy)
    ) {
      throw new ProviderError(
        "provider_pin_denied",
        Code.PermissionDenied,
        ErrorCategory.AUTHORIZATION,
      );
    }
    if (new Date(this.config.resolved_at).getTime() > timestampDate(context.requestedAt).getTime())
      throw new ProviderError("future_model_resolution");
    if (streaming && !selected.route.features.streaming)
      throw new ProviderError(
        "provider_stream_unavailable",
        Code.Unimplemented,
        ErrorCategory.DEPENDENCY,
      );
    const budget = invocation.budget;
    const maximum = budget?.maximumWallTime;
    if (
      !budget ||
      !maximum ||
      maximum.seconds < 0n ||
      maximum.seconds > 300n ||
      maximum.nanos < 0 ||
      maximum.nanos >= 1_000_000_000 ||
      budget.maximumInputTokens < 1n ||
      budget.maximumOutputTokens < 1n ||
      budget.maximumCost?.currencyCode !== "USD" ||
      !budget.maximumCost.amount
    ) {
      throw new ProviderError("invalid_invocation_budget");
    }
    const wall_time = Number(maximum.seconds) * 1000 + Math.ceil(maximum.nanos / 1_000_000);
    const max_cost = decimal_units(budget.maximumCost.amount.value);
    const input_price = [
      selected.route.input_usd,
      selected.route.cached_usd,
      selected.route.cache_creation_usd ?? "0",
    ]
      .map(decimal_units)
      .reduce((highest, price) => (price > highest ? price : highest), 0n);
    const reserved =
      (budget.maximumInputTokens * input_price +
        budget.maximumOutputTokens * decimal_units(selected.route.output_usd) +
        999_999n) /
      1_000_000n;
    if (
      wall_time < 1 ||
      wall_time > this.config.policy.wall_time_ms ||
      budget.maximumInputTokens >
        BigInt(Math.min(this.config.policy.input_tokens, selected.route.context_tokens)) ||
      budget.maximumOutputTokens >
        BigInt(Math.min(this.config.policy.output_tokens, selected.route.output_tokens)) ||
      reserved > max_cost ||
      max_cost > decimal_units(this.config.policy.maximum_usd)
    ) {
      throw new ProviderError(
        "provider_budget_denied",
        Code.ResourceExhausted,
        ErrorCategory.BUDGET_EXHAUSTED,
      );
    }
    const plugin = this.plugins.get(selected.route.id);
    if (!plugin)
      throw new ProviderError(
        "provider_credentials_missing",
        Code.Unavailable,
        ErrorCategory.DEPENDENCY,
      );
    if (signal.aborted)
      throw new ProviderError("provider_cancelled", Code.Canceled, ErrorCategory.CANCELLED);
    if (this.active >= this.config.policy.concurrency)
      throw new ProviderError(
        "provider_capacity",
        Code.ResourceExhausted,
        ErrorCategory.RATE_LIMIT,
      );
    this.active += 1;
    const deadline = AbortSignal.timeout(wall_time);
    const stopped = new AbortController();
    const combined = AbortSignal.any([signal, deadline, stopped.signal]);
    let sequence = 0n;
    const stamp = (event: ModelStreamEvent["event"]) => {
      const time = Date.now();
      if (time < this.last_time)
        throw new ProviderError(
          "provider_clock_regressed",
          Code.Unavailable,
          ErrorCategory.INTERNAL,
        );
      this.last_time = time;
      combined.throwIfAborted();
      return create(ModelStreamEventSchema, {
        requestId: invocation.requestId,
        sequence: ++sequence,
        emittedAt: timestampNow(),
        event,
      });
    };
    try {
      const input = await this.content.prepare(
        invocation,
        selected.route,
        principal.actor_id,
        selected.snapshot,
      );
      combined.throwIfAborted();
      const slot = await claim_invocation(this.config.journal, key, {
        schema: "loop.provider-claim/v1",
        actor: principal.actor_id,
        request_sha256: hex_digest(
          digest_json("loop.provider-invocation/v1", toJson(InvokeModelRequestSchema, command)),
        ),
        reserved_nano_usd: reserved.toString(),
      });
      yield stamp({
        case: "started",
        value: { $typeName: "loop.v1.StreamStarted", resolutionId: selected.snapshot.resolutionId },
      });
      if (slot.cached) {
        const cached = fromBinary(ModelResponseSchema, slot.cached);
        if (
          cached.requestId?.value !== id ||
          cached.resolutionId?.value !== selected.snapshot.resolutionId?.value ||
          !cached.usage ||
          cached.usage.inputTokens > budget.maximumInputTokens ||
          cached.usage.outputTokens > budget.maximumOutputTokens
        ) {
          throw new ProviderError(
            "provider_receipt_corrupt",
            Code.DataLoss,
            ErrorCategory.INTERNAL,
          );
        }
        yield stamp({
          case: "usageUpdate",
          value: { $typeName: "loop.v1.UsageUpdate", usage: cached.usage },
        });
        yield stamp({
          case: "completed",
          value: { $typeName: "loop.v1.StreamCompleted", response: cached },
        });
        return;
      }
      combined.throwIfAborted();
      const counted = await plugin.count_input(input, combined);
      if (
        BigInt(counted) > budget.maximumInputTokens ||
        (selected.route.input_token_limit === undefined &&
          counted + input.output_tokens > selected.route.context_tokens)
      ) {
        throw new ProviderError(
          "provider_input_budget",
          Code.ResourceExhausted,
          ErrorCategory.BUDGET_EXHAUSTED,
        );
      }
      combined.throwIfAborted();
      let reply: NativeReply | undefined;
      if (streaming) {
        const evidence = new StreamEvidence();
        for await (const event of plugin.stream(input, combined)) {
          if (reply) stream_invalid();
          if (event.kind === "delta") {
            evidence.record(event.delta);
            yield stamp({ case: "contentDelta", value: event.delta });
          } else {
            evidence.verify(event.reply);
            reply = event.reply;
          }
        }
        if (!reply) stream_invalid();
      } else reply = await plugin.invoke(input, combined);
      combined.throwIfAborted();
      const usage = response_usage(reply.usage);
      if (
        usage.inputTokens > budget.maximumInputTokens ||
        usage.outputTokens > budget.maximumOutputTokens ||
        usage.inputTokens + usage.outputTokens > BigInt(selected.route.context_tokens)
      ) {
        throw new ProviderError(
          "provider_usage_exceeded",
          Code.ResourceExhausted,
          ErrorCategory.BUDGET_EXHAUSTED,
        );
      }
      const content = await this.content
        .response(reply, input, principal.actor_id, selected.snapshot)
        .catch((error: unknown) => {
          if (error instanceof ProviderError && error.category === ErrorCategory.VALIDATION)
            throw new ProviderError(error.code, Code.DataLoss, ErrorCategory.DEPENDENCY);
          throw error;
        });
      const response = create(ModelResponseSchema, {
        requestId: invocation.requestId,
        resolutionId: selected.snapshot.resolutionId,
        content,
        finishReason: response_finish(reply),
        usage,
      });
      combined.throwIfAborted();
      await finish_invocation(this.config.journal, slot, toBinary(ModelResponseSchema, response));
      yield stamp({ case: "usageUpdate", value: { $typeName: "loop.v1.UsageUpdate", usage } });
      yield stamp({ case: "completed", value: { $typeName: "loop.v1.StreamCompleted", response } });
    } catch (error) {
      if (signal.aborted)
        throw new ProviderError("provider_cancelled", Code.Canceled, ErrorCategory.CANCELLED);
      if (deadline.aborted)
        throw new ProviderError("provider_deadline", Code.DeadlineExceeded, ErrorCategory.TIMEOUT);
      if (error instanceof JournalError)
        throw new ProviderError(error.code, Code.FailedPrecondition, ErrorCategory.CONFLICT);
      throw error;
    } finally {
      stopped.abort();
      this.active -= 1;
    }
  }
}
