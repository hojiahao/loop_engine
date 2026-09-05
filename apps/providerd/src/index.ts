import { createProviderServer } from "./server.js";

const port = Number.parseInt(process.env.PROVIDERD_PORT ?? "8090", 10);
if (!Number.isSafeInteger(port) || port < 1 || port > 65_535) {
  throw new Error("PROVIDERD_PORT must be an integer between 1 and 65535");
}

createProviderServer().listen(port, "127.0.0.1", () => {
  process.stdout.write(`${JSON.stringify({ component: "providerd", event: "listening", port })}\n`);
});
