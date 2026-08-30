---
name: Cohort Analysis
description: Find out what actually moved a metric, before anybody reacts to it.
category: Analytics
---

# Cohort Analysis

A metric that moved has a cause. The failure mode here is supplying a story
instead, and then acting on the story.

## When to use

- Retention, ARPDAU, conversion or install volume moves.
- A change shipped and somebody wants to know whether it worked.

## Steps

1. **Enumerate the candidates first.** Read `economy-changes` and `events` for
   the window. Five things usually shipped that fortnight; a story that names
   one of them without checking the others is a guess.
2. **Split by cohort, not by day.** Install cohort, spend tier, platform,
   geography. An average across new and paying players describes neither.
3. **Check whether the mix changed.** A metric moves when the population moves,
   and a UA campaign that brought in cheaper users moves everything at once
   without any product change at all. This is the single most common false
   alarm.
4. **Compare against the same cohort's prior period,** not against the
   aggregate.
5. **Check the instrumentation.** A change in a metric that coincides with a
   client release is a tracking question until ruled out.
6. **State the cause with its confidence** — and say when the honest answer is
   that several changes are confounded and nothing can be attributed.

## Output

A cause with the cohorts that show it, or an honest statement that the changes
are confounded. The second is a legitimate and common result; reporting a
confident cause instead is how a live game reacts to noise.
