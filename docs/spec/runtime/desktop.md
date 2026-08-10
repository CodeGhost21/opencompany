# The desktop client

OpenCompany ships as an orchestration **server** and a **desktop client**. The
console stays one codebase and the server stays one binary; what the split adds
are the seams a desktop needs — several hosts at once, a credential a webview
can carry, and a host running in-process.

Code: `src-tauri/` (a separate crate, not a workspace member) and
`frontend/src/connections/`.

## Why `src-tauri` is not a workspace member

Making it one would put the whole Tauri tree into `cargo metadata --locked`,
which CI runs first on a runner with no webkit or GTK — turning a desktop-only
dependency into a hard requirement for checking the server. The host stays a
plain `path` dependency, so a change to it is picked up with no publishing step.

The cost is that no root `cargo` invocation reaches the desktop, including
`--all-features`. The `Desktop` job in `.github/workflows/ci.yml` is what builds
and tests it; without that lane the crate would be compiled by nothing, which is
[issue #475](https://github.com/tinyhumansai/opencompany/issues/475)'s shape.

## N connections, and no active one

`frontend/src/connections/registry.ts` holds a map of connections and
deliberately has **no "active connection"** field. Selecting a host in the UI is
a rendering choice, not a state change: every connection stays registered and
probed, so one host being unreachable reddens one row rather than blanking the
app.

That single-valued field is what stops comparable clients from holding more than
one host at a time, and it would be just as limiting above the seam as below it.
The Rust `ProxyRegistry` has the same shape for the same reason, and every
command takes an explicit `connection_id`.

Every browser-local key is namespaced by `(connection, company)` through
`scopedKey`. Company alone is wrong as soon as two hosts serve a company of the
same name; connection alone is wrong as soon as one host serves two companies.
Anything reading or writing that state must depend on **both** — a callback that
closes over the scope but depends only on the company will write under the host
the operator just switched away from.

## The transport seam

`Transport` has two implementations, chosen at runtime by `isDesktopRuntime()`
so `frontend/` stays one build artifact:

- `BrowserTransport` — `fetch` and `EventSource`, a literal restatement of what
  the console did inline before. The browser build's behaviour is unchanged.
- `ProxyTransport` — every request and event stream through the app's own Rust
  core.

The desktop routes through Rust for three reasons, in the order they bite:

1. **CORS.** A webview origin is cross-origin with every host, so a direct fetch
   would need each operator to allow-list `tauri://localhost` before their
   desktop could connect — configuration standing in front of the headline
   feature. Requests made from Rust are not subject to CORS.
2. **The credential.** A device token attached in Rust never enters the webview.
3. **Streaming.** `EventSource` cannot set a request header, so it cannot carry
   the session header, and a `SameSite=Lax` cookie is never sent cross-site.

`src-tauri/tests/proxy_parity.rs` runs both transports against one real host and
compares, because the console's error handling reads the status, the body and a
response header — a transport that differed in any of them would produce
different `ApiError`s on the desktop for the same server behaviour.

### Registration precedes traffic

The core resolves a connection id against its own registry, so the console must
call `oc_connect` before any request for that id. `addConnection` is synchronous
(React renders off it) and `oc_connect` is not, so each registration is kept as
a promise and awaited by `ProxyTransport` before its first call. Firing and
forgetting loses the race on a fast probe, and the symptom — a host that is
unreachable once and fine on retry — reads like a network fault.

### What the proxy will not carry

Caller-supplied `x-opencompany-session`, `authorization`, `cookie` and
`proxy-authorization` headers are dropped before the connection's own credential
is attached. `RequestBuilder::header` appends and axum reads the *first* value,
so a header from the webview would otherwise be the one the host honoured.
Keeping the token out of the webview is worth little if the webview still
decides what a request authenticates as.

The webview also runs under a CSP (`src-tauri/tauri.conf.json`) whose
`connect-src` allows the IPC origin only. All host traffic goes through Rust and
needs nothing else.

## The embedded host

`src-tauri/src/embedded.rs` runs a real host in-process on `127.0.0.1:0`,
holding the data root's lock (see [`data-root.md`](data-root.md)). It becomes an
ordinary connection in the console, discovered through `oc_embedded` because
only the core knows which port the OS chose.

Loopback and never `0.0.0.0`: an embedded instance is this machine's, and
binding a routable address would publish someone's company to their network.

It binds a real socket rather than driving the `Router` in-process through
`tower::Service`. Skipping the socket would save microseconds and buy a second
code path through the auth extractors, the CORS branch and the error envelopes —
divergence in an auth extractor being precisely the class of bug that cannot be
afforded. With a real listener, every Playwright spec and every proxy test is
valid evidence about embedded mode too.

`None` when it could not start — most often because another process holds the
data root. The console renders that as a row; the desktop still holds remote
hosts, which is the point of holding several.

## Authenticating as a person

A desktop cannot hold a session cookie: `SameSite=Lax` means the browser never
sends one cross-site, and a webview is cross-site with every server. The only
other header credential was the platform bearer, which maps to `actor: None` —
every write anonymous in the journal.

So a session has a second carrier and a way to get one. Both are documented in
[`users.md`](users.md) → "Two carriers, one session" and "Device pairing".

The token lives in the OS keychain (`src-tauri/src/keychain.rs`), and the
console never sees it. `oc_connect` takes no device material: the core resolves
a paired session by connection id. Pairing runs entirely in Rust —
`oc_pair_device` performs the claim, writes the result to the keychain, and
answers with the company, device id and expiry — so the token exists for one
HTTP response that the webview is not on the path of. That is the difference
between a design where the webview *should not* hold the credential and one
where it *cannot*.

The console's `Credential { kind: "device", ref }` is therefore a record that
this machine is paired, not something the core is told. `ref` is the host's
device id, useful when deciding what to revoke from the host's device list.

Backend selection, the test store, and the Linux session-keyring caveat (a
pairing there does not survive a logout) are documented in the module.

## ACP

`src-tauri/src/acp/` is the client half: it spawns a locally-installed harness
over stdio and serves the `fs/read_text_file` and `fs/write_text_file` methods
the agent calls back with. Path confinement is enforced in Rust, below the UI —
the console renders the permission prompt but must never be the thing that
enforces the answer. A renderer decides what a person sees; it must not decide
what a model can reach.

The server half (`src/server/acp/`, `src/harness/acp_run_turn.rs`) is behind the
`acp` feature and **not yet mounted on any router**. `/acp` is a reserved prefix
either way, so a build without it answers a protocol probe with a 404 rather
than the console shell with a `200`.
