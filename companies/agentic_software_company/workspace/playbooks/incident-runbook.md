# Incident runbook

The order of operations when production is wrong. It is deliberately not the
order that feels natural: diagnosing first and telling people afterwards is how
an hour disappears with nobody outside the incident knowing it exists.

1. **Open the row.** `incidents`, with a severity and an owner, before you
   understand anything. Guess the severity high.
2. **Mitigate.** Roll back, disable the flag, shed the load. Service first,
   explanation second.
3. **Say something.** One sentence about what is broken beats silence and beats
   a precise message an hour later.
4. **Correlate against `releases`.** Most incidents start at a deploy, and the
   release timeline answers that in seconds.
5. **Fix, then file the prevention** as a task, and put its id on the row.
6. **Close with the reason.** A resolved incident with no prevention is a
   rehearsal for the same incident.

One owner per incident — two owners is none. Security-shaped causes also get a
`security-findings` row; see [[Security standards]] and the
[[Release checklist]].
