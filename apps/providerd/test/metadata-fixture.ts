import { once } from "node:events";
import { createServer, type IncomingHttpHeaders } from "node:http";
import type { AddressInfo } from "node:net";

/** Local HTTP transport; the requested origin stays visible for authority checks. */
export async function metadata_fixture() {
  const requests: { url: URL; headers: IncomingHttpHeaders; method?: string }[] = [];
  const state = {
    status: 200,
    content_type: "application/json",
    pages: [] as unknown[],
    raw: undefined as string | undefined,
    stall: false,
  };
  const server = createServer((request, response) => {
    requests.push({
      url: new URL(request.url ?? "/", "http://fixture.invalid"),
      headers: request.headers,
      method: request.method,
    });
    if (state.stall) return;
    response.writeHead(state.status, {
      "content-type": state.content_type,
      ...(state.status === 302 ? { location: "/credential-trap" } : {}),
    });
    response.end(state.raw ?? JSON.stringify(state.pages[requests.length - 1] ?? {}));
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const port = (server.address() as AddressInfo).port;
  const origins: string[] = [];
  const fetcher: typeof fetch = async (input, init) => {
    const url = new URL(input instanceof Request ? input.url : String(input));
    origins.push(url.origin);
    return fetch(`http://127.0.0.1:${port}${url.pathname}${url.search}`, init);
  };
  return {
    base_url: `http://127.0.0.1:${port}`,
    requests,
    origins,
    state,
    fetcher,
    async close() {
      server.closeAllConnections();
      await new Promise<void>((resolve) => server.close(() => resolve()));
    },
  };
}
