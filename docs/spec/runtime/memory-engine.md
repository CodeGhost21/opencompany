# The memory engine overlay

`OPENCOMPANY_MEMORY` and the in-pod engine: what each mode does, and why an
ephemeral data root refuses to boot rather than silently losing memory.

Split out of [`storage.md`](storage.md), which was over the repository's 500-line
ceiling.

## Memory engine overlay (`OPENCOMPANY_MEMORY`)

Memory is a separable concern. `OPENCOMPANY_STORAGE` picks the durable base for
all fourteen ports; `OPENCOMPANY_MEMORY` optionally swaps the three
knowledge ports — `MemoryStore`, `ContextStore` and `FactStore` — onto a
dedicated memory engine layered on top of that base. The base still owns every other port
(companies, events, secrets, tasks, …).

| Value | Engine | Feature flag | Notes |
|---|---|---|---|
| `store` (default) | The base backend's own memory | — | fs substring recall, or sqlite/mongodb |
| `embedded` (or `tinycortex`) | In-pod TinyCortex engine | `tinycortex` | Persistent per-company store; vector-first recall with lexical/recency fallback when no embeddings backend resolves |
| `remote` | A hosted memory service | `tinymemory` | Bound through the `MemoryProvider` contract; needs a URL and a credential |
| `null` | Nothing | `tinymemory` | Writes accepted and discarded, reads empty |

`embedded` and `tinycortex` are the **same value**. Issue #914 introduced the
first spelling; the second keeps parsing indefinitely, because renaming it would
break every deployment that already sets it — including hosted tenants whose
environment the control plane injects — for a cosmetic gain. The same applies to
`cortex`, and to `mongo` on `OPENCOMPANY_STORAGE`. Only one name is reported
back out (`/spec` says `embedded`), so a client never has to know both.

## Choosing a hosted engine (`remote`)

| Env var | Required | Notes |
|---|---|---|
| `OPENCOMPANY_MEMORY_ALLOW_UNPROVEN_REMOTE` | yes | The operator accepting that the hosted adapters are not conformance-proven. See below. |
| `OPENCOMPANY_MEMORY_DRIVER` | yes | `supermemory`, `mem0`, or `cognee`. No default — see below. |
| `OPENCOMPANY_MEMORY_URL` | yes | The engine's endpoint. |
| `OPENCOMPANY_MEMORY_API_KEY` | yes | The outbound credential. |

### `remote` refuses until you accept an unproven adapter

`remote` routes a company's **entire memory** at a third-party HTTP service.
The adapters that speak to those services are covered upstream by a handful of
happy-path tests — no error mapping, no pagination, no taint preservation, no
`Unsupported` behaviour. tinymemory#18 §E1 names a driver conformance suite as
the gate for turning this on at all, and until that suite runs against those
adapters, "it compiled" is the strongest available claim about them.

So the mode exists, refuses by default, and lifts on
`OPENCOMPANY_MEMORY_ALLOW_UNPROVEN_REMOTE=1`. Same shape as
`OPENCOMPANY_MEMORY_ALLOW_EPHEMERAL`, and for the same reason: a memory engine
that loses provenance or silently drops a page fails in ways nothing surfaces
until the memory is needed, which is far too late to notice.

This is a gate on *confidence*, not on configuration, so it is meant to be
deleted rather than lived with. When the conformance suite covers the hosted
adapters, the flag and its refusal go — and `remote` becomes an ordinary choice
needing only a driver, a URL, and a credential.

**Every one of these refuses at boot when missing, naming the knob.** There is
deliberately no fall back to the embedded engine. A company that believes it is
writing to its hosted memory and is not is worse off than one that fails to
start: the second failure is visible immediately, and the first is invisible
until the memory is needed and turns out not to be there.

There is no default driver id for the same reason. Guessing which hosted service
an operator meant would write a company's memory somewhere it cannot be read
back from, and that is not a recoverable mistake.

### The credential is a secret; the endpoint is topology

Neither appears in logs, `/healthz`, `/spec`, status output, or an export.
`StorageSettings` and the driver config both carry hand-written `Debug` impls
rendering `<set>` rather than the value, because both types are reachable from
boot logging where a bare `{:?}` is one keystroke away.

