# Agentic Venture Studio

> A studio that conceives, builds, launches, and operates a portfolio of startups — with humans holding only capital and major strategy.

## What it can do

- Continuously scout and score market opportunities.
- Stand up a new startup: thesis, product spec, MVP, brand, and go-to-market.
- Staff each venture with the functional agents it needs.
- Operate live companies — ship features, run growth, handle support, stay compliant.
- Roll portfolio performance up to the human capital allocators.

## Agent roster

| Agent | Responsibility |
| --- | --- |
| Opportunity Scout | Surface and score market opportunities and startup theses. |
| Founder | Turn a thesis into a company: vision, roadmap, and priorities. |
| Engineer | Build and ship the product. |
| Designer | Own product and brand design. |
| Marketer | Positioning, demand generation, and launches. |
| Lawyer | Incorporation, contracts, and compliance. |
| Finance | Financial modeling, runway, and reporting. |
| Customer Support | Resolve customer issues and feed insight back to product. |
| Recruiter | Source and staff each venture with agents or humans. |

## Human in the loop

Humans keep **capital allocation and major strategic decisions**; the agents run everything else. The output of this harness is **a portfolio of startups**.

## Tool servers

Shared assets and venture documents live in a workspace; each venture's build is tracked where its team already works.

Declared in [`mcp.json`](mcp.json) and merged with anything the install
ships and anything an operator adds from the console. A server marked
*needs a token* is declared but off: write its credential from
Settings → Connections, then enable it there.

| Server | What it is for | Ships |
| --- | --- | --- |
| `notion` | The workspace this company's documents already live in. Needs a token. | off — needs a token |
| `linear` | Issues and cycles, when the work is tracked outside this board. Needs a token. | off — needs a token |

## Run it

```sh
cargo run --bin opencompany -- serve --company companies/agentic_venture_studio
```
