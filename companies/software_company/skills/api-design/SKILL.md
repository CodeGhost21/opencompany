---
name: API Design
description: Review or design a public interface against the things that cannot be changed later.
category: Engineering
---

# API Design

A public interface is a promise with no expiry. This pass is about the parts
that are expensive to undo, not the parts that are easy to argue about.

## When to use

- A new endpoint, event, webhook or SDK method is being added.
- An existing one is changing shape in any way a client could notice.

## Steps

1. **Name what cannot change**: the resource names, the identifiers, the
   pagination shape, the error contract. Everything else can be added to later.
2. **Check it against what exists.** An API that is internally consistent and
   locally sensible is worse than one that is consistent with its neighbours.
3. **Model the failures, not just the success.** What does a client do on a
   partial write, a rate limit, a retry of something that already landed? An
   idempotency story decided later is a story clients already got wrong.
4. **Ask what a client does on the next version.** If the answer is "we version
   the whole API", the design is not finished.
5. **Write the example first** — the smallest complete request and response a
   developer could copy. An interface whose example is hard to write is hard to
   use.
6. **Record the call.** `record_entry` on `decisions` with what was chosen and
   the alternatives rejected; interface shape is exactly what gets re-litigated
   six months on.

## Output

The interface, its example request and response, its error contract, and a
`decisions` row naming what was rejected and why. Breaking anything already
public is a person's call, not this pass's.
