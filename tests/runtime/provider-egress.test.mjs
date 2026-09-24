import assert from "node:assert/strict";
import { Resolver } from "node:dns/promises";
import { once } from "node:events";
import { request } from "node:http";
import { createConnection, createServer } from "node:net";
import { test } from "node:test";
import { create_egress, validate_routes } from "../../infra/provider-egress.mjs";

async function tunnel(port, destination, headers = {}) {
  return new Promise((resolve, reject) => {
    const client = request({
      host: "127.0.0.1",
      port,
      method: "CONNECT",
      path: destination,
      headers,
    });
    client.on("connect", (response, socket) => resolve({ status: response.statusCode, socket }));
    client.on("error", reject);
    client.setTimeout(1000, () => client.destroy(new Error("fixture_timeout")));
    client.end();
  });
}

test("egress permits an exact approved route and denies every other destination", async (t) => {
  const target = createServer((socket) => socket.pipe(socket));
  target.listen(0, "127.0.0.1");
  await once(target, "listening");
  const gateway = create_egress(
    validate_routes({
      schema: "loop.provider-egress/v1",
      routes: [
        {
          host: "supplier.test",
          port: 443,
          target_host: "127.0.0.1",
          target_port: target.address().port,
          allow_private: true,
        },
      ],
    }),
  );
  gateway.server.listen(0, "127.0.0.1");
  await once(gateway.server, "listening");
  t.after(() => {
    gateway.close();
    target.close();
  });
  const port = gateway.server.address().port;
  const allowed = await tunnel(port, "supplier.test:443");
  assert.equal(allowed.status, 200);
  const received = once(allowed.socket, "data");
  allowed.socket.write("tls-owned-by-endpoints");
  assert.equal(String((await received)[0]), "tls-owned-by-endpoints");
  allowed.socket.destroy();
  for (const destination of [
    "supplier.test:80",
    "supplier.test.evil:443",
    "127.0.0.1:443",
    "user@supplier.test:443",
  ]) {
    const denied = await tunnel(port, destination);
    assert.equal(denied.status, 403);
    denied.socket.destroy();
  }
  const authorization = await tunnel(port, "supplier.test:443", {
    "proxy-authorization": "secret",
  });
  assert.equal(authorization.status, 403);
  authorization.socket.destroy();
  assert.equal((await fetch(`http://127.0.0.1:${port}/unapproved`)).status, 405);
});

test("egress rejects private resolution without explicit administrative approval", async (t) => {
  const gateway = create_egress(
    validate_routes({
      schema: "loop.provider-egress/v1",
      routes: [{ host: "localhost", port: 443 }],
    }),
  );
  gateway.server.listen(0, "127.0.0.1");
  await once(gateway.server, "listening");
  t.after(gateway.close);
  await assert.rejects(tunnel(gateway.server.address().port, "localhost:443"));
});

test("CONNECT preserves early payload while resolution is pending", {
  timeout: 3000,
}, async (t) => {
  const target = createServer((socket) => socket.pipe(socket));
  target.listen(0, "127.0.0.1");
  await once(target, "listening");
  let release;
  const resolving = new Promise((resolve) => {
    release = resolve;
  });
  // Delay only resolution; both sides of the tunnel still use real TCP sockets.
  t.mock.method(Resolver.prototype, "resolve4", async () => {
    await resolving;
    return ["127.0.0.1"];
  });
  t.after(() => {
    release();
    target.close();
  });
  const gateway = create_egress(
    validate_routes({
      schema: "loop.provider-egress/v1",
      routes: [
        {
          host: "supplier.test",
          port: 443,
          target_host: "pending.test",
          target_port: target.address().port,
          allow_private: true,
        },
      ],
    }),
  );
  gateway.server.listen(0, "127.0.0.1");
  await once(gateway.server, "listening");
  t.after(gateway.close);
  const client = createConnection({ host: "127.0.0.1", port: gateway.server.address().port });
  t.after(() => client.destroy());
  const echoed = new Promise((resolve, reject) => {
    let result = "";
    client.on("error", reject);
    client.on("data", (bytes) => {
      result += bytes.toString();
      if (result.endsWith("early-tunnel-payload")) resolve(result);
    });
  });
  const connected = once(gateway.server, "connect");
  client.write("CONNECT supplier.test:443 HTTP/1.1\r\nHost: supplier.test:443\r\n\r\n");
  await connected;
  // Send a separate TCP write after CONNECT has been parsed, before DNS completes.
  client.write("early-tunnel-payload");
  await new Promise((resolve) => setTimeout(resolve, 100));
  release();
  assert.equal(await echoed, "HTTP/1.1 200 Connection Established\r\n\r\nearly-tunnel-payload");
});

test("denied CONNECT closes a half-open peer and releases its socket", {
  timeout: 3000,
}, async (t) => {
  const gateway = create_egress(
    validate_routes({
      schema: "loop.provider-egress/v1",
      routes: [{ host: "supplier.test", port: 443 }],
    }),
  );
  gateway.server.listen(0, "127.0.0.1");
  await once(gateway.server, "listening");
  t.after(gateway.close);
  const connected = once(gateway.server, "connection");
  const client = createConnection({
    host: "127.0.0.1",
    port: gateway.server.address().port,
    allowHalfOpen: true,
  });
  t.after(() => client.destroy());
  client.on("error", () => {});
  const [accepted] = await connected;
  const closed = once(accepted, "close");
  const received = once(client, "data");
  client.write("CONNECT denied.test:443 HTTP/1.1\r\nHost: denied.test:443\r\n\r\n");
  assert.match(String((await received)[0]), /^HTTP\/1\.1 403 Forbidden/);
  await closed;
  assert.equal(accepted.destroyed, true);
  assert.equal(client.writableEnded, false);
});

test("denied CONNECT handles errors on its real socket", { timeout: 3000 }, async (t) => {
  const gateway = create_egress(
    validate_routes({
      schema: "loop.provider-egress/v1",
      routes: [{ host: "supplier.test", port: 443 }],
    }),
  );
  // Observe the socket while validation is rejecting it, before its write drains.
  let rejected;
  gateway.server.on("connect", (_request, socket) => {
    rejected = socket;
    socket.emit("error", new Error("peer_reset"));
  });
  gateway.server.listen(0, "127.0.0.1");
  await once(gateway.server, "listening");
  t.after(gateway.close);
  const client = createConnection({ host: "127.0.0.1", port: gateway.server.address().port });
  t.after(() => client.destroy());
  client.on("error", () => {});
  const connected = once(gateway.server, "connect");
  client.write("CONNECT denied.test:443 HTTP/1.1\r\nHost: denied.test:443\r\n\r\n");
  await connected;
  assert.ok(rejected);
  assert.equal(rejected.destroyed, true);
});

test("egress policy has no wildcard, URL, credential or implicit route", () => {
  for (const host of [
    "*.example.com",
    "https://example.com",
    "a..example.com",
    "EXAMPLE.com",
    "user:password@example.com",
  ])
    assert.throws(() =>
      validate_routes({ schema: "loop.provider-egress/v1", routes: [{ host, port: 443 }] }),
    );
  assert.throws(() => validate_routes({ schema: "loop.provider-egress/v1", routes: [] }));
  assert.throws(() =>
    validate_routes({
      schema: "loop.provider-egress/v1",
      routes: [{ host: "example.com", port: 443, secret: "denied" }],
    }),
  );
  assert.throws(() =>
    validate_routes({
      schema: "loop.provider-egress/v1",
      routes: [
        { host: "example.com", port: 443 },
        { host: "example.com", port: 443 },
      ],
    }),
  );
});
