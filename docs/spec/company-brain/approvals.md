# Checkpoints and Approvals

The trust core of the product: agents act freely inside the fence; anything
irreversible waits for the Operator. This doc is normative.

## Checkpoint taxonomy

Effect kinds that MAY require sign-off, grouped by what they risk:

| Group | Effect kinds (examples) | Default in `supervised` mode |
| --- | --- | --- |
| **Spend** | `payment.send`, `subscription.start`, x402 outbound above cap | approval above `auto_approve_under_usd` |
| **Send** | `email.send`, `dm.external`, any first message to a new counterparty | approval for new counterparties; allowed for established threads |
| **Sign** | `filing.submit`, `contract.accept` | always approval |
| **Publish** | `external.publish`, Agent Card / price changes, website deploys | always approval |
| **Hire** | outbound A2A engagement with a new company; firing a vendor | approval above threshold or first-time counterparty |
| **Identity** | handle registration/renewal, key rotation, delegated signer mint/expand | always approval |

`readonly` mode gates *every* effect; `full` mode auto-allows everything
except `[policy].always_approve` entries. `auto` sits between `supervised` and
`full`: the agent's own sandbox writes and its outward reads run unattended,
and anything that leaves the company or spends on submit still parks — see
[the tier line](grants.md#the-auto-tier).

Three of the four names are OpenHuman's own security tiers. `auto` is not, so
the mapping is no longer 1:1 and `PolicyMode::security_tier()` — the accessor
that asserted it was — has been deleted rather than made to lie. Where the two
vocabularies still have to meet, `harness::toolbelt::autonomy_for` borrows
`Supervised` for `auto`; the argument is on that function and matters, because
a workflow `tool_call` node has no `ApprovalPolicy` above it.

## Approval lifecycle

```text
effect emitted ─▶ evaluate ─▶ Allow ─▶ execute, journal
                      │
                      ├─▶ Deny ─▶ returned to brain as refusal (it replans)
                      │
                      └─▶ RequireApproval ─▶ park (ApprovalId)
                                              │  surfaces in approvals inbox + chat
                                              ▼
                            operator resolves: approve │ deny │ edit
                                              │
                                              ▼
                            ApprovalResolved event ─▶ follow-up cycle
```

- **Default-deny on silence**: parked approvals expire (default 7 days,
  configurable) to `deny`. Nothing irreversible ever happens because the
  Operator was on vacation.
- **Edit** lets the Operator amend the effect payload (fix the email, lower
  the amount) and approve the amended version; the brain sees both the
  original and the edit.
- Resolution requires operator auth ([runtime/api.md](../runtime/api.md));
  the resolving `Actor` is journaled.
- Approve executes the parked effect exactly once
  (journal-before-execute, [runtime/lifecycle.md](../runtime/lifecycle.md));
  deny feeds the refusal back so the brain replans rather than retries.
- **Resolution is idempotent.** Resolving an approval that is no longer parked
  — a double-submit, a retried request, two operators on the same queue —
  is a no-op with a fixed reply. It writes no journal record and runs no
  follow-up cycle.

## Emergency stop (the governance kill switch)

`POST /api/v1/companies/{id}/emergency-pause` denies **every** new effect
outside the `Other` group, ahead of every policy rule including
`always_approve`. `POST .../emergency-resume` releases it. Both are
owner-scoped and both take a confirmation phrase in the body. Normative:

- **It denies, it does not park.** Parking would make the approval queue an
  escape hatch from the switch: an operator could approve the very effects they
  just stopped without ever releasing it. Denial returns to the brain as a
  refusal it replans around, which is what "park all new work" has to mean.
- **`Other` stays allowed, so chat survives.** The operator has to be able to
  ask the company what it was doing. Effects classified as `Other` remain
  allowed while the emergency stop is engaged; the gate otherwise treats
  `Other` as a catch-all group, not a chat-only one, and does not police which
  tools it covers.
- **It is orthogonal to `lifecycle`.** `lifecycle = "paused"` rejects every
  request with a `409`, chat included — the opposite of what an emergency
  needs. A company can be `running` *and* stopped; resuming one does not resume
  the other. `GET /api/v1/companies/{id}` reports `emergency_paused`
  separately, and a console that reads only `lifecycle` will show a stopped
  company as healthy.
- **Already-parked approvals stay resolvable.** The switch gates `evaluate`,
  which runs before an effect executes; resolution does not pass through it.
  New work stops, in-flight decisions the operator was already asked for do
  not become unanswerable.
- **The event log is the durable state.** The last
  `CompanyEvent::EmergencyPauseChanged` decides, replayed at boot, so a stop
  survives a restart. There is deliberately no `CompanyRecord` field: a second
  copy of a safety flag is a second thing that can disagree with the first.
- **Fails safe.** If the log cannot be read at boot, the company comes up
  **stopped**. A company wrongly stopped is a visible, one-request problem; a
  company wrongly running is the failure the switch exists to prevent, and
  nothing would surface it.
- **Engaging is eager, releasing is durable-first.** Engaging flips the
  in-memory flag before journaling, so enforcement never waits on I/O that can
  fail. Releasing journals first and only clears the flag on success, so a
  failed write leaves the company stopped. The unsafe direction is never taken
  on a best-effort basis.
- **No timeout, ever.** Unlike a parked approval, the stop does not expire and
  is untouched by the TTL sweep. Only a deliberate `emergency-resume` by an
  identified operator clears it. A kill switch that lets itself go would resume
  work at 3am with nobody watching.
- **Both transitions are journaled with the acting `Actor`** and an optional
  operator note.

Confirmation is asymmetric on purpose: engaging takes the fixed phrase
`EMERGENCY-PAUSE` (an operator reaching for a panic button should not have to
look up an id), while releasing takes **the company's own id**, so the only way
out of the stop cannot be reached by replaying one body across companies.

Credential revocation — the other half of the audit's "emergency pause and
credential revocation" — is not covered here; it needs token scoping in the
harness.

### Settling the verdict is not running the follow-up

Resolving is two halves with very different durations, and the runtime keeps
them apart:

1. **Settle** — record the verdict, journal it, mint the grant (or execute the
   native effect). Milliseconds. When it returns, the operator's decision is
   permanent.
2. **Follow-up cycle** — a full agent turn, so the brain learns the verdict and
   re-issues the granted call. Can take minutes.

The follow-up always runs on its **own task**, which the resolve then awaits.
That makes it drop-safe: a client that disappears mid-turn — a closed tab, or a
reverse proxy giving up on a slow upstream — abandons the *waiting*, not the
work. Fused, the two halves meant a dropped connection cancelled the
re-dispatch after the grant had already been spent, so the operator's approval
bought nothing and the conversation never resumed.

A resolve can also **detach** (`"detach": true`), answering the moment the
verdict is durable rather than holding the response open for the turn. The
continuation then arrives on the event stream's `agent_reply` frame. The
blocking form remains the default and its response body is unchanged.

A follow-up cycle that *fails* is logged host-side and leaves a recoverable
state, never a stranded one: the verdict and grant are already durable, and
re-approving is the idempotent no-op above, so a retry mints no second grant.

## Approving a blocked tool call: single-use grants

Moved to [`grants.md`](grants.md) — this file was over the repository's 500-line limit. See that page for the full detail. What moved, in the order it appears there:

- single-use grants: what approving a blocked **tool** call mints, versus a **native** effect the runtime performs itself (approving one of those executes it, per the lifecycle above);
- [standing grants](grants.md#standing-grants-this-tool-for-this-teammate-until-a-deadline) — this tool, for this teammate, until a deadline — and [what can never be granted broadly](grants.md#what-can-never-be-granted-broadly);
- [the `auto` tier](grants.md#the-auto-tier) that low-consequence middle defines, and [listing and revoking](grants.md#listing-and-revoking) live standing grants;
- [which tier a new company gets](grants.md#which-tier-a-new-company-gets) (issue #605);
- [what an `always_approve` entry names](grants.md#what-an-always_approve-entry-names-issue-684) (issue #684);
- [precedence at the tool gate](grants.md#precedence-at-the-tool-gate), and its step 7, [per-call judgement](grants.md#per-call-judgement-issue-338) (issue #338).

## Approvals inside a workflow run (issue #395)

A workflow run is not a cycle, and for a long time that meant nothing a run
parked ever reached this queue. Two distinct holes, both now closed.

**A gated tool call inside an `agent` node.** The node's turn runs on the same
pool and the same `ApprovalPolicy` as chat, so a blocked call *was* recorded on
the shared request queue — but the only drain lived inside `run_cycle` behind a
`CycleHost`, which the workflow path never reaches. The request sat there until
the next chat cycle cleared the queue, and the operator was never asked. The
node now takes a queue boundary before its turn, claims only the tail its own
turn added, and parks each entry. Approving one is the ordinary single-use grant
above: the workflow has already finished with the refusal narrated, so the
approved call **runs standalone** and does not feed downstream nodes.

**A node marked `requires_approval`.** The engine reports these as node ids on
the run outcome, which reached the HTTP response and the `WorkflowRunFinished`
line — neither of which is an approval. Each pending gate now parks a
`workflow.approve` effect carrying the workflow id, the node id and the trigger
input, deduped on that triple so a re-run does not stack a second card for one
decision. It is a **native** effect (no `agent`), so approving performs it
rather than minting a grant.

### Approving a paused gate re-runs the workflow

The engine **settles** a paused run — nothing holds a task, a connection or a
continuation. So there is nothing to resume, and "continue" necessarily means
starting a fresh supervised run with the approved gate id unioned into the
trigger input's `approvals` array. The parked effect carries everything that
needs, which is what makes it survive a restart: journal replay rehydrates the
card and approving it still continues the work.

The cost is stated rather than hidden: **upstream nodes re-execute**. Agent
nodes re-spend tokens, and a reached `output` node **re-delivers** — a
warm-recipient email sends again, because the established-thread check is
state-based, not run-based. A gate normally sits *before* the side-effecting
node it guards, which is the entire reason to author one, so this is acceptable
for now; it is a real constraint on where a gate belongs in a graph.

A gate nobody decides ages out on the ordinary TTL to a default deny. Since the
paused run settled long ago, that costs nothing and cancels nothing.

## Where the request is raised (issue #379)

An approval is not only a queue entry; it is an interruption of a conversation.
So a park records **which conversation** — `ApprovalParked.thread`, stamped by
the cycle from its own trigger events, surfaced on `ApprovalSummary.thread`, and
carried onto `GrantedCall.origin_thread` when the approval mints a grant.

The id is `OperatorMessage.chat`: a desk id for a channel, a roster agent id for
a direct message. `Effect.agent` cannot stand in for it, and that is the whole
reason the field exists — a desk channel and a direct message to that desk's
lead are answered by the same teammate, so a request placed by asker would be
raised inside the wrong one of the two.

It follows the work rather than the queue entry. A resolution inherits the
thread of the approval it settles, so a follow-up turn that needs a **second**
sign-off re-parks in the channel the first was asked in instead of falling out
of the conversation. And the redeemed grant's continuation is journaled into
that thread too — approving something visibly causes the next thing to happen,
in the place the operator was already reading.

The stamp is refused rather than guessed. A cycle batching two conversations,
or an addressed turn beside an unaddressed one, or beside a task dispatch,
stamps nothing. An approval with no thread — a workflow delivery, a scheduler
tick, anything parked before this shipped — belongs to no conversation and is
shown on the Approvals page alone, which is where every approval was shown
before. The page always lists everything; the in-conversation card is additive.

The event log carries the park itself (`CompanyEvent::ApprovalParked`) so the
card can appear live. It is deliberately thin — an id, a dotted kind, a thread —
because the effect's payload is redacted in exactly one place and must not
acquire a second. A reader re-reads the approvals feed for the rest.

## Delegation levels (standing rules)

Prosumers adjust the fence in plain language, which compiles to policy:

- "Auto-approve spending under $5" → `auto_approve_under_usd = 5.0`
- "Never contact my customers directly" → `never_do` → `Deny` on
  `dm.external` matching the customer list
- "You can post to the blog without asking" → remove `publish_artifact` from
  `always_approve` for that channel. Nothing to remove unless the operator put
  it there: `always_approve` defaults to empty, and under `supervised` it is
  the checkpoint taxonomy — not the list — that parks a publish

Standing-rule changes are themselves Charter edits with provenance and audit
([charter.md](charter.md)); loosening a rule takes effect for *future*
effects only.

## Audit

The approval log is immutable: every evaluate decision, park, resolution
(with actor and timestamp), expiry, and execution outcome is an `EventLog`
entry, and money-touching effects additionally journal to the ledger. The
operator surface renders this as plain history ("you approved sending the
Acme invoice on June 2").
