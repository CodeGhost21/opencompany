# One memory contract

*Phase P0. Replacing three bespoke ports and a hand-rolled client with
`tinymemory`.*

Terms: [glossary](../../glossary.md). Supersedes the backend half of
[company-brain/memory.md](../../company-brain/memory.md); the operator rights in
that document are unchanged and become requirements on the decorator specified
here.

---

## The situation

OpenCompany independently built a thin version of the seam `tinymemory` exists
to provide. [`src/store/tinycortex.rs`](../../../../src/store/tinycortex.rs)
defines a bespoke `CortexClient` trait with `CortexMemoryStore: MemoryStore` and
`CortexContextStore: ContextStore` over it, alongside an in-process engine
implementation.

Meanwhile `tinymemory` is already vendored — transitively, as a submodule of
`vendor/openhuman` — and OpenHuman already binds it properly across hundreds of
call sites. We are maintaining a second, weaker integration of the same engine.

`tinymemory` is not an engine. It is an engine-neutral **contract**, a driver
registry, and a set of adapters: `MemoryProvider`, with three mandatory
capability families and ten optional ones, negotiated once at bind time. The
default engine is embedded and makes no network call; remote adapters exist for
several hosted services.

## What changes

**Replace `CortexClient` with `dyn MemoryProvider`.** Keep `MemoryStore`,
`ContextStore` and `FactStore` as **typed facades** over one provider rather
than as three independent storage backends.

The ports stay because their types are the company's vocabulary. The backends
collapse because there is no reason to have three.

| Port | Maps to | Notes |
| --- | --- | --- |
| `FactStore` | `MemoryCore` store / get / forget / list | near-exact fit; `forget` already returns a bool. Upgrades a substring scan to ranked retrieval |
| `ContextStore` | `MemoryCore` + `MemoryRecall` | `put` → store keyed by content address; `search` → recall. **Gap:** ranged `peek` becomes a host-side slice after a full read |
| `MemoryStore` | `MemoryCore` in a traces namespace | **Gap:** `evict` — see below |

### Sequence

`FactStore` first: the best fit and the weakest current implementation.
`ContextStore` second, accepting whole-entry reads. `MemoryStore` last, or not
at all — append-only, eviction-driven trace rows are the shape the contract
suits least, and there is no obligation to move them.

---

## The decorator is not optional

`tinymemory` deliberately owns no policy. Tier enforcement, scope predicates,
taint stamping, redaction, egress checks and audit belong to the host, on the
path every caller takes — because a driver that could be swapped for one that
skips enforcement is the entire reason a policy layer exists.

For OpenCompany that is a hard requirement, and here is why.

> **The three ports take `&CompanyId` as an explicit first argument. That is a
> compiler-enforced tenant-isolation invariant.** `tinymemory` has only a
> `namespace: &str`. A missing prefix is a silent cross-tenant leak with no
> type-level guard.

So the decorator MUST be the **only** public constructor for a company's memory
handle, and it MUST derive the namespace from the `CompanyId` itself. Code that
can name a raw namespace string must not exist outside it. The existing
requirement stands unchanged: company A's facts MUST be invisible to company B.

The decorator additionally owns:

- **Per-agent and per-desk scoping.** Both cognition ports key on `CompanyId`
  alone today, so there is no partition below the company and no
  shared-versus-private distinction. The
  [alignment layer](alignment.md) needs one.
- **The scratch firewall.** Provisional working-out MUST be unreachable from
  durable recall **by construction**, not by routing. The sibling runtime
  enforces this by excluding the scratch dataset from the recall path entirely;
  the roles that judge get neither half of it, because unsettled working-out
  read as progress is what keeps a loop retrying.
- **Archive on evict.** Our `evict` *archives rather than destroys*, and a test
  asserts it. `tinymemory` has no archive tier and no bulk delete by predicate,
  so this behaviour moves into the decorator. It MUST NOT be quietly downgraded
  to deletion.
- **Taint.** Every inbound-channel write MUST be stamped external-trust. None of
  the three ports model provenance today, and a company that reads the web needs
  it. Note the contract's own warning: the taint-aware store method has a
  default implementation that **drops** the taint, so any driver we wrap MUST
  override it — laundering external content into internal-trust content is the
  failure it exists to prevent.
- **Operator rights.** Inspect, delete, redact and export from
  [company-brain/memory.md](../../company-brain/memory.md) are decorator
  responsibilities. Export is already mandatory in the contract, which helps: a
  user who cannot export cannot leave.

---

## Version pinning

Build against the **vendored** copy under `vendor/openhuman/vendor/tinymemory`,
which is the copy OpenHuman itself pins. A standalone checkout may be on an
older contract major, and compatibility is rejected across that gap — binding
the wrong copy yields two incompatible versions of the same trait in one
process.

Engine adapters name their engines by version requirement rather than by path,
specifically so a host that already pins its own engine checkout unifies onto
one copy. Ours does. Keep it that way.

## Cost

`tinymemory-core` brings a bundled SQLite and a substantial subsystem. The
current direct dependency is documented as adding no new compilation to an
`openhuman` build, so this is a real increase, and there are two ways to avoid
paying it: bind only the contract crate plus an adapter, or take the loadable
module route, which keeps the engine out of the host's compile graph entirely.

Decide this before P0 lands, not after.

---

## What we gain beyond parity

- **One seam.** Three ad-hoc ports plus a bespoke client become one driver
  contract, with the engine chosen by configuration.
- **Ranked hybrid retrieval** with an explainable score breakdown, replacing a
  degraded lexical-and-recency path. [Demand
  dedup](demand-ledger.md#identity-and-dedup) depends on this — it is what
  turns a lexical hash into a semantic check.
- **Portability for free**, rather than hand-rolled per store.
- **A goal anchor.** OpenCompany has no goals entity at all — no store, no
  manifest section. The contract has a goals family, and it is deliberately thin
  (an ordered list of id-and-text). That is the right weight: the goal is an
  anchor the roles share, and the [demand ledger](demand-ledger.md) is the work
  model.
- **A summary tree.** The contract ships levelled summaries with drill-down,
  sealing and cascade. This is worth noting because the sibling runtime
  *documents* such a tree and never implemented it — the modules its own
  architecture doc names do not exist. Bind the one that does rather than
  rebuilding the one that does not.

## What we must not lose

- **Typed records.** The port types become encoded content on the way in. Where
  structure matters, use the documents family rather than flattening to a
  string — but note the default engine adapter may not advertise it, in which
  case the facade owns the encoding and its round-trip test.
- **The isolation invariant.** Covered above; it is the single largest risk in
  this phase.
- **Archive-not-destroy** and the ranged read. Both are host-side now.

---

## Verification

- A cross-company namespace leak is unrepresentable: the decorator is the only
  constructor, and no call site can name a raw namespace.
- The existing eviction test still passes — `evict` archives, and does not
  destroy.
- Taint survives an export and re-import, and the wrapped driver overrides the
  taint-dropping default.
- Scratch content is unreachable from durable recall, asserted against the
  recall path rather than against a routing table.
- Judging roles hold neither half of the scratch.
- Capability advertisement matches reachable accessors for every bound driver.
- Every facade round-trips its typed record through the provider without loss.
- The conformance suite still holds fs, sqlite and mongodb to identical answers
  for any port that keeps a non-provider backend.
