import { Code } from "@connectrpc/connect";
import { ErrorCategory, type ModelUsage } from "@loop-engine/protocol/provider";
import type { ModelRoute } from "./config.js";
import { ProviderError } from "./errors.js";

/** USD to integer nanodollars; no binary floating-point money arithmetic. */
export function decimal_units(value: string): bigint {
  if (!/^(?:0|[1-9][0-9]{0,5})(?:\.[0-9]{0,8}[1-9])?$/.test(value))
    throw new ProviderError("invalid_money");
  const [whole = "0", fraction = ""] = value.split(".");
  return BigInt(whole) * 1_000_000_000n + BigInt(fraction.padEnd(9, "0"));
}

/** Declared non-token ceilings are reserves, never measured invoice amounts. */
export function extra_cost(model: ModelRoute): bigint {
  return (
    decimal_units(
      model.cloud?.kind === "bedrock" ? (model.cloud.guardrail?.maximum_usd ?? "0") : "0",
    ) +
    decimal_units(model.vendor?.maximum_extra_usd ?? "0") +
    decimal_units(model.compatible?.gateway?.maximum_extra_usd ?? "0")
  );
}

export function reserve_cost(model: ModelRoute, input: bigint, output: bigint): bigint {
  const maximum = [model.input_usd, model.cached_usd, model.cache_creation_usd ?? "0"]
    .map(decimal_units)
    .reduce((left, right) => (left > right ? left : right));
  return (
    (input * maximum + output * decimal_units(model.output_usd) + 999_999n) / 1_000_000n +
    extra_cost(model)
  );
}

/** Token-price estimate only. Reasoning is already included in output tokens. */
export function usage_cost(model: ModelRoute, usage: ModelUsage): bigint {
  const fresh = usage.inputTokens - usage.cachedInputTokens - usage.cacheCreationInputTokens;
  if (
    [
      fresh,
      usage.cachedInputTokens,
      usage.cacheCreationInputTokens,
      usage.outputTokens,
      usage.reasoningTokens,
    ].some((value) => value < 0n) ||
    usage.reasoningTokens > usage.outputTokens ||
    (usage.cacheCreationInputTokens > 0n && model.cache_creation_usd === undefined)
  )
    throw new ProviderError("invalid_priced_usage", Code.DataLoss, ErrorCategory.DEPENDENCY);
  return (
    (fresh * decimal_units(model.input_usd) +
      usage.cachedInputTokens * decimal_units(model.cached_usd) +
      usage.cacheCreationInputTokens * decimal_units(model.cache_creation_usd ?? "0") +
      usage.outputTokens * decimal_units(model.output_usd) +
      999_999n) /
    1_000_000n
  );
}
