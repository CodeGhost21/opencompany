# First-run setup

One flow that configures an instance, at `/api/v1/setup` and — in the console —
the setup wizard (`frontend/src/views/setup/`).

## What it exists for

Everything an operator has to decide to get a spun-up harness running used to
live in four places that never met:

| Decision | Where it lived |
|---|---|
| Host settings (`bind`, `auth_mode`, `brain_mode`, credentials, `[workspace]`) | a hand-edited `config.toml` |
| Which company template to start from | a `serve --company <dir>` flag, or the desktop's hardcoded `DEFAULT_PRESET_ID` |
| Per-company settings (inference, MCP, team, connections) | six Settings sub-pages |
| Whether any of it had happened | nowhere |

A `serve` host with no `--company` booted an empty registry, and the console
dead-ended on *"No companies are running on this host."* That is the hole this
fills. It is the static-template half of
[guided company blueprints](../feature-audit/01-guided-company-blueprints.md);
the conversational Architect interview in [agentic/setup.md](../agentic/setup.md)
is separate and still to come.

## The two routes

`GET /api/v1/setup` returns everything the wizard needs to draw itself: each
configurable field with **the layer that owns it**, the shipped template
catalog, the sign-in modes this host accepts, and which optional surfaces are
compiled into the build.

`POST /api/v1/setup` applies a completed wizard: writes `config.toml`, seeds the
chosen template when the registry is empty, and stamps `setup_completed_at`.
Validation happens before anything is written — a partial apply is worse than a
refused one, because nothing tells the operator which half landed.

`GET /spec` additionally carries `setup_complete`. It is reported on that
unauthenticated handshake because an instance nobody has configured has nobody
who *can* sign in; gating the answer behind auth would make the wizard
unreachable exactly when it is needed. The boolean is the whole disclosure — the
configuration itself stays behind `/api/v1/setup`.

## Every field carries its layer

Resolution is `env ⟵ config.toml ⟵ manifest ⟵ default`
([config.md](config.md)), and this flow can only write the **second** layer. A
hosted tenant has `OPENCOMPANY_BIND`, `OPENCOMPANY_DATA_DIR` and friends
injected by the control plane, so a wizard that accepted an edit to `bind` there
would write a file, report success, and change nothing at the next boot.

So each field reports its `ConfigLayer` and an `editable` flag, the console
renders an env-owned field read-only with the reason, and the route **refuses**
a write to one (`409`) rather than pretending. Silently ignored configuration is
the failure mode this surface exists to prevent.

The layer attribution is not computed here: it is the same `ConfigProvenance`
that `opencompany doctor` already reports (`src/app/doctor.rs`).

## Applied, or only staged

Host-level fields are read once, at boot — `bind` binds a socket, `[workspace]`
decides the data-dir lifecycle. Writing those is a *staged* change, and each says
so through `requires_restart`.

`auth_mode` is deliberately **not** in that category, though it is also resolved
at build and cached on the runtime ([auth-modes.md](auth-modes.md)). Choosing a
sign-in mode and then being shown a sign-in form is the most confusing thing this
flow could do, and "restart the host yourself" is not an answer on a first run.
So the apply makes the mode live on `AppState` *before* it builds anything — the
seeded company is built with it — and then rebuilds any company that was already
registered, through `runtime::rebuild_company`. `AppState::auth_mode_override`
exists for exactly this: `AppConfig`'s field is the value boot resolved and can
never change, and a rebuild reading that frozen value would re-apply the mode the
process started with.

A host with no `RuntimeRebuilder` wired is the only case that still needs a
restart for it, and it is reported per company rather than assumed either way.
`restart_required` in the response names what is genuinely still pending — the
console shows that answer rather than its own prediction, and its button is
labelled "Open the console **anyway**" when something is outstanding, because
that button re-enters the console and cannot restart a process.

Where a restart genuinely is needed, it has to actually work. `serve` therefore
**adopts** the companies its data root already holds
(`desktop::adopt_companies`). Before, only `--company` put a company in the
registry, so an operator who finished setup, was told to restart, and did, came
back to an empty host with their company sitting unread on disk. Adopting is not
seeding: an empty root still starts empty. That registration path also applies
the host-wide `auth_mode` override, which it previously ignored.

Per-company settings (inference, MCP servers, team) are not written here at all —
they go through the existing `ops` routes, which apply live.

## Writing `config.toml`

`app::config::write_config_toml` merges through `toml_edit` rather than
serializing a `ConfigFile`. The shipped file carries commented
`[[default_mcp_server]]` documentation that an operator is meant to read and
uncomment, and a struct round-trip would delete it along with every other
comment and any key ordering. Only the named keys are touched.

The write is atomic — rendered to `config.toml.tmp` in the same directory, then
`rename`d — so a crash mid-write cannot leave a config the next boot refuses to
parse. A malformed existing file is a hard error, matching `ConfigFile::load`:
merging into a document that could not be parsed would overwrite whatever the
operator actually had.

Clearing a field **removes the key** rather than writing `""`. An absent key
falls through to the next layer; a blank string is a set-but-empty value that
shadows it.

## Who may call it

Unauthenticated access is loopback-only, and only in exactly two situations, both
meaning "there is nobody who could authorize it":

1. setup has never completed, **or**
2. the host has no companies, so there is no roster to hold an admin.

"Loopback-only" here means the configured bind and `public_url` say this host is
unreachable from outside *and* the request itself backs that up: its actual TCP
peer is loopback and it carries none of the proxy-forwarding headers
(`X-Forwarded-For`, `X-Forwarded-Host`, `Forwarded`, `X-Real-IP`) that would mean
something in front of this process terminated a connection it did not. Both
checks matter — a loopback-bound listener sitting behind an undeclared reverse
proxy would otherwise pass the first and fail only the second.

Openness on a routable host would let whoever reached a fresh deployment first
configure it; openness on a *configured* laptop would let any page in the browser
rewrite its settings. The second condition is not a nicety either — setup can
complete without seeding a company (an operator who only changes host settings),
and gating that host behind an admin check would leave it with no company to sign
in to and no way back into setup to create one, which is this flow's own dead end
reintroduced one step later.

Outside those two unauthenticated cases — a configured host with at least one
company — the ordinary admin check applies instead: a live session belonging to
an admin of the sole company, resolved the same way `server/users/admin.rs`
resolves it. This path accepts an authenticated human session from anywhere, not
just loopback; it is `require_admin`, not the loopback gate, that authorizes it.
A host serving several companies has no single roster that could speak for the
instance, and setup refuses outright rather than guessing which one.

The console's 401 handling is excluded while the wizard is showing: on an
unconfigured host every authenticated route answers 401, and letting that swap
the wizard for a sign-in form would restore the dead end this flow removes.

## What it does not configure

ACP is a **cargo feature** (`acp`), not a setting, and no `/acp` handler is
mounted in this tree — only the session and permission model plus the reserved
path (`src/server/routes.rs`). The flow therefore reports `acp_in_build` and
`acp_transport_mounted` separately and offers no switch, which is the difference
between telling an operator "not available" and sending a client at an endpoint
that 404s. The same reporting-not-writing rule covers `mcp`, `openhuman` and
`oauth`.

`data_dir` is excluded too: a running host has already opened and locked its
data root, so writing a new one into the file that lives *inside* that root
would produce a config nothing reads. Moving a data root is a relocation, not a
setting.
