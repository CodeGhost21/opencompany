---
name: Vertical Slice
description: Build the smallest complete piece of the real game, so what is proved is the game rather than the pitch.
category: Production
---

# Vertical Slice

One part of the game, at shipping quality, running in the real build. The point
is that it cannot lie: a slice proves the game exists in a way no document or
trailer can.

## When to use

- The studio needs to show the game to somebody who will decide something.
- A design is being argued about in the abstract and has been for a while.

## Steps

1. **Pick the piece that is most likely to be wrong,** not the one that is
   easiest to finish. A slice built from the safe parts proves the safe parts.
2. **Take it to shipping quality.** Art, audio, feel, and the failure states. A
   slice with placeholder feedback is a prototype with a longer schedule.
3. **Use the real build.** A bespoke demo scene proves the demo scene, and every
   integration problem it avoids is one that arrives later at full price.
4. **Check `features.depends_on`.** Slices are where seams between systems
   surface, which is most of their value.
5. **Play it with people who have not seen it,** and record what they did on
   `playtests` with the build named.
6. **Update `features.playable`** with what the slice actually shows, honestly —
   this is the moment the feature list gets corrected.

## Output

A playable slice in the real build, `playtests` rows from people outside the
studio, and a corrected feature list. Anything shown outside the studio is the
operator's call.
