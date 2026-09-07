// The browser transport: `fetch` and `EventSource`, exactly as the console has
// always used them.
//
// This file must stay a *literal* restatement of what `client.ts` and
// `use-events.ts` did inline before the seam existed. Every option here —
// `credentials: "include"`, `withCredentials: true`, the readyState check that
// distinguishes a dead stream from a reconnecting one — was load-bearing then
// and is load-bearing now. The gate on this refactor is that the end-to-end
// suite passes unmodified, and that is only meaningful if nothing in here
// "improves" on the original along the way.

import type {
  StreamHandlers,
  Transport,
  TransportRequest,
  TransportResponse,
} from "./types";

export class BrowserTransport implements Transport {
  async request(req: TransportRequest): Promise<TransportResponse> {
    const res = await fetch(req.url, {
      method: req.method,
      headers: req.headers,
      body: req.body,
      // The session is an HttpOnly cookie, so it only travels if we ask.
      // Same-origin (the normal deployment, baseUrl "") this is a no-op.
      // Cross-origin dev (`?api=http://localhost:8080` from a Vite server on
      // another port) additionally needs the host to allow-list the origin with
      // Access-Control-Allow-Credentials — use the Vite proxy so the console
      // stays same-origin.
      credentials: "include",
      // Only ever set by a request issued from `pagehide` — see
      // `TransportRequest.keepalive`. `undefined` for every other call, which
      // `fetch` treats as `false`.
      keepalive: req.keepalive,
      // The request deadline the client attaches, so a host that accepts the
      // connection and never answers cancels the socket instead of hanging
      // this promise forever. `undefined` for a call with no bound.
      signal: req.signal,
    });

    // Read the body here rather than handing the caller a live `Response`: the
    // interface promises a plain value, and a transport that runs the request
    // in another process cannot hand back a stream anyway.
    const text = await res.text();
    return {
      status: res.status,
      statusText: res.statusText,
      url: res.url,
      text,
      header: (name) => res.headers.get(name),
    };
  }

  subscribe(
    url: string,
    handlers: StreamHandlers,
    headers?: Record<string, string>,
  ): () => void {
    // A session carried in a header cannot ride an `EventSource` — the API has
    // no way to set one. Falling through to it anyway would open the stream
    // *anonymously*: the host answers 401, the console quietly degrades to its
    // poll, and a hub console would render as a company where nothing ever
    // happens rather than as one it is not signed in to. So a credentialed
    // stream takes the fetch lane instead.
    if (headers && Object.keys(headers).length > 0) {
      return streamWithFetch(url, handlers, headers);
    }

    // `EventSource` throws in an environment that has none, and on a malformed
    // URL. Both mean "no stream here"; the caller falls back to its poll.
    const source = new EventSource(url, { withCredentials: true });

    source.onopen = () => handlers.onOpen?.();
    source.onmessage = (msg) => handlers.onMessage(msg.data as string);
    source.onerror = () => {
      // On a 404 or a wrong content-type the browser closes the stream and
      // does not reconnect (readyState === CLOSED); on a transient drop it
      // reconnects on its own. Reporting which one happened is what lets the
      // caller stay silent about the second.
      const closed = source.readyState === EventSource.CLOSED;
      handlers.onError?.({ reconnecting: !closed });
      if (closed) source.close();
    };

    return () => source.close();
  }
}

/**
 * How long to wait before reopening a stream that dropped.
 *
 * `EventSource` reconnects on its own; this lane has to do it by hand, and the
 * delay is what stops a host that closes the connection immediately from
 * becoming a request loop. Three seconds is `EventSource`'s own common default.
 */
const RECONNECT_DELAY_MS = 3_000;