`driver_id()` **is** safe to surface, and `/spec` reports it alongside the
capability families the driver negotiated at bind time — a hosted engine
typically has no summary tree, no graph and no taint tier, and an operator
should be able to read that rather than discover it from a failed cycle.

### Class is decided by the host, never by the driver

`OPENCOMPANY_MEMORY=remote` pins the driver class to `External` and cross-checks
it against the registry's reserved table, so naming the embedded engine under
the remote mode is refused rather than quietly resolved. The contract crate
excludes driver class on purpose: a driver that self-reported it could claim to
be embedded and skip the egress and trust checks that class gates.

### A driver that over-claims its capabilities is refused at bind

`capabilities()` is a claim the driver writes by hand; `provides()` is derived
from the accessors it actually returns. The host compares the two once, at bind
(`audit_capabilities` in `src/store/memory/driver.rs`), because it registers RPC
methods and assembles agent tools from the *claim* and never re-checks. A driver
advertising a family it does not implement would otherwise produce a surface
that exists, is offered to an agent, and fails on its first call — inside a
tenant, at the moment the memory is needed.

The two directions of mismatch are not the same failure and are not treated
alike:

- **Advertised but absent** refuses the bind, naming the families. There is no
  opt-in flag: this is an adapter bug, not a deployment choice, so no
  environment variable lifts it.
- **Present but unadvertised** logs a warning and boots. The family works but
  nothing routes to it, because routing follows the claim. That is dead surface
  from a forgotten `capabilities()` entry — refusing a boot over it would turn
  an upstream oversight into a tenant outage.

Structurally neither should fire: every adapter reachable from here is composed
through `MemoryTraitProvider`, which derives its advertisement from its
accessors. The check runs anyway because that guarantee lives upstream, in a
submodule this repository pins by gitlink, and a gitlink bump is exactly when it
would quietly stop holding.

## Which contract this binds

`tinymemory-api`, at `vendor/openhuman/vendor/tinymemory/api` — the same path
`vendor/openhuman` itself path-depends on, which is what keeps the
`MemoryProvider` trait identity single across the process.

**Not `tinycortex-api`.** They are distinct crates on incompatible contract
majors (`(1, 0)` against `(2, 0)`, and `is_compatible` is major-equality only),
and OpenHuman's own inlined contract documents `tinycortex-api` as a deprecated
re-export. The `tinycortex` crate remains pinned as the *engine* behind the
embedded mode; only the contract moved.

## Why `embedded` does not go through the provider seam

`remote` and `null` bind a provider. `embedded` keeps the `EngineCortex` overlay
it has always had, and that is a durability decision rather than an unfinished
edge.

The obvious construction — `tinymemory_tinycortex::provider(…)` over a
`tinycortex::memory::Memory` backend — cannot currently be durable. The only
concrete `Memory` implementation in the vendored engine is `InMemoryMemoryStore`,
a `BTreeMap` behind an `RwLock`. Binding it would swap the per-company SQLite
workspaces under `<data_dir>/memory/` for a store that is empty after every
restart, and would do so *silently*: every read would succeed, returning
nothing. This page already documents a hard boot refusal for exactly that class
of failure (see the `/data`-is-scratch caveat below), so introducing it through
a contract migration would be a strange thing to do.

Moving the embedded engine onto the seam needs a durable `Memory` implementation
over the engine's KV tier first.

## Tenant isolation across the seam

The three memory ports take `&CompanyId` as an explicit first argument — a
compiler-enforced isolation invariant. `MemoryProvider` has only
`namespace: &str`, and a missing prefix would be a silent cross-tenant leak with
no type-level guard. With a hosted engine it is worse: the namespace string is
the only thing separating tenants inside somebody else's database.

`store::memory::BoundMemory` is therefore the only public way to get a memory
port out of a provider. Its `Namespace` type has no public constructor and is
derived from the company id through an injective sanitize-plus-hash — sanitizing
alone would collapse `acme:1`, `acme/1` and `acme_1` onto one namespace.

The namespace is derived **per call**, from the `&CompanyId` the port method was
given, not fixed when the facade is built. One overlay is opened per process and
shared by every company on the host, so a namespace fixed at construction would
be one tenant's namespace serving all of them. Deriving per call also makes the
namespace a pure function of the argument the port contract already requires, so
it cannot be stale or mismatched with the caller's intent.

Every read is additionally re-checked against the namespace it asked for, and
entries reported outside it are dropped with a warning. That filter should never
fire; if it does, the alternative was serving one tenant another's memory.

