---
name: Compound Triage
description: Narrow a hit list to the compounds worth a person's bench time, and say why the rest were dropped.
category: Discovery
---

# Compound Triage

A screen produces more hits than anybody can follow. The value of this pass is
in what it removes and in the reasons it records for removing them.

## When to use

- A screen or virtual screen returns a hit list.
- A series has to be narrowed before it goes to the bench.

## Steps

1. **Read `experiments` for what has been tried.** Re-testing a compound already
   found to be an assay artefact is the commonest waste in discovery and it is
   entirely preventable.
2. **Remove the known artefacts first** — aggregators, reactive groups,
   assay-interference chemotypes. Removed with the reason, not silently.
3. **Check the properties that decide developability,** not just potency. A
   potent compound with no viable route to exposure is a paper, not a candidate.
4. **Look for series, not singletons.** A structure-activity relationship across
   several related compounds is evidence; a single potent hit is a lottery
   ticket.
5. **State what would confirm each survivor,** and what a human would have to do
   at the bench to get it.
6. **Record the drops.** Every removed compound gets a reason on `experiments`
   or the program row, because the next screen will surface it again.

## Output

A short ranked list with what would confirm each one, and an explicit record of
what was dropped and why. Nothing here asserts activity in a person; that needs
wet results and the people qualified to read them.
