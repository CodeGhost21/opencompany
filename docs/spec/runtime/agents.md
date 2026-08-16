# Agent definitions

*How a teammate is declared, and what reaches its system prompt.*

Terms: [glossary](../glossary.md). Tool scoping is
[tools.md](tools.md); which workspace documents a role is routed is
[orchestration/context-routing.md](orchestration/context-routing.md).

---

## Two authoring forms

A company's roster may be written either way:

**Inline** — `[[agent]]` blocks in `company.toml`. Unchanged, still valid, still
the smallest thing that works.

**Per file** — one `agents/<id>.toml` per teammate under the company bundle.

```
companies/acme/
├── company.toml          # everything except the roster
├── agents/
│   ├── copywriter.toml   # the id is the filename
│   ├── seo_specialist.toml
│   └── prompts/
│       └── house-style.md
└── workspace/
```

The per-file form exists because a teammate is more than four fields once it
carries a custom prompt and its own briefing documents. A multi-line TOML string
inside an array-of-tables is unreadable at roster length, and prose belongs
beside the agent it configures.

### The two forms are exclusive

A bundle with both an `agents/` directory and `[[agent]]` entries is a
**validation error**, not a precedence rule. Either precedence rule silently
discards teammates an operator wrote down, and the roster is the one part of a
manifest where a silent omission stays invisible until the missing teammate
fails to answer.

### The filename is the id

`agents/copywriter.toml` declares the agent `copywriter`. An `id` key inside the
file is accepted only when it agrees with the stem; a mismatch is an error
naming both, because silently preferring one leaves an operator renaming the
other and wondering why nothing changed.

Files are read **sorted by stem**. Roster order is load-bearing — a company that
tags nobody `tier = "orchestrator"` gets its first-listed teammate — and
readdir order varies by filesystem, so an unsorted read would make which agent
runs the company depend on which machine parsed the bundle. A company that
relied on declaration order under the inline form MUST state
`tier = "orchestrator"` when moving to the per-file form.

Only the immediate directory is read. `agents/prompts/` holds documents, not
teammates.

## Schema

Every key below is available in **both** forms — one type, one validator, one
consumer, so adopting a custom prompt does not require adopting the bundle
layout first.

```toml
# agents/copywriter.toml
role = "Copywriter"                     # required
description = "Write ads and campaign copy."
tier = "reasoning"                      # cognition hint; never selects a model
tools = ["docs.*", "mcp:notion"]        # grant globs — see tools.md
delegates_to = ["creative"]             # desks this agent may hand work to
budget_usd_daily = 5.0                  # per-agent daily cap

prompt = """                            # appended to the generated persona
Write for the reader, not the client.
"""
prompt_files = ["prompts/house-style.md"]   # checked-in, bundle-relative
context = ["Brand/Brand voice.md"]          # live workspace documents
classes = ["evidence"]                      # routing exclusions — see below
```

## The prompt

An agent's system prompt is assembled in this order, and the order is a decision:

1. the generated **persona** — who this teammate is, at which company;
2. its inline **`prompt`**;
3. its **`prompt_files`** bodies;
4. tool briefs (workspace, publishing, skills catalogue);
5. its routed **`context`** documents.

Static material first, volatile last. The prompt prefix is what a provider cache
reuses across turns, so a workspace note the operator edits between two turns
must not invalidate the briefing behind it.

> **Step 5 is not yet wired into the harness.** The selection
> (`routed_documents`), the workspace read (`resolve_routed_documents`) and the
> rendering (`context_section`) are implemented and tested; carrying the result
> through `HarnessDeps` into `build::build_agent` is the remaining step. See
> [context-routing.md](orchestration/context-routing.md#the-rule). Steps 1–4
> are live.

**`prompt` is appended, never substituted.** The generated line is what binds the
agent to *this* role at *this* company; a prompt that replaced it would silently
cost the agent its identity and hand it back the runtime's own assistant
persona. What belongs in `prompt` is how the role works, not who it is.

### `prompt_files` versus `context`

They are the static and dynamic halves of the same idea, and they differ on
exactly one rule that matters:

| | `prompt_files` | `context` |
| --- | --- | --- |
| Source | the company bundle, under `agents/` | the live workspace tree |
| Read | once, at manifest load | on every roster rebuild |
| Missing file | **validation error** | skipped |
| Position | early (cache-stable) | last (volatile) |

The missing-file split is deliberate. A `context` entry names operator-owned
live state that may legitimately not exist yet. A `prompt_files` entry names a
file in the same commit as the agent referencing it, so a typo there yields a
role whose prompt was written around a briefing it silently never received —
which fails confidently rather than visibly.

A `prompt_files` path may not escape `agents/`. The check is on path components,
before touching the filesystem, rather than by canonicalizing: canonical
comparison resolves symlinks, and whether a bundle is valid must not depend on
how the checkout was laid out on the reading machine.

### Budgets

Each document section is clamped to `PROMPT_FILE_BUDGET_CHARS` (10,000
codepoints, a tokenizer-free upper bound on the brief budget in
[alignment.md](orchestration/alignment.md)). The clamp keeps the **leading**
portion, cuts on a character boundary, and appends a visible marker.

The budget applies to the **section**, not per document: a role routed five
documents and a role routed one spend from the same prompt. Clamping happens at
assembly, where the text is spent — refusing the read would cost the company the
whole document, while clamping the tail costs only the tail.

An empty or whitespace-only document is dropped rather than rendered as a bare
heading. An empty section reads to the model as a source that exists and says
nothing, which is worse than its absence.

## `classes`

The explicit epistemic classification
[context-routing.md](orchestration/context-routing.md) requires. Three values,
each subtracting one document:

| Class | Excludes | Prevents |
| --- | --- | --- |
| `evidence` | the assertion board | a role weighing evidence scoring an unevidenced sentence beside a real one |
| `judge` | the scratch | provisional working-out read as progress, which keeps a loop retrying |
| `directive` | the claim ledger | a role carrying out an instruction filing that instruction as a finding |

Declaring none is *unclassified*, which imposes no exclusion and is the right
default: an ordinary teammate is not judging anything.

An exclusion **outranks** both the tier default and an explicit `context` list.
That is what makes a declared class a control rather than a suggestion someone
can edit away. The universal method document is exempt — it is method, not
assertion, and a role excluded from it could not follow it.

The classification MUST be declared, never inferred from `role`: `role` is prose
an operator writes for humans, so matching on it would make a company that
renames "Critic" to "Reviewer" silently lose an exclusion, and a control a
rename can switch off is not a control.

## Where this lives

| Concern | File |
| --- | --- |
| Bundle loading, `prompt_files` resolution | `src/company/agent_file.rs` |
| Prompt composition and clamping | `src/company/prompt.rs` |
| Routing table and exclusions | `src/company/context_routing.rs` |
| Roster type and constants | `src/company/types.rs` |
| Manifest wiring and validation | `src/company/manifest.rs` |

The first three are **always compiled**, though the harness that spends the
prompt is behind the `openhuman` feature. Composition, clamping and the
exclusion table are pure decisions with real edge cases, and the exclusions are
controls — they deserve tests in every build, not only where the agent runtime
links.
