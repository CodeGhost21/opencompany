# In-use guards: the wire shape all three agents share

Part of [the keys rework](README.md) (issue #2306). This is the shared
contract for "you are about to remove, clear, disable or switch something
other config depends on" across Agent A's scope (Composio, search, account
key), Agent B's scope (provider rows, the default, agent pairs — this file's
primary author), and Agent C's console. Written first and pushed early so all
three agents code against the same shape without asking.

## 1. `usedBy` on DTOs

Any DTO for a thing another piece of config can depend on carries an optional
`usedBy` field:

```ts
usedBy?: {
  default?: true;               // this is the company's inference default
  agents?: { id: string; name: string }[]; // agent pairs naming this provider
  surfaces?: ("llm" | "composio" | "search")[]; // which product surfaces use it
}
```

Rules:

- Every sub-field is omitted when empty (`skip_serializing_if` on the Rust
  side; no empty arrays or `false` on the wire).
- The whole `usedBy` field is omitted (not `null`, not `{}`) when nothing uses
  the thing, so `"usedBy" in dto` is itself the in-use check on the console.
- `agents` lists every agent whose pair (§6, `Agent`/`OverlayAgent`/
  `AgentOverride.provider`) names this provider's slug, by id and display
  name, in roster order.
- `default` is `true` iff `store::load_default(...)` is `DefaultChoice::Full`
  or `DefaultChoice::ProviderOnly` naming this provider's slug. A bare-slug
  default counts: switching that provider off or deleting it still needs
  confirmation, because the operator's stated intent is "this is my provider."

**Agent B's `usedBy` producers (provider rows):**

- `ProviderDto.usedBy` on every row in `GET .../inference` and
  `GET .../inference/providers`: `default` from `load_default()`, `agents`
  from every agent pair naming the row's slug (manifest agents, overlay
  agents, and overlay edits — the same merge `effective_manifest_agent` does).
- A provider **key** clear or disable (`POST .../providers/{slug}/enabled`
  with `enabled: false`, and the key-clearing paths) carries the same
  `usedBy` the row's status would show, because disabling or clearing the key
  makes the row unable to serve exactly the same dependents a delete would
  strand.

**Agent A's `usedBy` producers:** Composio token/BYOK key clears, the account
key clear/rotate. **Agent C:** search provider removal, if it depends on
anything else (search has no default/agent-pair equivalent, so it is unlikely
to need this at all — confirm before wiring a dialog for it).

## 2. Refusal without confirmation

Any mutation that **removes, clears, disables, or switches** something with a
non-empty `usedBy` is refused unless the request carries `confirmInUse:
true`:

- in the JSON body for POST/PUT/PATCH;
- as the query parameter `?confirmInUse=true` for DELETE (which has no body
  on this API).

**Refusal is HTTP 409**, in the repo's existing API error envelope. That
envelope is `ApiError` (`src/server/error.rs`), built from
`crate::error::OpenCompanyError` (`src/error.rs`). Every error renders as
`{"error": "<sentence>", "code": "<stable_snake_case>"}` — `error` is the
`Display` string, `code` is `OpenCompanyError::code()`. One variant,
`WorkflowInvalid`, additively carries one extra top-level key (`problems`) by
special-casing the match arm in `ApiError`'s `IntoResponse` impl
(`src/server/error.rs:153-163`); nothing else does. Do **not** invent a second
envelope. The keys-rework code follows that exact precedent: one new
variant, `OpenCompanyError::InUse { message: String, used_by:
serde_json::Value }`, mapped to `StatusCode::CONFLICT` in `ApiError::status()`,
`"in_use"` in `OpenCompanyError::code()`, and one more special-cased arm in
`IntoResponse` that adds `"usedBy": used_by` alongside `error` and `code`:

```json
{
  "error": "Anthropic is used by the company default and 2 agents: Researcher, Web search.",
  "code": "in_use",
  "usedBy": { "default": true, "agents": [{"id": "researcher", "name": "Researcher"}, {"id": "web_search", "name": "Web search"}] }
}
```

