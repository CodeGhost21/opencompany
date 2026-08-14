/**
 * Give the jsdom suites jsdom's `localStorage`, on every Node this repo can be
 * checked out on (issue #852).
 *
 * # The failure this exists to prevent
 *
 * Three suites — `connection-registry`, `desktop-bridge`, `tour-resume` — open
 * with `window.localStorage.clear()`. On Node 25 all 57 of their tests fail on
 * that line with:
 *
 *     TypeError: window.localStorage.clear is not a function
 *
 * On Node 22, the version CI pins and `frontend/Dockerfile` ships, they pass.
 * The jsdom version is not the variable: 27.0.1 and 27.4.0 both pass on Node 22
 * and both fail on Node 25.
 *
 * # Why
 *
 * Node 25 exposes WebStorage as an unflagged global, so `localStorage` is now a
 * property of `globalThis` before any test environment is built. Vitest's
 * `populateGlobal` copies the jsdom window onto the global, but skips any key
 * the global already owns unless that key is on its own hard-coded list — and
 * `localStorage`/`sessionStorage` are not on it:
 *
 *     if (k in global) return keysArray.includes(k)
 *
 * So on Node 22 (`'localStorage' in globalThis === false`) the key is copied and
 * `window.localStorage` is jsdom's `Storage`. On Node 25 the key is dropped and
 * `window.localStorage` resolves to Node's own built-in — which, with no
 * `--localstorage-file` given, is an empty object with no `clear`, no `getItem`
 * and no `setItem`. Hence the error, and hence its shape: not a Storage that
 * behaves differently, a non-Storage.
 *
 * # What this does
 *
 * Re-points the two storage globals at the jsdom window's real ones. Vitest
 * hands us the `JSDOM` instance as `globalThis.jsdom`, so this reaches the
 * genuine `Storage` objects rather than substituting a hand-written stand-in:
 * the suites keep testing against the same implementation they always did, on
 * Node 22 and Node 25 alike. `window === globalThis` under this environment, so
 * defining the property here is what fixes `window.localStorage` in the tests.
 *
 * On Node 22 this is a no-op — the globals are already jsdom's and the identity
 * check short-circuits. Under `environment: "node"`, which is this suite's
 * default and most of its files, there is no `globalThis.jsdom` and nothing
 * happens at all; a node-environment test still sees whatever its Node version
 * gives it.
 *
 * This is a workaround for a Vitest/Node interaction, not for a jsdom one.
 * When Vitest adds `localStorage` to the keys it carries over from the window,
 * this file can go.
 */

type StorageName = "localStorage" | "sessionStorage";

const dom = (globalThis as { jsdom?: { window: Record<StorageName, Storage> } }).jsdom;

if (dom?.window) {
  for (const name of ["localStorage", "sessionStorage"] as const) {
    const fromJsdom = dom.window[name];

    // Identity, not feature-detection. `globalThis[name]` on Node 25 is an
    // object, and truthy, and would survive a `typeof` check; the only thing
    // that distinguishes it from the one the tests need is that it is not the
    // window's.
    if (fromJsdom && globalThis[name] !== fromJsdom) {
      Object.defineProperty(globalThis, name, {
        value: fromJsdom,
        // Writable and configurable because Vitest's teardown deletes and
        // restores globals between files, and a locked property would make it
        // throw rather than tear down.
        writable: true,
        configurable: true,
      });
    }
  }
}
