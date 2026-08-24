# Agentic Customer Support Company

> A support org of agents that resolves tickets, writes docs, files bug reports, escalates hard cases, and handles refunds — humans own escalation and policy.

## What it can do

- Resolve inbound support tickets.
- Write and maintain help documentation.
- File actionable bug reports to engineering.
- Escalate complex cases.
- Handle refunds within policy.

## Agent roster

| Agent | Responsibility |
| --- | --- |
| Support Agent | Resolve inbound customer tickets. |
| Docs Writer | Write and maintain help docs. |
| Bug Reporter | File actionable bug reports. |
| Escalation Manager | Route and manage escalations. |
| Refund Handler | Process refunds within policy. |

## Human in the loop

Humans keep **escalation and policy**; the agents run everything else. The output of this harness is **resolved tickets and current documentation**.

## Tool servers

Tickets arrive in the support inbox, escalations become engineering issues, and the documentation the answers cite is the workspace's.

Declared in [`mcp.json`](mcp.json) and merged with anything the install
ships and anything an operator adds from the console. A server marked
*needs a token* is declared but off: write its credential from
Settings → Connections, then enable it there.

| Server | What it is for | Ships |
| --- | --- | --- |
| `intercom` | Live conversations and help-centre articles. Needs a token. | off — needs a token |
| `linear` | Issues and cycles, when the work is tracked outside this board. Needs a token. | off — needs a token |
| `notion` | The workspace this company's documents already live in. Needs a token. | off — needs a token |

## Run it

```sh
cargo run --bin opencompany -- serve --company companies/agentic_customer_support
```
