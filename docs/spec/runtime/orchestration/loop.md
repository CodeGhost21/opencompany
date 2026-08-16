# The attempt loop

*Phase P3. What happens when the first try is wrong.*

A [demand](demand-ledger.md) is worked by an explicit **attempt → evaluate →
route** cycle. There is no single-shot path: a hard piece of work's first
approach is usually wrong, and a single-shot path differs from a loop only by
throwing that information away.

Terms: [glossary](../../glossary.md).

---

## What exists today

The [Planning station](../planning.md) is one model call, no tools, no retry,
120 seconds hard, three exits. It is a good contract for what it does — and what
it does is decide, once, whether a card can start. Nothing in the runtime
re-approaches work that was attempted and came back wrong; a failed run returns
the card to `todo` carrying its error, and the next attempt begins from the same
standing start with the failure recorded only as prose on a card.

This section adds the missing half: the loop that reads that failure and routes
on it. It subsumes the `planning` column — one tool-less call becomes the first
attempt of something that can retry.

---

## The shape

```text
   ┌──────────────────────────────────────────────────┐
   │                                                  │
   ▼                                                  │
attempt ──┬──> judge ──────────┐                      │
          ├──> verify ─────────┤                      │
          ├──> critique ───────┼──> merge ──> route ──┤
          └──> completeness ───┘                │     │
                                                ├─ retry ──┘
                                                ├─ diversify
                                                ├─ blocked
                                                └─ answered
```

Everything after an attempt is a **fan-out, not a chain**. The arms read the
same attempt and none reads another's answer, so a cycle costs the slowest arm
rather than the sum of all of them.

### The arms MUST be graph nodes

They MUST NOT be spawned tasks hidden inside a step body. The sibling runtime
did exactly that and had to undo it: the graph could not draw them, graph policy
could not bound them, and no checkpoint could land between them. Three
consequences that all read as flakiness rather than as a design error.

Since our arms are `tinyflows` nodes, they inherit the existing run store, the
console's workflow visualisation, cancellation, and per-node retry for free.

---

## Four questions, one merge

This is the split that makes the loop terminate correctly, and the pair most
easily collapsed into a single "did it work" call is judge and verify.

| Arm | Asks | Can end the loop |
| --- | --- | --- |
| **verify** | Is the result *right*? | **yes — only this arm** |
| **judge** | Was the attempt *conducted* in a way the next one should inherit? | no |
| **critique** | What is wrong with, or missing from, the result as reasoning — independent of whether it is *right*? | no |
| **completeness** | Does the result actually answer the demand's `falsifies`, or only its topic? | no |

`critique` and `completeness` feed the same merge step as `judge` and
`verify`, and share their constraint: neither can itself route the loop to
`Answered`. They exist because "right" and "well-conducted" do not cover
every way an attempt can fail.

