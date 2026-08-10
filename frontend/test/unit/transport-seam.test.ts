import { describe, expect, it } from "vitest";

import { OpenCompanyClient } from "@/api/client";
import type {
  StreamHandlers,
  Transport,
  TransportRequest,
  TransportResponse,
} from "@/api/transport";
import { ApiError } from "@/api/types";

/**
 * The transport seam's contract.
 *
 * `OpenCompanyClient` used to call `fetch` and `EventSource` directly. It now
 * calls a `Transport`, so that a desktop shell can route the same client
 * through its own core — where neither CORS nor the `SameSite=Lax` cookie rule
 * applies — without the console knowing which wire it is on.
 *
 * These are not tests of `fetch`. They pin the part a second implementation has
 * to reproduce exactly: what the client hands down, and what it does with what
 * comes back. The desktop's proxy transport is only correct if a client on top
 * of it behaves identically to one on top of the browser, and "identically"
 * means these assertions.
 */

/** Records what it was asked for and answers with whatever the test staged. */
class StubTransport implements Transport {
  readonly seen: TransportRequest[] = [];
  readonly subscribed: string[] = [];
  handlers: StreamHandlers | null = null;
  unsubscribed = 0;

  constructor(
    private readonly reply: (req: TransportRequest) => Partial<TransportResponse> | Error,
  ) {}

  async request(req: TransportRequest): Promise<TransportResponse> {
    this.seen.push(req);
    const staged = this.reply(req);
    if (staged instanceof Error) throw staged;
    return {
      status: staged.status ?? 200,
      statusText: staged.statusText ?? "",
      url: staged.url ?? req.url,
      text: staged.text ?? "",
      header: staged.header ?? (() => null),
    };
  }

  subscribe(url: string, handlers: StreamHandlers): () => void {
    this.subscribed.push(url);
    this.handlers = handlers;
    return () => {
      this.unsubscribed += 1;
    };
  }
}

/**
 * The `ApiError` `run` threw.
 *
 * Typed rather than a bare `.catch(e => e)`: `client.get<T>` infers `T` as
 * `unknown` when the caller does not name it, so awaiting the catch yields
 * `unknown` and every assertion below would need its own cast.
 */
async function caught(run: () => Promise<unknown>): Promise<ApiError> {
  try {
    await run();
  } catch (error) {
    return error as ApiError;
  }
  throw new Error("expected the call to reject");
}

function clientOn(
  transport: Transport,
  config: { baseUrl?: string; company?: string | null; operatorToken?: string | null } = {},
): OpenCompanyClient {
  return new OpenCompanyClient(
    {
      baseUrl: config.baseUrl ?? "",
      company: config.company ?? null,
      operatorToken: config.operatorToken ?? null,
    },
    transport,
  );
}

