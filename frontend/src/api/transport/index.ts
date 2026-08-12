// Picking a transport for the environment the console happens to be running in.

import { BrowserTransport } from "./browser";
import { ProxyTransport } from "./proxy";
import type { Transport } from "./types";

export type {
  StreamHandlers,
  Transport,
  TransportRequest,
  TransportResponse,
} from "./types";
export { BrowserTransport } from "./browser";
export { ProxyTransport } from "./proxy";

/**
 * Whether this console is running inside a Tauri webview with the bridge the
 * desktop transport reads.
 *
 * A **runtime** check, not a build flag, and deliberately so: `frontend/` stays
 * one build artifact that the Rust host can serve from
 * `OPENCOMPANY_CONSOLE_DIR` and the desktop can load from disk. Two builds
 * would mean the desktop-only paths stop being typechecked by the web build,
 * which is exactly how a shared codebase quietly stops being shared.
 *
 * Probes `__TAURI__` — the global that `app.withGlobalTauri` in `tauri.conf.json`
 * injects — because that is the very object `tauriCore()` reads. Tauri v2 always
 * injects the lower-level `__TAURI_INTERNALS__`, but only the `withGlobalTauri`
 * global carries `core.invoke` and `core.Channel`; a probe and a reader that
 * looked at different globals would select the desktop transport in an
 * environment where it cannot bridge. Also probed defensively because this runs
 * under Vitest's `node` environment, where `window` does not exist at all.
 *
 * Presence, not shape — `tauriCore()` is what knows the shape, and the split is
 * deliberate. A `__TAURI__` whose `core.invoke` does not resolve is a broken
 * desktop rather than a browser, and `ProxyTransport` saying so beats
 * `BrowserTransport` quietly failing CORS against every host
 * ([#616](https://github.com/tinyhumansai/opencompany/issues/616)).
 */
export function isDesktopRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI__" in window;
}

/**
 * Whether a base url names a host this runtime can actually reach.
 *
 * A browser can reach anything it is given, including the empty string, which
 * means *same origin*: `opencompany serve` mounts the console at the origin
 * serving these assets, so `BrowserTransport` resolves `/api/v1/…` against a
 * document origin that answers. Nothing about that path changes here.
 *
 * The desktop is the runtime with a rule, and the rule is **absolute or
 * nothing**. `ProxyTransport` hands the base url to Rust, which joins it to a
 * path by concatenation — so anything without an authority yields a *relative*
 * url, and `reqwest` refuses those at `send()`. The request never reaches a
 * socket, and the console says "could not be reached" about a host it never
 * addressed. That is issue #613, whose reported form was the empty
 * string — the shortest relative url, and only the shortest. `"/api"` and
 * `"localhost:8080"` fail identically, and the second is what a person types
 * into "Add a host".
 *
 * The protocol check is not belt and braces: `URL` parses `tauri://localhost`
 * and `file:///x` without complaint, and neither is a company host.
 *
 * A runtime check rather than a build flag, and beside `defaultTransport`
 * because the two are one decision — what a base url means depends on which
 * transport is going to carry it.
 */
export function isAddressableBaseUrl(baseUrl: string): boolean {
  if (!isDesktopRuntime()) return true;
  try {
    const { protocol } = new URL(baseUrl);
    return protocol === "http:" || protocol === "https:";
  } catch {
    return false;
  }
}

/**
 * The transport for this environment.
 *
 * Selected here so no caller has to learn the difference. A `connectionId` is
 * required for the desktop lane — the Rust core addresses hosts by id and has
 * no notion of a current one — so a call without it stays on the browser
 * transport, which is correct for the one place that has no connection yet
 * (the bootstrap probe).
 */
export function defaultTransport(connectionId?: string): Transport {
  if (connectionId && isDesktopRuntime()) return new ProxyTransport(connectionId);
  return new BrowserTransport();
}
