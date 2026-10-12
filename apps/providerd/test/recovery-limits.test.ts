import { randomUUID } from "node:crypto";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { create, toJson } from "@bufbuild/protobuf";
import { timestampNow } from "@bufbuild/protobuf/wkt";
import {
  InvocationState,
  InvokeModelRequestSchema,
  LookupInvocationRequestSchema,
} from "@loop-engine/protocol/provider";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { digest_json } from "../src/identity.js";
import { type InvocationRecord, read_invocation } from "../src/journal.js";
import { test_fixture, test_request } from "./fixture.js";

// Only delay the OS read boundary. Authentication, Host concurrency, deadlines,
// cancellation and generation retain their production implementations.
vi.mock("../src/journal.js", async (import_original) => {
  const original = await import_original<typeof import("../src/journal.js")>();
  return { ...original, read_invocation: vi.fn() };
});

const read = vi.mocked(read_invocation);
let directory: string;
let fixture: Awaited<ReturnType<typeof test_fixture>> | undefined;

beforeEach(async () => {
  directory = await mkdtemp(join(tmpdir(), "loop-recovery-limits-"));
  read.mockReset().mockResolvedValue({ state: "absent" });
  fixture = await test_fixture(directory, (config) => {
    config.policy.concurrency = 1;
    config.policy.wall_time_ms = 1000;
  });
});

afterEach(async () => {
  vi.restoreAllMocks();
  await fixture?.close();
  fixture = undefined;
  if (directory) await rm(directory, { recursive: true, force: true });
});

function lookup_request() {
  if (!fixture) throw new Error("missing_fixture");
  const original = test_request(fixture.host);
  return create(LookupInvocationRequestSchema, {
    context: {
      ...original.context,
      requestId: { $typeName: "loop.v1.RequestId", value: randomUUID() },
      idempotencyKey: { $typeName: "loop.v1.IdempotencyKey", value: randomUUID() },
      requestedAt: timestampNow(),
    },
    originalRequestId: original.context?.requestId,
    originalIdempotencyKey: original.context?.idempotencyKey,
    requestSha256: {
      $typeName: "loop.v1.Sha256Digest",
      value: digest_json("loop.provider-invocation/v1", toJson(InvokeModelRequestSchema, original)),
    },
  });
}

async function lookup(signal = new AbortController().signal) {
  if (!fixture) throw new Error("missing_fixture");
  return fixture.host.lookup(lookup_request(), fixture.principal, signal);
}

function generation_request() {
  if (!fixture) throw new Error("missing_fixture");
  const request = test_request(fixture.host);
  if (!request.invocation?.budget) throw new Error("missing_budget");
  request.invocation.budget.maximumWallTime = {
    $typeName: "google.protobuf.Duration",
    seconds: 1n,
    nanos: 0,
  };
  return request;
}

describe("bounded invocation recovery lifecycle", () => {
  it("rejects wall-clock regression before reading any additional evidence", async () => {
    const now = Date.now();
    const clock = vi.spyOn(Date, "now").mockReturnValue(now);
    expect((await lookup()).state).toBe(InvocationState.ABSENT);
    clock.mockReturnValue(now - 1);
    await expect(lookup()).rejects.toMatchObject({ code: "provider_clock_regressed" });
    expect(read).toHaveBeenCalledTimes(1);
  });

  it("shares the same bounded capacity across pending reads and generation", async () => {
    if (!fixture) throw new Error("missing_fixture");
    const pending = Promise.withResolvers<InvocationRecord>();
    read.mockReturnValueOnce(pending.promise);
    const first = lookup();
    try {
      await expect(lookup()).rejects.toMatchObject({ code: "provider_capacity" });
      await expect(
        fixture.host.invoke(generation_request(), fixture.principal, new AbortController().signal),
      ).rejects.toMatchObject({ code: "provider_capacity" });
      expect(fixture.requests).toHaveLength(0);
      expect(read).toHaveBeenCalledTimes(1);
    } finally {
      pending.resolve({ state: "absent" });
      await first;
    }
    const started = Promise.withResolvers<void>();
    fixture.state.on_request = () => started.resolve();
    fixture.state.delay = 20;
    const generated = fixture.host.invoke(
      generation_request(),
      fixture.principal,
      new AbortController().signal,
    );
    try {
      await Promise.race([
        started.promise,
        generated.then(() => {
          throw new Error("generation_ended_early");
        }),
      ]);
      await expect(lookup()).rejects.toMatchObject({ code: "provider_capacity" });
    } finally {
      await generated;
    }
    expect((await lookup()).state).toBe(InvocationState.ABSENT);
    expect(fixture.requests).toHaveLength(2);
  });

  it("keeps a cancelled read's slot until the underlying read settles", async () => {
    const pending = Promise.withResolvers<InvocationRecord>();
    read.mockReturnValueOnce(pending.promise);
    const controller = new AbortController();
    const first = lookup(controller.signal);
    controller.abort();
    try {
      await expect(first).rejects.toMatchObject({ code: "provider_cancelled" });
      await expect(lookup()).rejects.toMatchObject({ code: "provider_capacity" });
      expect(read).toHaveBeenCalledTimes(1);
    } finally {
      pending.resolve({ state: "absent" });
      await pending.promise;
    }
    expect((await lookup()).state).toBe(InvocationState.ABSENT);
    expect(read).toHaveBeenCalledTimes(2);
    expect(fixture?.requests).toHaveLength(0);
  });

  it("keeps an expired read's slot until the underlying read settles", async () => {
    const pending = Promise.withResolvers<InvocationRecord>();
    read.mockReturnValueOnce(pending.promise);
    const first = lookup();
    try {
      await expect(first).rejects.toMatchObject({ code: "provider_deadline" });
      await expect(lookup()).rejects.toMatchObject({ code: "provider_capacity" });
      expect(read).toHaveBeenCalledTimes(1);
    } finally {
      pending.resolve({ state: "absent" });
      await pending.promise;
    }
    expect((await lookup()).state).toBe(InvocationState.ABSENT);
    expect(read).toHaveBeenCalledTimes(2);
    expect(fixture?.requests).toHaveLength(0);
  });
});
