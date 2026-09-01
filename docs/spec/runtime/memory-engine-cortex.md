# Hosting Cortex behind the memory seam

A design record for [#1936](https://github.com/tinyhumansai/opencompany/issues/1936):
can we host [`tinycortex`](https://github.com/tinyhumansai/tinycortex) ourselves and
let tenants bind to it through the `remote` seam, on equal footing with
`supermemory`, `mem0` and `cognee`?

Companion to [`memory-engine.md`](memory-engine.md), which specifies the seam this
would bind through. **That document describes what ships; this one describes a
proposal and the measurements behind it.** Nothing here is implemented.

## Findings first

A CortexDB instance was deployed and exercised to answer this from evidence
rather than from the product page. Five results change the shape of the
question, and the recommendation follows from them.

1. **`tinyhumansai/tinycortex` is not the deployable artifact.** It is a Rust
   *library* crate — no binary target, no server, no Dockerfile. The server is
   **closed-source**, distributed only as prebuilt artifacts (`cortexdbai/cortexdb-releases`,
   Docker Hub `cortexdb/cortexdb`). Whatever we build treats it as an opaque
   upstream binary we cannot patch.
2. **The self-hosted build cannot issue per-tenant credentials.** Under
   `CORTEX_DEPLOYMENT_PRESET=cloud_shared_saas` the PASETO minter answers
   `NOT_CONFIGURED`; its keypair generator is not in the distribution; and the
   bundled `PRODUCTION_DEPLOYMENT.md` describes `CORTEX_API_KEY` as the
   bootstrap key for the *default* tenant, with real per-tenant keys coming from
   Cortex's own hosted key store.
3. **Three of the five memory layers advertise and return nothing.** Facts,
   Beliefs and Understanding stay empty with the extraction and enrichment
   routers enabled and reporting healthy. Only Events and Episodes hold data.
4. **Retrieval quality is real, and comes from embeddings alone.** Ranked recall
   over the Events layer is good and needs no LLM lanes at all.
5. **A conformant driver cannot be written against v0.9.8 at all.** The contract's
   `(namespace, key)` upsert has no expressible mapping onto an append-only event
   log whose keys are immutable. This blocks Phase 1 outright.

## The isolation choice collapses

The issue frames a choice between Cortex-DB-per-tenant and one shared Cortex
with namespace-only separation, and says to default to the stronger. Finding 2
removes the middle option:

| Option | Isolation tier | Reachable self-hosted? |
|---|---|---|
| One shared instance, one bootstrap credential | Namespace-only — the **weak** tier | Yes |
| **One instance per tenant**, own key and own data dir | Credential *and* storage isolation | **Yes** |
| Shared instance, real per-tenant credentials | Strong | **No** — needs Cortex's hosted key store |

`memory-engine.md` is unambiguous about why the weak tier is not acceptable as a
default: with a hosted engine "the namespace string is the only thing separating
tenants inside somebody else's database", and the engine-level `memory migrate`
copies *every namespace a source credential can see*, which is "exactly wrong if
two tenants ever shared one".

**Recommendation: instance-per-tenant, co-located on shared infrastructure.** It
is the only self-hosted route to the tier the seam already assumes. Note what
this does to the issue's own framing: "shared hosting" becomes shared
*infrastructure*, not a shared engine process. The per-tenant Cortex credential
then slots into `OPENCOMPANY_MEMORY_API_KEY` exactly like any other hosted
engine's, and the migrate caution above is satisfied by construction.

## Capability audit: the strongest reason for caution

`memory-engine.md` refuses a bind when a driver advertises a family it does not
implement, because the host "registers RPC methods and assembles agent tools
from the *claim* and never re-checks". Cortex triggers precisely that failure,
in three families at once.

Measured on the deployment, with the LLM lanes configured and healthy:

| Layer | Endpoint | Contents |
|---|---|---|
| Events | `/v1/events` | populated |
| Episodes | `/v1/episodes` | populated |
| Facts | `/v1/facts` | **empty** |
| Beliefs | `/v1/beliefs` | **empty** |
| Understanding | `/v1/understanding` | **empty**; errors every scheduler tick |

Every signal a driver could reasonably key on reports success. The routes exist.
Boot logs say `LLM router enabled for entity extraction` and
`Enrichment LLM router enabled for fact augmentation`. `/v1/derivation/status`
returns `built: {episodes: 3, beliefs: 3, concepts: 0}` for a scope whose belief
store is empty — those counters count *scopes processed*, not records produced.

A `cortex` driver deriving `provides()` from routes, from configuration, or from
the engine's own build counters would advertise Facts, Beliefs and Understanding,
pass `audit_capabilities` cleanly, and then return empty on every read — inside a
tenant, at the moment the memory is needed, which is the exact harm the audit
exists to prevent.

**Requirement for the driver plan:** `provides()` must be derived from *observed
layer contents*, not from configuration or engine self-report. This is stronger
than the conformance suite (tinymemory#18 §E1) currently implies and should be an
explicit acceptance gate.

Two upstream defects sit behind this: concept reflection cannot resolve the
router two other lanes use, and facts never extract, so beliefs can never build.
Reproductions are recorded at
[cortexdb-releases#1](https://github.com/cortexdbai/cortexdb-releases/issues/1)
and [#2](https://github.com/cortexdbai/cortexdb-releases/issues/2); both are
closed there because that tracker is scoped to packaging, and they are being
raised with CortexDB directly.

## A conformant driver cannot be written against v0.9.8

This is the finding that decides the phasing, and it is not about capability
breadth — it is that the two data models disagree.

`Memory::store` is an **idempotent upsert keyed by `(namespace, key)`**: storing
twice at one key replaces the previous content "rather than erroring or creating
a duplicate". The conformance suite asserts it directly
(`assert_upsert_replaces_rather_than_duplicates`) — write `first`, write
`second`, expect one row holding `second`. `memory-engine.md` makes that suite
the thing that retired the unproven-remote flag, so passing it is not optional.

CortexDB is an append-only event log. Measured against the deployment:

| Attempt | Result |
|---|---|
| Same key, different content | `409 IDEMPOTENCY_CONFLICT` — "idempotency key reused with a different body" |
| Same key, identical content | `202`, `replayed_from_idempotency: true` |
| Update route | none — `/v1/events` and `/v1/events/{id}` are GET-only, `/v1/experience` POST-only |
| Forget, then rewrite the key | forget succeeds, rewrite still `409` — **and the scope is left holding nothing** |
| Carry our own key in the envelope | `422` — closed schema, and `/v1/events` has no metadata filter regardless |

The idempotency ledger is independent of the event store and survives
`/v1/forget`, so delete-then-rewrite loses the original *and* refuses the
replacement. It is strictly worse than not attempting it.

The only technically viable adapter keeps an **external index** of
`(namespace, key) → event_id`, writes with fresh idempotency keys, and forgets
the prior event on overwrite. That makes the driver stateful — it carries a
database of its own — across two non-atomic calls, resting on a `forget` that
reported `deleted.events: 2` for a selector naming one event id. That is a lot
of correctness risk to absorb for something an upstream `on_conflict: replace`
would remove entirely.

The reproduction is recorded at
[cortexdb-releases#3](https://github.com/cortexdbai/cortexdb-releases/issues/3)
and is being raised with CortexDB directly, alongside the licensing question.
**Phase 1 is blocked on their answer, not on our effort.**

## Belief revision is not reachable

Worth stating separately because it is much of what would justify preferring
Cortex over the drivers we already have. Tested directly: three events
establishing an owner, then two contradicting them.

```
POST /v1/beliefs/build
{"built":0,"facts_scanned":0,"events_scanned":5,"belief_events_found":0,
 "reasons":{"no_belief_shaped_events":1,"no_facts_in_scope":1}}
```

Beliefs are gated on Facts; Facts never extract; beliefs never build.

What recall does with the contradiction is adequate *by accident*: the correction
ranks first on semantic similarity, while the superseded claim is still returned
with nothing marking it stale. A `FactStore` consumer would be reading a pack
mixing live and superseded claims with no provenance distinction between them.

## What does work

Ranked retrieval is genuinely good, and it is available from embeddings alone.
In a fresh scope, twelve events, queried twelve seconds after writing and before
any derived layer had built:

- *"who runs mobile releases?"* → `Mobile releases ship every second Wednesday`,
  then `Kai Tanaka manages the mobile release train`
- *"how long do contract reviews take?"* → `Contract reviews take about five
  business days`

Tenant separation also holds. Scopes are slash-delimited `type:id` paths
(`^[a-z][a-z0-9_]{0,31}:[A-Za-z0-9_-]{1,128}(/…){0,31}$`), which maps cleanly onto
`BoundMemory`'s injective sanitize-plus-hash derivation from `&CompanyId`. Writes
to one scope never surfaced in another across every test run.

So Cortex can back `MemoryStore` and `ContextStore` today. It cannot back
`FactStore` with anything the layer model promises.

## Fitting the seam's invariants

`memory-engine.md` makes several properties load-bearing. Three need work
host-side, because Cortex does not provide them:

- **Boot refuses rather than silently degrades.** Cortex does the opposite. With
  no embedding credential it does not refuse: it logs a warning, falls back to
  **mock embeddings**, pins the data directory to `mock::1536`, and reports
  `{"status":"healthy"}`. Recall then returns confident, meaningless results —
  the precise failure the seam's no-fallback rule exists to prevent. **A `cortex`
  driver must probe for the mock provider and refuse the bind itself.**
- **Class is decided by the host.** Unaffected — `remote` pins `External`, and
  nothing about Cortex self-reports class.
- **Credential and endpoint never logged.** Compatible: Cortex's own config
  surface redacts, and the host's `Debug` impls already handle this.
- **Host-owned policy** — archive-on-evict ordering, the scratch firewall,
  `ExternalSync` taint stamping, per-agent and per-desk scoping — all remain
  host-side and are unaffected by the engine choice. Cortex has its own
  provenance/taint notion; it is not the host's and should not be relied on.

## Infrastructure

Per-instance footprint is the open number. The binary's own config lint projects
**~18 GiB steady-state RAM** on a 3.9 GiB box, and that estimate did not move
under any remediation it suggests — probed across `CORTEX_VECTOR_RESIDENT_MAX`
at 150000 / 20000 / 5000, tenant shards on and off, and `CORTEX_VECTOR_SHARDS` at
8 and 2, all returning ~18 GiB. Idle RSS is ~27 MiB, so the projection is a
ceiling model rather than a floor, but **the real per-instance figure has to come
from Cortex before any capacity plan is credible**. Under instance-per-tenant
that number is multiplied by tenant count, which makes it the dominant cost term.

Other operational notes:

- `CORTEX_VECTOR_TENANT_SHARDS` latches on disk at pool creation
  (`pool_manifest.json`) — it must be set before real data lands.
- Changing embedding provider or dimension re-pins the data directory and
  requires wiping it. Vectors from two providers cannot be mixed.
- Per-call cost is negligible at evaluation scale: embeddings plus LLM lanes
  across ~50 events and several hours of scheduler ticks totalled **$0.000022**.
  Capability, not cost, is the constraint.

## Licensing permits this

Checked against the licence text rather than the release README's one-line
summary, which is lossy in a way that matters. **CortexDB Community License
v1.0 clause 2** explicitly allows what this design does:

> The Software may be used to power internal or commercial applications,
> including those sold to third parties, provided that: (a) the third party
> does not access the Software directly as a general-purpose memory database
> (i.e., you may build products on top of CortexDB and sell those products; you
> may not resell CortexDB itself as a service); and (b) attribution to
> "CortexDB" appears in product documentation where reasonable.

Selling OpenCompany with Cortex behind the memory seam is building a product on
top of CortexDB, not reselling CortexDB. Condition (a) is satisfied **by
construction**: tenants reach memory only through the `MemoryProvider` seam, and
the credential and endpoint "never appear in logs, `/healthz`, `/spec`, status
output, or an export" (`memory-engine.md`). A tenant cannot address the engine
directly.

Two obligations follow, neither of them a gate:

- **Never hand a tenant its own Cortex endpoint or credential.** Under
  instance-per-tenant that would be easy to do casually — a BYO-engine feature,
  or exposing the URL in a console — and it is precisely what (a) forbids. The
  seam's existing redaction already prevents it; keep it that way.
- **Attribute CortexDB in product documentation** where reasonable (clause 2b).

Clause 3 permits mirroring the binary on an internal artifact store for our own
use, which instance-per-tenant provisioning needs. Clause 5 points source access
and cloud-hosted offerings at sales@cortexdb.ai — neither is required here.

## Phased plan

**Phase 0 — decide.** Ratify instance-per-tenant, or accept the weak tier
explicitly and write down why. Licensing is settled (clause 2 permits it); what
remains is the topology decision, which gates everything below.

**Phase 1 — a driver scoped to what returns data. Currently blocked upstream.**
A `cortex` driver over `tinymemory-api` advertising only the families Cortex
actually serves — `MemoryStore` and `ContextStore` shaped. This cannot start
until CortexDB offers a way to replace a value at a key
([cortexdb-releases#3](https://github.com/cortexdbai/cortexdb-releases/issues/3));
without it the driver fails a mandatory conformance assertion no amount of
adapter work can satisfy. Acceptance, once unblocked:

- the full driver conformance suite (tinymemory#18 §E1);
- failure-path tests for error mapping and malformed responses, as the existing
  remote adapters carry;
- **a bind refusal when the engine reports the mock embedding provider**;
- **`provides()` derived from observed layer contents**, with a test that an
  engine reporting a family it cannot serve fails the bind.

**Phase 2 — provisioning.** Per-tenant instance lifecycle through
opencompany-manager: create, inject `OPENCOMPANY_MEMORY_*` alongside the existing
`OPENCOMPANY_MONGODB_URI`/`_DB` injection, health-probe, back up, destroy. Sizing
waits on a real per-instance figure from Cortex.

**Phase 3 — migration.** `opencompany memory migrate --to cortex` over the
Portability family. The existing runbook in `memory-engine.md` applies unchanged;
its per-tenant-credential caution is satisfied by the instance-per-tenant
topology. Hosted-target enumeration cost still applies.

**Phase 4 — revisit the derived layers**, only if the upstream defects are fixed.
That is the point at which Cortex would offer something the incumbent drivers do
not.

## Open questions

- Does CortexDB agree with our reading of clause 2? Worth confirming in writing
  when we contact them, though the text is not ambiguous.
- What is the true per-instance memory floor, from Cortex rather than the lint?
- Will the two filed defects be accepted? The release tracker is scoped to
  binary/packaging issues, with source bugs directed to Cortex Cloud support —
  so a self-hosted deployment's support path is itself unproven.
- Is there an undocumented prerequisite for fact extraction that we missed?
- Will Cortex add an upsert path? Without one, the only route is a stateful
  driver carrying its own key index — is that acceptable, or disqualifying?
- Self-hosted deployments have no support channel we can reach: the public
  tracker is packaging-only and Cortex Cloud support presumes a customer
  relationship. That is worth settling in the same conversation as licensing.
- If Facts and Beliefs stay unreachable, does Cortex still beat `supermemory` /
  `mem0` / `cognee` on retrieval alone — and is that enough to justify running
  one instance per tenant?
