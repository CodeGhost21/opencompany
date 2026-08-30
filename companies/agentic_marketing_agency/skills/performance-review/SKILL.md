---
name: Performance Review
description: Read a campaign's numbers against the measure it set, and recommend killing or continuing it.
category: Analytics
---

# Performance Review

Read the numbers honestly, against the measure that was fixed beforehand, and
say what should happen next.

## When to use

- A campaign ends.
- A live campaign has spent enough to be judged.
- A number moved and nobody knows why.

## Steps

1. **Read the `campaigns` row first**, specifically `measure`. Judge against
   that, not against whatever the numbers happen to be good at.
2. **Pull from the source.** Dashboards, not memory, not last week's report.
   Note the date range and the currency; both are how reports go quietly wrong.
3. **Compare like with like.** Against the measure, against the prior period,
   and against a comparable closed campaign if one exists.
4. **Separate signal from sample.** A 40% lift on 200 sessions is not a lift.
   Say so plainly rather than reporting it with a caveat nobody reads.
5. **Say what you cannot attribute.** Most of it, usually. A report that
   attributes everything is a report that invented something.
6. **Recommend.** Continue, change one thing, or kill — with the number that
   supports it. Then update `spent` and `result` on the row.

## Output

A short read: what the measure said, what moved, what cannot be attributed, and
one recommendation. If the answer is that the campaign is not working, that goes
in the first line, not the last.
