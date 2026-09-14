---
name: Payroll Run
description: Prepare a payroll run and its remittances, with the checks that catch the errors people actually notice.
category: Payroll
---

# Payroll Run

Payroll is the one output where an error is noticed immediately, by somebody who
is directly affected by it, and remembered.

## When to use

- A pay period is due.
- Somebody joins, leaves, or changes rate, hours or status.

## Steps

1. **Reconcile the roster first.** Starters, leavers, and changed rates are
   where nearly every payroll error originates — not in the calculation.
2. **Check the period boundaries.** A leaver paid for a full period and a
   starter paid from the wrong date are the two commonest.
3. **Calculate,** then compare against last period line by line. Any movement
   that is not explained by a known change is an error until proved otherwise.
4. **Check the deductions and remittances** separately — the amounts owed to an
   authority have their own dates, and those dates go on `filings`.
5. **Park for approval.** Payroll leaves the company and moves money, so it
   waits for a person whatever the policy tier says.
6. **Record the remittances** as `filings` rows with their due dates the moment
   the run is prepared, not after it is paid.

## Output

A payroll run ready for approval, a variance note against the prior period
explaining every movement, and `filings` rows for each remittance with its
statutory date.
