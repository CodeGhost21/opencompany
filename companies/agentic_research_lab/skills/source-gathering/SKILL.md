---
name: Source Gathering
description: Find and retrieve primary sources for a line of inquiry, and log them without reading them.
category: Research
---

# Source Gathering

Retrieve the evidence. Do not interpret it — that is a different agent's job,
and the separation is what stops a search-result snippet being reported as a
source that was read.

## When to use

- A `questions` row needs evidence it does not have.
- A finding rests on something nobody has actually retrieved.

## Steps

1. **Read `sources` first.** The commonest waste here is fetching what is
   already downloaded, and the second commonest is re-fetching something already
   discarded as unusable.
2. **Search to discover, fetch to retrieve.** `web_search` finds a locator;
   `web_fetch` retrieves what is behind it. Nothing in the `web` namespace can
   find a URL on its own, and a snippet is not a source.
3. **Prefer the original.** A paper over a press release about the paper, a
   filing over an article about the filing, a dataset over a chart drawn from
   it.
4. **Record each one** with `record_entry` on `sources`: the locator, and
   `kind` as primary, secondary or commentary. Be honest about `kind`; it is
   what most evidential disagreements turn out to be about.
5. **Note the interest.** Who produced this and what they gain from it, in a
   sentence. Not a score.
6. **Stop at gathered.** Do not summarize what you retrieved. `establishes` is
   filled in by whoever reads it.

## Output

`sources` rows in `gathered`, each with a locator somebody else can re-open, and
nothing said about what they mean.
