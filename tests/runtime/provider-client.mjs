import { createHash, randomUUID } from "node:crypto";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire("/opt/loop-engine/apps/providerd/package.json");
const { create, fromJson, toBinary, toJson } = require("@bufbuild/protobuf");
const { timestampNow } = require("@bufbuild/protobuf/wkt");
const { createClient } = require("@connectrpc/connect");
const { createGrpcTransport } = require("@connectrpc/connect-node");
const {
  ActorKind,
  InvocationState,
  InvokeModelRequestSchema,
  LookupInvocationRequestSchema,
  ModelResponseSchema,
  ModelRole,
  ModelResolutionSnapshotSchema,
  PolicyReferenceSchema,
  ProviderService,
} = require("@loop-engine/protocol/provider");
const pins = JSON.parse(readFileSync(0, "utf8"));
const client = createClient(
  ProviderService,
  createGrpcTransport({
    baseUrl: "https://provider:8091",
    idleConnectionTimeoutMs: 100,
    nodeOptions: {
      ca: readFileSync("/fixture/ca.pem"),
      cert: readFileSync("/fixture/client.pem"),
      key: readFileSync("/fixture/client.key"),
    },
  }),
);
const id = randomUUID();
const command = create(InvokeModelRequestSchema, {
  context: {
    requestId: { value: id },
    correlationId: { value: "isolation" },
    idempotencyKey: { value: randomUUID() },
    requestedAt: timestampNow(),
    actor: { actorId: { value: "loopd-fixture" }, kind: ActorKind.SERVICE },
  },
  invocation: {
    requestId: { value: id },
    model: fromJson(ModelResolutionSnapshotSchema, pins.models[0].snapshot),
    requestPolicy: fromJson(PolicyReferenceSchema, pins.request_policy),
    messages: [
      {
        role: ModelRole.USER,
        content: [{ content: { case: "text", value: { text: "synthetic research idea" } } }],
      },
    ],
    budget: {
      maximumInputTokens: 128n,
      maximumOutputTokens: 64n,
      maximumCost: { currencyCode: "USD", amount: { value: "0.001" } },
      maximumWallTime: { seconds: 4n },
    },
  },
});
const response = (await client.invokeModel(command, { timeoutMs: 4500 })).response;
if (
  response?.content[0]?.content.value.text !== "isolated invocation" ||
  response.usage.chargedCost !== undefined
)
  throw new Error("invalid_isolated_reply");

// Compute the documented request fingerprint independently of Host helpers.
function canonical_json(value) {
  if (Array.isArray(value)) return `[${value.map(canonical_json).join(",")}]`;
  if (value !== null && typeof value === "object")
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonical_json(value[key])}`)
      .join(",")}}`;
  return JSON.stringify(value);
}
const recovered = await client.lookupInvocation(
  create(LookupInvocationRequestSchema, {
    context: {
      ...command.context,
      requestId: { $typeName: "loop.v1.RequestId", value: randomUUID() },
      idempotencyKey: { $typeName: "loop.v1.IdempotencyKey", value: randomUUID() },
      requestedAt: timestampNow(),
    },
    originalRequestId: command.context.requestId,
    originalIdempotencyKey: command.context.idempotencyKey,
    requestSha256: {
      value: createHash("sha256")
        .update("loop.provider-invocation/v1\0")
        .update(canonical_json(toJson(InvokeModelRequestSchema, command)))
        .digest(),
    },
  }),
  { timeoutMs: 4500 },
);
if (
  recovered.state !== InvocationState.COMPLETED ||
  !recovered.response ||
  !Buffer.from(toBinary(ModelResponseSchema, recovered.response)).equals(
    toBinary(ModelResponseSchema, response),
  )
)
  throw new Error("invalid_isolated_recovery");
process.stdout.write("isolated invocation\n");
