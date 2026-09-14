---
name: Reconciliation
description: Tie an account out to its source, and raise what does not reconcile as an exception rather than a plug.
category: Bookkeeping
---

# Reconciliation

Tie the ledger to the thing outside it — the bank, the processor, the payroll
provider — and account for every difference.

## When to use

- A period is closing.
- An account balance is being relied on for anything.

## Steps

1. **Fix the scope.** Entity, account, period, currency. Assuming any of the
   four is how most reconciliation errors happen.
2. **Get the external source**, not a copy of it from another system. A
   statement, an export, a provider report.
3. **Match** what matches, then work the difference. Timing differences,
   uncleared items and fees are the usual three, in that order of frequency.
4. **Explain every remaining difference.** Not summarise — explain, with the
   transaction behind it.
5. **Never plug.** Anything still unexplained is an `exceptions` row with the
   amount and what it was traced to. `unknown` is a legitimate value; a rounding
   entry that makes the difference disappear is not.
6. **Record the account on the `closes` row** under `reconciled`, named rather
   than summarised.

## Output

A tied-out account, an `exceptions` row for every unexplained difference, and
the `closes` row updated. A period does not close over an open exception without
somebody saying so explicitly.
