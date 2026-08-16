# Delegation, direction, and the end of desks

*Phase P4. Waiting for delegated work, steering a run in flight, and collapsing
desks into workflows.*

Terms: [glossary](../../glossary.md).

---

## The join primitive

### The gap

Delegation today is fire-and-forget. `spawn_task` and `delegate_to_desk` push
onto a `DelegationQueue` that is drained **after** the parent turn by the
`DelegationRunner`. There is no handle a caller can hold, no future to block on,
and **no way for a turn to wait for work it asked for**.

That single absence is why several other things are shaped the way they are:
`delegate_to_desk` has to run its child's turn *synchronously inside* the
delegating turn to return a reply at all, and the workflow runner has to refuse
`delegate_to_desk` outright because "a run has nowhere to land a synchronous
reply".

### What ships

- **`spawn_task` returns an id immediately.** A spawn is not a wait.
- **`await_task` / `await_tasks`** join over a set, concurrently, so a batch
  costs the slowest child rather than the sum. Omitting the id list waits on
  everything outstanding.
- **`peek_task`** reads status without blocking.

### Rules that are not optional

**A caller MUST be able to outwait its child's full budget.** The await ceiling
is derived from the child's run budget, not set independently. An await that
expires before the work it is waiting for can finish is a timeout that reads as
a failure.

**A permit is held for a child's entire life, including while that child awaits
its own children.** It follows that the concurrency semaphore MUST have headroom
well above the maximum fan-out depth. That headroom *is* the deadlock argument —
if every permit can be held by a parent waiting on a child that cannot get a
permit, the system stops. Size it accordingly and say so where it is set.

**A batch validates every brief before launching any of them.** A half-launched
batch leaves children running for a call that returned an error.

**Depth and cycles are enforced at the tool boundary**, dynamically, against the
live scope chain — not by which tools were wired. Belts are cached per roster,
so depth cannot be a property of the belt.

---

## Operator directives

### The gap

A run in flight is closed to its operator. Prompts are assembled once, budgets
are read at launch, and editing a document mid-run changes nothing because it
was already read into every system message that will ever be sent. Someone
watching a run take a wrong turn can only keep watching.

OpenCompany has the *enforcement* half — a stop hook checked between tool-loop
iterations, and pause/cancel/redirect actions. It does not have the *operator*
half: a durable place to put an instruction that a running loop will pick up.

### The queue

An append-only JSONL queue with a separate cursor, and the writer split is the
whole design:

- The **host only appends** to the queue.
- The **runtime only writes** the cursor.
- Neither side writes what the other writes, so **neither needs a lock**, and
  the one number they share is owned by the side that advances it.

**A directive's id is its line number.** Not a stored field. So a line the
reader cannot parse is skipped **and still counted** — a torn append costs one
directive rather than the alignment of every later one.

The cursor MUST be written staged-and-renamed, so a torn cursor reads as zero
rather than as a garbage offset.

### Delivery

- Drained **once**, by a single consumer, which is what preserves the cursor
  guarantee. It is then posted to every interested party's mailbox.
- Delivered **verbatim** into the next attempt, above whatever the loop
  concluded on its own, and labelled as coming from the operator.
- **Nothing waits for it.** A directive reaches the work in seconds to minutes,
  and the run keeps going whether or not anyone is watching. A loop that blocked
  on a human would be the slow-participant failure with no ceiling at all.
- A **receipt is written whatever happened**. An operator who sees nothing
  cannot tell a directive still queued from one picked up and lost.

### What a directive cannot do

A directive is **asserted, not established**. It MUST NOT be filed as a claim,
and the role acting on one MUST NOT be routed the
[claim ledger](alignment.md#claimsmd--the-evidence-ledger).

It MUST NOT force a restart, end a run, or make unverified work count as
answered. Redirecting work and fabricating a result are different powers.

---

## Narrowing budgets

Budget constructors that can **only ever narrow**: a judging budget, a
housekeeping budget. A curator or a judge does not need a worker's allowance,
and a narrowed budget that could widen is not a bound.

These are constructors on the existing capability budget, not a second budget
system.

---

## Desks are workflows

### The claim

A desk is `{id, name, description, members}` plus three overlay types. Its
entire runtime behaviour is: resolve the desk, take the first member who is a
real roster teammate as the lead, run that member's turn, relay the reply.

A workflow `agent` node already runs the same harness turn with the same
toolbelt, the same persona, model, memory, approval policy and metering — and
adds retries, `on_error` routing, `requires_approval` gating, conditions and
switches, `sub_workflow` nesting, cron scheduling, cancellation, and supervised
metered runs.

The recursive-delegation feature is, in effect, a dynamic depth-capped
cycle-checked graph. A workflow is the static, declarative, inspectable version
of the same thing:

| Delegation concept | Workflow equivalent |
| --- | --- |
| `MemberScope` — which desks a member may hand to | the allowed sub-graph |
| `max_delegation_depth` | `sub_workflow` nesting depth |
| scope-chain cycle rejection | the parser's self-reference rejection |
| desk lead | the entry node |

So the desk is not a weaker workflow. It is a workflow with one node, no error
handling, and a bespoke resolver.

### Why it lands in P4

**The synchronous relay.** `delegate_to_desk` returns a reply into the caller's
turn. Workflow runs are supervised, journalled, asynchronous and cancellable,
which is exactly why the runner refuses `delegate_to_desk` from inside a run.

[`await_task`](#the-join-primitive) is the missing landing place. Once a turn
can wait for a run, a desk hand-off becomes "start the sub-workflow, await it,
relay its result" — with the relay preserved rather than dropped.

### The real obstacle: a desk id means four things

A desk id is simultaneously:

1. a **chat thread id** (with several legacy spellings for the default desk),
2. a **channel adapter id**, one adapter per desk,
3. a valid **assignee** value, resolved alongside roster teammates,
4. a workflow **output destination**.

Collapsing desks therefore means workflows become addressable as chat threads
and as assignees, or those four consumers each get a migration. That — plus
roughly 133 desk references in the operator surface alone, six REST routes,
three overlay types with a legacy migration path, and a desk-completion event —
is the actual cost. The delegation semantics are the easy part.

### Sequence

1. Ship a `desk` workflow template: one entry `agent` node, members as the
   allowed sub-graph.
2. Make `delegate_to_desk` a thin alias over `run_workflow` + `await_task`,
   preserving the relay. Behaviour is unchanged from the caller's side.
3. Migrate the four id consumers, one at a time, each behind its own change.
4. Remove `GroupChat` from the manifest and the operator surface.

Steps 1 and 2 are reversible and observable; step 3 is where the risk is; step 4
is bookkeeping.

### Migration of shipped companies

Four shipped companies declare desks. Their `[[group_chat]]` blocks become
workflow files with an entry node per lead and the members as the permitted
sub-graph. A company that declares no desks is unaffected.

---

## Verification

- An await outlives its child's full budget.
- A batch validates every brief before launching any; a rejected brief launches
  nothing.
- The concurrency semaphore's headroom exceeds maximum fan-out depth, asserted
  where it is configured.
- A parent awaiting a child that itself delegates does not deadlock.
- Depth and cycle rejection are enforced against the live scope chain, not the
  wired belt.
- A torn line in the directive queue costs exactly one directive; every later
  directive still lands.
- A torn cursor reads as zero, never as a garbage offset.
- A receipt is written even when the consuming step failed.
- A directive cannot mark unverified work answered, force a restart, or end a
  run.
- The role acting on a directive is not routed the claim ledger.
- A desk-as-workflow hand-off returns the same relayed reply the desk did, for
  each of the four shipped companies that declare desks.
