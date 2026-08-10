// The desktop transport: every request and every event stream goes through the
// app's own Rust core rather than out of the webview.
//
// Same `Transport` interface as `BrowserTransport`, so the console above it is
// unchanged — which is the point of the seam. What differs is what carries the
// traffic, and it differs for three reasons the browser cannot work around:
//
//   1. A webview origin is cross-origin with every host, so a direct fetch
//      would need each operator to allow-list `tauri://localhost` before their
//      desktop could connect. Requests made from Rust are not subject to CORS.
//   2. The device token stays in the OS keychain and is attached in Rust, so an
//      XSS in rendered agent markdown cannot exfiltrate N hosts' credentials.
//   3. `EventSource` cannot set a request header at all, and a `SameSite=Lax`
//      cookie is never sent cross-site — so a desktop webview has no way to
//      open the event stream itself.
//
// The Rust side is `src-tauri/src/proxy`, and `tests/proxy_parity.rs` asserts
// against a real host that what comes back here is byte-identical to what a
// browser's `fetch` would have seen.

import type {
  StreamHandlers,
  Transport,
  TransportRequest,
  TransportResponse,
} from "./types";

/** What `oc_request` answers with. Mirrors `ProxyResponse` in Rust. */
interface ProxyReply {
  status: number;
  statusText: string;
  url: string;
  text: string;
  headers: Record<string, string>;
}

/** The Tauri entry points this transport uses. */
interface TauriBridge {
  invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
  Channel: new <T>() => { onmessage: ((message: T) => void) | null };
}

/**
 * Reads the Tauri bridge off the window.
 *
 * Accessed lazily rather than imported: `frontend/` is one bundle that must
 * also load in a plain browser, where `@tauri-apps/api` is not present. A
 * static import would make the web build depend on it.
 */
function bridge(): TauriBridge {
  const internals = (window as unknown as { __TAURI__?: TauriBridge }).__TAURI__;
  if (!internals) {
    throw new Error("the desktop bridge is unavailable: not running under Tauri");
  }
  return internals;
}

export class ProxyTransport implements Transport {
  /**
   * @param connectionId Which host this transport speaks for. Explicit, and
   *   passed on every call — the Rust side has no notion of a "current"
   *   connection, deliberately, because that single-valued field is what stops
   *   comparable clients from holding more than one host at a time.
   */
  constructor(private readonly connectionId: string) {}

  async request(req: TransportRequest): Promise<TransportResponse> {
    const reply = await bridge().invoke<ProxyReply>("oc_request", {
      connectionId: this.connectionId,
      request: {
        method: req.method,
        // Origin-relative. The host belongs to the connection, so a caller
        // cannot aim one connection's credential at another origin.
        path: pathOf(req.url),
        headers: req.headers,
        body: req.body,
      },
    });

    return {
      status: reply.status,
      statusText: reply.statusText,
      url: reply.url,
      text: reply.text,
      // Rust lowercases every key on the way out, so this lookup matches the
      // case-insensitive `Headers.get` the browser transport hands back.
      header: (name) => reply.headers[name.toLowerCase()] ?? null,
    };
  }

  subscribe(url: string, handlers: StreamHandlers): () => void {
    const { invoke, Channel } = bridge();
    const channel = new Channel<string>();
    let live = true;

    channel.onmessage = (data) => {
      if (live) handlers.onMessage(data);
    };

    void invoke("oc_subscribe", {
      connectionId: this.connectionId,
      path: pathOf(url),
      channel,
    })
      .then(() => handlers.onOpen?.())
      .catch(() => {
        // The stream could not be opened. Reported as terminal rather than
        // reconnecting, because nothing here retries — the console's poll is
        // the safety net, and a second invisible retry loop underneath it would
        // make a dead host look intermittently alive.
        if (live) handlers.onError?.({ reconnecting: false });
      });

    return () => {
      live = false;
      channel.onmessage = null;
    };
  }
}

/**
 * The origin-relative part of a URL the console built.
 *
 * The console composes `${baseUrl}${path}`, and for a desktop connection
 * `baseUrl` is the host's. Stripping it back off here keeps the Rust side the
 * only thing that decides which origin a connection's credential is sent to —
 * so a bug in URL composition cannot leak a token to another host.
 */
function pathOf(url: string): string {
  // A protocol-relative `//authority/path` must not pass through unchanged: the
  // Rust side joins by string concatenation, so against an empty base it would
  // resolve into an authority-bearing URL and carry the connection's session
  // header to a different origin. Rooting it keeps it same-origin.
  if (url.startsWith("//")) return `/${url.slice(2)}`;
  if (url.startsWith("/")) return url;
  try {
    const parsed = new URL(url);
    return `${parsed.pathname}${parsed.search}`;
  } catch {
    // Not absolute and not rooted: hand it over as a same-origin path.
    return `/${url}`;
  }
}
