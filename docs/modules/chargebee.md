# Chargebee MCP server

A first-party MCP server that lets a company agent run billing operations from a
natural-language instruction — "create an invoice for Acme Corp for $500 for the
Pro plan" — against the Chargebee Billing API. Issue #788.

Source: `src/chargebee/`. Binary: `chargebee_mcp`, behind the `chargebee` Cargo
feature. It is **not** part of the tenant workload: OpenCompany is an MCP
*client*, and this is a separate process it reaches over `https://`.

## Why this is ours and not Chargebee's

Chargebee publishes MCP servers. None of them can create an invoice.

| What Chargebee ships | Transport | Why it does not serve |
| --- | --- | --- |
| `@chargebee/mcp` (npm) | stdio | Deprecated. Exposes `chargebee_documentation_search` and `chargebee_code_planner` — developer tooling, not billing calls. Also stdio, which `McpServer` rejects outright |
| Knowledge Base MCP | hosted HTTP | Answers questions from the docs. Read-only |
| Data Lookup MCP | hosted HTTP | Reads customers, subscriptions, invoices from a live site. Read-only |
| Onboarding MCP | hosted HTTP | Creates catalogs and demo data for test sites. Not invoicing |

Chargebee's own documentation lists write actions as forthcoming, and names
credits, dunning and subscription adjustments rather than invoice creation. So
the five operations issue #788 scopes are not obtainable from a vendor server at
any transport, and this module exists.

## The five tools

All amounts are in the currency's **minor unit** — 50000 is $500.00 USD.

| Tool | Chargebee endpoint |
| --- | --- |
| `chargebee_create_invoice` | `POST /invoices/create_for_charge_items_and_charges` |
| `chargebee_list_invoices` | `GET /invoices` |
| `chargebee_get_subscription` | `GET /subscriptions/{id}` |
| `chargebee_upsert_customer` | `POST /customers` or `POST /customers/{id}` |
| `chargebee_record_payment` | `POST /invoices/{id}/record_payment` |

Every path and parameter above is checked against
`spec/chargebee_api_v2_pc_v2_spec.json` in `chargebee/openapi`, not written from
memory.

### Three wire-format traps

Verified end-to-end by `src/chargebee/tools.rs` tests, which drive a local stub
over a real socket rather than stopping at argument validation:

1. **Writes are form-encoded, not JSON.** Posting JSON fails with a *parameter*
   error, so the mistake reads as a bad request body rather than a bad
   content type.
2. **Nesting uses bracket arrays, indexed per field**, not per object:
   `charges[description][0]`, `charges[amount][0]`, `charges[description][1]`.
3. **Auth is HTTP Basic with the API key as the username** and an empty
   password. Not a bearer token.

### Why every money field is named `*_in_minor_units`

An agent reading "$500" from a prompt and filling a field called `amount` will
write `500` — a $5.00 invoice that succeeds, returns a plausible invoice object,
and is wrong by two orders of magnitude. The unit is in the field name so the
error has to be made deliberately. Local validation additionally enforces
Chargebee's floor of 1; it cannot infer intent above that, which is exactly why
the naming carries the weight.

`chargebee_create_invoice` and `chargebee_record_payment` both accept an
`idempotency_key`, forwarded as `chargebee-idempotency-key`. A retried agent turn
that reuses the key gets the original invoice back instead of billing twice.

## Credentials

The Chargebee API key is read from `CHARGEBEE_API_KEY` at startup and never
leaves the process: it is not a tool argument, not in any tool result, and not
part of the MCP registration OpenCompany holds.

That placement is load-bearing rather than incidental. `normalize_default_servers`
**rejects** any `[[default_mcp_server]]` entry naming an `auth_secret`, because a
default ships to every company unattended and must carry no secret (see
`docs/spec/runtime/config.md`). Issue #788 asks for both a runtime-injected API
key *and* a listing in the default MCP config — mutually exclusive if the key
lives on OpenCompany's side. Holding it here satisfies both.

