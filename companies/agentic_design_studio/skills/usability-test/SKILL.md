---
name: Usability Test
description: Find out whether people can actually use it, and record what they did rather than what they said.
category: Research
---

# Usability Test

Watch people try to do the thing. The output is behaviour, not opinion — what
somebody says about a design is weakly related to whether they can use it.

## When to use

- A design rests on an assumption about how people behave.
- Two directions disagree and the argument is about taste.
- Anything with a task in it is about to ship.

## Steps

1. **Write the task, not the question.** "Change your billing address" — never
   "what do you think of this screen".
2. **Recruit for the behaviour,** not the demographic. People who have actually
   needed to do this.
3. **Say nothing while they work.** The single hardest step, and every prompt
   you give destroys the finding you were about to get.
4. **Record what they did** — where they hesitated, what they clicked first,
   where they gave up. Time and clicks matter less than the moment of
   hesitation.
5. **Separate usability from preference.** Five people is a real sample for "can
   they do it" and no sample at all for "which do they prefer". Say which you
   ran.
6. **File it.** `record_entry` on `research-findings` with the method, the
   sample and the implication. A finding with no implication is trivia.

## Output

`research-findings` rows stating what people did, with the sample beside each.
Findings that contradict the current direction go in first, and they are the
reason this pass exists.
