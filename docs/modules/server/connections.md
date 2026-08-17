# Connections: hosted, the self-hosted hatch, and releasing one

Split out of [README.md](README.md) when that file reached the 500-line cap.
The write routes are `ops::connections` (feature `oauth`) and `ops::composio`;
the read plane is `ops::connections_read`.

## Hosted versus the self-hosted hatch

`ops::connections` (feature `oauth`) runs OAuth with **this host's own provider
application** — a client id/secret an operator registered themselves and handed
to the process as `OPENCOMPANY_OAUTH_<PROVIDER>_ID` / `_SECRET`. It is a hatch,
not a deployment mode — the same framing `ops::composio` uses for its BYO token.
A hosted tenant is injected no `OPENCOMPANY_OAUTH_*` variable at all, so on that
host `provider_config` resolves nothing and a local Connect can only fail.

**The console no longer offers this route** (issue #822). The routes below are
live and unchanged; what changed is that nothing invites an operator down them.
The reason is #396: `oauth_key(provider)` — `"oauth/{provider}"` — is written by
the callback and read by *no agent tool*, zero occurrences under `src/harness/`.
So the hatch worked and conferred nothing, and a self-hoster could register a
provider application, complete a real handshake, see the tile turn green, and
give their agents no ability whatsoever. `frontend/src/lib/connections.ts`'s
`connectRoute` therefore answers `composio`, `managed` or `unavailable` and never
`native`, and `provider-grid.ts` builds the grid from the backend's Composio
catalog rather than from the console's own provider metadata.

Two things this deliberately does **not** do. It does not remove the routes —
settling #396 by wiring the credential into the harness makes the offer honest
again, and reinstating it is one arm in `connectRoute`. And it does not hide a
credential already stored: a provider `GET …/connections` reports connected keeps
its tile, its `via: ["native"]` and its Disconnect, whether or not the Composio
catalog carries it.

The read plane says which it is. `ops::connections_read::connect_route` answers
one question per provider — *can a Connect click possibly succeed here, and by
which route?* — as a `credentialSource` tier, stored-wins:

| Tier | When | Console |
| --- | --- | --- |
| `static` | a token is already stored for this provider (BYO override), **or** this host registered its own provider app *and* has a state signing secret (the hatch) | no local Connect since #822 — Composio's if it has one, else "not available here" |
| `attested` | no stored token, and the pod carries a platform-**projected** identity (`TINYHUMANS_TOKEN_FILE` naming a file that exists) | "Managed by the platform", no local Connect |
| `none` | neither | read-only "not available on this host" |

The tier is still the honest answer to *can a Connect click possibly succeed
here* — it is what the route itself decides by, and `start` still refuses on
`none`. What #822 changed is that the console stopped acting on `static`: the
question it renders is no longer "could this succeed" but "would this confer
anything", and for the native hatch the answer is no until #396 is settled.

**The hatch also needs `OPENCOMPANY_OAUTH_STATE_SECRET`** (issue #318). The
`state` nonce binds an in-flight authorization to one company, provider and
expiry, and the callback verifies it before exchanging the code — it is the
flow's CSRF defence. That signing key used to fall back to a literal baked into
this repository, which made the value public, identical across every
unconfigured deployment, and constructible rather than obtainable: verifying it
proved only that it was well-formed. There is now **no default**. A host with a
registered provider application but no secret reports `none` rather than
offering a button whose check is void, `start` refuses with a message naming the
variable, and the process logs the misconfiguration once — a tile has no room to
name a variable, and an operator reads logs. Whitespace-only counts as unset, so
an empty shell expansion gets the closed door rather than a secret of `" "`.

`attested` deliberately requires the projected-file tier, not
`TinyhumansTokenSource::from_env` as a whole: that resolver also accepts a
long-lived `TINYHUMANS_API_KEY`, which a self-hoster commonly sets to buy
inference. Accepting it here would tell such an operator their working Connect
button is platform-managed and take it away. Both the REST route and the GraphQL
`Company.connections` resolver project the field through the same
`connect_route_from_env`, so the two read shapes cannot drift.

**Provider mapping to the platform backend.** Its registered OAuth providers are
`notion`, `google`, `gmail`, `github`, `twitter`, `discord` and `instagram`. Two
consequences for the console catalog: `gmail` is a registered provider *name* but
not a separate provider application — it is Google's app requested with the Gmail
skill scopes, so a Gmail connect and a Google connect share one grant (which is
why the backend merges scopes incrementally rather than replacing them). And
there is **no Slack provider** at all (the backend's only Slack credential is an
internal alerting bot), so Slack has no hosted route except Composio, which runs
its own OAuth.

## One connection status, for one console list (issue #582)

`GET …/connections` is the **only** answer to "what is connected". It reconciles
the native `oauth/{provider}` catalog with a live Composio probe into one row per
provider, marking which namespaces reported it in `via` — and the console renders
exactly one provider grid from it (`frontend/src/lib/provider-grid.ts`).

That took removing a gate. `composio_view` used to discard the Composio half
unless the company explicitly granted the `composio` tool namespace, while
`GET …/composio/connections` — which the console's *other* provider list read —
never consulted the grant. The two lists therefore disagreed by construction, not
by timing: 13 of the 21 shipped companies grant no `composio`, so for most of
them one screen said both "connected" and "not connected" about the same account,
and the second list's Connect button was actionable.

The grant governs whether **agents receive Composio tools**. It never governed
whether a handshake completes — `resolve_tenant` reads the credential and the
toolkit allowlist and nothing else, so the sign-in the gate hid worked perfectly
well from the surface that ignored it. It is now reported (`granted` on
`GET …/composio`) and stated as a caveat next to the connected badge, rather than
silently deciding what the page may show.

Two consequences worth knowing:

- The probe now runs for any company holding a credential, not only granting
  ones. It stays bounded by `COMPOSIO_PROBE_TIMEOUT` and still degrades to
  `unverified` rather than to a confident "not connected".
- `reconcile` is split from `project_connections` purely as a test seam: the
  probe is a network call with no injection point, and the removed gate lived on
  the far side of it, which is how it survived unasserted.

## The capabilities panel's Composio verdict is a tier, not a stored token (issue #886)

`GET …/capabilities` is the panel an operator checks first when a tool looks
missing, so a wrong answer there sends the whole debugging session the wrong
way. It used to compute its Composio verdict from `composio::token_configured`,
which reads exactly one secret slot — the BYO override `composio/token`.

The credential is resolved over **three** tiers, and the toolbelt gates on all
three (`composio::resolve_credential`, the seam issue #586 established): the BYO
override, then the company's own TinyHumans key, then this instance's platform
identity. On a hosted tenant nobody pastes a BYO token — the third tier answers,
and the tools wire up and are ready to attempt calls. Credential resolution
proves bearer presence, not that a later call succeeds — that is the probe's
job (above), a separate axis. The one-tier probe reported `false` throughout.

So the route now sends **both**, answering two different questions:

| Field | Question | Shape |
| --- | --- | --- |
| `composioTokenConfigured` | did *this company* paste a BYO token? | boolean, unchanged meaning |
| `composioCredentialSource` | which Composio credential tier resolves? | `attested` for the projected platform identity, `company` for the company's TinyHumans key, `static` for a BYO or static instance key, and `none` when no credential resolves |

Three properties are load-bearing:

- **The tier comes from the resolver, never from a second copy of its
  precedence.** This is the same rule the `GET …/composio` status route already
  follows; #886 is the copy that route's migration missed. The console must
  never be able to name a tier the agents are not on.
- **An unreadable secret store omits the field**, rather than reporting `none`.
  `none` is a verdict — "nothing resolves, no tools are wired" — and claiming it
  on a transient hiccup sends an operator to paste a token they already have.
  The console treats an absent field as unknown and must not render it in the
  alarm colour.
- **It is a resolution verdict, not a liveness one.** `attested` says a bearer
  can be obtained, not that Composio answered or that any account is connected.
  `GET …/connections` above is the axis that answers those, and a company with a
  valid bearer and zero connections is a working empty account, not a fault.

The evidence pack the planning station builds reads the same resolver, for the
same reason: it used to tell operators "this company has no Composio credential,
so no Composio account can be reached" on a card whose own evidence listed the
connectors as connected.

## Releasing a connection: two routes, not interchangeable (issue #404)

There are two disconnects and they act on different things:

- `POST …/connections/{provider}/disconnect` blanks this host's own
  `oauth/{provider}` secret and best-effort revokes it upstream. It has never
  touched Composio.
- `DELETE …/composio/connections/{connection_id}` revokes **one connected
  account** at Composio. Addressed by connection id, because a company can hold
  two accounts for one toolkit and exactly one of them is being released.

Until the second route existed the console sent every Disconnect to the first,
so a Composio-connected provider answered `200`, reported success, and was still
connected on the next refresh. The console now routes by what the provider is
actually connected through (`disconnectRouteFor` in
`frontend/src/lib/provider-grid.ts`), preferring the Composio account when a
provider is connected through both — the native secret is inert until #396
lands, so blanking it would release nothing an agent can feel.

`GET …/composio/connections` carries the accounts a revoke is addressed to:
`accounts[]` per toolkit, each with Composio's verbatim `status`, its
`createdAt` when Composio published one, and the account label when the provider
did. `toolkit` and `connected` keep their previous meaning, so the callers that
read only those two are untouched.

**Which account an agent acts as is not decided here.** `composio_execute` posts
`{tool, arguments}` and no connection id (`src/harness/composio.rs`), so
Composio resolves it for the entity — nothing in this codebase selects, orders
or defaults an account, and the console's provider detail view says so rather
than marking one as the default. Changing that means sending a connection id on
execute, which is a harness change, not a console one.

