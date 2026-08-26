# Product analytics

**Status: implemented (issue #1739).** What OpenCompany reports about how it is
being used, what it deliberately never reports, which installs report at all,
and how to turn it off.

The short version, and the only three sentences most readers need:

- A **desktop or self-hosted install sends nothing.** Not "sends nothing by
  default" in the sense of a flag someone could flip in a config file — the
  network client is behind a cargo feature the shipped default build does not
  compile, so there is no code in that binary that could make the request.
  Getting one out of that state takes a **recompile**: `--features analytics`
  *and* an explicit `OPENCOMPANY_ANALYTICS=on`, both deliberate, and neither
  reachable from anything a shipped binary reads at runtime. See
  [Configuration](#configuration) for the four conditions in full.
- A **hosted tenant** — a container the OpenCompany platform provisioned and
  operates — reports **shape and outcome only**, under an **opaque id**.
- Nothing an operator or an agent wrote ever leaves the process this way. Not
  message text, prompts, file names, ledger values, tool arguments, addresses,
  or company names.

## Why the default is silence

This repository is GPL-3.0 and self-hostable. An open-source instance that
phones home by default is a betrayal of that, whatever the payload contains.

It is also the posture the rest of the tree already takes. `tests/offline_e2e.rs`
runs inside a network namespace with no routes and asserts that the jail holds;
[offline.md](offline.md) says outright that a new cloud call on a shared path
*should* turn that lane red, and that widening the namespace to make it pass is
not an option. An analytics client firing at boot would do exactly that. The
feature gate is what keeps both things true at once.

And [`roadmap.md`](../roadmap.md)'s non-goal — "no private feedback backend" —
still holds and is unchanged by this: **feedback** goes to public GitHub issues
or stays local, and never rides this channel.

## What is collected

Three events today. Each carries the context envelope below.

| Event | Fired when | Properties |
|---|---|---|
| `instance_started` | the host finished booting and registered its companies | `companies` (count), `storage` (`fs`/`sqlite`/`mongodb`), `setup_complete` |
| `turn_finished` | one cycle — the product's unit of work — ended | `trigger`, `outcome` (`ok`/`failed`), `failure` (coarse class), `duration_ms`, `effects_executed`, `approvals_parked` |
| `turn_metered` | one usage sample was recorded | `sample_kind`, `provider`, `input_tokens`, `output_tokens`, `cached_input_tokens`, `cost_usd`, `attributed_to_run` |

### The context envelope

Set once at boot, attached to every event:

| Property | Value |
|---|---|
| `distinct_id` | the opaque identity — see below |
| `deployment` | `desktop` \| `self-hosted` \| `hosted-tenant` |
| `app_version` | the crate version |
| `os`, `arch` | `std::env::consts` |
| `cognition_path` | `harness` \| `hosted` \| `echo` \| `sidecar` \| `custom` |
| `cognition_provider` | `openrouter` \| `subscription` \| `managed` \| `ollama` \| `byok` \| … |
| `cognition_metering` | `per-turn` \| `per-cycle` \| `none` |
| `harness_in_build`, `mcp_in_build`, `acp_in_build`, `oauth_in_build`, `analytics_in_build` | the compiled feature set |

`cognition_*` is read off [`ports::brain::Cognition`](ports-cognition.md), the
descriptor the runtime already keeps, rather than re-derived from configuration
beside the code that picks a brain.

## What is never collected

Message text. Prompts. Agent output. File and workspace paths. Ledger values and
row contents. Tool names and tool arguments. Email addresses. MCP server names.
Company names and ids. Agent names. Task titles. Error messages. Credentials of
any kind.

**This is a structural guarantee, not a review-time rule.** A property value in
this crate is one of exactly four things:

```rust
PropValue::Word(&'static str) | Count(u64) | Amount(f64) | Flag(bool)
```

There is no `String` variant and no `serde_json::Value`. A `&'static str` cannot
be produced from runtime data without deliberately leaking memory, so every
textual property is a literal written in this repository. A call site with a
runtime string — a provider slug, an error, a trigger — must pass it through a
classifier that maps it onto a fixed list, and **anything unrecognised becomes
`other`**. The dangerous direction is a value nobody anticipated, so that is the
direction the design fails in.

Two consequences worth stating plainly, because both look like a bug until you
know they are the point:

- An MCP tool call reports the provider `mcp`, not the name the operator gave
  their server. That name is frequently a customer or a project.
- A failure reports a coarse class (`store`, `refused`, `cognition`, …), not the
  error message. `Display` on this crate's error type embeds absolute host
  paths, company ids, tool names, ledger slugs and agent text — it is the
  richest source of user content in the tree.

`src/analytics/test.rs` asserts both halves: that hostile inputs do not survive
into a payload, and that **every** string in a rendered payload is either the
opaque id, a platform constant, or a word from a hand-written vocabulary.

## Identity

`distinct_id` is opaque and stable, and it is one of two things:

- `t_<32 hex>` — the **SHA-256 of the tenant slug**, for a hosted tenant. Hashed
  rather than passed through because a tenant slug is usually the customer's own
  brand. Every question analytics asks — uniques, funnels, segmentation,
  retention — needs only that the same tenant maps to the same value every time.
- `i_<32 hex>` — this host's **instance id** otherwise: 16 random bytes minted
  on first boot and persisted under the data root
  ([data-root.md](data-root.md)). Random, not derived: `src/app/instance.rs`
  argues that at length, and the reasons are the same here.

One caveat, inherited from that module: on an **unwritable data root** the
instance id cannot be persisted and a fresh one is minted per process, so such a
host looks like a new install on every boot. It logs a warning when this
happens. A read-only root is a misconfiguration ([storage.md](storage.md)); the
symptom in analytics is inflated install counts, not lost data.

## Configuration

| Variable | Meaning |
|---|---|
| `OPENCOMPANY_DEPLOYMENT` | `desktop` \| `self-hosted` \| `hosted-tenant`. Declared by whoever launches the process. Default and fallback: `self-hosted`. |
| `OPENCOMPANY_ANALYTICS` | `on` forces reporting; `off` forbids it and outranks everything else. |
| `OPENCOMPANY_ANALYTICS_TOKEN` | the Mixpanel project token. **Configuration, never a compiled-in constant** — a token baked into a public binary is a token everyone has. |
| `OPENCOMPANY_ANALYTICS_ENDPOINT` | overrides the collector URL. |

Reporting happens only when **all** of these hold:

1. the binary was built with `--features analytics`;
2. `OPENCOMPANY_ANALYTICS` is not `off`;
3. the deployment is `hosted-tenant`, **or** `OPENCOMPANY_ANALYTICS=on`;
4. a project token is configured.

Condition 1 is met in exactly one place in this repository: `TENANT_FEATURES` in
`.github/workflows/deploy-staging.yml`, the hosted tenant image's feature set.
Nothing else compiles the feature — not the desktop (`src-tauri/Cargo.toml`),
not the default build, not any CI lane but the scoped analytics one. A hosted
image whose feature list drops `analytics` reports nothing however the manager
configures it, and says so at boot rather than failing quietly.

`OPENCOMPANY_TENANT_ID` implies `hosted-tenant` when `OPENCOMPANY_DEPLOYMENT`
says nothing — the control plane injects it and nothing else does. That is the
only inference taken. A discriminator sniffed from something incidental (the
data dir, the bind address, `harness_in_build`) inverts the day someone changes
an unrelated setting, silently, and points at the wrong file.

An unrecognised value for either switch resolves to **silence**, never to
reporting — on a hosted tenant too. Both directions of that typo matter and only
one is obvious. A typo must not *upgrade* an install into one that reports; it
must also not fail to *downgrade* one, which is what happened while an
unreadable value fell through to the deployment default: an operator who meant
`OPENCOMPANY_ANALYTICS=off` and typed `of` kept reporting, and their boot line
said "reporting to …" rather than anything that would send them back to look.
Silence is the answer to "I cannot tell what you asked for", and the boot line
names the reason. A **blank** value is treated as absent rather than unreadable,
so a launcher that exports an empty variable changes nothing.

### How to turn it off

Set `OPENCOMPANY_ANALYTICS=off`. It outranks the deployment kind and the token,
and it is the first thing checked. Boot prints one line either way:

```text
analytics: off (not a hosted tenant and no explicit opt-in)
analytics: off (operator opted out)
analytics: off (the OPENCOMPANY_ANALYTICS value is not recognised)
analytics: off (reporting to https://api.mixpanel.com/track was configured, but this build was compiled without the `analytics` feature)
analytics: reporting to https://api.mixpanel.com/track
```

The endpoint is named; the token never is — and the endpoint is named
**sanitized**. `OPENCOMPANY_ANALYTICS_ENDPOINT` exists so a deployment can front
Mixpanel with its own proxy, and an authenticated proxy carries its key in the
two places a URL can hold one: userinfo (`https://user:pass@host/track`) and the
query string (`?key=…`). Both are stripped before the line is printed, leaving
scheme, host and path, and the line says `(credentials redacted)` when it
shortened anything — a silently truncated URL is its own hour of confusion. The
`ProjectToken` redaction does not cover this; it guards a different string.

The fourth line is the one worth reading twice. It reports what the process will
**do**, not what was configured: a build without the `analytics` feature
resolves to reporting and then gets a `NullTracker`, because there is no
transport in it to hand back. Saying "reporting to …" there would be the exact
opposite of the truth, and the `mixpanel::build` line that explains it is a
`tracing::info!` the CLI's default `EnvFilter` swallows — which is why every
boot line here is a `println!` in the first place.

## Where it hooks in

| Seam | File | Why there |
|---|---|---|
| `turn_finished` | `runtime::cycle::CycleRunner::run_bracketed` | The cycle's whole span, including the wait on the per-company serial lock — which is the part an operator experiences as "nothing is happening". |
| `turn_metered` | `analytics::meter::TrackingUsageMeter`, a decorator over the `UsageMeter` port | Every `metering::record_*` path ends there, on every build. The harness cost hook is richer but `openhuman`-gated, and the cycle-level path deliberately reports zero tokens on that build so spend is not double-counted — so an event at either one is blind on the other half of the fleet. |
| `instance_started` | `analytics::boot::install` | After companies register **and after the port is bound**: the company count and the cognition path are not known before the first, and a host that never took its address never started in any sense worth counting. |
| cognition relabel | `Tracker::observe_cognition`, from `server::provision` and `runtime::rebuild` | Boot's answer stops being true in two ways. A hosted host provisioned into an empty registry had no runtime to read and recorded `custom`/`unknown`; and a company that configures inference for the first time is rebuilt in place (issue #290), which moves it from `echo` to `harness`. Most recent observation wins. Events already sent are not revised. |

**Cognition is a host-level label, and on a multi-company host it is
approximate.** Inference is configured per company, so a host serving two
companies — one configured, one on the echo fallback — has two cognition paths
and one envelope, and whichever was observed last answers for both. Making it
exact means moving cognition off the envelope's super-properties and onto
`turn_finished` and `turn_metered` themselves, which changes the payload shape
this document describes and does not fit `instance_started`, which has no
company. That is an analytics-contract decision rather than a defect, raised in
review on PR #1751 and left for its own change.
| flush | `src/bin/opencompany.rs`, after the bound host stops serving | The server has already drained, so a last-moment turn's event still leaves. |

Failure is silent by construction: `Tracker::track` is synchronous and
infallible and returns nothing, so a call site cannot await a network or branch
on a telemetry error. A dead collector drops batches after one `debug!` line.
The buffer is bounded at 500 events — if the collector is unreachable long
enough to fill it, the right outcome is losing telemetry, not a tenant
container.

## What is deliberately not instrumented yet

Named so the gaps are countable rather than implied. Each is a follow-up, not an
oversight:

- approvals, tools, workflows, ledgers, connections/MCP, and console views —
  the surfaces #1739 lists as candidates;
- **model name**, which exists at no metering seam today. Adding it is a change
  to `UsageSample`/`Cognition`, not to this module, and belongs in its own
  change;
- **build commit**, which this crate does not stamp at build time;
- a timed flush shared with `MaintenanceTicker` rather than the transport's own
  30-second loop.

## Testing

| Lane | Command |
|---|---|
| default build — the decision, the vocabulary, the payload builder, the meter decorator, the cycle hook | `cargo test --locked` |
| gated — the transport, and the acceptance criteria that need it | `scripts/ci/run-scoped-suite.sh "analytics" analytics analytics` |

`scripts/ci/feature-lanes.txt` records the second as `partial`, per the rule in
[`CLAUDE.md`](../../../CLAUDE.md) that every feature says which lane runs its
tests.

The two tests that matter most are a pair, and they only mean something
together: `a_self_hosted_build_makes_no_request` stands up a local collector,
hands the process a token and an endpoint, declares no deployment, and asserts
**zero** requests; `a_hosted_tenant_reports_with_the_full_envelope` is its
positive control against the same collector, the same events and the same code
path with one variable changed. Without the second, a zero request count would
be indistinguishable from a test that never sends anything at all.
