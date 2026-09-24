import { Code } from "@connectrpc/connect";
import { ErrorCategory } from "@loop-engine/protocol/provider";
import type { Deployment } from "./config.js";
import { ProviderError } from "./errors.js";

/** Per-process traffic reservations. Durable run-wide budgets belong to loopd. */
export class RequestLimits {
  private entries: { time: number; tokens: bigint; cost: bigint }[] = [];
  private last_time = 0;

  constructor(
    private readonly policy: Deployment["policy"]["rate"],
    private readonly maximum: bigint,
  ) {}

  /** Synchronous admission precedes journal claims; no await can overbook it. */
  reserve(tokens: bigint, cost: bigint, now = performance.now()): void {
    if (!Number.isFinite(now) || now < this.last_time || tokens < 1n || cost < 0n)
      throw new ProviderError("provider_rate_clock", Code.Unavailable, ErrorCategory.INTERNAL);
    this.last_time = now;
    this.entries = this.entries.filter((entry) => entry.time > now - this.policy.window_ms);
    const reserved = this.entries.reduce(
      (total, entry) => ({ tokens: total.tokens + entry.tokens, cost: total.cost + entry.cost }),
      { tokens, cost },
    );
    if (
      this.entries.length >= this.policy.requests ||
      reserved.tokens > BigInt(this.policy.tokens) ||
      reserved.cost > this.maximum
    )
      throw new ProviderError(
        "provider_rate_capacity",
        Code.ResourceExhausted,
        ErrorCategory.RATE_LIMIT,
      );
    this.entries.push({ time: now, tokens, cost });
  }
}
