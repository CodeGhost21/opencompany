---
name: Implementation Plan
description: Turn an accepted recommendation into work somebody inside the client could actually start on Monday.
category: Delivery
---

# Implementation Plan

The difference between advice that gets implemented and advice that does not is
almost never the quality of the advice.

## When to use

- A `recommendations` row reaches `accepted`.
- A client asks what doing this would actually involve.

## Steps

1. **Name who does it.** Not a department — a role, inside the client, with the
   authority to act. A plan whose owner is "the business" has no owner.
2. **Find the first step that fits in a week.** A plan whose first step takes a
   quarter will not be started.
3. **Sequence by dependency, not by importance.** The most important step is
   frequently the third one, and starting there is how implementations stall.
4. **Name what has to stop.** Every plan competes with what the client is
   already doing, and the one that does not say what to stop is the one that
   gets absorbed and disappears.
5. **State the decision points** — where the plan would be reconsidered, and on
   what evidence.
6. **Record it against the recommendation**, so `outcome` can eventually be
   filled in with what actually happened.

## Output

A sequenced plan with a named owner per step, a first step that fits in a week,
what has to stop, and the points at which it would be reconsidered. Anything
committing this firm to delivery goes on `commitments` and past the operator.
