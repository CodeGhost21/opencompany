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
 * Whether this console is running inside a Tauri webview.
 *
 * A **runtime** check, not a build flag, and deliberately so: `frontend/` stays
 * one build artifact that the Rust host can serve from
 * `OPENCOMPANY_CONSOLE_DIR` and the desktop can load from disk. Two builds
 * would mean the desktop-only paths stop being typechecked by the web build,
 * which is exactly how a shared codebase quietly stops being shared.
 *
 * `__TAURI_INTERNALS__` is injected by the Tauri runtime before any app code
 * runs. Probed defensively because this also runs under Vitest's `node`
 * environment, where `window` does not exist at all.
 */
export function isDesktopRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
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
