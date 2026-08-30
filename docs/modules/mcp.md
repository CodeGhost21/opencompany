# MCP Servers (per-tenant tool servers)

Issue #50. Each company can expose remote **MCP tool servers** to its agents.
An agent granted a server reaches it through the generic bridge tools
(`mcp_list_servers`, `mcp_list_tools`, `mcp_call_tool`), reusing OpenHuman's
`mcp_client` registry, its HTTP transport, and its prompt-injection safety
filter over remote tool metadata.

Hosted v1 boundary: **HTTP transport only**. Out of scope for v1: stdio /
subprocess servers (rejected with a clear error), Smithery browsing,
MCP-server OAuth, and live pool invalidation.

## Where servers come from

A company's *effective* MCP servers are the union of two sources, merged by
name (a runtime entry overrides a manifest server of the same name but keeps
its `manifest` badge):

1. **Manifest** — `[[mcp_server]]` entries in `company.toml`
   ([`company::McpServer`](../../src/company/types.rs)). Declarative intent —
   an HTTP endpoint plus tool allow/deny lists and an optional *named* secret
   key — **never** an inline credential.

   ```toml
   [[mcp_server]]
   name = "notion"
   endpoint = "https://notion.example/mcp"
   allowed_tools = ["search", "read"]
   # auth_secret = "mcp/notion/auth"   # optional; names a SecretStore key
   ```

2. **Runtime** — servers the operator adds through the console, persisted as a
   single JSON index in the [`SecretStore`](../../src/ports/secrets.rs) under
   `mcp/servers`.

Validation (manifest + API): unique names, an `http(s)://` endpoint, and no
stdio `command`. See [`company::mcp`](../../src/company/mcp.rs).

## Credentials are write-only

A server's outbound token lives apart from its declaration, under the per-server
key `mcp/{name}/auth`. It is **write-only** over the API: set via the `token`
field on add/update, stored in the secret store, and **never** returned. The
read shape carries only an `authConfigured` boolean.

The agent-facing surface is redacted too: `OcMcpListServersTool`
([`harness::mcp`](../../src/harness/mcp.rs)) replaces OpenHuman's own
`mcp_list_servers` (which serializes bearer tokens into agent-visible output)
with a drop-in that emits the same shape minus any credential. A regression
test drives `mcp_call_tool` against an in-process MCP server and asserts the
bearer reaches the *server* over the wire but never appears in any `ToolResult`.

## Per-agent scoping

An agent reaches a server named `<slug>` only when its manifest `tools` grants
match `mcp:<slug>` — the same glob semantics as every other tool grant
(`mcp:*` grants all). `registry_for_agent` filters the resolved decls to the
enabled, granted set and folds them into a one-registry `oh::Config` with
`gitbooks.enabled = false` (so OpenHuman's default gitbooks server never leaks
into a tenant agent). An agent with no granted MCP server gets no bridge tools.

```toml
[[agent]]
id = "researcher"
role = "Researcher"
tools = ["mcp:notion", "mcp:linear"]   # or "mcp:*"
```

`mcp_call_tool` runs under a permissive OpenHuman `SecurityPolicy`. It is still
classified for audit, but policy-generated HITL is disabled.

## Approval behavior

`mcp_call_tool` does not automatically park under `supervised`. An agent that
needs sign-off calls `request_approval` explicitly before invoking it.
`readonly` remains a hard denial.

`mcp_list_servers` and `mcp_list_tools` do not require approval. They read
local registration state with credentials already redacted and reach nothing.
This matters more than one saved prompt: the persona brief appended to every
MCP-granted agent *instructs* it to answer capability questions from a live
`mcp_list_servers` call rather than from memory, so while these parked, the
guidance written to stop stale answers could only be followed by interrupting an
operator. An agent's very first move parked, before it had done anything.

The classifications remain declared in
[`policy::consequence`](../../src/policy/consequence.rs) for audit and for a
future policy-HITL mode.

## HTTP surface

Both scope forms are registered (`…/companies/{id}/…` and the single-company
alias `…/company/…`). See [`server::ops::mcp`](../../src/server/ops/mcp.rs).

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `…/mcp/servers` | Effective servers (`authConfigured`, never the token). |
| `POST` | `…/mcp/servers` | Add a runtime server (+ optional write-only `token`). |
| `PUT` | `…/mcp/servers/{name}` | Enable/disable, edit tool lists/endpoint, rotate token. A manifest server gets a runtime override entry. |
| `DELETE` | `…/mcp/servers/{name}` | Remove a runtime server. `409` for a manifest server (disable it instead). |
| `GET` | `…/mcp/servers/{name}/tools` | Live tool discovery through the registry. |

