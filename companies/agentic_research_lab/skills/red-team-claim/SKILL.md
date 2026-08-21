---
name: Red-Team a Claim
description: Attack a finding before the operator sees it, and record what survived.
category: Research
---

# Red-Team a Claim

Try to break the lab's own conclusion. An unattacked finding is not yet a
finding, and the point of this pass is to fail — the ones that survive a real
attempt are the ones worth reporting.

## When to use

- A `findings` row is about to move to `defended`.
- A conclusion feels obviously right. That feeling is the trigger, not the
  exemption.

## Steps

1. **Attack the evidence first.** Is the source primary? Was it read? Does the
   passage cited actually say this, or a narrower version of it?
2. **Attack the inference.** Does the evidence support the claim, or something
   adjacent that sounds the same? Correlation stated as mechanism is the
   commonest one here.
3. **Look for the missing arm.** What would this look like if it were false, and
   has anybody gone to find that?
4. **Check the sample and the scope.** How far does this generalise, and where
   has it silently been generalised past?
5. **Try to construct a counter-example.** Compute one if the analyst can.
6. **Record the attempt** in `attacked_by` — what was tried, what survived, and
   what did not. Where the finding does not survive, move it to `refuted` with
   the reason, which is worth more to the lab than a missing row.

## Output

The `findings` row updated with what was tried and what held, moved to
`defended` or `refuted`. "I could not find a problem" is a legitimate result and
must say what was actually attempted, or it is indistinguishable from not having
looked.
