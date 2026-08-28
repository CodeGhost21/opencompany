# HTTP API

The Axum surface the runtime exposes. Existing routes (`GET /healthz`, `GET
/spec`, `GET /tiny`) are kept unchanged. Routes are grouped by audience;
handlers live as focused groups under `src/server/`, never in the binary.

## Operator API

Auth: a human's session cookie ([users.md](users.md)), or a platform-issued
token in platform mode (see below). There is no unauthenticated path and no
operator token — see [config.md](config.md#authentication).

Provisioning and suspension require the `platform` scope, which no session can
ever hold.

```text
GET    /api/v1/companies                       list running companies
POST   /api/v1/companies                       boot from an uploaded manifest (platform)
GET    /api/v1/companies/{id}                  status: charter, roster, budget burn,
                                               lifecycle state, tiny.place state
POST   /api/v1/companies/{id}/chat             operator message → event; SSE reply stream
POST   /api/v1/companies/{id}/chat/upload      multipart file → attachment reference (#1682)
GET    /api/v1/companies/{id}/chat/history     one desk's transcript (?desk=<thread>)
POST   /api/v1/companies/{id}/chat/messages/{seq}/reactions
                                               { "emoji": "👍", "on": true } → 204
GET    /api/v1/companies/{id}/events?since=SEQ SSE stream of events/effects (work feed)
GET    /api/v1/companies/{id}/approvals        pending approvals
GET    /api/v1/companies/{id}/notifications  unread notifications for the signed-in person
PUT    /api/v1/companies/{id}/notifications  mark notifications read (`{ "ids": [...] }`; empty body or null ids marks all)
POST   /api/v1/companies/{id}/approvals/{aid}  { "verdict": "approve"|"deny", "note": "…",
                                               "detach": false }
POST   /api/v1/companies/{id}/feedback         submit feedback (see feedback-loop/)
GET    /api/v1/companies/{id}/feedback         past reports (no operator words)
GET    /api/v1/companies/{id}/feedback/board   the shared board, one page
                                               ?sort=hot|top|new&type=feature|bug
                                               &status=open|planned|completed
                                               &page=1&limit=20
GET    .../feedback/board/{item}               one board item + its comments
POST   .../feedback/board/{item}/vote          { "value": 1 | -1 | 0 }
POST   .../feedback/board/{item}/comments      { "body": "…" }
GET    /api/v1/companies/{id}/memory/traces    inspect working memory (debug)
GET    .../memory/archives                    traces retained on eviction
                                             (provider-backed engines only; 404
                                             when the engine keeps no archive)
POST   /api/v1/companies/{id}/export           export bundle (tar)
POST   /api/v1/companies/{id}/pause            pause / resume lifecycle transitions
GET    /api/v1/companies/{id}/desks            the company's desks and channels
POST   /api/v1/companies/{id}/desks            create one ({ name, description?, id?,
                                               members?, responder? })
DELETE /api/v1/companies/{id}/desks/{desk}     delete an overlay desk
POST   /api/v1/companies/{id}/desks/{desk}/members         add a member
DELETE /api/v1/companies/{id}/desks/{desk}/members/{agent} remove an overlay member
PUT    /api/v1/companies/{id}/desks/{desk}/order           reorder (hierarchy)
```

Single-company (prosumer) mode aliases everything under `/api/v1/company/...`
with no `{id}`.

`GET …/notifications` returns only unread `mention` notifications addressed to the
signed-in human, newest first. Each row includes its subject, title, creation
 time, and optional chat context; `unread` is the returned count. Machine
credentials, which have no person identity, receive `401`. `PUT` accepts an
optional `ids` array and returns the remaining unread count. An omitted or null
`ids` value marks all notifications for that person; an empty array marks none.

The `/feedback/board/...` routes are a **proxy** of the TinyHumans hub's shared
board, spent with this instance's credential so a browser never holds one. An
instance without a credential has no board and every one of them answers
`404 tinyhumans_no_board` — the console hides the surface rather than rendering
an empty board. A vote is the *instance's* vote, since every console on a host
shares its one hub account. See
[feedback-loop/README.md](../feedback-loop/README.md).

`detach` on the approval resolve chooses what the response waits for. Omitted
(or `false`) it holds the response open for the agent's follow-up turn and
answers with that cycle's messages — the long-standing contract, unchanged.
Set, it answers `200 { "recorded": true, "alreadyResolved": bool }` as soon as
the verdict is durable and the grant minted, and the continuation arrives on the
`agent_reply` event-stream frame instead. `alreadyResolved` is a success: a
second resolve of the same approval is an idempotent no-op that mints no second
grant.

Either way the resolve survives a dropped connection — the follow-up cycle runs
on its own task, so it is no longer cancelled when a client or a reverse proxy
gives up mid-turn. `detach` removes the *wait*; it is not what provides the
drop-safety. See
[company-brain/approvals.md](../company-brain/approvals.md#settling-the-verdict-is-not-running-the-follow-up).

## Desks and channels: the `responder` mode

A desk row carries `responder: "lead" | "auto"` (issue #1835), **omitted when
`lead`** — which is every manifest `[[group_chat]]` (the blueprint syntax has
no such field) and every desk created before the field existed, so old
consoles and old wire shapes are byte-for-byte unchanged.

`"lead"` is the standing model: `members[0]` leads, and an unmentioned message
addressed to the desk is answered by that lead. `"auto"` is a **channel**: no
lead exists — the org chart crowns nobody, the members pane badges nobody, and
`delegate_to_desk` refuses it with a reason — and an unmentioned message's
answerer is picked **per message**, by a single tool-less model call over the
channel's own membership (id, role, description), clamped to that membership.
An `@`-mention outranks the pick everywhere, and wherever selection cannot run
— the default build (the selector compiles under the harness feature), the
small-talk fast path, a failure, a timeout — the answer is the channel's first
roster member: exactly what a lead desk would have answered, so the worst case
of the new mode is the old mode. Selection spend is metered under its own
usage kind (`selectorCall`), charged to the whole-company bucket.


### Chat attachments (issue #1682)

```text
POST   …/chat/upload                          multipart file → { nodeId, name, mime, size }
POST   …/chat                                  { "message": "…", "attachments": ["<nodeId>", …] }
```

Two steps, deliberately not one: the byte-transfer half is decoupled from the
synchronous, turn-running `/chat` POST, so a large upload never blocks the
turn and a turn never blocks on bytes. `/chat/upload` is a **binary-only**
sibling of the workspace's `POST …/workspace` create — a chat attachment is a
file hung on a message, not a document someone maintains, so it always stores
bytes and is served back through the existing hardened
`GET …/workspace/blob/{nodeId}` (no second blob route). It shares
`admit_upload`'s size/quota gate and the workspace's filename sanitizer with
that route, and is subject to the same sibling-name collision rule a
workspace create enforces — two attachments in different messages sharing an
exact filename would collide there, so this route retries once, transparently,
under a name disambiguated from the upload's own id rather than surfacing the
`409`.

`/chat`'s `attachments` field is **node ids only**. The host re-resolves each
id against the sending company's own workspace tree and takes the name / mime
/ size from the store — never the client's claim — the same discipline a
`parent` thread reference gets; an id that resolves to no binary node in this
company is a `400`, on the same terms a bad `parent` is. Server-side, the host
also extracts each attachment's text where the format and size allow it (PDF,
DOCX, PPTX, XLSX, plain text — the same `ingest::extract` pipeline
`POST …/memory/ingest` runs; see [memory.md](../company-brain/memory.md)) and
carries it in the journaled event, capped, so a brain that later reads the
message off the wire has the attachment's actual words rather than only a
node id it has no tool to resolve. An image or a scan with no text layer
carries no extracted text; the reference alone still rides the wire.

### Running and stopping a workflow (issue #383)

```text
POST   …/workflows/{wid}/run                 { "input": {…}, "detach": false }
POST   …/workflows/runs/{runId}/cancel       stop a run that is still walking its graph
```

`detach` is the same idea as the approval one and reads the same way. Omitted
(or `false`) the response is the settled run — `{ output, pendingApprovals,
deliveries, runId }`, byte-unchanged, plus `cancelled: true` **only** when the
run was stopped while the request was still open. A synchronous run is
cancellable like any other: its id is registered before the first node runs and
the console learns it from the `workflow_run_started` frame, so a cancel can
land mid-request. Without that flag the resulting `output: null` with no
approvals and no deliveries would be indistinguishable from a run that
legitimately produced nothing. Set, the host answers **`202 Accepted`**
with `{ "runId": "…", "detached": true }` before the engine walks a node; the
run is then followed through the `workflow_run_started` / `workflow_node_finished`
/ `workflow_run_finished` frames it already keys by that `runId`, and read back
from `GET …/workflows/runs`, whose fold reports `running: true` until it settles.

That read is **paged** (issue #1012): `{ runs, hasMore, nextBeforeSeq }`, where
`nextBeforeSeq` is the cursor to pass back as `?before_seq=` and is omitted once
`hasMore` is `false`. The page is cut by `seq` and only then sorted for display
by `(atMillis, seq)`, so the cursor is the page's *lowest* `seq` rather than its
last row — clients must send back what the host issued rather than deriving it,
and a client talking to a host that omits the field falls back to the old
`runs.at(-1).seq` derivation, never to "no more pages". Why the two keys differ,
and the partition argument that makes paging lossless under a clock regression:
[server/run-history-paging.md](../../modules/server/run-history-paging.md).

**Clients must discriminate on the response shape, not on what they sent.** A
host predating this ignores the unknown `detach` field and answers the full
synchronous `200`, so `output` present means "already settled" and `detached`
present means "watch the stream". Both directions are compatible: an older
client never sends the field and is unaffected.

Either way the run survives a dropped connection. It runs on its own task, so a
closed tab or a proxy giving up no longer cancels it mid-graph — before this it
did, and because a run journals a start first, the abandoned run then folded as
`running: true` until the next host restart swept it.

**A company bounds how many runs it will execute at once** (issue #401). Every
run — this manual route, a cron fire, an approved gate's continuation, and one
an orchestrator agent starts — counts against `[workflows].max_in_flight_runs`
(default 8; see `manifest.md`). A run over the ceiling is **refused, never
queued**: the route answers **`429 Too Many Requests`** with the standard
`{ "error", "code": "workflow_run_limit" }` envelope and **no `runId`**, because
nothing started. Both `detach` modes refuse identically — the check precedes the
detach/sync branch, so a rejected run journals no `WorkflowRunStarted`. The
message names the three levers: wait for a run to finish, stop one via
`…/workflows/runs/{runId}/cancel`, or raise the manifest cap. A slot frees the
moment a run settles (including on cancel or panic), so a refused run succeeds on
the next attempt once the company is back under its ceiling.

`…/runs/{runId}/cancel` answers `200 { "cancelling": true }` when the run is
live and `404` when the run is unknown **or has already settled** — one answer,
because they mean the same thing to the caller: there is nothing to stop. It is
behind the same `ScopedCompany` guard as every other route here, so any operator
of the company may stop any of its runs.

`cancelling`, not `cancelled`: the route fires a signal and returns. The run
settles a moment later with a `WorkflowRunFinished` carrying `cancelled: true`
and **no error** — a stop somebody asked for is not a failure, and a reader that
only checks `error` would render it as a clean success.

**Stopping is not finishing.** The executing node is dropped mid-await rather
than allowed to complete, so an external side effect it had started may be
half-done — the same class of outcome as the host being killed, only
operator-initiated. Nodes that already completed keep their journal rows, and
approvals earlier nodes parked stay valid in the queue: they are journal-backed
and independent of the run, so they can still be approved or denied afterwards.
No minted grant is revoked. See
[workflow-events.md](workflow-events.md#stopping-a-run-issue-383).

## Console write plane (`src/server/ops/`)

Moved to [`api-write-plane.md`](api-write-plane.md) — this file was over the repository's 500-line limit. See that page for the full detail.

## Read plane — GraphQL (`/graphql`)

Moved to [`api-graphql.md`](api-graphql.md) — this file was over the repository's 500-line limit. See that page for the full detail.

## Agent-facing (tiny.place-compatible)

Enabled per company by `[place].discoverable`; served only with the
`tinyplace` feature.

```text
POST   /a2a/{handle}                        A2A JSON-RPC (tasks/send …), SIWX-verified
GET    /a2a/{handle}/skill.md               capability discovery doc
GET    /.well-known/agent-card.json         single-company mode
GET    /companies/{handle}/.well-known/agent-card.json   platform mode
```

- Inbound requests carry tiny.place per-action signatures
  (`Authorization: tiny.place <agentId>:<signature>:<timestamp>`); the
  runtime verifies via the `tinyplace` SDK before anything reaches the brain.
- **x402-priced skills**: if the requested skill has a price on the Agent
  Card, the route responds `402 Payment Required` with the x402 challenge;
  on resubmission the payment is verified through
  `AgentEconomy`/the facilitator, receipted to the ledger, and the task
  enters the event queue as `A2aTaskReceived`.
- Untrusted counterparty text is prompt-guard sanitized before it reaches the
  brain (mirroring tiny.place's own promptguard practice).

## Inbound integrations

```text
POST   /hooks/{companyId}/{channel}         webhooks → CompanyEvent
```

HMAC-verified per channel secret from the `SecretStore`; unverifiable
payloads are dropped with a 401 and never become events.

## Auth model

| Caller | Mechanism |
| --- | --- |
| Prosumer operator (local) | Operator token minted at first run, stored in the OS keychain / config dir; the desktop UI holds it. |
| Platform | Platform-issued JWT per tenant; `POST /api/v1/companies` and suspend/archive require a platform-scope claim. |
| Peer agents (A2A) | tiny.place SIWX signatures + optional x402 payment; no accounts. |
| Webhook senders | Per-channel HMAC secrets. |

The runtime's own upstream credential (`TINYHUMANS_API_KEY` / JWT) is never
accepted inbound; it is outbound-only ([config.md](config.md)).

## Errors

JSON error envelope `{ "error": string, "code": string }` with stable `code`
values; 4xx for caller mistakes, 402 reserved for x402 challenges, 409 for
lifecycle-state conflicts (e.g. chatting with an archived company).

## Platform webhooks (Phase 5)

Platform mode can register outbound webhooks per tenant for
`approval.requested`, `work.completed`, `feedback.created`, and
`budget.exhausted` so hosts can build their own surfaces without polling
SSE. Delivery is at-least-once with signature headers; see
[product/platform.md](../product/platform.md) for the requirements source.
