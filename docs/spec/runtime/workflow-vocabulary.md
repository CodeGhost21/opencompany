# Workflow node-kind vocabulary

Three node-kind sets sit behind a company workflow, and they are deliberately
**not** the same size. This document is the authoring contract: which node
kinds an author may write, what each one becomes when it runs, and — just as
importantly — which engine kinds OpenCompany refuses on purpose and why.

## The authoring contract is `workflow_file.rs`, not the vendored catalog

The tinyflows engine defines a catalog of node kinds in its own source
(`NODE_KINDS` in the vendored `tinyflows/src/catalog.rs`). That catalog is the
*engine's* vocabulary, not OpenCompany's. The set an author may actually write
is the narrower `WORKFLOW_NODE_KINDS` in
[`src/company/workflow_file.rs`](../../../src/company/workflow_file.rs): the
parser accepts exactly those kinds and rejects everything else at parse time,
listing the accepted set in the error. Reading the vendored catalog to learn
"what a workflow may contain" gives the wrong answer — always read
`WORKFLOW_NODE_KINDS`.

The relationship is a strict nesting:

```
builder (BUILDER_NODE_KINDS)  ⊂  parser (WORKFLOW_NODE_KINDS)  ⊂  engine (NODE_KINDS)
        4 kinds                          12 kinds                       15 kinds
```

Each layer is a superset of the one to its left. An author writes within the
parser set; the host's automatic builder emits within an even narrower set (see
[The builder tier is narrower still](#the-builder-tier-is-narrower-still)); the
engine can run more than either accepts.

## The 12 accepted kinds and what each lowers to

`translate()` in
[`src/workflows/translate.rs`](../../../src/workflows/translate.rs) maps every
accepted kind onto a tinyflows engine kind. Eleven are identity mappings — the
on-disk kind and the engine kind are the same string. The one exception is
`output`, which the engine has no kind for, so it lowers to a bare
`transform`.

| OpenCompany kind | Lowers to (tinyflows) | Note |
| --- | --- | --- |
| `trigger` | `trigger` | identity |
| `agent` | `agent` | identity; `agent_ref` routes to the company `HarnessPool` |
| `tool_call` | `tool_call` | identity; runs a real toolbelt tool, fail-closed on `[tools].allow` |
| `http_request` | `http_request` | identity; routes through the SSRF-guarded `GuardedHttpClient` |
| `condition` | `condition` | identity; edge labels map to `true`/`false` ports |
| `output` | `transform` | **not identity** — see below |
| `switch` | `switch` | identity |
| `merge` | `merge` | identity |
| `split_out` | `split_out` | identity |
| `transform` | `transform` | identity |
| `output_parser` | `output_parser` | identity |
| `sub_workflow` | `sub_workflow` | identity |

`tool_call` and `http_request` are fully wired and execute for real (see
[`src/workflows/caps/mod.rs`](../../../src/workflows/caps/mod.rs)); they are not
structural placeholders.

## `output` lowering and host-side delivery

tinyflows has no `output` kind. An `output` node lowers to a `transform` node
with no `set` config — a pure pass-through, which is exactly the terminal
"report back" semantics: its predecessors' items flow through unchanged.

An `output` node may also carry a `destination` (`owner` / `email` / `channel`,
from `WORKFLOW_DESTINATION_KINDS`). That destination is **deliberately not
translated** into the engine graph. Delivery runs host-side, after the engine
returns, in [`src/workflows/delivery.rs`](../../../src/workflows/delivery.rs);
the engine has no use for a `destination` key and it would be inert cargo in
node config. A destination-bearing `output` node therefore lowers to the same
bare pass-through `transform` as one without a destination — pinned by the
`an_output_destination_never_reaches_the_engine_graph` test in `translate.rs`.

## The engine-only kinds OpenCompany rejects

The engine catalog carries four kinds the parser does **not** accept:
`code`, `memory`, `dedup`, and `loop`. A workflow file naming any of them fails
at parse with the usual unknown-kind error — they never reach `translate()`.
Each is left out for a specific, recorded reason (rationale lives in
[`src/workflows/caps/mod.rs`](../../../src/workflows/caps/mod.rs)):

| Engine-only kind | Why it is rejected |
| --- | --- |
| `code` | The `CodeRunner` capability is an explicit loud-failure stub (`UnwiredCode`): code execution for company workflows is not built, so the capability returns a clear error rather than a silent no-op. Accepting the kind would only let an author author a node that can never run. |
| `memory` | Deliberately undecided, not merely unbuilt. A `MemoryProvider` would give a workflow read **and write** access to agent memory, and which scopes a workflow may touch has not been settled. The capability is left `None` and pinned by `the_memory_capability_is_left_unwired_on_purpose`, so the answer must be given rather than defaulted into. |
| `dedup` | A tinyflows 0.6 catalog addition (arrived with the #499 pin bump) that OpenCompany has not adopted into the authoring set. |
| `loop` | A tinyflows 0.6 catalog addition (arrived with the #499 pin bump) that OpenCompany has not adopted into the authoring set. |

Rejection happens **at parse**, before translation — an author cannot smuggle
one of these into a running graph. Widening the parser to accept any of them is
a deliberate feature decision (settling the memory-scope policy, building the
code runner, or adopting the 0.6 additions), out of scope for this contract.

## The builder tier is narrower still

The host's automatic workflow builder — the plan → workflow bridge in
[`workflow-build.md`](workflow-build.md) — emits an even smaller set,
`BUILDER_NODE_KINDS` in
[`src/harness/workflow_build.rs`](../../../src/harness/workflow_build.rs):
`trigger`, `agent`, `condition`, `output`. A graph the builder proposes stays
inside those four kinds; a human author editing the file directly may use the
full 12-kind parser set. This is the strict nesting from the top of this
document: builder ⊂ parser ⊂ engine.

## Provenance

The engine-side counts and the specific 0.6-only kinds above are true **as of
the tinyflows 0.6.x pin (#499)**. When that pin is bumped, re-verify this
document against the new `NODE_KINDS` and re-decide whether any newly added
engine kind should be adopted into `WORKFLOW_NODE_KINDS`. Grep this file for
"#499" when bumping the pin.
