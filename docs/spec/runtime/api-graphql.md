# The GraphQL read plane

The `/graphql` read surface and how it relates to the REST reads.

Split out of [`api.md`](api.md), which was over the repository's 500-line ceiling.

## Read plane — GraphQL (`/graphql`)

Every console **read** is served by a single async-graphql query surface at
`POST /graphql` (with a `GET /graphql` GraphiQL explorer in development). The
REST **read exceptions** are the console reads that ship over REST instead —
the two inbox `GET`s and the three workspace `GET`s, the task export, the
skill-registry browse, the agent detail `GET`, the policy read, and the
credential status `GET` — each because the console ships no GraphQL client and
that view needs a reachable read (issues #173, #177, #607, #352, #264, #562;
see the [write plane](api-write-plane.md)). The schema is query-only — REST
otherwise owns writes — and is **built once at startup** and stored on
`AppState`; each request injects its resolved `GqlAuth` principal.

The schema is rooted at a **`Company` aggregation object** so a view fetches
everything it needs in one round trip; the only top-level queries are
`companies`, `company(id)` (the sole company when `id` is omitted in
single-company mode), and `skillRegistry` (the unscoped shared library). Under
`Company` hang `team`, `chats`/`chat(id)`, `inboxes`, `tasks`, `skills`,
`workspaceTree`/`workspaceFile(id)`, `memory`, `workflows`/`workflow(id)`,
`usage`, `finances`, `connections`, `domain`, and `smtp`. The authoritative
contract is the SDL snapshot at
[`src/server/graphql/schema.graphql`](../../../src/server/graphql/schema.graphql)
(`graphql::sdl()` regenerates it). GraphQL mutations and subscriptions are out
of scope — streaming is wired over REST instead: `/chat` (below, the one
conversational surface) and the `/events` work feed
([events.md](events.md)).

- **`/chat`** enqueues an `OperatorMessage` event and streams the resulting
  cycle's channel responses over SSE. One conversational surface, one voice:
  the operator talks to the company, not to individual teammates.
- **`/chat` thread addressing is a load-bearing contract, not just routing.**
  The body's `chat` field names a desk; three behaviours follow from it, and
  the console's per-workflow copilot (issue #303) is built entirely on them,
  with no route of its own:
  1. An **unknown** thread id falls through to the orchestrator — the brain
     tries desk-lead, then roster agent, then its own responder.
  2. Replies are journaled against that thread, and the desk filter
     (`server::chat_history::owns`) matches the id **exactly**; the General
     catch-all applies only when General is the desk being *read*. So an
     addressed thread is isolated from the team's chat in both directions.
  3. `GET /chat/history?desk=<thread>` therefore replays exactly that thread.

