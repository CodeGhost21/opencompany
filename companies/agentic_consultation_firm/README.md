# Agentic Consulting Firm

> A McKinsey-shaped firm of agents: research, analysis, modeling, and deck-building that produces strategy and implementation plans humans present in executive workshops.

## What it can do

- Run desk research and structured stakeholder interviews.
- Perform industry and competitive analysis.
- Build financial models and strategy recommendations.
- Produce client-ready decks and implementation roadmaps.

## Agent roster

| Agent | Responsibility |
| --- | --- |
| Researcher | Desk research and evidence gathering. |
| Interviewer | Conduct and synthesize stakeholder interviews. |
| Industry Analyst | Industry, market, and competitive analysis. |
| Strategist | Synthesize findings into strategy recommendations. |
| Financial Modeler | Build the supporting financial models. |
| Deck Builder | Produce client-ready presentations. |
| Implementation Planner | Turn strategy into an execution roadmap. |

## Human in the loop

Humans keep **executive workshops**; the agents run everything else. The output of this harness is **strategy decks and implementation plans**.

## Tool servers

Engagement documents and client deliverables live in a shared workspace.

Declared in [`mcp.json`](mcp.json) and merged with anything the install
ships and anything an operator adds from the console. A server marked
*needs a token* is declared but off: write its credential from
Settings → Connections, then enable it there.

| Server | What it is for | Ships |
| --- | --- | --- |
| `notion` | The workspace this company's documents already live in. Needs a token. | off — needs a token |

## Run it

```sh
cargo run --bin opencompany -- serve --company companies/agentic_consultation_firm
```
