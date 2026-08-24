# Agentic Venture Capital Firm

> Sources founders, evaluates opportunities, and supports portfolio companies — leaving the actual investment decision to human partners.

## What it can do

- Source founders and inbound deal flow.
- Evaluate pitch decks and analyze codebases and traction.
- Run reference checks and size markets.
- Draft investment memos for partner review.
- Provide hands-on support to portfolio companies.

## Agent roster

| Agent | Responsibility |
| --- | --- |
| Founder Sourcer | Source founders and deal flow. |
| Deck Evaluator | Evaluate pitch decks and business models. |
| Code Analyst | Analyze codebases, product, and technical traction. |
| Reference Checker | Run founder and customer reference checks. |
| Market Sizer | Size markets and model opportunity. |
| Portfolio Support | Support portfolio companies post-investment. |

## Human in the loop

Humans keep **investment decisions**; the agents run everything else. The output of this harness is **investment memos and a managed portfolio**.

## Tool servers

Diligence files and investment memos live in a shared workspace.

Declared in [`mcp.json`](mcp.json) and merged with anything the install
ships and anything an operator adds from the console. A server marked
*needs a token* is declared but off: write its credential from
Settings → Connections, then enable it there.

| Server | What it is for | Ships |
| --- | --- | --- |
| `notion` | The workspace this company's documents already live in. Needs a token. | off — needs a token |

## Run it

```sh
cargo run --bin opencompany -- serve --company companies/agentic_venture_capital
```