## What the host owns, because the contract does not

The contract deliberately carries no policy, which leaves these host-side:

- **Archive on evict.** `evict` *moves* traces to an archive namespace rather
  than forgetting them, because the contract has no archive tier and
  `docs/spec/company-brain/memory.md` makes archiving normative. The archive
  write is ordered **before** the live delete: there is no transaction spanning
  two provider calls, so a crash in between leaves a duplicate the next read
  reconciles rather than a hole.
- **The scratch firewall.** Provisional working-out lives in its own namespace,
  a sibling of every durable scope, so durable recall cannot reach it even if a
  driver ignores the namespace filter.
- **Taint.** Inbound-channel writes are stamped `ExternalSync`. Note the
  contract's `MemoryCore::store` *requires* taint on every call and has no
  dropping default — the defaulted `store_with_taint` is on the engine-side
  `Memory` trait, which is why nothing here wraps a bare `Memory`.
- **Per-agent and per-desk scoping**, which neither cognition port has.

This is why TinyCortex is not a `StorageKind`: it implements only memory +
context, so it cannot be a full backend — it overlays. `serve` and platform
provisioning build the overlay once (`open_memory_overlay`,
`src/store/select.rs`) and apply it to each company's `RuntimeBuilder` via
`with_memory_overlay`, **after** `with_stores`, so the engine's ports win while
the base keeps the rest. A selected-but-unavailable engine (feature disabled)
aborts boot, same as the storage backend.

### In-pod engine (`EngineCortex`)

