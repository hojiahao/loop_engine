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
import {
  catalog_digest,
  deployment_digest,
  load_catalog,
  profile_digest,
} from "./catalog-store.js";
import type { CatalogStatus } from "./catalog-types.js";
import type { CloudIdentity } from "./cloud-auth.js";
import { azure_plugin, vertex_plugin } from "./cloud-plugins.js";
import { compatible_id } from "./compatible-config.js";
import {
  type Deployment,
  type ModelRoute,
  model_snapshot,
  type Principal,
  request_policy,
} from "./config.js";
import { ProviderContent } from "./content.js";
import { ProviderError } from "./errors.js";
import { digest_json, hex_digest } from "./identity.js";
import { claim_invocation, finish_invocation, JournalError } from "./journal.js";
import { type NativePlugin, type NativeReply, response_finish, response_usage } from "./native.js";
import { anthropic_plugin } from "./native-anthropic.js";
import { bedrock_plugin } from "./native-bedrock.js";
import { cohere_plugin } from "./native-cohere.js";
import { compatible_plugin } from "./native-compatible.js";
import { google_plugin } from "./native-google.js";
import { openai_plugin } from "./native-openai.js";
import { vendor_plugin } from "./native-vendor.js";
import { StreamEvidence, stream_invalid } from "./stream.js";
import { vendor_id } from "./vendor-registry.js";

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

interface ResolvedRoute {
  readonly route: ModelRoute;
  readonly snapshot: ReturnType<typeof model_snapshot>;
  readonly expires_at?: number;
}

interface HostRoutes {
  readonly current: readonly ResolvedRoute[];
  readonly resolutions: ReadonlyMap<string, ResolvedRoute>;
  readonly plugins: ReadonlyMap<string, NativePlugin>;
  readonly statuses: ReadonlyMap<string, CatalogStatus>;
  readonly generation?: { revision: number; sha256: string };
}

