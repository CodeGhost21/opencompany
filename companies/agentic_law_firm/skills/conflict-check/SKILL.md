---
name: Conflict Check
description: Establish whether this firm may act at all, before any substantive work on a new matter.
category: Ops
---

# Conflict Check

The first thing that happens on any new matter, and the one place in this
company where the correct outcome may be to stop entirely.

## When to use

- A prospective client asks for anything.
- An existing client brings a new matter.
- A matter's parties change — a new defendant, an acquired entity, a joined
  third party.

## Steps

1. **Name every party**, not just the client: counterparties, affiliates,
   parents and subsidiaries, and the individuals behind them where the firm
   would recognise them.
2. **Search the `matters` ledger** for each name, including closed and declined
   rows. A declined matter is kept precisely because its reason is usually a
   conflict.
3. **Check adversity in both directions.** Acting *against* a current client is
   the obvious one; acting for a party whose interests diverge from an existing
   client's on the same question is the one that gets missed.
4. **Check information, not just parties.** Confidential knowledge from a past
   matter can disqualify the firm even where nobody is adverse today.
5. **Record the result on the matter row** — `conflicts`, with what was searched
   and when. `none found` and `not run` are different answers and only one of
   them unblocks work.
6. **Escalate anything unclear to the operator.** A close call is a licensed
   human's call, always.

## Output

The `conflicts` field on the matter, stating what was searched, what was found,
and the date. Where a conflict exists, a `declined` matter row with the reason
recorded — which is worth more to the firm than the matter would have been.