With the `tinycortex` feature and a data directory present, the overlay is
`EngineCortex` (`src/store/tinycortex_engine.rs`): the OpenHuman `tinycortex`
engine crate running **inside the pod** with durable local storage. Each company
gets its own workspace at `<OPENCOMPANY_DATA_DIR>/memory/<workspace-name>/` — the
path-safe, stable name derived from the full company ID (`EngineCortex::workspace_name`
sanitizes the id and appends a stable hash), the same `<workspace>` the config
examples below render — and the engine's canonical per-workspace SQLite database
(opened + migrated through the crate's own shared connection) holds that
company's traces, task results, and context chunks. The local workspace
persistence layer does not make network calls; a configured hosted embeddings
backend may make outbound requests during embedding and recall. When no data
directory is present (tests, no-data-dir callers) the overlay selects the
offline in-memory backend (`InMemoryCortex`). An error opening a company
workspace propagates to the caller rather than silently switching to in-memory.

**Vector-first recall, with a loud lexical/recency fallback (188c2).** This
slice builds the engine's `MemoryConfig` directly with `embedding.strict =
false`, so the crate's own summary-tree embedder stays inert regardless — but
when a hosted embeddings backend resolves from the environment (see
"Embeddings configuration" below), each stored chunk is separately embedded
into a per-company [`VectorStore`], and `search_chunks` runs cosine recall
**first**, topped up with the existing lexical token-overlap scorer (the same
`[0, 1]`-scored, snippet-bearing contract the in-memory backend defines) up to
the caller's limit — see the two-tier recall in
`src/store/tinycortex_engine.rs`. When **no** embeddings backend resolves — or
on any embedding/search outage — recall degrades to **pure lexical**
(substring/recency token-overlap), **not** the vector/semantic recall the
`tinycortex` name implies, so the overlay announces the degraded mode once,
loudly, at open (`tracing::warn` in `src/store/select.rs`). Because the
crate's retrieval primitives rank only by admission-score/recency in fully
degraded mode (their keyword/graph scorers are defined but not yet wired), and
its `ingest` path re-chunks documents under its own ids — which cannot
round-trip OpenCompany's content-address / label-prefix / peek contract —
chunk bodies and metadata are persisted through the engine's **KV tier** (on
the same per-company workspace database) rather than the crate's
ingest/retrieval primitives, with the vector index layered beside it. Wiring
the crate's own retrieval-scorer `Embedder` / summary-tree seal path (the
hard-768-dim path, plus a full-corpus reconcile beyond the bounded backfill) is
deferred to #198 — this slice injects only the `VectorStore` store+search
compute, which is dimension-agnostic and runs at the configured embedding
dimension (1024 by default).

#### Embeddings configuration

The hosted embeddings backend (`src/harness/embeddings.rs`, `openhuman`-gated
harness build only) shares its credential + base URL with the chat inference
client and layers two overrides on top:

| Env var | Default | Notes |
|---|---|---|
| `OPENCOMPANY_EMBEDDINGS_MODEL` | `embedding-v1` | The managed embeddings model id. `embedding-v1` is 1024-dim and rejects the OpenAI `dimensions` request param. |
| `OPENCOMPANY_EMBEDDINGS_DIM` | `1024` | The model's native dimensionality. Must parse as a positive integer; only meaningful alongside a model whose native dim differs from 1024. |

Every returned vector is validated against the configured dimensionality; a
wrong length is an error, never silently truncated.

### Durability contract & the `/data`-is-scratch caveat

`EngineCortex` is durable **only to the extent the data directory is durable**.
On a host with a persistent `OPENCOMPANY_DATA_DIR` (a mounted volume, or the
default `$HOME/.opencompany`), engine memory survives restarts. But under the
hosted multi-tenant model with `OPENCOMPANY_STORAGE=mongodb`, the durable base is
the database and the container's `/data` is treated as **ephemeral scratch** — so
engine memory written to `<data_dir>/memory` would **not** survive a container
restart. Because that failure mode is *silent* memory loss on restart, selecting
`OPENCOMPANY_MEMORY=tinycortex` together with `OPENCOMPANY_STORAGE=mongodb` is by
default a hard **refuse-to-open** error at boot (`src/store/select.rs`), not a
warning: the overlay never opens a doomed engine.

Storage-kind is only a *proxy* for "ephemeral `/data`", though — a mongodb
deployment that HAS mounted a persistent volume at the data dir is perfectly
safe to run the in-pod engine on. So the refusal is an explicit **durability
contract**, not a hard storage-kind rejection. To run the in-pod engine you can:

- mount a persistent volume at `OPENCOMPANY_DATA_DIR` and use
  `OPENCOMPANY_STORAGE=fs` or `sqlite` (durable `/data`); or
- keep memory on the base store (`OPENCOMPANY_MEMORY=store`); or
- under `OPENCOMPANY_STORAGE=mongodb`, if you have mounted a genuinely durable
  volume at `OPENCOMPANY_DATA_DIR`, set **`OPENCOMPANY_MEMORY_ALLOW_EPHEMERAL=1`**
  to assert that durability and lift the refusal. Unset (or any non-truthy value)
  keeps the safe default: refuse. Truthy values are `1`/`true`/`yes`/`on`.

#### Config examples

**(a) Supported persistent config** — durable base + in-pod engine. The data dir
is a real mounted volume, so engine memory survives restarts and no override is
needed:

```sh
OPENCOMPANY_STORAGE=sqlite            # durable /data (single SQLite file)
OPENCOMPANY_MEMORY=tinycortex         # in-pod engine overlay
OPENCOMPANY_DATA_DIR=/data            # a persistent volume mount
# → boots; per-company workspaces persist under /data/memory/<workspace>/
```

**(b) MongoDB config — the boot-time refusal and how the opt-in changes it.**
With mongodb as the durable base, `/data` is treated as ephemeral scratch, so the
engine is refused by default:

```sh
OPENCOMPANY_STORAGE=mongodb           # durable base is the database; /data is scratch
OPENCOMPANY_MEMORY=tinycortex
OPENCOMPANY_DATA_DIR=/data
OPENCOMPANY_MONGODB_URI=mongodb://…   # (tenant-scoped)
# → REFUSES to boot: hard OpenCompanyError::Config. The operator-visible result is
#   a boot abort naming the silent-memory-loss risk and the OPENCOMPANY_MEMORY_ALLOW_EPHEMERAL
#   opt-in — the engine never opens, so no memory is written to a doomed /data.
```

If — and only if — the operator has actually mounted a durable volume at
`/data`, asserting it lifts the refusal:

```sh
OPENCOMPANY_STORAGE=mongodb
OPENCOMPANY_MEMORY=tinycortex
OPENCOMPANY_DATA_DIR=/data            # a genuinely persistent volume
OPENCOMPANY_MEMORY_ALLOW_EPHEMERAL=1  # operator asserts /data is durable
OPENCOMPANY_MONGODB_URI=mongodb://…
# → boots; engine memory persists under /data/memory/<workspace>/ as usual.
```
