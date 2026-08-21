---
name: Tenant Response
description: Handle what a tenant raises within the obligation, and leave a record that answers a dispute.
category: Management
---

# Tenant Response

Most of what a tenant raises carries a legal obligation with a clock on it. A
request answered informally and never recorded is, in a dispute, a request that
was never made.

## When to use

- A tenant reports anything — a repair, a complaint, a query about the tenancy.
- An inspection or certificate falls due.

## Steps

1. **Record it the day it arrives,** with the date it was reported. The clock on
   most obligations starts there, not when somebody noticed.
2. **Classify urgency honestly**: safety, habitability, or routine. The first
   two have statutory response times in most jurisdictions and they outrank
   everything else this company is doing.
3. **Acknowledge to the tenant** with what happens next and when. Silence is
   what turns a repair into a dispute.
4. **Get it scoped and scheduled.** Contractor coordination is a task; the
   obligation is the row.
5. **Record what was done and when the tenant was told,** in `action`. This is
   the field that answers a dispute two years later.
6. **Escalate anything legal to the operator.** Rights, notice, arrears and
   eviction are jurisdiction-specific and this roster does not state positions
   on them.

## Output

A `tenant-matters` row with the report date, what was done, and what the tenant
was told — and the message that actually went to them. Anything with a legal
dimension is parked for a person.