describe("the transport seam", () => {
  it("hands the transport a fully resolved request", async () => {
    const t = new StubTransport(() => ({ text: '{"ok":true}' }));
    const client = clientOn(t, { baseUrl: "https://host.test", operatorToken: "tok" });

    await client.post("/api/v1/company/chat", { message: "hi" });

    expect(t.seen).toHaveLength(1);
    const [req] = t.seen;
    // Absolute URL, not a base plus a path: a transport that runs in another
    // process cannot be expected to re-derive the join.
    expect(req.url).toBe("https://host.test/api/v1/company/chat");
    expect(req.method).toBe("POST");
    expect(req.headers["content-type"]).toBe("application/json");
    expect(req.headers.authorization).toBe("Bearer tok");
    // Already serialized. The transport never sees an object to stringify, so
    // two transports cannot disagree about how a body is encoded.
    expect(req.body).toBe('{"message":"hi"}');
  });

  it("sends no content-type and no body for a bodyless request", async () => {
    const t = new StubTransport(() => ({ text: "" }));
    await clientOn(t).get("/healthz");

    const [req] = t.seen;
    expect(req.body).toBeUndefined();
    expect(req.headers["content-type"]).toBeUndefined();
    expect(req.headers.authorization).toBeUndefined();
  });

  it("treats any 2xx as success and everything else as an error", async () => {
    for (const status of [200, 201, 204]) {
      const t = new StubTransport(() => ({ status, text: "" }));
      await expect(clientOn(t).get("/healthz")).resolves.toBeUndefined();
    }
    for (const status of [300, 400, 500]) {
      const t = new StubTransport(() => ({ status, text: "" }));
      await expect(clientOn(t).get("/healthz")).rejects.toBeInstanceOf(ApiError);
    }
  });

  it("reads the host's error envelope, and keeps an unrecognised body as detail", async () => {
    const enveloped = new StubTransport(() => ({
      status: 409,
      text: '{"error":"already running","code":"conflict"}',
    }));
    const err = await caught(() => clientOn(enveloped).get("/api/v1/company"));
    expect(err).toBeInstanceOf(ApiError);
    expect(err.status).toBe(409);
    expect(err.code).toBe("conflict");
    expect(err.message).toBe("already running");

    // A proxy's HTML error page is the only clue to which hop gave up, so it is
    // kept rather than discarded — the property #380 established.
    const proxied = new StubTransport(() => ({
      status: 502,
      statusText: "Bad Gateway",
      text: "<html>nginx</html>",
    }));
    const gateway = await caught(() => clientOn(proxied).get("/api/v1/company"));
    expect(gateway.code).toBe("http_502");
    expect(gateway.message).toBe("Bad Gateway");
    expect(gateway.detail).toBe("<html>nginx</html>");
  });

  it("falls back to HTTP <status> when the transport reports no status text", async () => {
    // A proxy transport crossing a process boundary may not carry one; the
    // message must still say something.
    const t = new StubTransport(() => ({ status: 503, statusText: "", text: "" }));
    const err = await caught(() => clientOn(t).get("/api/v1/company"));
    expect(err.message).toBe("HTTP 503");
  });

  it("turns an unreachable host into a network_error, whatever the transport threw", async () => {
    const t = new StubTransport(() => new Error("ECONNREFUSED"));
    const err = await caught(() => clientOn(t, { baseUrl: "https://host.test" }).get("/healthz"));
    expect(err).toBeInstanceOf(ApiError);
    expect(err.status).toBe(0);
    expect(err.code).toBe("network_error");
    // The transport's own message is deliberately not surfaced: it is a
    // different string per transport, and this one is shown to a person.
    expect(err.message).toContain("https://host.test");
  });

  it("notifies on a 401, except from the auth routes", async () => {
    const t = new StubTransport(() => ({ status: 401, text: "" }));
    const client = clientOn(t);
    let notified = 0;
    client.onUnauthorized = () => {
      notified += 1;
    };

    await client.get("/api/v1/company").catch(() => {});
    expect(notified).toBe(1);

    // A failed sign-in 401s as its normal answer; treating that as an expired
    // session would loop the login view.
    await client.post("/api/v1/company/auth/verify", { code: "x" }).catch(() => {});
    expect(notified).toBe(1);
  });

  it("reads the document filename through the transport's header accessor", async () => {
    const t = new StubTransport(() => ({
      text: "<h1>export</h1>",
      header: (name: string) =>
        name.toLowerCase() === "content-disposition"
          ? 'attachment; filename="task-42.html"'
          : null,
    }));
    const doc = await clientOn(t).getDocument("/api/v1/company/tasks/42/export");
    expect(doc.text).toBe("<h1>export</h1>");
    expect(doc.filename).toBe("task-42.html");
  });

  it("subscribes to the addressed company's event stream", () => {
    const single = new StubTransport(() => ({}));
    clientOn(single).subscribeToEvents(null, { onMessage: () => {} });
    expect(single.subscribed).toEqual(["/api/v1/company/events"]);

    const scoped = new StubTransport(() => ({}));
    const client = clientOn(scoped, { baseUrl: "https://host.test", company: "acme" });
    const stop = client.subscribeToEvents(undefined, { onMessage: () => {} });
    expect(scoped.subscribed).toEqual(["https://host.test/api/v1/companies/acme/events"]);

    stop();
    expect(scoped.unsubscribed).toBe(1);
  });

  it("delivers stream payloads as raw text for the caller to parse", () => {
    const t = new StubTransport(() => ({}));
    const seen: string[] = [];
    clientOn(t).subscribeToEvents(null, { onMessage: (data) => seen.push(data) });

    t.handlers?.onMessage('{"type":"agent_reply"}');
    expect(seen).toEqual(['{"type":"agent_reply"}']);
  });
});
