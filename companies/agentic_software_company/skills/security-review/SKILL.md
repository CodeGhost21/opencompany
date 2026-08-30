---
name: Security Review
description: Review a change for what it lets an attacker do, and file what it finds on the security-findings ledger.
category: Engineering
---

# Security Review

Not a checklist pass. The question is what somebody could do that they should
not be able to do, and the answer is usually about identity and boundaries
rather than about the code being reviewed.

## When to use

- Auth, sessions, permissions, payments, file upload, or anything that keeps one
  customer's data away from another's.
- A dependency with a published advisory.
- Anything that takes input from outside and does something consequential with
  it.

## Steps

1. **Name the trust boundary** the change crosses. Data moving from
   untrusted to trusted is where nearly everything lives.
2. **Ask who the caller is, and how you know.** Authentication answers who;
   authorization answers whether they may. A change that only checks the first
   is the most common real finding.
3. **Follow the identifier.** Anything a client supplies that names a record —
   an id, a path, a filename, a tenant slug — is an access-control question
   until proved otherwise.
4. **Check the failure path.** Errors that leak, retries that duplicate a
   charge, timeouts that fall open rather than closed.
5. **File what you find.** `record_entry` on `security-findings` with the
   severity judged on what it lets somebody *do*, not on how hard it was to
   spot, and the `exploitability` stating what an attacker needs first.
6. **Close with reasoning.** `not_exploitable` is a legitimate close and it must
   carry the argument. Silence here is what makes the same finding cost the
   company twice.

## Output

A verdict on the change, plus a `security-findings` row for anything real.
Nothing about a live, unfixed finding goes anywhere public — the audience for
an unfixed finding is this roster and the operator, and nobody else.
