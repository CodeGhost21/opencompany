---
name: Customer Follow-up
description: Close the loop with the customers who reported something, once it is fixed.
category: Support
---

# Customer Follow-up

A fix nobody told the reporter about is, from the reporter's side, not a fix.
This is the cheapest goodwill available to a support organization and the most
reliably skipped.

## When to use

- A `known-issues` row moves to fixed.
- An escalation resolves.
- Something a customer was promised has landed.

## Steps

1. **Read `waiting` on the issue** — that is the list, and it exists precisely
   so this step does not depend on anybody's memory.
2. **Check it is actually fixed** for them, not merely released. A fix behind a
   flag they are not on is not their fix.
3. **Write once, to each of them.** Not a broadcast; they reported it
   individually and a bulk mail reads as one.
4. **Say what changed, plainly,** and what they should do — reload, re-run,
   nothing.
5. **Acknowledge the wait** where it was long. One sentence, no elaboration.
6. **Record it.** Close the issue with who was told, and any `commitments` row
   this discharges.

## Output

A message per waiting customer, and the ledger rows closed with who was told.
Anything going to a large group is a person's call before it is sent.
