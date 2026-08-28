# Agentic Accounting Firm

> An accounting firm of agents that keeps books, prepares taxes and payroll, forecasts, and readies audits — a human signs off on filings.

## What it can do

- Keep the books and reconcile accounts.
- Prepare taxes.
- Run payroll.
- Build financial forecasts.
- Prepare for audits.

## Agent roster

| Agent | Responsibility |
| --- | --- |
| Bookkeeper | Record transactions and reconcile accounts. |
| Tax Preparer | Prepare tax filings. |
| Payroll Agent | Run payroll and related filings. |
| Forecaster | Build financial forecasts and budgets. |
| Audit Preparer | Assemble documentation for audits. |

## Human in the loop

Humans keep **sign-off on filings**; the agents run everything else. The output of this harness is **books, taxes, payroll, and forecasts**.

## Tool servers

The books close against the payment processor that is the ledger of record, and client documents arrive through a shared workspace.

Declared in [`mcp.json`](mcp.json) and merged with anything the install
ships and anything an operator adds from the console. A server marked
*needs a token* is declared but off: write its credential from
Settings → Connections, then enable it there.

| Server | What it is for | Ships |
| --- | --- | --- |
| `stripe` | Payments, subscriptions and invoices as the ledger of record. Needs a token. | off — needs a token |
| `notion` | The workspace this company's documents already live in. Needs a token. | off — needs a token |

## Run it

```sh
cargo run --bin opencompany -- serve --company companies/agentic_accounting_firm
```
