---
name: Escalation Handoff
description: Hand a problem support cannot fix to somebody who can, without the customer paying for the handoff.
category: Support
---

# Escalation Handoff

An escalation is a customer being disappointed once and trusting you anyway. The
failure they remember is not that it took time; it is the silence afterwards.

## When to use

- Support has tried what it can and the customer still has the problem.
- A ticket needs authority support does not hold.

## Steps

1. **Check `escalations` and `known-issues` first.** The same problem escalated
   twice is usually one nobody closed the loop on, and the second escalation
   will land on somebody who already answered it.
2. **Write what was tried.** An escalation with no attempt behind it is a queue
   transfer, and it will be sent back.
3. **State the customer's actual problem,** not the internal diagnosis. The
   person picking it up needs to be able to check whether the diagnosis is even
   right.
4. **Name who holds it now** — a person or a desk, not "engineering".
5. **Tell the customer,** in the same session: that it has moved, who has it,
   and when they will next hear something. Only give a date you actually hold;
   if you give one, it goes on `commitments`.
6. **Set a check-back.** A stalled escalation is the one place customers are
   actively being let down, and nothing surfaces it except somebody looking.

## Output

An `escalations` row with what was tried, who holds it, and what the customer
was told — plus the message that actually went to the customer.
