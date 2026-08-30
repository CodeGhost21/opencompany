---
name: Financial Model
description: Build a model whose assumptions are enumerable and whose conclusion can be tested against them.
category: Analysis
---

# Financial Model

Build the model so that somebody can disagree with it precisely, rather than
generally. A model that cannot be disagreed with in detail will be dismissed
wholesale.

## When to use

- A recommendation depends on numbers.
- A client asks what something is worth, costs, or returns.

## Steps

1. **Separate inputs from calculation.** Every number that came from outside is
   an input, in one place, each with its source.
2. **Record every input as an `assumptions` row** — the source, and honestly
   `estimate` where it is one. This is not paperwork; it is what lets a
   falsified input be traced to everything that depended on it.
3. **Build the base case only.** Optimistic and pessimistic cases built first
   are decoration; they are meaningful only as sensitivities off a base.
4. **Run the sensitivities that matter.** Vary each input and record which ones
   actually move the conclusion in `sensitivity`. Usually two do, and the deck
   spends its time on the other nine.
5. **Find the break-even.** What would have to be true for this to be a bad
   idea? That single sentence is worth more than the model's output.
6. **Sanity-check against something external** — a comparable, a market size, a
   published benchmark. A model that agrees with nothing outside itself is a
   spreadsheet.

## Output

A model with its inputs named and sourced, `assumptions` rows for each, the two
or three sensitivities that actually matter, and the break-even case stated in a
sentence.