- **critique** asks a [critic-shaped](alignment.md#context-routing) question of
  the result itself: does it contradict a claim already on the [claim
  ledger](alignment.md#claimsmd--the-evidence-ledger), does it assert
  something as established that the workspace does not support, is the
  reasoning it gives sufficient for the conclusion it draws. Its output is a
  list of specific objections (possibly empty), each citable, that merge folds
  into the judge's `Steer` guidance for the next attempt — critique never
  produces its own `Verdict`; it feeds the one `judge` produces.
- **completeness** asks whether the attempt answers the [demand](demand-ledger.md)
  it was dispatched against, not merely its subject: it checks the result
  against the demand's `falsifies` field specifically, because a topically
  relevant result that does not touch what would show the current belief
  wrong is the "search, not a question" failure the demand ledger's shape
  exists to rule out. Its output folds into `Route`: a `verify`-passed result
  that `completeness` judges partial routes to `Diversify` rather than
  `Answered`, on the reasoning that a result cannot answer a demand it does
  not address, however verifiably true it is.

`verify` and `judge` remain the only arms whose output is itself a routable
value (`Verdict` for judge; verification's pass/fail is what unlocks `Answered`
in the [routing ladder](#routing)); `critique` and `completeness` are inputs
the merge step folds into those two before routing runs.

The judge returns `Proceed`, `Steer`, or `Restart`, and its one-sentence
guidance is carried into the next attempt's prompt. It scores conduct — did the
attempt actually execute and check its work, or produce confident prose with
nothing behind it.

Verification runs after **every** attempt, not only after failures. The lesson
from a partial success is what stops the next attempt repeating it.

### Judge the evidence, not the report

The judge MUST receive a briefing counted off the workspace, not only the
attempt's own report.

The ordinary way a long attempt ends is a timeout, which destroys its report and
its context and leaves every file it wrote. A judge given only the report is
then scoring silence. The sibling runtime recorded an evening in which three
live attempts died at exactly their deadline and every following verdict was
1/5 with "no progress" — one of them against a workspace holding both supplied
check values reproduced to ten digits and 38 points cross-validated two ways.

---

## Routing

`Verdict` and `Route` are wire values. Each MUST have an explicit
`as_str`/`parse` pair — `Debug` is not a wire format — and an unparsable value
MUST default to the conservative arm rather than erroring the run.

```rust
enum Verdict { Proceed, Steer, Restart }
enum Route   { Answered, Reported, Retry, Diversify, Blocked }
```

**The conservative default for each:**

- `Verdict::parse` defaults to **`Restart`**. An unparsable judge verdict means
  the loop has no read on whether the attempt was conducted well, so the safe
  assumption is the worse one: discard the direction and steer fresh, rather
  than silently defaulting to `Proceed` and inheriting an approach nobody
  actually endorsed.
- `Route::parse` defaults to **`Blocked`**. An unparsable route means the loop
  has no read on whether the work is done, so it MUST NOT guess `Answered` or
  `Reported` — either would risk closing a demand nothing verified — and it
  MUST NOT silently guess `Retry` or `Diversify` either, which would spend
  budget on a state nobody chose. `Blocked` is the one route that does
  neither: it halts and surfaces the parse failure rather than acting on a
  value the loop could not read.

Both defaults MUST be covered by the [parity sweep](#two-engines-one-policy)
alongside every reachable named state, so a translation bug that produces an
unrecognized wire value is caught by the same test that catches a routing
disagreement.

The ladder is ordered, and order is the policy:

1. `blocked` — a hard prerequisite is missing or a provider is refusing
2. `verify` passed and `completeness` is satisfied against the demand's
   `falsifies` → `Answered`
3. the attempt cap is reached, regardless of `verify`/`completeness` → `Reported`
4. verified but `completeness` judges the result partial → `Diversify`
5. unverified beyond threshold → `Diversify`
6. unproductive beyond threshold → `Diversify`
7. otherwise → `Retry`

The two outcomes step 2 and step 3 used to share a line are not the same
event and MUST NOT be produced by the same condition: `Answered` means
verification actually passed; `Reported` means the loop gave up at its cap
and is handing back whatever it has, verified or not. Collapsing them would
let a capped-out, unverified attempt read as `Answered` — exactly what
["only the verification arm can move a demand to `answered`"](#verification)
forbids. Step 3 is checked after step 2 so a result that both verifies and
lands on the same attempt as the cap still reads as `Answered`, not
`Reported`.

Step 4 is why `completeness` is a distinct arm from `verify` in the
[four-question fan-out](#four-questions-one-merge): a result can be verified
true and still not be what the demand asked for, and that case must not fall
through step 2's `Answered` or step 7's default `Retry` — retrying repeats an
approach that was never wrong, and answering closes a demand nothing actually
resolved.

`Diversify` means change approach rather than repeat it: re-open research, ask
for a different angle, or state a new [demand](demand-ledger.md) for what is
missing.

### Thresholds are a struct

All bounds live in one `Thresholds` value, defaulted per company
`[policy].mode`. Every routing decision reads the passed struct. There MUST NOT
be a second set of constants anywhere — a threshold that exists twice is a
threshold that will disagree with itself.

Diversification MUST trigger on **consecutive** unproductive attempts, so work
making thin but genuine progress never reaches it.

### Restart is not a route

A restart is what the judge *writes* — discard the direction, set the steer,
increment the counter, mark the attempt unproductive. Because the arms are
concurrent, by the time anything routes, verification has already happened and
there is nothing left for a restart to skip.

---

## Two engines, one policy

The loop runs on the `tinyflows` workflow engine, with the routing ladder
generated as jq and each step a `tool_call` node. The Rust ladder remains the
executable specification.

That means the policy exists twice, and the two copies MUST be proven equal.

> **A parity test is mandatory.** It MUST sweep *every* combination of the
> routed counters, with ranges derived from the thresholds under test, and
> assert both engines route every reachable state identically.

Two failures make this non-optional:

- The engine will happily run a graph whose every binding resolved to `null` and
  **report success**. A translation error in a routing ladder is silent. The
  suite MUST therefore also assert the jq produces real port names rather than
  `null`.
- Exhaustive beats sampled. The states that diverge are the boundary ones, and a
  sampled corpus is exactly what misses them.

---

## Human direction

An operator MUST be able to redirect a loop already in flight. That mechanism is
the directive queue, specified in
[delegation.md](delegation.md#operator-directives), and its rule here is:

- A directive reaches the **next attempt**, verbatim, above whatever the loop
  concluded on its own.
- The loop never waits for one.
- A directive MUST NOT force a restart, end the run, or make unverified work
  count as answered.

---

## Bounds

- A per-attempt budget, and a separate wall-clock ceiling for the whole loop.
  These are different things: the first bounds one attempt, the second stops a
  loop that is technically progressing and will not finish.
- An attempt cap, after which the loop stops and **returns what it has** rather
  than discarding it.
- Evaluation arms run on a narrowed budget — see
  [delegation.md](delegation.md#narrowing-budgets). A judge does not need a
  worker's allowance.

---

## Verification

- The parity sweep: the Rust ladder and the generated jq route every reachable
  state identically, with the corpus derived from the thresholds under test.
- The jq ladder yields real port names, never `null`.
- Every school of threshold values — including any non-default `[policy].mode` —
  is covered, and a new one is covered automatically rather than by being
  remembered.
- Only the verification arm can move a demand to `answered`.
- An attempt that times out with no report still produces a judge verdict
  informed by what is on disk.
- Diversification triggers on consecutive unproductive attempts, not cumulative.
- Reaching the attempt cap returns partial work rather than discarding it, and
  routes to `Reported`, never `Answered`, unless verification independently
  passed on the same attempt.
- A verified attempt whose `completeness` result is partial routes to
  `Diversify`, not `Answered` or `Retry`.
- An unparsable `Verdict` defaults to `Restart`, and an unparsable `Route`
  defaults to `Blocked` — neither fails the run.
