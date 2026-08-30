# Agentic Game Business

> The business layer around a live game: user acquisition, monetization design, LiveOps events, community, store optimization, and player support.

## What it can do

- Run user-acquisition campaigns and track LTV/CAC.
- Design monetization, offers, and economy.
- Plan and run LiveOps events and content updates.
- Optimize store listings (ASO) and conversion.
- Support players and manage the community.

## Agent roster

| Agent | Responsibility |
| --- | --- |
| User Acquisition | Run paid and organic UA campaigns. |
| Monetization Designer | Design offers, pricing, and the in-game economy. |
| LiveOps Manager | Plan and run events and content updates. |
| Store Optimizer | App-store optimization and conversion. |
| Analytics Analyst | Track KPIs, LTV, retention, and cohorts. |
| Community Manager | Grow and moderate the player community. |
| Player Support | Resolve player issues and refunds. |

## Human in the loop

Humans keep **monetization and growth strategy**; the agents run everything else. The output of this harness is **liveops, user acquisition, and monetization for a live game**.

## Tool servers

Monetization reads the payment system directly; LiveOps calendars live in a workspace and economy changes are tracked as work.

Declared in [`mcp.json`](mcp.json) and merged with anything the install
ships and anything an operator adds from the console. A server marked
*needs a token* is declared but off: write its credential from
Settings → Connections, then enable it there.

| Server | What it is for | Ships |
| --- | --- | --- |
| `stripe` | Payments, subscriptions and invoices as the ledger of record. Needs a token. | off — needs a token |
| `notion` | The workspace this company's documents already live in. Needs a token. | off — needs a token |
| `linear` | Issues and cycles, when the work is tracked outside this board. Needs a token. | off — needs a token |

## Run it

```sh
cargo run --bin opencompany -- serve --company companies/agentic_game_business
```
