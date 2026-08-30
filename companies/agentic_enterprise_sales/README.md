# Agentic Enterprise Sales Organization

> A sales org of agents that generates leads, personalizes outreach, keeps the CRM clean, writes proposals and contracts, and follows up — humans close strategic accounts.

## What it can do

- Generate and qualify leads.
- Run personalized multi-touch outreach.
- Keep CRM records current.
- Write proposals and generate contracts.
- Follow up and nurture pipeline.

## Agent roster

| Agent | Responsibility |
| --- | --- |
| Lead Generation | Generate and qualify leads. |
| Outreach Personalizer | Craft personalized outreach at scale. |
| CRM Updater | Keep CRM records accurate and current. |
| Proposal Writer | Write tailored proposals. |
| Contract Generator | Generate contracts from templates. |
| Follow-up Agent | Nurture and follow up on pipeline. |

## Human in the loop

Humans keep **closing strategic accounts**; the agents run everything else. The output of this harness is **qualified pipeline and proposals**.

## Tool servers

Account plans and proposals live in a shared workspace; what a customer actually pays for is the billing system's answer, not the CRM's.

Declared in [`mcp.json`](mcp.json) and merged with anything the install
ships and anything an operator adds from the console. A server marked
*needs a token* is declared but off: write its credential from
Settings → Connections, then enable it there.

| Server | What it is for | Ships |
| --- | --- | --- |
| `notion` | The workspace this company's documents already live in. Needs a token. | off — needs a token |
| `stripe` | Payments, subscriptions and invoices as the ledger of record. Needs a token. | off — needs a token |

## Run it

```sh
cargo run --bin opencompany -- serve --company companies/agentic_enterprise_sales
```
