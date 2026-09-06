import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { DEFAULT_REQUEST_TIMEOUT_MS, OpenCompanyClient } from "@/api/client";
import type {
  StreamHandlers,
  Transport,
  TransportRequest,
  TransportResponse,
} from "@/api/transport";
import { ApiError } from "@/api/types";

/**
 * The request deadline (issue #2014).
 *
 * Every `client.get` behind a view's load path — MCP servers, connections,
 * agents, workspace, memory — reaches the host through the one shared `request`
 * path, and that path had no timeout: a host that accepted the connection and
 * then never answered left the read's promise pending forever, so the `catch`
 * every view already holds for a failed load never ran and the view sat on its
 * skeleton. These pin that a read now rejects at its bound instead of hanging,
 * that a mutation is left unbounded, and that a caller's own abort stays
 * distinguishable from a timeout.
 *
 * The never-settling stub below is exactly the hang. On the pre-fix client the
 * first assertion never resolves — the test times out rather than passing.
 */

/** A transport whose reply the test stages; records every request it is handed. */
class StubTransport implements Transport {
  readonly seen: TransportRequest[] = [];

  constructor(
    private readonly reply: (req: TransportRequest) => Promise<Partial<TransportResponse>>,
  ) {}

  async request(req: TransportRequest): Promise<TransportResponse> {
    this.seen.push(req);
    const staged = await this.reply(req);
    return {
      status: staged.status ?? 200,
      statusText: staged.statusText ?? "",
      url: staged.url ?? req.url,
      text: staged.text ?? "",
      header: staged.header ?? (() => null),
    };
  }

  subscribe(_url: string, _handlers: StreamHandlers): () => void {
    return () => {};
  }
}

/** A transport that accepts the connection and never answers. */
const NEVER = () => new Promise<Partial<TransportResponse>>(() => {});

function clientOn(transport: Transport): OpenCompanyClient {
  return new OpenCompanyClient(
    { baseUrl: "https://host.test", company: null, operatorToken: null, sessionHeader: null },
    transport,
  );
}

describe("the request deadline", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("rejects a stalled GET with a timeout error at the default bound", async () => {
    const client = clientOn(new StubTransport(NEVER));

    const read = client.get("/api/v1/company/mcp/servers");
    const rejection = expect(read).rejects.toMatchObject({ status: 0, code: "timeout" });

    await vi.advanceTimersByTimeAsync(DEFAULT_REQUEST_TIMEOUT_MS);
    await rejection;
  });

  it("hands the transport a signal so a browser fetch can cancel the socket", async () => {
    const stub = new StubTransport(async () => ({ text: "[]" }));
    await clientOn(stub).get("/api/v1/company/mcp/servers");

    expect(stub.seen).toHaveLength(1);
    expect(stub.seen[0].signal).toBeInstanceOf(AbortSignal);
    expect(stub.seen[0].signal?.aborted).toBe(false);
  });

  it("leaves a fast GET untouched and arms no late rejection", async () => {
    const client = clientOn(new StubTransport(async () => ({ text: '{"ok":true}' })));

    await expect(client.get("/api/v1/company")).resolves.toEqual({ ok: true });
    // The timer for the resolved read is cleared, so time passing is a no-op.
    await vi.advanceTimersByTimeAsync(DEFAULT_REQUEST_TIMEOUT_MS * 2);
  });

  it("honours a caller's shorter deadline", async () => {
    const client = clientOn(new StubTransport(NEVER));

    const read = client.get("/api/v1/company/memory", { timeoutMs: 5_000 });
    const rejection = expect(read).rejects.toMatchObject({ code: "timeout" });

    await vi.advanceTimersByTimeAsync(5_000);
    await rejection;
  });

  it("lets a caller disable the bound for a read expected to run long", async () => {
    const client = clientOn(new StubTransport(NEVER));

    let settled = false;
    void client.get("/api/v1/company/slow", { timeoutMs: null }).then(
      () => (settled = true),
      () => (settled = true),
    );

    await vi.advanceTimersByTimeAsync(DEFAULT_REQUEST_TIMEOUT_MS * 3);
    expect(settled).toBe(false);
  });

  it("does not bound a mutation by default", async () => {
    const client = clientOn(new StubTransport(NEVER));

    let settled = false;
    void client.post("/api/v1/company/chat", { text: "hi" }).then(
      () => (settled = true),
      () => (settled = true),
    );

    await vi.advanceTimersByTimeAsync(DEFAULT_REQUEST_TIMEOUT_MS * 3);
    expect(settled).toBe(false);
  });

  it("surfaces a caller's abort as an AbortError, not a timeout", async () => {
    const client = clientOn(new StubTransport(NEVER));
    const abort = new AbortController();

    const read = client.get("/api/v1/company/mcp/servers", { signal: abort.signal });
    abort.abort();

    const err = await read.then(
      () => {
        throw new Error("expected the read to reject");
      },
      (e: unknown) => e,
    );
    expect(err).toBeInstanceOf(Error);
    expect((err as Error).name).toBe("AbortError");
    expect(err).not.toBeInstanceOf(ApiError);
  });

  it("normalizes a caller's abort reason to AbortError even when it is an ordinary Error", async () => {
    const client = clientOn(new StubTransport(NEVER));
    const abort = new AbortController();

    const read = client.get("/api/v1/company/mcp/servers", { signal: abort.signal });
    abort.abort(new Error("superseded"));

    const err = await read.then(
      () => {
        throw new Error("expected the read to reject");
      },
      (e: unknown) => e,
    );
    expect(err).toBeInstanceOf(Error);
    expect((err as Error).name).toBe("AbortError");
  });
});
