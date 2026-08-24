# Agentic Influencer Business

> Operates a creator brand around the clock — scripting, editing, thumbnails, posting, analytics, community, and sponsorships — with the human appearing occasionally or via an avatar.

## What it can do

- Detect trends and script content.
- Edit video and generate thumbnails.
- Publish on a schedule and analyze performance.
- Manage community and pursue sponsorships.

## Agent roster

| Agent | Responsibility |
| --- | --- |
| Scriptwriter | Write video and post scripts. |
| Trend Scout | Detect trends and content opportunities. |
| Video Editor | Edit video content. |
| Thumbnail Designer | Generate thumbnails and cover art. |
| Publisher | Schedule and post content. |
| Analytics Analyst | Analyze performance and advise. |
| Community Manager | Engage and moderate the community. |
| Sponsorship Outreach | Source and negotiate sponsorships. |

## Human in the loop

Humans keep **occasional appearance or ai avatar**; the agents run everything else. The output of this harness is **a creator brand that never sleeps**.

## Tool servers

The content calendar and the sponsorship terms live in a shared workspace.

Declared in [`mcp.json`](mcp.json) and merged with anything the install
ships and anything an operator adds from the console. A server marked
*needs a token* is declared but off: write its credential from
Settings → Connections, then enable it there.

| Server | What it is for | Ships |
| --- | --- | --- |
| `notion` | The workspace this company's documents already live in. Needs a token. | off — needs a token |

## Run it

```sh
cargo run --bin opencompany -- serve --company companies/agentic_influencer_business
```
