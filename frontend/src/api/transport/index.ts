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
 * injects — because that is the very object `bridge()` reads in `proxy.ts`.
 * Tauri v2 always injects the lower-level `__TAURI_INTERNALS__`, but only the
 * `withGlobalTauri` global carries `invoke` and `Channel`; a probe and a reader
 * that looked at different globals would select the desktop transport in an
 * environment where it cannot bridge. Also probed defensively because this runs
 * under Vitest's `node` environment, where `window` does not exist at all.
 */
export function isDesktopRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI__" in window;
}

/**
 * Whether a base url names a host this runtime can actually reach.
 *
 * An empty base url means *same origin*, and that is a real host only in a
 * browser: `opencompany serve` mounts the console at the origin serving these
 * assets, so `BrowserTransport` resolves `/api/v1/…` against a document origin
 * that answers. In the desktop the document origin is `tauri://localhost`.
 * Nothing serves an API there and nothing ever will — and the request does not
 * even reach a socket to fail: `ProxyTransport` hands the base url to Rust,
 * where joining it to a path yields a *relative* url `reqwest` cannot request.
 *
 * So this is a property of the transport rather than a preference. The same
 * empty string is a working host in one shell and an unaddressable one in the
 * other, which is why the check is a runtime one and lives beside
 * `defaultTransport` — the two decisions are the same decision.
 *
 * See issue #613: the desktop added such a connection on launch, selected it,
 * and showed "Couldn't reach a company host at this origin" every time, while
 * its embedded host sat healthy and unselected in the rail.
 */
export function isAddressableBaseUrl(baseUrl: string): boolean {
  return baseUrl !== "" || !isDesktopRuntime();
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
