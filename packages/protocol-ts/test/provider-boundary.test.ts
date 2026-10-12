import { readFileSync } from "node:fs";
import { fromBinary, toBinary } from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";

import * as provider from "../src/wire/provider.js";

function fixture(name: string): Uint8Array {
  return new Uint8Array(
    readFileSync(new URL(`../../../fixtures/contracts/protocol/v1/${name}`, import.meta.url)),
  );
}

describe("provider public wire boundary", () => {
  it("exports model transport types without research, job, or holdout symbols", () => {
    expect(provider).toHaveProperty("ProviderService");
    expect(provider).toHaveProperty("ModelInvocationSchema");
    expect(provider).toHaveProperty("ModelStreamEventSchema");

    for (const symbol of Object.keys(provider)) {
      expect(symbol).not.toMatch(/Approval|Backtest|Factor|Holdout|Job|Research/);
    }
  });

  it("keeps lookup and original invocation identities separate across the wire", () => {
    const request = fromBinary(
      provider.LookupInvocationRequestSchema,
      fixture("provider_lookup_v1.binpb"),
    );
    expect(request.context?.requestId?.value).toBe("lookup.1");
    expect(request.originalRequestId?.value).toBe("invoke.1");
    expect(request.originalIdempotencyKey?.value).toBe("invoke-key.1");
    expect(request.requestSha256?.value).toEqual(Uint8Array.from({ length: 32 }, (_, i) => i));
    expect(
      fromBinary(
        provider.LookupInvocationRequestSchema,
        toBinary(provider.LookupInvocationRequestSchema, request),
      ),
    ).toEqual(request);
    expect(provider.ProviderService.method.lookupInvocation.input).toBe(
      provider.LookupInvocationRequestSchema,
    );
  });

  it("preserves a completed result and distinguishes reservation from billing", () => {
    const response = fromBinary(
      provider.LookupInvocationResponseSchema,
      fixture("provider_completed_v1.binpb"),
    );
    expect(response.state).toBe(provider.InvocationState.COMPLETED);
    expect(response.response?.requestId?.value).toBe("invoke.1");
    expect(response.response?.content[0]?.content).toMatchObject({
      case: "text",
      value: { text: "fixture result" },
    });
    expect(response.response?.usage?.chargedCost).toBeUndefined();
    expect(response.reservedCost?.amount?.value).toBe("0.125");
    expect(response.reservedCost?.currencyCode).toBe("USD");
    expect(
      fromBinary(
        provider.LookupInvocationResponseSchema,
        toBinary(provider.LookupInvocationResponseSchema, response),
      ),
    ).toEqual(response);
  });

  it("keeps unknown states distinguishable from all declared outcomes", () => {
    const response = fromBinary(provider.LookupInvocationResponseSchema, Uint8Array.of(8, 127));
    expect(response.state).toBe(127);
    expect(
      Object.values(provider.InvocationStateSchema.values).map((state) => state.number),
    ).not.toContain(response.state);
    expect(response.response).toBeUndefined();
    expect(response.reservedCost).toBeUndefined();
  });
});
