// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";

import { BrowserTransport } from "@/api/transport";

/**
 * The event stream a cross-origin console has to use.
 *
 * `EventSource` cannot set a request header, so a console carrying its own
 * session cannot use it — it would open the stream anonymously, take a 401, and
 * fall back to polling. The console would then render a company where nothing
 * ever happens, which looks like a quiet company rather than a broken one.
 *
 * So a credentialed stream is read over `fetch` instead, and this file pins the
 * parts `EventSource` was doing for free. Framing is the one to watch: chunks
 * arrive where the network splits them, not where the protocol does, so a
 * parser that assumes one read is one frame works perfectly in development and
 * drops events under load.
 */

/** A response whose body yields `chunks` in order, as an SSE stream would. */
function streamOf(chunks: string[], status = 200): Response {
  const encoder = new TextEncoder();
  let i = 0;
  return {
    ok: status >= 200 && status < 300,
    status,
    body: {
      getReader: () => ({
        read: async () =>
          i < chunks.length
            ? { done: false, value: encoder.encode(chunks[i++]) }
            : // Never resolves again: a live stream simply waits. Ending would
              // trip the reconnect path and confuse what each test is pinning.
              new Promise<never>(() => {}),
      }),
    },
  } as unknown as Response;
}

/** Lets assertions run after the reader's promises have settled. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("the fetch event stream", () => {
  it("carries the credential EventSource could not", async () => {
    const fetchMock = vi.fn(async (_url: string, _init?: RequestInit) => streamOf([]));
    vi.stubGlobal("fetch", fetchMock);

    new BrowserTransport().subscribe(
      "https://acme.example.com/api/v1/company/events",
      { onMessage: () => {} },
      { "x-opencompany-session": "acme.tok" },
    );
    await settle();

    const [, init] = fetchMock.mock.calls[0];
    const headers = init.headers as Record<string, string>;
    expect(headers["x-opencompany-session"]).toBe("acme.tok");
    expect(headers["accept"]).toBe("text/event-stream");
  });

  it("delivers one event per frame", async () => {
    vi.stubGlobal("fetch", async () => streamOf(['data: {"a":1}\n\ndata: {"b":2}\n\n']));
    const seen: string[] = [];

    new BrowserTransport().subscribe(
      "https://acme.example.com/api/v1/company/events",
      { onMessage: (d) => seen.push(d) },
      { "x-opencompany-session": "acme.tok" },
    );
    await settle();

    expect(seen).toEqual(['{"a":1}', '{"b":2}']);
  });

  it("reassembles a frame split across chunks", async () => {
    // THE regression this parser exists to prevent. A chunk boundary falls
    // wherever the network put it — mid-payload here — and a reader that
    // treated each read as a frame would emit two truncated events and lose
    // the real one, only under load.
    vi.stubGlobal("fetch", async () => streamOf(['data: {"a"', ':1}\n\n']));
    const seen: string[] = [];

    new BrowserTransport().subscribe(
      "https://acme.example.com/api/v1/company/events",
      { onMessage: (d) => seen.push(d) },
      { "x-opencompany-session": "acme.tok" },
    );
    await settle();

    expect(seen).toEqual(['{"a":1}']);
  });

  it("ignores keep-alive comments", async () => {
    // A heartbeat is not an event. Delivering one as an empty string would have
    // the console attempt `JSON.parse("")` on every beat.
    vi.stubGlobal("fetch", async () => streamOf([': ping\n\ndata: real\n\n']));
    const seen: string[] = [];

    new BrowserTransport().subscribe(
      "https://acme.example.com/api/v1/company/events",
      { onMessage: (d) => seen.push(d) },
      { "x-opencompany-session": "acme.tok" },
    );
    await settle();

    expect(seen).toEqual(["real"]);
  });

  it("gives up on a refused credential instead of retrying it", async () => {
    // A 401 will be a 401 next time too. Reporting it as retryable would turn a
    // signed-out console into a request loop against the host.
    vi.stubGlobal("fetch", async () => streamOf([], 401));
    const errors: Array<{ reconnecting: boolean }> = [];

    new BrowserTransport().subscribe(
      "https://acme.example.com/api/v1/company/events",
      { onMessage: () => {}, onError: (e) => errors.push(e) },
      { "x-opencompany-session": "acme.tok" },
    );
    await settle();

    expect(errors).toEqual([{ reconnecting: false }]);
  });

  it("stops reading once unsubscribed", async () => {
    // The read blocks until the next frame, which on a quiet company is
    // minutes. Without the abort, unsubscribing would take effect at the next
    // event rather than immediately — and a switched-away host would keep
    // delivering into a dead view.
    let aborted = false;
    vi.stubGlobal("fetch", async (_url: string, init: RequestInit) => {
      init.signal?.addEventListener("abort", () => {
        aborted = true;
      });
      return streamOf([]);
    });

    const stop = new BrowserTransport().subscribe(
      "https://acme.example.com/api/v1/company/events",
      { onMessage: () => {} },
      { "x-opencompany-session": "acme.tok" },
    );
    await settle();
    stop();

    expect(aborted).toBe(true);
  });

  it("leaves an uncredentialed stream on EventSource", () => {
    // The same-origin console must keep the browser's own reconnect and cookie
    // handling. Routing it through this lane would be a rewrite of a working
    // path for no gain.
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    const constructed: string[] = [];
    vi.stubGlobal(
      "EventSource",
      class {
        static readonly CLOSED = 2;
        readyState = 0;
        constructor(url: string) {
          constructed.push(url);
        }
        close() {}
      },
    );

    new BrowserTransport().subscribe("/api/v1/company/events", { onMessage: () => {} });

    expect(constructed).toEqual(["/api/v1/company/events"]);
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