function route_plugin(
  model: ModelRoute,
  secrets: Readonly<Record<string, string | undefined>>,
  fetcher: typeof fetch,
  identity: CloudIdentity,
): NativePlugin | undefined {
  const secret = model.secret_env ? secrets[model.secret_env] : undefined;
  const valid = secret !== undefined && /^[\x21-\x7e]{1,4096}$/.test(secret);
  if (model.plugin === "azure_responses" || model.plugin === "azure_chat") {
    return valid || (model.cloud?.kind === "azure" && model.cloud.auth === "entra")
      ? azure_plugin(model, secret, fetcher, identity)
      : undefined;
  }
  if (model.plugin === "vertex_generate") return vertex_plugin(model, fetcher, identity);
  if (model.plugin === "bedrock_converse") return bedrock_plugin(model, secrets, fetcher);
  if (compatible_id(model.plugin)) {
    const reference = model.compatible?.gateway?.upstream_key_env;
    const upstream = reference ? secrets[reference] : undefined;
    return (valid || model.compatible?.auth === "none") &&
      (model.plugin !== "portkey" || (upstream && /^[\x21-\x7e]{1,4096}$/.test(upstream)))
      ? compatible_plugin(model, secret, secrets, fetcher)
      : undefined;
  }
  if (vendor_id(model.plugin)) return valid ? vendor_plugin(model, secret, fetcher) : undefined;
  return valid ? factories[model.plugin](secret, fetcher) : undefined;
}

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
  private routes: HostRoutes;
  private readonly content: ProviderContent;
  readonly policy;

  constructor(
    readonly config: Deployment,
    private readonly implementation: Uint8Array,
    private readonly secrets: Readonly<Record<string, string | undefined>>,
    private readonly fetcher: typeof fetch = fetch,
    private readonly identity: CloudIdentity = {},
  ) {
    this.content = new ProviderContent(config);
    const models = (config.catalog ? [] : config.models).map((model) => ({
      route: model,
      snapshot: model_snapshot(config, model, implementation),
    }));
    this.policy = request_policy(config);
    const plugins = new Map<string, NativePlugin>();
    const resolutions = new Map<string, ResolvedRoute>();
    for (const model of models) {
      const id = validate_id(model.snapshot.resolutionId?.value);
      resolutions.set(id, model);
      const plugin = route_plugin(model.route, secrets, fetcher, identity);
      if (plugin) plugins.set(id, plugin);
    }
    this.routes = { current: models, resolutions, plugins, statuses: new Map() };
  }

  get models() {
    return this.routes.current;
  }

  get catalog_status() {
    return {
      generation: this.routes.generation,
      models: this.routes.current.map(({ route }) =>
        this.routes.statuses.get(`${route.id}\0${route.model}`),
      ),
    };
  }

  /** Build a complete candidate, then swap once. Failure preserves existing calls. */
  async reload_catalog(): Promise<void> {
    if (!this.config.catalog) throw new ProviderError("provider_catalog_missing");
    const records = await load_catalog(this.config);
    const latest = records.at(-1);
    const now = Date.now();
    if (
      !latest ||
      now < this.last_time ||
      Date.parse(latest.resolved_at) > now ||
      Date.parse(latest.expires_at) <= now ||
      latest.plugin_sha256 !== hex_digest(this.implementation) ||
      latest.deployment_sha256 !== deployment_digest(this.config)
    )
      throw new ProviderError("provider_catalog_unavailable");
    const digest = catalog_digest(latest);
    const active = this.routes.generation;
    if (
      active &&
      (latest.revision < active.revision ||
        (latest.revision === active.revision && digest !== active.sha256))
    )
      throw new ProviderError("provider_catalog_rollback");
    if (active?.sha256 === digest) return;
    const resolutions = new Map<string, ResolvedRoute>();
    const plugins = new Map<string, NativePlugin>();
    const statuses = new Map<string, CatalogStatus>();
    const cache = new Map<string, NativePlugin>();
    let current: ResolvedRoute[] = [];
    for (const record of records) {
      if (
        record.plugin_sha256 !== latest.plugin_sha256 ||
        record.deployment_sha256 !== latest.deployment_sha256
      )
        continue;
      const config = { ...this.config, models: record.models, resolved_at: record.resolved_at };
      const pin = Buffer.from(catalog_digest(record), "hex");
      const entries = record.models.map((route) => ({
        route,
        snapshot: model_snapshot(config, route, this.implementation, pin),
        expires_at: Date.parse(record.expires_at),
      }));
      for (const entry of entries) {
        const id = validate_id(entry.snapshot.resolutionId?.value);
        resolutions.set(id, entry);
        const profile = profile_digest(entry.route);
        const plugin =
          cache.get(profile) ??
          route_plugin(entry.route, this.secrets, this.fetcher, this.identity);
        if (plugin) {
          cache.set(profile, plugin);
          plugins.set(id, plugin);
        }
      }
      for (const status of record.statuses)
        statuses.set(`${status.route_id}\0${status.model}`, status);
      if (record === latest) current = entries;
    }
    this.routes = {
      current,
      resolutions,
      plugins,
      statuses,
      generation: { revision: latest.revision, sha256: digest },
    };
    this.last_time = now;
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
    const routes = this.routes;
    const selected = routes.resolutions.get(invocation.model?.resolutionId?.value ?? "");
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
    if (
      !selected.snapshot.resolvedAt ||
      timestampDate(selected.snapshot.resolvedAt).getTime() >
        timestampDate(context.requestedAt).getTime()
    )
      throw new ProviderError("future_model_resolution");
    const status = routes.statuses.get(`${selected.route.id}\0${selected.route.model}`);
    if (
      (selected.expires_at !== undefined && selected.expires_at <= now) ||
      (this.config.catalog && (!status || !["active", "deprecated"].includes(status.availability)))
    )
      throw new ProviderError(
        "provider_model_unavailable",
        Code.Unavailable,
        ErrorCategory.DEPENDENCY,
      );
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
        1_000_000n +
      decimal_units(
        selected.route.cloud?.kind === "bedrock"
          ? (selected.route.cloud.guardrail?.maximum_usd ?? "0")
          : "0",
      ) +
      decimal_units(selected.route.vendor?.maximum_extra_usd ?? "0") +
      decimal_units(selected.route.compatible?.gateway?.maximum_extra_usd ?? "0");
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
    const plugin = routes.plugins.get(selected.snapshot.resolutionId?.value ?? "");
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
