import { Resolver } from "node:dns/promises";
import { constants } from "node:fs";
import { open } from "node:fs/promises";
import { createServer } from "node:http";
import { BlockList, createConnection, isIP } from "node:net";
import { fileURLToPath } from "node:url";

const private_ips = new BlockList();
for (const [address, prefix] of [
  ["0.0.0.0", 8],
  ["10.0.0.0", 8],
  ["100.64.0.0", 10],
  ["127.0.0.0", 8],
  ["169.254.0.0", 16],
  ["172.16.0.0", 12],
  ["192.0.0.0", 24],
  ["192.0.2.0", 24],
  ["192.168.0.0", 16],
  ["198.18.0.0", 15],
  ["198.51.100.0", 24],
  ["203.0.113.0", 24],
  ["224.0.0.0", 4],
  ["240.0.0.0", 4],
])
  private_ips.addSubnet(address, prefix, "ipv4");

function valid_host(value) {
  return (
    typeof value === "string" &&
    value.length <= 253 &&
    /^[a-z0-9](?:[a-z0-9.-]*[a-z0-9])?$/.test(value) &&
    value
      .split(".")
      .every(
        (part) =>
          part.length > 0 && part.length <= 63 && !part.startsWith("-") && !part.endsWith("-"),
      )
  );
}

/** Explicit administrative routes only. The client never selects a DNS target. */
export function validate_routes(value) {
  if (
    !value ||
    typeof value !== "object" ||
    Array.isArray(value) ||
    Object.keys(value).sort().join(",") !== "routes,schema" ||
    value.schema !== "loop.provider-egress/v1" ||
    !Array.isArray(value.routes) ||
    value.routes.length < 1 ||
    value.routes.length > 128
  )
    throw new Error("invalid_egress_policy");
  const routes = new Map();
  for (const item of value.routes) {
    if (
      !item ||
      typeof item !== "object" ||
      Array.isArray(item) ||
      Object.keys(item).some(
        (key) => !["host", "port", "target_host", "target_port", "allow_private"].includes(key),
      ) ||
      !valid_host(item.host) ||
      !Number.isInteger(item.port) ||
      item.port < 1 ||
      item.port > 65535 ||
      (item.target_host !== undefined && !valid_host(item.target_host)) ||
      (item.target_port !== undefined &&
        (!Number.isInteger(item.target_port) ||
          item.target_port < 1 ||
          item.target_port > 65535)) ||
      (item.allow_private !== undefined && typeof item.allow_private !== "boolean")
    )
      throw new Error("invalid_egress_policy");
    const key = `${item.host}:${item.port}`;
    if (routes.has(key)) throw new Error("duplicate_egress_route");
    routes.set(key, {
      host: item.target_host ?? item.host,
      port: item.target_port ?? item.port,
      allow_private: item.allow_private ?? false,
    });
  }
  return routes;
}

/** CONNECT tunnels preserve end-to-end supplier TLS; no payload is logged. */
export function create_egress(routes) {
  let active = 0;
  const sockets = new Set();
  const server = createServer(
    { maxHeaderSize: 8192, headersTimeout: 5000, requestTimeout: 5000 },
    (request, response) => {
      response.writeHead(request.method === "GET" && request.url === "/healthz" ? 200 : 405);
      response.end();
    },
  );
  server.maxConnections = 64;
  server.on("connect", (request, client, head) => {
    // CONNECT sockets leave HTTP server ownership, including denied requests.
    // Track them before validation and never rely on a peer to finish closing.
    sockets.add(client);
    client.on("error", () => client.destroy());
    client.once("close", () => sockets.delete(client));
    const route = routes.get(request.url);
    if (
      !route ||
      active >= 32 ||
      head.length > 16_384 ||
      request.headers.authorization ||
      request.headers["proxy-authorization"]
    ) {
      const deadline = setTimeout(() => client.destroy(), 1000);
      client.once("close", () => clearTimeout(deadline));
      client.end("HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n", () => client.destroy());
      return;
    }
    active++;
    let upstream;
    const resolver = new Resolver({ timeout: 1000, tries: 1 });
    let closed = false;
    let count = head.length;
    const timer = setTimeout(close_tunnel, 300_000);
    const connect_timer = setTimeout(close_tunnel, 5000);
    function close_tunnel() {
      if (closed) return;
      closed = true;
      active--;
      clearTimeout(timer);
      clearTimeout(connect_timer);
      client.destroy();
      upstream?.destroy();
      resolver.cancel();
      sockets.delete(client);
    }
    function count_bytes(bytes) {
      count += bytes.length;
      if (count > 33_554_432) close_tunnel();
    }
    client.on("error", close_tunnel);
    client.on("close", close_tunnel);
    // Keep early tunnel bytes buffered until DNS/TCP setup can forward them.
    // Installing a data listener without pausing would otherwise discard them.
    client.pause();
    client.on("data", count_bytes);
    (async () => {
      const resolved =
        isIP(route.host) === 4 ? route.host : (await resolver.resolve4(route.host))[0];
      if (closed) return;
      if (!resolved) return close_tunnel();
      if (!route.allow_private && private_ips.check(resolved, "ipv4")) return close_tunnel();
      upstream = createConnection({ host: resolved, port: route.port, family: 4 });
      upstream.on("error", close_tunnel);
      upstream.on("close", close_tunnel);
      upstream.on("data", count_bytes);
      upstream.once("connect", () => {
        if (closed) return;
        clearTimeout(connect_timer);
        client.write("HTTP/1.1 200 Connection Established\r\n\r\n");
        if (head.length) upstream.write(head);
        client.pipe(upstream);
        upstream.pipe(client);
      });
    })().catch(close_tunnel);
  });
  server.on("clientError", (_error, socket) => socket.destroy());
  return {
    server,
    close() {
      for (const socket of sockets) socket.destroy();
      server.closeAllConnections();
      server.close();
    },
  };
}

async function main() {
  if (process.argv.length !== 3) throw new Error("egress_policy_required");
  const handle = await open(process.argv[2], constants.O_RDONLY | constants.O_NOFOLLOW);
  let value;
  try {
    const info = await handle.stat();
    if (
      !info.isFile() ||
      info.uid !== process.getuid() ||
      (info.mode & 0o077) !== 0 ||
      info.size > 65_536
    )
      throw new Error("invalid_egress_file");
    value = JSON.parse(await handle.readFile("utf8"));
  } finally {
    await handle.close();
  }
  const gateway = create_egress(validate_routes(value));
  gateway.server.listen(8080, "0.0.0.0");
  process.once("SIGTERM", gateway.close);
  process.once("SIGINT", gateway.close);
}

if (process.argv[1] === fileURLToPath(import.meta.url))
  main().catch(() => {
    process.stderr.write("provider_egress_failed\n");
    process.exitCode = 1;
  });
