---
name: Incident Response
description: Drive a production incident from "something is wrong" to resolved, with customers told and a prevention filed.
category: Engineering
---

# Incident Response

Production is wrong. This is the order of operations, and it is deliberately not
the order that feels natural — the natural order is to diagnose first and tell
people afterwards, which is how an hour disappears with nobody outside the
incident knowing it exists.

## When to use

- Customers cannot do something they could do an hour ago.
- An alert fired and the cause is not immediately obvious.
- A support thread describes the same failure twice.

## Steps

1. **Open the row first.** `record_entry` on `incidents` with a severity, an
   owner and what you know — which may be almost nothing. Guess the severity
   high; downgrading costs nothing and starting low costs the response.
2. **Stop the bleeding before understanding it.** Roll back, disable the flag,
   or shed the load. A mitigation you can explain later beats a diagnosis you
   are still working on.
3. **Say something.** Update `told_customers` with what went out and when. "We
   are investigating an issue with exports" is a complete first message.
4. **Find the cause.** Read [[Release checklist]] and the `releases` ledger:
   most incidents start at a deploy, and the timeline answers that in seconds.
5. **Fix it, and record the fix** against the same row rather than a new one.
6. **File the prevention.** Open a task, put its id in `prevention`, and only
   then `close_entry` with what happened. A resolved incident with no
   prevention is a rehearsal for the same incident.

## Output

One `incidents` row carrying the whole story — impact, what customers were
told, cause, and the task that stops a repeat — plus whatever customer-facing
message actually went out. If the cause is genuinely unknown, `cause` says
`unknown` and the row stays open; a plausible guess written as fact is worse
than an empty field.