/**
 * An SSE stream over `fetch`, for a credential `EventSource` cannot carry.
 *
 * Deliberately a *minimal* reader rather than a general SSE implementation: it
 * parses the one framing the host emits (`data:` lines, blank-line separated)
 * and ignores `event:`, `id:` and `retry:`, none of which the console reads. A
 * fuller parser here would be a second protocol to keep correct for no caller.
 *
 * It reproduces exactly the two things the `EventSource` lane promises its
 * caller and nothing more: `onOpen` when the stream is live, and an `onError`
 * whose `reconnecting` says whether anything further will arrive. A 401 is
 * terminal — retrying a refused credential just refuses it again — while a
 * dropped connection is retried, which is the split `EventSource` makes for
 * itself and which `use-events.ts` already branches on.
 */
function streamWithFetch(
  url: string,
  handlers: StreamHandlers,
  headers: Record<string, string>,
): () => void {
  // Not `let cancelled = false` plus a check: the read below blocks until the
  // next frame arrives, which on a quiet company is minutes. Aborting is what
  // makes unsubscribing take effect immediately rather than at the next event.
  const abort = new AbortController();
  let retry: ReturnType<typeof setTimeout> | null = null;
  let stopped = false;

  const open = async (): Promise<void> => {
    let res: Response;
    try {
      res = await fetch(url, {
        headers: { ...headers, accept: "text/event-stream" },
        signal: abort.signal,
        // Harmless where the credential is a header, and correct for a host
        // that also set a cookie — matching the request lane exactly.
        credentials: "include",
      });
    } catch {
      // An abort lands here too, and must not be reported as a failure: the
      // caller asked for this and is no longer listening.
      if (!stopped) fail({ reconnecting: true });
      return;
    }

    if (res.status === 401 || res.status === 403) {
      // A refused credential will be refused again. Say so once, terminally,
      // so the caller falls back to polling instead of hammering the host.
      fail({ reconnecting: false });
      return;
    }
    if (!res.ok || !res.body) {
      // No body means no stream — a 404, or a host that answered with
      // something that is not an event stream at all.
      fail({ reconnecting: false });
      return;
    }

    handlers.onOpen?.();

    const reader = res.body.getReader();
    const decoder = new TextDecoder();
    // Frames are split on a blank line, so a partial one has to survive across
    // reads: a chunk boundary falls wherever the network put it, not where the
    // protocol did.
    let buffer = "";
    for (;;) {
      let chunk: ReadableStreamReadResult<Uint8Array>;
      try {
        chunk = await reader.read();
      } catch {
        if (!stopped) fail({ reconnecting: true });
        return;
      }
      if (chunk.done) {
        // The host closed a stream that was working. Reopen — this is the
        // ordinary end of a long-lived connection, not a failure.
        if (!stopped) fail({ reconnecting: true });
        return;
      }
      buffer += decoder.decode(chunk.value, { stream: true });
      // Everything up to the last frame boundary is complete; the remainder is
      // the start of the next frame and stays in the buffer.
      const frames = buffer.split("\n\n");
      buffer = frames.pop() ?? "";
      for (const frame of frames) {
        const data = dataOf(frame);
        if (data !== null) handlers.onMessage(data);
      }
    }
  };

  const fail = (info: { reconnecting: boolean }): void => {
    if (stopped) return;
    handlers.onError?.(info);
    if (!info.reconnecting) return;
    retry = setTimeout(() => {
      if (!stopped) void open();
    }, RECONNECT_DELAY_MS);
  };

  void open();

  return () => {
    stopped = true;
    if (retry !== null) clearTimeout(retry);
    abort.abort();
  };
}

/**
 * The payload of one SSE frame, or `null` when it carries none.
 *
 * A frame may spread its payload over several `data:` lines, which the protocol
 * says to rejoin with newlines. Comment lines (`:` first) are keep-alives and
 * must not be delivered as an empty event — the console would try to parse one
 * as JSON on every heartbeat.
 */
function dataOf(frame: string): string | null {
  const lines = frame
    .split("\n")
    .filter((line) => line.startsWith("data:"))
    // `data: x` and `data:x` are the same frame; exactly one leading space is
    // part of the framing rather than of the payload.
    .map((line) => line.slice(5).replace(/^ /, ""));
  return lines.length > 0 ? lines.join("\n") : null;
}
