---
name: Release Notes
description: Turn a release into notes a customer can act on, without leaking internals or overstating the change.
category: Go-to-Market
---

# Release Notes

Write what changed for the person using the product. Not what was merged, not
what the team is proud of.

## When to use

- A `releases` row moves to rolling out.
- A customer asks what is new, or what they are running.

## Steps

1. **Read the release row**, not the commit log. `contents` is already grouped
   the way customers experience the change; the commit log is grouped the way
   the work happened.
2. **Sort by who it affects.** Anything that changes existing behaviour comes
   first, then new capability, then fixes. Something a customer must *do* comes
   above all of it, labelled as such.
3. **Write each line as an outcome.** "Exports now include archived records" —
   not "fixed export filter predicate".
4. **Name the breaking changes plainly**, with what to do about each. A breaking
   change described gently is one somebody discovers in production.
5. **Check against the docs.** Anything described here that the docs still
   describe the old way is a doc task, opened now.
6. **Leave internals out.** Service names, ticket numbers and internal codenames
   mean nothing outside and sometimes mean too much.

## Output

Release notes ready to publish, ordered action-required → changed → new →
fixed, plus any doc tasks the pass turned up. Publishing is a person's call.
