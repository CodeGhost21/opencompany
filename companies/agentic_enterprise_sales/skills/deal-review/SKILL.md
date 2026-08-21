---
name: Deal Review
description: Test a deal against what has actually been proved, and advance it or downgrade it honestly.
category: Sales
---

# Deal Review

A stage is a claim about what has been proved, not about how the last call felt.
This pass tests the claim.

## When to use

- A deal is about to be forecast.
- A close date has moved once. Do not wait for twice.
- A deal has been in one stage longer than the stage usually takes.

## Steps

1. **Name the economic buyer.** Not the champion. If nobody can name them, the
   deal is in discovery whatever the row says — this is the single commonest
   late-stage stall.
2. **Check what the champion actually has.** Budget, authority to convene, and a
   problem urgent enough to survive their quarter. A champion with none of the
   three is a friendly contact.
3. **Test the close date.** What has to happen between now and then, and who
   does each thing? A date with no sequence behind it is a guess with a calendar
   entry.
4. **Write what would kill it** honestly. If the answer is "nothing", the deal
   has not been qualified.
5. **Check `accounts.how_they_buy`.** Security review and procurement are what
   make enterprise deals close in quarters; a date that ignores them is wrong by
   a quarter.
6. **Advance or downgrade the row.** Downgrading is the point of this pass, and
   `no_decision` is a legitimate forecast outcome.

## Output

The `deals` row updated with the buyer, the tested date, and what would kill it
— or moved back a stage with the reason. A deal that survives this pass is
forecastable; one that does not was not, whatever it looked like yesterday.
