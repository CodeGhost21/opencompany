// The one place that reads Tauri's injected global.
//
// `app.withGlobalTauri` assigns `window.__TAURI__` the whole `@tauri-apps/api`
// bundle, and **Tauri v2 namespaces it by module**: the top-level keys are
// `app`, `core`, `dpi`, `event`, `image`, `menu`, `path`, `tray`, `webview`,
// `webviewWindow` and `window`. `invoke` and `Channel` live under `core`. The
// bare `__TAURI__.invoke` is the **v1** shape, and under v2 it is `undefined`.
//
// That one level of nesting cost the desktop every command it makes
// ([#616](https://github.com/tinyhumansai/opencompany/issues/616)). Two readers
// dereferenced the v1 shape, so `oc_connect` never ran, no connection was
// registered, and the console reported an unreachable host — a network-shaped
// symptom for a bug that never opened a socket.
//
// ## One reader
//
// Three call sites read this global and two of them read it wrongly, which is
// the whole failure: a shape that lives in more than one place is a shape that
// can be wrong in one of them. Everything that needs the bridge comes through
// `tauriCore()`, so the next version bump that moves a name is one edit and one
// failing test rather than a silent no-op per call site.
//
// ## Read lazily, never imported
//
// `frontend/` is one bundle that must also load in a plain browser, where
// `@tauri-apps/api` is not present. A static import would make the web build
// depend on it — so the runtime shape is read off the window, and the package
// is imported only by the test that pins this file to it.

/**
 * The Tauri v2 `core` module, narrowed to what the desktop transport uses.
 *
 * Structurally identical to `invoke` and `Channel` from `@tauri-apps/api/core`;
 * `test/unit/desktop-bridge.test.ts` asserts that against the real package, so
 * this interface cannot drift from it unnoticed.
 */
export interface TauriCore {
  invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
  Channel: new <T>() => { onmessage: ((message: T) => void) | null };
}

/**
 * The Tauri bridge, or `null` when this is not a Tauri webview.
 *
 * `invoke` is probed rather than assumed: a `__TAURI__` carrying no callable
 * `core.invoke` is not a bridge, whatever else it holds, and returning it would
 * move the failure to a `TypeError` at the first command. `window` itself is
 * probed because unit tests run under Vitest's `node` environment.
 */
export function tauriCore(): TauriCore | null {
  if (typeof window === "undefined") return null;
  const core = (window as { __TAURI__?: { core?: TauriCore } }).__TAURI__?.core;
  return typeof core?.invoke === "function" ? core : null;
}
