---
name: Accessibility Audit
description: Check a design against the requirements that are cheaper to meet than to retrofit.
category: Design
---

# Accessibility Audit

Not a final polish pass. Nearly everything here is a constraint on decisions
already made — colour, type scale, target size, motion — which is why it is
cheap now and expensive later.

## When to use

- Any design that ships to people. That is all of them.
- Before a design system is handed over, since it will replicate whatever it
  encodes.

## Steps

1. **Contrast first.** Text against its background, at its actual weight and
   size, including states — disabled, placeholder, and text over images.
2. **Check it without colour.** Anything that only colour distinguishes fails
   for a substantial number of people and is usually trivial to fix at this
   stage.
3. **Target size and spacing.** Anything interactive, on the smallest supported
   device, hit by a thumb rather than a cursor.
4. **Walk it by keyboard.** Order, focus visibility, and whether anything traps
   focus. If you cannot complete the task, neither can a screen-reader user.
5. **Check motion.** Anything that moves needs a reduced-motion answer, and
   parallax and autoplay are the two that reliably harm people.
6. **Record the constraints as `design-decisions`**, so the next revision does
   not undo them as arbitrary.

## Output

A list of failures with the specific requirement each one misses, and
`design-decisions` rows for the constraints that must survive the next
revision. Anything that cannot be met is a `risks` row, not a silent omission.