Discovery is gated on the `openhuman` feature (the MCP transport lives there);
without it the route reports `not_wired` and the console falls back to the
declared tool lists. Every mutating response carries a `note` reminder.

## Which builds can honour a server (issue #567)

The management routes above are **ungated** — they ship in every build. The
agent-side bridge is not: `registry_for_agent` is pushed onto a teammate's belt
behind `#[cfg(feature = "mcp")]`. Three configurations, only one of which the
routes alone distinguish:

| Build | CRUD | Discovery / probe | Agent tools |
|-------|------|-------------------|-------------|
| default (no `openhuman`) | works | `not_wired` | none — no harness |
| `openhuman`, no `mcp` | works | **works for real** | **none** |
| `openhuman` + `mcp` | works | works | yes |

The middle row is the one worth stating outright: every read on the screen
answers correctly, so a healthy badge and a live tool list sit above a server no
teammate can call. The console cannot infer this — an empty tool belt is not
visible over HTTP — so `GET …/capabilities` carries **`mcpInBuild`**
(`cfg!(feature = "mcp")`, alongside `mediaInBuild` / `composioInBuild` /
`searchInBuild`), and `McpServersSection` renders a stated degraded state when it
is explicitly `false`. A host that omits the field is *unknown*, never
"absent" — an older build must not be reported as broken.

Writes stay open on every build deliberately. A manifest can declare servers for
a deployment that runs elsewhere with the feature, and configuration entered
before the capability arrives survives the rebuild; refusing the write would turn
that into a hard error while fixing nothing an operator can act on. Staging
builds with `mcp` (`TENANT_FEATURES` in `deploy-staging.yml`); the default
`docker-compose` build does not.

## Console surface

One component reads these routes —
[`McpServersSection`](../../frontend/src/views/connections/McpServersSection.tsx),
over the standalone functions in `frontend/src/api/mcp.ts` — rendered from two
places: inline on Connections, and as the whole of Settings, MCP Servers.

There is deliberately no MCP method on `OpenCompanyClient`. A second set used to
sit there, declaring a `{ servers }` wrapper around this table's bare array,
`server_id` keys, and `/connect` / `/disconnect` routes that exist nowhere; the
Settings page built on it crashed on open (issue #414). The client casts an
unparsed body to the declared type, so a second surface is never caught by the
compiler — only by whoever opens the page.

### Opening one server

A row's name opens the server into
[`ProviderDetail`](../../frontend/src/views/connections/ProviderDetail.tsx) — the
same panel a Composio provider opens into (issues #404, #821), on a
`ConnectionSubject` union rather than a second MCP-specific panel. The reason is
the paragraph above one level up: two surfaces describing the same idea acquire
two vocabularies and then drift.

The panel is read-only. Enable, `Test`, `Tools` and `Remove` stay on the row;
what it adds is what the row cannot say —

- **Connected, and as what.** MCP has no connection object, so this is assembled
  from two facts a single badge would collapse: `enabled` (whether any agent
  receives the tools at all) and the last probe (whether the endpoint answered
  when someone last asked). A server nobody has pressed `Test` on has no `health`
  at all, and "never probed" is neither reachable nor broken. See `mcpStanding`
  in `frontend/src/lib/connection-detail.ts`.
- **Usage**, read from `byProvider` under the `mcp:<server>` key this module's
  metering records (`src/metering/oauth.rs`) — never the bare slug, which is the
  same-named Composio toolkit's row.
- **No connection date**, stated rather than left blank. There is no connect step
  to record one; the probe timestamp the host *does* keep sits beside it.
- **What a disconnect reaches**: the tool belt on the next turn, and nothing at
  the server's own end. A manifest server says it cannot be removed at all.

## Pool-staleness caveat

Agents materialize their MCP registry once, when the
[`HarnessPool`](../../src/harness/mod.rs) builds a company's roster. Mid-session
edits (add / disable / token rotation) therefore reach a live agent only on the
next `HarnessPool.ensure()` rebuild — practically, a company restart. Every
mutating API response says so. Live pool invalidation is out of scope for v1.
