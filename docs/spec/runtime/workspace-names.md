# Workspace naming: lowercase, dashed

Every folder and file the runtime puts in a workspace is named in lowercase with
dashes: `playbooks/close-checklist.md`, `agents/page-builder/`, `pages/<slug>/page.tsx`.

Implemented by [`src/company/workspace_names.rs`](../../../src/company/workspace_names.rs);
this file is the contract and the compatibility story.

## The rule

One function, [`kebab_name`], defines it, and every other question about a name
is asked through that one implementation:

| Input | Result |
| --- | --- |
| ASCII letters | lowercased |
| digits | kept |
| `.` | kept — so an extension survives — but never leading, so no hidden file and no `.`/`..` |
| everything else (spaces, `_`, `&`, `/`, punctuation, non-ASCII) | one `-`, with runs collapsed |
| a dash beside a dot, or at either end | removed |
| nothing survives (`🎉`, `""`, `"."`) | the caller's fallback, else `untitled` |

Bounded at 96 bytes, which is a name, not a path — a path is several of these
joined.

`is_kebab_name` is defined as the *fixed point* of `kebab_name` rather than as a
second transcription of the grammar, so "needs renaming" and "was renamed to"
cannot drift apart.

## Why a name's shape is load-bearing

Identity in the workspace is by path, so this is not tidiness:

- A name with a space in it needs quoting everywhere one is typed — a tool
  argument, a `[[wikilink]]`, a URL, a shell command.
- `Close checklist.md` and `Close Checklist.md` are two nodes on the sqlite and
  mongodb backends and one node on a case-insensitive filesystem. The same tree
  meant different things per backend.
- An agent told to "put it in Playbooks" had to guess the capitalization, and a
  wrong guess mints a rival folder rather than failing — and a duplicated name
  makes that path ambiguous for **every** agent from then on
  ([`artifacts.md`](artifacts.md), [issue #759](../../../src/company/workspace_repair.rs)).

## Where it is applied

At every point the runtime **mints** a name:

| Site | What it names |
| --- | --- |
| `company::workspace_scaffold` | the system roots `agents/`, `desks/`, `secrets/`, and `secrets/readme.md` |
| `company::workspace_scaffold::ensure_agent_folder` | `agents/<dashed roster id>/` — `page_builder` lands at `page-builder` |
| `company::artifact_mirror` | every folder and the file a publish mirrors into the tree |
| `harness::workspace_tools` (`workspace_create`) | the node the agent asks for |
| `harness::workspace_tools::lifecycle` (`workspace_rename`) | the node's new name |
| `harness::pages_tools` | `pages/<slug>/{page.toml,page.tsx,page.compiled.mjs}` |
| `harness::build` | the agent sandbox on disk, `<home>/harness/<company>/<agent>/` |
| `company::context_routing` | the routed documents `method.md`, `agents.md`, `brief.md`, `claims.md`, `threads.md`, `board.md`, `scratch.md` |

Normalizing at the boundary is the call [issue #580] made for workflow ids: the
host owns the name, so the model cannot pick an unsafe or unspellable one. The
tool reply always echoes the path the write actually landed at — including the
*stored* spelling of the parent folder, which is not always the spelling that
was typed.

Two names are deliberately **not** normalized, because they are contracts with
something outside this workspace: a skill bundle's `SKILL.md`, and a company
bundle's own `AGENTS.md` on disk.

An artifact record's `source` is not rewritten either. It names the file in the
agent's sandbox that was published, and it is the key a republish extends the
same record by ([`artifacts.md`](artifacts.md)); normalizing it would make the
record claim a path the agent cannot read back.

## Existing trees are not renamed

A tenant must not find its workspace rearranged by an upgrade it did not ask
for. Issues #570, #645, #700 and #759 each made that call, and a rename also
breaks every reference somebody kept to the old name.

So old names stay, and every **reader** is widened to reach them:

- `workspace_scaffold::find` matches a name case-insensitively, so a legacy
  `Agents/` root is adopted rather than joined by a lowercase twin — which would
  split one agent's home in two. `ensure_agent_folder` additionally adopts the
  roster id spelled verbatim (`page_builder` before `page-builder`).
- The agent tools' path index carries a normalized key beside the literal one,
  so `playbooks/close-checklist.md` and `Playbooks/Close checklist.md` name the
  same note whichever is typed. The literal match wins, so a tree holding both
  spellings resolves each to itself.
- Context routing, the page tools and the `pages` HTTP route match the same way.
  A manifest written before the rule still says `BRIEF.md` in its `context`
  list, and still routes.
- Wiki links resolve on the normalized name, on both the host
  (`company::workspace_links`) and the console
  (`frontend/src/lib/workspace.ts`), so `[[Close checklist]]` still resolves to
  `close-checklist.md`. Link text is how a document reads; the file name is what
  the tree is kept in, and the two need not be the same string.

The one exception to "nothing is renamed" is the agent **sandbox** directory,
which `harness::build::ensure_agent_workspace` moves onto its canonical path
once. That is private scratch, addressed only from inside the process — no ids,
no links, no console points into it — so nothing outside can notice the move. A
failed move leaves the agent with a fresh empty sandbox rather than failing the
turn.

Converting an existing *tree* in place is left as an operator action, and there
is no such action yet: it belongs beside the `workspace_repair` pass, whose
dry-run-then-apply shape it needs.

[`kebab_name`]: ../../../src/company/workspace_names.rs
[issue #580]: ../../../src/harness/built_in/workflow_build.rs
