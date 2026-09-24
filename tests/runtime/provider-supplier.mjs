import { readFileSync } from "node:fs";
import { createServer } from "node:https";

// Synthetic HTTPS supplier. No real upstream is contacted by this fixture.
const server = createServer(
  { cert: readFileSync("/fixture/supplier.pem"), key: readFileSync("/fixture/supplier.key") },
  async (request, response) => {
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    const body = JSON.parse(Buffer.concat(chunks).toString("utf8"));
    if (request.headers.authorization !== "Bearer synthetic-provider-key") {
      response.writeHead(401, { "content-type": "application/json" });
      response.end('{"error":"fixture_auth"}');
      return;
    }
    response.writeHead(200, { "content-type": "application/json" });
    response.end(
      JSON.stringify(
        request.url.endsWith("input_tokens")
          ? { input_tokens: 12, object: "response.input_tokens" }
          : {
              id: "response-fixture",
              object: "response",
              model: body.model,
              status: "completed",
              error: null,
              output: [
                {
                  type: "message",
                  id: "message-fixture",
                  role: "assistant",
                  status: "completed",
                  content: [{ type: "output_text", text: "isolated invocation", annotations: [] }],
                },
              ],
              usage: {
                input_tokens: 12,
                output_tokens: 5,
                total_tokens: 17,
                input_tokens_details: { cached_tokens: 3 },
                output_tokens_details: { reasoning_tokens: 0 },
              },
            },
      ),
    );
  },
);
server.listen(8443, "0.0.0.0");
process.once("SIGTERM", () => {
  server.closeAllConnections();
  server.close();
});
