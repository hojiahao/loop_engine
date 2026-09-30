// Real compiled Provider and local HTTP supplier for Rust orchestration tests.
// It never accepts production credentials or sends traffic outside loopback.
import { createHash, X509Certificate } from "node:crypto";
import { once } from "node:events";
import { readFile, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { toBinary } from "@bufbuild/protobuf";
import { Code } from "@connectrpc/connect";
import {
  ErrorCategory,
  ModelResolutionSnapshotSchema,
  PolicyReferenceSchema,
} from "@loop-engine/protocol/provider";
import { plugin_digest, validate_deployment } from "../dist/config.js";
import { ProviderError } from "../dist/errors.js";
import { ProviderHost } from "../dist/host.js";
import { open_journal } from "../dist/journal.js";
import { create_provider_rpc } from "../dist/rpc.js";

const root = process.argv[2];
const recovery = process.argv[3] === "recover";
const resume = process.argv[3] === "resume";
if (!root?.startsWith("/")) throw new Error("fixture_root_required");
const saved =
  recovery || resume ? JSON.parse(await readFile(join(root, "provider.json"), "utf8")) : undefined;
let calls = JSON.parse(await readFile(join(root, "supplier-calls.json"), "utf8").catch(() => "0"));
const bodies = JSON.parse(
  await readFile(join(root, "supplier-bodies.json"), "utf8").catch(() => "[]"),
);
const supplier = createServer(async (request, response) => {
  const chunks = [];
  for await (const chunk of request) chunks.push(chunk);
  const body = JSON.parse(Buffer.concat(chunks).toString());
  if (request.url !== "/v1/chat/completions") throw new Error("fixture_route_denied");
  calls++;
  bodies.push(body);
  await writeFile(join(root, "supplier-calls.json"), JSON.stringify(calls), { mode: 0o600 });
  await writeFile(join(root, "supplier-bodies.json"), JSON.stringify(bodies), { mode: 0o600 });
  const wait = await readFile(join(root, "supplier-delay"), "utf8").catch(() => "0");
  if (wait === "2000") await delay(2000);
  response.writeHead(200, { "content-type": "application/json" });
  const field = await readFile(join(root, "invalid-ast"))
    .then(() => "market.unknown")
    .catch(() => "market.close");
  const result = body.messages.find((message) => message.role === "tool");
  const first = body.tools?.length > 0 && !result;
  const invalid = await readFile(join(root, "invalid-tool"), "utf8").catch(() => "");
  if (result) {
    const description = JSON.parse(result.content);
    const assistant = body.messages.find((message) => message.role === "assistant");
    if (
      body.tool_choice !== "none" ||
      result.tool_call_id !== "call_describe" ||
      assistant?.tool_calls?.[0]?.id !== result.tool_call_id ||
      description.schema !== "loop.research-description/v1" ||
      description.sample.role !== "in_sample" ||
      description.snapshot_ids[0] !== "snapshot.discovery" ||
      !description.fields.some((field) => field.name === "market.close") ||
      "uri" in description ||
      "path" in description
    ) {
      throw new Error("fixture_tool_context_invalid");
    }
  }
  response.end(
    JSON.stringify({
      id: "synthetic-discovery",
      object: "chat.completion",
      created: 1,
      model: body.model,
      choices: [
        {
          index: 0,
          finish_reason: first ? "tool_calls" : "stop",
          message: first
            ? {
                role: "assistant",
                content: null,
                tool_calls: [
                  {
                    id: "call_describe",
                    type: "function",
                    function: {
                      name: invalid === "unknown" ? "unregistered_tool" : "research_describe",
                      arguments:
                        invalid === "arguments" ? JSON.stringify({ path: "/tmp/private" }) : "{}",
                    },
                  },
                ],
              }
            : {
                role: "assistant",
                content: JSON.stringify({ ast: { node: "field", field } }),
              },
        },
      ],
      usage: { prompt_tokens: 12, completion_tokens: 18, total_tokens: 30 },
    }),
  );
});
supplier.listen(
  resume ? Number(new URL(saved.models[0].compatible.base_url).port) : 0,
  "127.0.0.1",
);
await once(supplier, "listening");
const schema = JSON.parse(await readFile(join(root, "schema-reference.json"), "utf8"));
const certificate = new X509Certificate(await readFile(join(root, "tls/client.pem")));
const tool_schemas = await Promise.all(
  ["tool-input", "tool-result"].map(async (name) => ({
    ...JSON.parse(await readFile(join(root, `${name}-reference.json`), "utf8")),
    path: join(root, `${name}.json`),
  })),
);
const configuration = saved
  ? saved
  : {
      schema: "loop.provider-deployment/v1",
      resolved_at: new Date().toISOString(),
      port: 8091,
      tls: {
        ca: join(root, "tls/ca.pem"),
        certificate: join(root, "tls/server.pem"),
        key: join(root, "tls/server.key"),
      },
      journal: join(root, "journal"),
      schemas: [{ ...schema, path: join(root, "ast-schema.json") }, ...tool_schemas],
      principals: [
        {
          certificate_sha256: createHash("sha256").update(certificate.raw).digest("hex"),
          actor_id: "agent.discovery",
          actor_kind: "agent",
          model_ids: ["synthetic"],
        },
      ],
      policy: {
        id: "discovery-provider",
        revision: "1",
        input_tokens: 4096,
        output_tokens: 128,
        maximum_usd: "1",
        wall_time_ms: 5000,
        concurrency: 1,
      },
      models: [
        {
          id: "synthetic",
          plugin: "openai_compatible",
          model: "synthetic-model-v1",
          alias: "synthetic",
          context_tokens: 8192,
          input_token_limit: 4096,
          output_tokens: 128,
          input_usd: "1",
          output_usd: "2",
          cached_usd: "0.1",
          features: { structured_output: true, tools: true },
          compatible: {
            base_url: `http://127.0.0.1:${supplier.address().port}/v1`,
            wire: "chat",
            auth: "none",
            // This synthetic endpoint exercises the strict tool contract. The
            // compatible adapter otherwise defaults to denying strict tools.
            strict_tools: true,
          },
        },
      ],
    };
const config = validate_deployment(configuration);
const host = new ProviderHost(config, await plugin_digest(), {}, fetch, {}, recovery);
const invoke = host.invoke.bind(host);
host.invoke = async (...args) => {
  const response = await invoke(...args);
  const invalid = await readFile(join(root, "invalid-response"), "utf8").catch(() => "");
  if (invalid === "usage") response.usage = undefined;
  if (invalid === "identity")
    response.requestId = { ...response.requestId, value: "wrong.request" };
  return response;
};
// Faults apply only to this local test listener; successful requests still use
// the actual authenticated ProviderHost lookup and immutable journal.
let lookups = JSON.parse(await readFile(join(root, "lookup-calls.json"), "utf8").catch(() => "0"));
const lookup = host.lookup.bind(host);
host.lookup = async (...args) => {
  lookups++;
  await writeFile(join(root, "lookup-calls.json"), JSON.stringify(lookups), { mode: 0o600 });
  const failures = Number(await readFile(join(root, "lookup-failures"), "utf8").catch(() => "0"));
  if (lookups <= failures) {
    throw new ProviderError("fixture_lookup_transient", Code.Unavailable, ErrorCategory.DEPENDENCY);
  }
  return lookup(...args);
};
await open_journal(config.journal, !recovery);
const shutdown = new AbortController();
const rpc = await create_provider_rpc(host, shutdown.signal);
rpc.listen(saved ? config.port : 0, "127.0.0.1");
await once(rpc, "listening");
if (!saved) {
  config.port = rpc.address().port;
  await writeFile(join(root, "provider.json"), JSON.stringify(config), { mode: 0o600 });
}
process.stdout.write(
  `${JSON.stringify({
    port: rpc.address().port,
    model: recovery
      ? ""
      : Buffer.from(toBinary(ModelResolutionSnapshotSchema, host.models[0].snapshot)).toString(
          "base64",
        ),
    policy: Buffer.from(toBinary(PolicyReferenceSchema, host.policy)).toString("base64"),
  })}\n`,
);
const stop = () => {
  shutdown.abort();
  rpc.close();
  supplier.closeAllConnections();
  supplier.close();
};
process.once("SIGTERM", stop);
process.once("SIGINT", stop);
