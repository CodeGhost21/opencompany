# The memory engine overlay

`OPENCOMPANY_MEMORY` and the in-pod engine: what each mode does, and why an
ephemeral data root refuses to boot rather than silently losing memory.

Split out of [`storage.md`](storage.md), which was over the repository's 500-line
ceiling.

## Memory engine overlay (`OPENCOMPANY_MEMORY`)

Memory is a separable concern. `OPENCOMPANY_STORAGE` picks the durable base for
all fourteen ports; `OPENCOMPANY_MEMORY` optionally swaps **just** the two
knowledge ports — `MemoryStore` + `ContextStore` — onto a dedicated memory
engine layered on top of that base. The base still owns every other port
(companies, events, secrets, tasks, …).

| Value | Engine | Feature flag | Notes |
|---|---|---|---|
| `store` (default) | The base backend's own memory | — | fs substring recall, or sqlite/mongodb |
| `tinycortex` | In-pod TinyCortex engine | `tinycortex` | Persistent per-company store; vector-first recall with lexical/recency fallback when no embeddings backend resolves |

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
gets its own workspace at `<OPENCOMPANY_DATA_DIR>/memory/<company>/`, and the
engine's canonical per-workspace SQLite database (opened + migrated through the
crate's own shared connection) holds that company's traces, task results, and
context chunks. The engine never makes a network call. When no data directory is
present (tests, no-data-dir callers) the overlay falls back to the offline
in-memory backend (`InMemoryCortex`), which is also the compiled fallback when a
company workspace cannot be opened.

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
compute, which is dimension-agnostic and runs at the backend's native 1024.

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
