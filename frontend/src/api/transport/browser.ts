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
