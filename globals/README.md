# The global baseline

What every company gets, whichever vertical it started from. A company bundle
under `companies/<name>/` describes one vertical; this directory describes the
part that is the same in all of them.

| Surface | Lives in | In every company because |
| --- | --- | --- |
| Agents | `agents/*.toml` | Research, writing, and keeping the record straight are not a vertical. |
| Workflows | `workflows/*.toml` | A weekly review and a research request run the same in a law firm and a game studio. |
| Skills | `[skills].always` in `globals.toml` | A few shared-library skills are installed rather than offered. |
| Tools | `[tools].baseline` in `globals.toml` | A teammate with no sandbox, no documents, and no workspace cannot work at all. |

The contents are embedded into the binary at build time, so they are present in
a platform-provisioned container that has no repository checkout beside it —
the same reason `src/ledger/registry.rs` carries its built-in ledgers as code.

## A company always wins

Globals are a floor, never a ceiling. On an id collision the company's own
definition supersedes the global one outright — its `agents/researcher.toml`
replaces this directory's, its `workflows/weekly_review.toml` replaces this
directory's. Nothing merges field-by-field, because a half-global teammate is
nobody's design.

To drop a global instead of replacing it, name it in the manifest:

```toml
[globals]
disable = ["agent:researcher", "workflow:weekly_review", "skill:meeting-brief"]
```

Every entry is `<kind>:<id>` and must name a global that exists — a typo is a
validation error, not a silently ignored line.

Global agents load **after** the company's own roster, and no global agent is
tagged `tier = "orchestrator"`. Both facts protect the same thing: which
teammate runs the company is decided by the company, and
[`orchestrator_id`](../src/company/types.rs) falls back to the first agent
declared when nobody is tagged.

The full contract is `../docs/spec/runtime/globals.md`.
