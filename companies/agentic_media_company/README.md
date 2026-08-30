# Agentic Media Company

> A BuzzFeed-shaped newsroom of agents that finds stories, verifies sources, writes, illustrates, translates, and distributes — under human editorial standards.

## What it can do

- Find and pitch stories.
- Verify sources and fact-check.
- Write, illustrate, and translate articles.
- Optimize for search and publish.
- Distribute across social channels.

## Agent roster

| Agent | Responsibility |
| --- | --- |
| Story Scout | Find and pitch story ideas. |
| Source Verifier | Verify sources and fact-check. |
| Writer | Write and edit articles. |
| Illustrator | Create article illustrations. |
| Translator | Localize stories across languages. |
| SEO Optimizer | Optimize articles for search. |
| Publisher | Publish to the CMS. |
| Social Distributor | Distribute across social channels. |

## Human in the loop

Humans keep **editorial standards**; the agents run everything else. The output of this harness is **published, distributed stories**.

## Tool servers

Drafts, source notes and the corrections log live in a shared workspace.

Declared in [`mcp.json`](mcp.json) and merged with anything the install
ships and anything an operator adds from the console. A server marked
*needs a token* is declared but off: write its credential from
Settings → Connections, then enable it there.

| Server | What it is for | Ships |
| --- | --- | --- |
| `notion` | The workspace this company's documents already live in. Needs a token. | off — needs a token |

## Run it

```sh
cargo run --bin opencompany -- serve --company companies/agentic_media_company
```
