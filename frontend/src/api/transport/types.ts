// The seam between the console and whatever actually carries its HTTP.
//
// The console has exactly two ways to touch a host: `fetch` in
// `api/client.ts`, and `EventSource` in `hooks/use-events.ts`. Both are browser
// APIs, and both stop working the moment the console is not a same-origin
// browser page:
//
//   - `EventSource` cannot set a request header, so it cannot carry the session
//     header a desktop client authenticates with. It also cannot carry a cookie
//     cross-site, because the session cookie is `SameSite=Lax`.
//   - `fetch` can set headers, but a cross-origin call still needs the host to
//     have allow-listed the caller's origin — a per-server configuration step
//     the desktop's headline feature (connect to N servers) cannot demand.
//
// So the desktop routes both through its own Rust core instead, where neither
// CORS nor the cookie rules apply. This interface is the narrowest description
// of what the console needs, so that the browser and the desktop can each
// satisfy it without either knowing about the other.
//
// Deliberately not a `fetch` polyfill. It carries only what the console
// actually uses: a method, a URL, string headers, a string body, and on the way
// back a status, a URL, one header reader, and the body text. Anything wider
// would be a second HTTP client to keep correct.

/** One request, fully resolved: `url` is absolute or origin-relative. */
export interface TransportRequest {
  method: string;
  url: string;
  headers: Record<string, string>;
  /** Already serialized. `undefined` for a bodyless request. */
  body?: string;
  /**
   * Ask the browser to keep this request alive past the page that issued it —
   * for the one caller that fires a request from `pagehide` and needs it to
   * still land after the document is gone (`disconnectPresenceBeacon`).
   * Meaningful only to `BrowserTransport`, which threads it straight to
   * `fetch`'s own `keepalive` option; `ProxyTransport` ignores it, because a
   * Tauri `invoke` call is core-process IPC, not a page-scoped browser
   * request, and has no equivalent teardown-survival problem to solve.
   */
  keepalive?: boolean;
  /**
   * Tears the request down when it fires. `BrowserTransport` threads it to
   * `fetch`, which cancels the socket; `ProxyTransport` cannot cancel an
   * in-flight Tauri `invoke` and ignores it, the same way it ignores
   * `keepalive`. The client races the abort itself either way, so a transport
   * that cannot honour the signal still stops the caller waiting.
   */
  signal?: AbortSignal;
}

export interface TransportResponse {
  status: number;
  /** May be empty; callers fall back to `HTTP <status>`. */
  statusText: string;
  /** The final URL, after any redirect. Used only in diagnostics. */
  url: string;
  text: string;
  /**
   * One response header, case-insensitively. Only `content-disposition` is read
   * today (the task-export filename); the whole header map is deliberately not
   * exposed so a transport is not obliged to reproduce one faithfully.
   */
  header(name: string): string | null;
}

/** Callbacks for a server-pushed event stream. */
export interface StreamHandlers {
  /** One event payload, as raw text. The caller parses it. */
  onMessage(data: string): void;
  onOpen?(): void;
  /**
   * `reconnecting: false` means the transport has given up and will not retry;
   * the caller falls back to polling. `true` means a retry is already in
   * flight and nothing is required of the caller.
   */
  onError?(info: { reconnecting: boolean }): void;
}

export interface Transport {
  /**
   * Performs one request.
   *
   * Rejects **only** when the host could not be reached at all. Every HTTP
   * status, including 5xx, resolves — deciding what a status means is the
   * caller's job, and a transport that threw on 401 would move session
   * handling in here.
   */
  request(req: TransportRequest): Promise<TransportResponse>;

  /**
   * Opens an event stream, returning an unsubscribe.
   *
   * Throws when streaming is unavailable in this environment, which the caller
   * treats as "poll instead" rather than as an error worth surfacing.
   *
   * `headers` carries the same credential the request path uses. It is passed
   * rather than left implicit because a stream that authenticated differently
   * from the requests beside it is the subtlest possible bug: the views would
   * load fine and simply never update, which reads as "quiet company" rather
   * than as "unauthorized". A transport that cannot set headers must degrade
   * loudly instead — see `BrowserTransport.subscribe`.
   */
  subscribe(
    url: string,
    handlers: StreamHandlers,
    headers?: Record<string, string>,
  ): () => void;
}
