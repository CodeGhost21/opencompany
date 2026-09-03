# Writing a driver against CortexDB v0.9.8

Companion to [`memory-engine-cortex.md`](memory-engine-cortex.md), which asks
whether we should host Cortex at all. This one assumes that question is still
open and answers a narrower one: what a `MemoryProvider` driver has to do to
speak to CortexDB v0.9.8, and what that costs at every call.

Split out of the design record when it outgrew the 500-line cap. The record kept
the decision; this kept the mechanics behind it.

The driver these notes describe is merged as
[tinymemory#128](https://github.com/tinyhumansai/tinymemory/pull/128) and is
deliberately unregistered — see the record's phased plan.

## The upsert gap, and what it costs to work around

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

**The engine can already do this; the HTTP surface cannot express it.** The
embedded adapter that #1568 removed (`tinymemory-tinycortex`) forwards `store`
straight through to the engine's own implementation, and that crate runs the
*full* conformance suite over `TinycortexProvider`
(`tests/full_provider_conformance.rs`) — including
`assert_upsert_replaces_rather_than_duplicates`. The engine core therefore
passes the exact assertion the hosted API fails.

That makes the upstream ask narrow and concrete: expose over HTTP what the
engine already implements and is already conformance-tested against, rather than
add a capability. It also suggests the gap is an API-surface oversight rather
than a deliberate design position.

Two emulations exist, and neither is comfortable. A driver can keep an
**external index** of `(namespace, key) → event_id`, writing with fresh
idempotency keys and forgetting the prior event on overwrite — which makes the
driver stateful, carrying a database of its own. Or it can put the key in an
**in-content envelope** and resolve it with a client-side scan; the host already
wraps records in a JSON envelope inside `content` (`Bound::put` calls
`encode(record)`), so that is the shape it uses anyway — but a scan-per-read is
slow and still non-atomic.

Both rest on a `forget` that reported `deleted.events: 2` for a selector naming
one event id. That is a lot of correctness risk to absorb for something an
upstream `on_conflict: replace` would remove entirely.

The reproduction is recorded at
[cortexdb-releases#3](https://github.com/cortexdbai/cortexdb-releases/issues/3)
— closed there, like #1 and #2, because that tracker is scoped to packaging, and
being raised with CortexDB directly instead.

**This was written as blocking Phase 1. It is not.** The driver appends every
write under a fresh idempotency key — which the engine always accepts — carrying
the logical key inside the payload, and folds to newest-per-key on read.
Replacement is reconstructed on the read side rather than performed on the write
side, and the conformance suite passes that way against a live v0.9.8.

The price is paid on every call, and it is the real input to the decision:

- **keyed reads are a scan.** `/v1/events` has no metadata filter, so `get` and
  `list` fetch the scope and fold it — a walk growing with everything the
  namespace has held.
- **writes cost seconds, not milliseconds.** `/v1/experience` answers
  `202 captured` and indexes after, so the driver waits for its own record to be
  readable: 1–4s to the listing, a second more to ranked recall.
- **superseded values reach `recall`, and this one is not merely a cost.** The
  fold removes them from keyed reads and cannot remove them from recall — the one
  path Cortex was wanted for. A caller can be handed a value `get` would never
  return. Whether that breaks the contract or is permitted backend-defined
  behaviour is unsettled, and **the conformance suite does not decide it**:
  `assert_upsert_replaces_rather_than_duplicates` reads back through `list` and
  never calls `recall`, so passing it says nothing about this axis. Treat the
  green suite as coverage that stops short here, not as proof.

None of the three is a defect in the driver; each is the cost of emulating
replacement on an engine that does not offer it. But **an upsert alone would only
remove the third.** `on_conflict: "replace"` is our proposed ask, not a documented
CortexDB option, and even if it shipped: keyed reads still scan, because that
needs a metadata filter on `/v1/events`; writes still wait, because that needs a
readiness signal the engine does not expose. Three asks, not one.

## What building it taught us

Five engine behaviours surfaced only against a running instance, and each one
produced a driver that passed every offline test and was wrong in production —
the conformance double and the driver were written from the same documentation,
so they agreed with each other rather than with the service. The catalogue lives
in `cortex.rs`'s module docs, beside the code that has to cope with each one,
which is where it stays current.

The generalisable part belongs here: **a double written from a vendor's
documentation cannot tell you the vendor's documentation is wrong.** The lane
that found these runs the same conformance suite against a live engine
(`tests/live_remote_engines.rs`), and it is worth pointing at every hosted engine
we bind, not just this one — which is the argument #1968 was opened to settle.