- **A message has a durable id, and things can refer to it (issue #364).**
  `POST /chat` answers with `messageId` — the sequence position the operator's
  own message was journaled under — and stamps the same on each reply bubble.
  Two things name a message by that id:

  - The `parent` field on the `/chat` body makes the send a **thread reply**.
    It is journaled onto both the `OperatorMessage` and the replies it draws,
    so the whole exchange comes back under the same row on the next read. A
    `parent` that is not a message id is a `400`, never a silently-flattened
    thread — a reply that quietly lands in the channel reads as a lost reply.
  - `POST /chat/messages/{seq}/reactions` sets or clears **one person's** one
    reaction. `on` is explicit rather than a toggle, which is what makes a
    retry or a double tap idempotent. The target must be a chat message —
    anything else is a `404`, so the log can never hold a reaction no reader
    could render — and the emoji is bounded and refused if it carries control
    characters. Authorized through the same gate a send passes: reacting is
    writing into a transcript, so it can be neither easier nor harder than
    saying something in it.

  Both project through the shared `MessageView`, so REST and GraphQL cannot
  disagree about the shape of a thread or who reacted. Reactions are
  deliberately absent from the `/events` stream — see [events.md](events.md).

  The copilot addresses `workflow-copilot:<workflowId>` (a `:` cannot occur in
  a manifest desk id, so it can never collide with a real desk, and it does not
  appear in `GET …/desks`). Making unknown thread ids a `404`, or loosening
  `owns` to match on prefix, would break that surface — see
  [`frontend/src/api/workflow-copilot.ts`](../../../frontend/src/api/workflow-copilot.ts).

  **Thread addressing isolates transcripts. For every thread but one, it does
  not scope authority.** The thread id decides who answers and where the
  exchange is journaled; for an ordinary thread it does **not** narrow the
  responder's context or tool grants, which stay company-wide however the turn
  is addressed.

  **The copilot thread is the exception (issue #416).** A `chat` id matching
  `workflow-copilot:<workflowId>` ([`company::copilot`](../../../src/company/copilot.rs))
  makes the turn **confined**, host-side, in two places that hold independently:

  - the harness runs it on an ephemeral agent with **no tools, no company
    memory and no delegation** ([`harness::confine`](../../../src/harness/confine.rs)),
    and skips the retrieve→inject step and the memory writeback, so the turn
    answers from the message it was sent and leaves nothing behind. Every tool
    call is denied by the host with a reason, so an empty toolbelt is a
    boundary rather than an absence;
  - the `/chat` handler does not open a board card from a copilot message, so a
    question phrased as a request cannot leave work on the board. That half is
    in the default build, not behind `openhuman`.

  Confinement narrows one **turn**; it is not an authorization check and must
  not be read as one. `/chat` is already authenticated and company-scoped, so
  an operator addressing a workflow thread gains nothing they could not get by
  opening the Chat tab or calling the workflow routes directly. What the
  copilot adds is a transcript that stays out of the team's chat and an answer
  drawn from one workflow rather than from everything the company knows.

  **A copilot answer may carry a proposed edit (issue #415), and that adds no
  route and no capability.** The proposal is a fenced block in the reply text —
  a list of node/edge operations — which the *console* turns into a candidate
  graph and applies through `PUT …/workflows/{wid}` with `expectedVersion`, the
  same write the canvas editor performs, after the operator has read the diff
  and pressed Apply. The confined turn still calls nothing: it emits text, and
  a person decides. So the host needs no notion of a proposal, and a proposal
  cannot produce a graph the editor could not have produced — including the
  `409` a graph that moved underneath it earns.

  Two more consequences worth knowing before reusing the seam. A chat turn
  runs the **whole** company cycle, so every message is first classified by
  `company::task_intent::triage_message` (#267) into `Track` (an instruction —
  the route opens a `todo` card), `Answer` (a question or read — no card), or
  `Chatter` (greetings, and anything ambiguous — no card). `Answer` is also the
  only class that *gates*: the harness narrows the issue-#453 delegation claim
  to answering for that turn, so the model's own `spawn_task` / `assign_task` /
  `review_task` fail at the tool boundary with the do-not-retry refusal.
  `delegate_to_desk` is deliberately **not** refused — it is how a question the
  orchestrator cannot answer alone reaches a desk that can — so it runs the
  desk lead and relays their reply, and only its board card stands down.
  `query_company` / `run_workflow` / `read_run_output` run inline and are
  untouched throughout. The turn loses the ability to *write*, never the
  ability to answer. Ambiguity falls to `Chatter`, which neither cards nor gates: a
  missed card costs one follow-up message, a spurious card pollutes the board
  permanently. The gate is harness-only — `HostedMedullaBrain` has no
  delegation stack to gate (#176) — while the triage itself is compiled into
  every build and fronts both brains. The card half is suppressed wholesale on
  a copilot thread (#416), precisely because the seam is being reused for a
  conversation that is not a request to the company. And an
  unconfigured company answers
  `200` with the echo brain's `"You said: …"` rather than an error, so a caller
  that needs a real answer must check `cognition` from `GET {scope}/inference`
  — there is no status code to catch.
- **`/events`** is the work feed's backend: each frame is a plain-language
  rendering of an event or executed effect plus the raw payload for
  programmatic consumers. Resumable via `since` (event sequence number).
