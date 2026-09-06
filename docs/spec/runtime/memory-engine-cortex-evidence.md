# What the Cortex evaluation measured

Split out of [`memory-engine-cortex.md`](memory-engine-cortex.md), which
carries the decision and the plan. This page carries the measurements behind
them, at the length they need. Read the parent's
[Correction](memory-engine-cortex.md#correction-2026-09-04) first: three of
the findings recorded here were later traced to our own configuration, and
each is marked where it appears.

## Layers are not capability families

The empty layers are real, but they are **not** a capability-audit finding, and
it is worth separating the two because conflating them points a driver plan at
the wrong thing.

Measured on the deployment, with the LLM lanes believed configured and the
server reporting healthy — see [Correction](#correction-2026-09-04) for why that
belief was wrong:

| Cortex layer | Endpoint | Contents |
|---|---|---|
| Events | `/v1/events` | populated |
| Episodes | `/v1/episodes` | populated |
| Facts | `/v1/facts` | **empty** |
| Beliefs | `/v1/beliefs` | **empty** |
| Understanding | `/v1/understanding` | **empty**; errors every scheduler tick |

The contract's `Capability` enum is a closed set — deliberately not
`#[non_exhaustive]`, so adding a variant is a compile error rather than a config
change — and it contains **no** Facts, Beliefs or Understanding variant. Cortex's
five layers are an engine-internal model, not families a driver advertises, so
`audit_provider` cannot fire on them however empty they are. If a hosted Cortex
driver carries an over-claim risk it lives in `Ingest`, `Entities`, `Tree` or
`Retrieval`, and a driver plan should name which of those it intends to
advertise and on what evidence.

What the empty layers *would* mean is commercial rather than structural: the
derived fact/belief tier is the reason to prefer Cortex over
`supermemory`/`mem0`/`cognee` at all. Whether it works is now open — the
measurement above did not test it.

### The audit gap that is real, and is not about Cortex

Separately — and this one holds regardless of engine — `audit_provider` compares
`capabilities()` against `provides()`, and **both are properties of the
adapter**. `provides()` is a defaulted trait method with a fixed body:
`Core | Recall | Portability` hardcoded `true`, everything else
`self.as_x().is_some()`. It is a structural Rust-type question, and its own doc
calls it "the implementation-side truth". Neither side asks whether the engine
answers.

So the three **mandatory** families can never fail the audit. Two of them,
`Core` and `Recall`, are what this host's knowledge ports are built on;
`Portability` is mandatory to the *contract* without being exercised by them. The lever is a live
probe or a conformance case, **not** `provides()` — asking `provides()` to
consult the engine would make the audit compare two runtime opinions instead of a
claim against a structure. Tracked as
[#1968](https://github.com/tinyhumansai/opencompany/issues/1968); a boot-time
probe of the mandatory families is proposed in
[#1973](https://github.com/tinyhumansai/opencompany/pull/1973).

## Belief revision — finding withdrawn

Worth keeping visible because it was twice wrong, and because the second
correction is a modelling lesson rather than a vendor one.

The first run reported `built: 0` with `no_facts_in_scope` against a scope
holding events. That was the enrichment-off misconfiguration, not belief
revision. Re-run correctly, beliefs build.

The second run then recorded that beliefs never *revise*: three events
establishing "Priya Raman owns billing", two handing it to Marcus Webb, and both
owners left live at `confidence: 1.0`, `valid_to: null`. That was filed as
[#2089](https://github.com/tinyhumansai/opencompany/issues/2089) and is now
**withdrawn too**, because `/docs/api-reference/facts` states the rule:

> Newer facts about the same `(subject, predicate)` supersede older ones.

The two facts were `ent_Priya_Raman | owns` and `ent_Marcus_Webb | owns` —
different subjects, so different keys, so nothing to supersede. The machinery
does fire on the documented key: in the same scope a `ent_Priya_Raman | owns`
fact carrying temporal language was closed at `valid_to 2026-08-29`, and
`GET /v1/facts/timeline` exposes the chain for a `(subject, predicate)` pair with
`include_superseded` for history. Neither had been used before the claim was
made.

**The constraint that is real, and is ours.** Single-valued ownership is not
expressible through natural-language ingest. Phrasing the input as "The billing
service is owned by Priya Raman" still extracts
`ent_Priya_Raman | owns | billing service` — the extractor puts the person in the
subject regardless. So a question like "who owns billing" resolves to two
independent claims by construction, and a consumer reading beliefs would see
both. That matters only if the derived layers ever reach the read path, which
today they do not.

## What does work

Ranked retrieval is genuinely good, and it is available from embeddings alone.
In a fresh scope, twelve events, queried twelve seconds after writing and before
any derived layer had built:

- *"who runs mobile releases?"* → `Mobile releases ship every second Wednesday`,
  then `Kai Tanaka manages the mobile release train`
- *"how long do contract reviews take?"* → `Contract reviews take about five
  business days`

Tenant separation also holds *within Cortex*: writes to one scope never
surfaced in another across every test run.

**But the namespace formats are incompatible, and a driver must translate.**
Cortex scopes are slash-delimited `type:id` segments
(`^[a-z][a-z0-9_]{0,31}:[A-Za-z0-9_-]{1,128}(/[a-z][a-z0-9_]{0,31}:[A-Za-z0-9_-]{1,128}){0,31}$`).
Host namespaces are nothing of the sort: `Namespace::company_root` emits
`oc/<slug>-<32 hex>`, children append a plain segment (`…/context`, `…/facts`,
`…/agent/<member>`), and `sanitize_segment` maps every character outside
`[A-Za-z0-9-_]` to `_` — so a colon can never appear and no segment is ever
`type:id`. Passed through unchanged, every store and recall would be rejected.

A `cortex` driver therefore needs an explicit and **reversible** translation.
Reversible is not a nicety: `Bound::recall` re-checks every returned entry with
`Namespace::contains` and drops mismatches, so a return the driver failed to map
back yields zero hits *silently* rather than an error.

So Cortex can back all three knowledge ports today — including `FactStore`.
That needs saying precisely, because the obvious reading is wrong.
`MemoryStore`, `ContextStore` and `FactStore` are **host ports**, not driver
families; a driver can neither advertise nor withhold them. All three are
facades over the same `Bound` helper using only `store`/`get`/`list`/`forget`/
`recall` — `Core` plus `Recall`, both mandatory supertraits.
`FactStore::upsert` is `self.bound.put(company, &fact.id, fact, "fact")`, which
writes operator-curated records into `oc/<slug>-<32 hex>/facts` — the full host
namespace derived from the `CompanyId` — through `MemoryCore::store`. It never
reads a derived facts layer, so Cortex's empty `/v1/facts` does not touch it.

The true statement is narrower, and still supports the conclusion: Cortex cannot
deliver the *derived* fact and belief tier. That is a reason it offers nothing
over `supermemory`/`mem0`/`cognee`, not a reason a port fails.
