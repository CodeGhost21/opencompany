# Agentic Real Estate Company

> A real-estate operator of agents that finds properties, analyzes neighborhoods, underwrites deals, coordinates contractors, and manages tenants — humans approve purchases.

## What it can do

- Find and screen properties.
- Analyze neighborhoods and comps.
- Underwrite deals and model returns.
- Coordinate contractors and renovations.
- Manage tenants and operations.

## Agent roster

| Agent | Responsibility |
| --- | --- |
| Property Scout | Find and screen candidate properties. |
| Neighborhood Analyst | Analyze neighborhoods, comps, and trends. |
| Deal Underwriter | Underwrite deals and model returns. |
| Contractor Coordinator | Coordinate contractors and renovations. |
| Tenant Manager | Manage tenants and property operations. |

## Human in the loop

Humans keep **purchase approvals**; the agents run everything else. The output of this harness is **underwritten deals and managed properties**.

## Tool servers

Deal files, leases and tenant correspondence live in a shared workspace.

Declared in [`mcp.json`](mcp.json) and merged with anything the install
ships and anything an operator adds from the console. A server marked
*needs a token* is declared but off: write its credential from
Settings → Connections, then enable it there.

| Server | What it is for | Ships |
| --- | --- | --- |
| `notion` | The workspace this company's documents already live in. Needs a token. | off — needs a token |

## Run it

```sh
cargo run --bin opencompany -- serve --company companies/agentic_realestate_company
```