The message names **every** user, in one sentence:

- `default` alone: `"<Label> is the company default."`
- `default` + N agents: `"<Label> is used by the company default and N agent(s): <names, comma-joined>."`
- agents only: `"<Label> is used by N agent(s): <names, comma-joined>."`
- a key clear/disable with only `surfaces`:
  `"<Label>'s key is used by <surfaces, comma-joined>."`

`N agent(s)` is `"1 agent"` or singular-free `"N agents"` — never
`"1 agents"`.

## 3. With confirmation

`confirmInUse: true` (or `?confirmInUse=true`) makes the mutation proceed
exactly as it would with no dependents, and the response **echoes** `usedBy`
(the same shape, computed **before** the mutation applied) so the caller's UI
can show what it just broke. `usedBy` on a success response uses the same
optional-field-omission rule as §1.

## 4. Q8 holds: confirmed removal keeps the default

A confirmed removal of a provider's key, or a confirmed disable of a provider
row, while that provider is the company default (or an agent pair), **never
rewrites `inference/default` or the agent's pair**. The stored default keeps
naming that (now-broken) provider. This matches decision Q8's TinyHumans case,
generalized to every provider: the operator asked for the removal and
confirmed it; a silent rewrite would be a second undocumented decision the
operator did not make. What happens next is the turn-time fail-closed
behaviour in §5, and the console's job (Agent C) is to make the broken state
visible, not to conjure a new default.

The single carve-out that already exists and stays: **deleting** a provider
row's index entry (not just its key) does trigger
`clear_default_if_marked` (unchanged pre-existing behaviour, 2c). That is a
row-existence rule, not a Q8 exception — the default cannot name a slug with
no row at all, because `get_provider` would return `None` and the turn would
already fail closed exactly as intended. A **disable** or a **key clear**
leaves the row (and therefore the marker) in place.

## 5. Turn-time fail-closed messages (F6)

When resolution reaches a provider that is missing, disabled, or has no key,
the turn error names the **agent** (when the choice is a pin), the
**provider**, and the **fix**, e.g.:

> "Researcher uses Anthropic, which was removed. Choose a provider and model
> for Researcher or the company default in Connections → LLM."

For the company default (no agent named):

> "The company default uses Anthropic, which was removed. Choose a default
> provider and model in Connections → LLM."

These are produced by `resolve_choice` (2b/3a) and the `TenantProvider::resolve`
pin check (3a); see `phase-2b-default-shape-part2.md` §4.3 and
`phase-3a-agent-pair-backend.md` §4.4 for the base wording, which this slice's
implementation keeps verbatim except for updating the settings path name to
match this codebase's actual console route ("Connections → LLM" here, since
that is where slice 2a/2c's provider list lives on this branch's console).

## 6. What counts as "used", precisely (Agent B's scope)

A provider row (by slug) is used by:

1. **The default** — `store::load_default(...)` is `Full(choice)` with
   `choice.provider == slug`, or `ProviderOnly(slug)`.
2. **An agent pair** — any of:
   - a manifest `[[agent]]` with `provider = "<slug>"`;
   - an `OverlayAgent` (console-added teammate) with `provider: Some(slug)`;
   - an `AgentOverride` (an edit on a manifest agent) with
     `provider: Some(slug)` (not `Some("")`, which is cleared).
   Computed the same way `runtime/builder.rs`'s `agent_pairs` helper (5a's
   sibling, added in 3a) computes it: manifest agents merged with their
   overlay edits, plus every overlay-only teammate, each filtered to
   `provider` and `model` both present and non-blank.

A provider **key** (credential) is used by the same two things, because a row
with no key cannot serve either dependent — clearing the key is functionally
equivalent to disabling the row from a dependent's point of view, so it gets
the same guard.

## 7. Routes are not part of this contract

`inference/routes` (until 5b removes it) is never a `usedBy` dependent and
never gates a guard: it is a legacy, unvalidated blob that 5a only ever reads
for a one-time carry-over, never as a live dependency. A provider named only
by a stale route can be deleted with no confirmation prompt.
