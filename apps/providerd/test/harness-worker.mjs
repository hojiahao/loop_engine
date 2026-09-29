// Real compiled Provider and local HTTP supplier for Rust orchestration tests.
// It never accepts production credentials or sends traffic outside loopback.
import { createHash, X509Certificate } from "node:crypto";
import { once } from "node:events";
import { readFile, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { join } from "node:path";
import { toBinary } from "@bufbuild/protobuf";
import {
  ModelResolutionSnapshotSchema,
  PolicyReferenceSchema,
} from "@loop-engine/protocol/provider";
import { plugin_digest, validate_deployment } from "../dist/config.js";
import { ProviderHost } from "../dist/host.js";
import { open_journal } from "../dist/journal.js";
import { create_provider_rpc } from "../dist/rpc.js";

const root = process.argv[2];
const recovery = process.argv[3] === "recover";
if (!root?.startsWith("/")) throw new Error("fixture_root_required");
let calls = 0;
const supplier = createServer(async (request, response) => {
  const chunks = [];
  for await (const chunk of request) chunks.push(chunk);
  const body = JSON.parse(Buffer.concat(chunks).toString());
  if (request.url !== "/v1/chat/completions") throw new Error("fixture_route_denied");
  calls++;
  await writeFile(join(root, "supplier-calls.json"), JSON.stringify(calls), { mode: 0o600 });
  response.writeHead(200, { "content-type": "application/json" });
  const field = await readFile(join(root, "invalid-ast"))
    .then(() => "market.unknown")
    .catch(() => "market.close");
  response.end(
    JSON.stringify({
      id: "synthetic-discovery",
      object: "chat.completion",
      created: 1,
      model: body.model,
      choices: [
        {
          index: 0,
          finish_reason: "stop",
          message: {
            role: "assistant",
            content: JSON.stringify({ ast: { node: "field", field } }),
          },
        },
      ],
      usage: { prompt_tokens: 12, completion_tokens: 18, total_tokens: 30 },
    }),
  );
});
supplier.listen(0, "127.0.0.1");
await once(supplier, "listening");
const schema = JSON.parse(await readFile(join(root, "schema-reference.json"), "utf8"));
const certificate = new X509Certificate(await readFile(join(root, "tls/client.pem")));
const configuration = recovery
  ? JSON.parse(await readFile(join(root, "provider.json"), "utf8"))
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
      schemas: [{ ...schema, path: join(root, "ast-schema.json") }],
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
          features: { structured_output: true },
          compatible: {
            base_url: `http://127.0.0.1:${supplier.address().port}/v1`,
            wire: "chat",
            auth: "none",
          },
        },
      ],
    };
const config = validate_deployment(configuration);
const host = new ProviderHost(config, await plugin_digest(), {}, fetch, {}, recovery);
await open_journal(config.journal, !recovery);
const shutdown = new AbortController();
const rpc = await create_provider_rpc(host, shutdown.signal);
rpc.listen(recovery ? config.port : 0, "127.0.0.1");
await once(rpc, "listening");
if (!recovery) {
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
