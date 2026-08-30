# Agentic Law Firm

> Within regulatory limits, a firm of agents does legal research, drafts contracts, supports litigation, runs discovery, and checks compliance — a licensed human approves filings.

## What it can do

- Perform legal research.
- Draft contracts and documents.
- Support litigation preparation.
- Run document discovery.
- Check regulatory compliance.

## Agent roster

| Agent | Responsibility |
| --- | --- |
| Legal Researcher | Case law and legal research. |
| Contract Drafter | Draft contracts and legal documents. |
| Litigation Support | Prepare materials for litigation. |
| Discovery Agent | Run and review document discovery. |
| Compliance Agent | Check regulatory compliance. |

## Human in the loop

Humans keep **approving filings**; the agents run everything else. The output of this harness is **research, drafts, and discovery**.

## Tool servers

Matter documents and client files live in a shared workspace. Deliberately narrow: a firm should decide what its agents may reach before granting it.

Declared in [`mcp.json`](mcp.json) and merged with anything the install
ships and anything an operator adds from the console. A server marked
*needs a token* is declared but off: write its credential from
Settings → Connections, then enable it there.

| Server | What it is for | Ships |
| --- | --- | --- |
| `notion` | The workspace this company's documents already live in. Needs a token. | off — needs a token |

## Run it

```sh
cargo run --bin opencompany -- serve --company companies/agentic_law_firm
```