The cost is honest: the key is **per deployment, not per company**. Every company
on the install bills the same Chargebee site. That is right for a partnership
demo against a sandbox and wrong for multi-tenant production, where the server
should instead be registered per company from the console with a per-company
`auth_secret` — at which point it is no longer a default.

`CHARGEBEE_MCP_BEARER` guards the inbound side. Unset, the server accepts
unauthenticated MCP requests and can create real invoices, which is acceptable
only behind a private network boundary; the binary logs a warning at startup
saying so. Set it for anything else — but note that setting it is what forces the
per-company registration path, since a default may name no `auth_secret`.

## Running it

```sh
CHARGEBEE_SITE=acme-test CHARGEBEE_API_KEY=… \
  cargo run --features chargebee --bin chargebee_mcp -- --bind 127.0.0.1:8790
```

`GET /healthz` for liveness; `POST /mcp` is the whole protocol surface.

```sh
curl -s -X POST http://127.0.0.1:8790/mcp -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
```

## Protocol surface

JSON-RPC 2.0 over one HTTP POST route: `initialize`, `tools/list`, `tools/call`,
and an ack for notifications. Protocol revision `2025-11-25`, matching what
OpenCompany's client announces. No SSE, no stdio — `McpServer::endpoint` is
`http(s)://`-only and a `command` is a validation error, because tenant agents
share a container where per-tenant subprocesses are out of scope.

**Tool failures come back as results, not JSON-RPC errors.** A JSON-RPC `error`
means the call could not be dispatched; a Chargebee rejection ("customer not
found") dispatched fine and produced an answer the agent can act on, so it
returns a normal result with `isError: true`. Collapsing the two would leave the
agent unable to tell "this server is broken" from "that customer does not exist".

## Wiring it to an agent

Registering the server is necessary but not sufficient. Three further things
each cause silent, error-free failure on their own:

1. **Build with `--features mcp`.** The MCP CRUD routes ship in every build, but
   the agent-facing bridge tools do not. Without it OpenCompany accepts the
   server, stores its token, probes it healthy — and hands the agent nothing.
   See `docs/modules/mcp.md`.
2. **Grant the agent the server.** `registry_for_agent` filters on
   `grants_cover_server`, so an agent without `mcp:chargebee` (or `mcp:*`) gets
   zero bridge tools and no error.
3. **Decide the approval mode.** `mcp_call_tool` is declared
   `Reach::Consequence` with `Standing::PerCall` in `src/policy/consequence.rs`:
   under `supervised` it parks for a per-call human approval, under `readonly` it
   is denied, and it can never be granted standing. A hands-free demo therefore
   needs the agent running autonomous. `mcp_list_servers` and `mcp_list_tools`
   are `Reach::Nothing`, so discovery is always free — it is specifically the
   billing write that parks.

### Driving the demo locally

With the server running on `127.0.0.1:8790` as above, point a company at it —
either from the console's Connections tab, or in `company.toml`:

```toml
[[mcp_server]]
name = "chargebee"
endpoint = "http://127.0.0.1:8790/mcp"
description = "Chargebee billing: create invoices, record payments, read subscriptions."
```

`companies/openhuman_demo` already grants `mcp:*`, so an agent there picks the
tools up with no further grant edit. Then run the host with the bridge compiled
in:

```sh
cargo run --features mcp,chargebee --bin opencompany -- \
  serve --company companies/openhuman_demo
```

A change to the server list lands on the next `HarnessPool::ensure()`, not
mid-turn.

## Testing

`cargo test --features chargebee --lib chargebee`. Everything is offline: the
protocol tests drive the axum router through `ServiceExt::oneshot`, the wire
tests drive a stub bound to an ephemeral port, and the validation tests point the
client at port 0 so anything that did reach the network fails fast instead of
hanging. CI runs it on the default lane via
`scripts/ci/run-scoped-suite.sh "chargebee mcp" chargebee chargebee`, classified
`partial` in `scripts/ci/feature-lanes.txt`.

Not covered: a live round trip against a real Chargebee site. It needs sandbox
credentials, so it stays a manual step rather than an `#[ignore]`d test that
looks like coverage.
