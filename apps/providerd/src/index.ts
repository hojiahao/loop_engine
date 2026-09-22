import { toJson } from "@bufbuild/protobuf";
import {
  ModelResolutionSnapshotSchema,
  PolicyReferenceSchema,
} from "@loop-engine/protocol/provider";

import { load_deployment, plugin_digest } from "./config.js";
import { ProviderHost } from "./host.js";
import { open_journal } from "./journal.js";
import { create_provider_rpc } from "./rpc.js";
import { create_provider_server } from "./server.js";

async function main() {
  const config_path = process.env.PROVIDERD_DEPLOYMENT;
  const config = config_path ? await load_deployment(config_path) : undefined;
  const host = config ? new ProviderHost(config, await plugin_digest(), process.env) : undefined;
  if (process.argv[2] === "--describe") {
    if (!host) throw new Error("provider_deployment_required");
    process.stdout.write(
      `${JSON.stringify(
        {
          models: host.models.map(({ route, snapshot }) => ({
            id: route.id,
            snapshot: toJson(ModelResolutionSnapshotSchema, snapshot),
          })),
          request_policy: toJson(PolicyReferenceSchema, host.policy),
          verification: "contract_only",
        },
        null,
        2,
      )}\n`,
    );
    return;
  }
  if (process.argv.length > 2) throw new Error("invalid_provider_arguments");
  const raw_port = process.env.PROVIDERD_PORT ?? "8090";
  const port = Number(raw_port);
  if (!/^[1-9][0-9]{0,4}$/.test(raw_port) || port > 65_535) throw new Error("invalid_health_port");
  const shutdown = new AbortController();
  if (host) {
    await open_journal(host.config.journal);
    const rpc = await create_provider_rpc(host, shutdown.signal);
    rpc.listen(host.config.port, "127.0.0.1");
    rpc.on("error", () => {
      process.stderr.write("provider_listener_failed\n");
      shutdown.abort();
      process.exitCode = 1;
    });
    shutdown.signal.addEventListener("abort", () => rpc.close(), { once: true });
  }
  const health = create_provider_server();
  health.listen(port, "127.0.0.1", () => {
    process.stdout.write(
      `${JSON.stringify({ component: "providerd", event: "listening", port, model_rpc_configured: Boolean(host) })}\n`,
    );
  });
  health.on("error", () => {
    process.stderr.write("provider_health_failed\n");
    shutdown.abort();
    process.exitCode = 1;
  });
  shutdown.signal.addEventListener("abort", () => health.close(), { once: true });
  process.once("SIGTERM", () => shutdown.abort());
  process.once("SIGINT", () => shutdown.abort());
}

main().catch(() => {
  process.stderr.write("provider_start_failed\n");
  process.exitCode = 1;
});
