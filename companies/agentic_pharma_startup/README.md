# Agentic Pharma Startup

> A drug-discovery startup of agents that reviews literature, proposes molecules, runs simulations, and plans trials — humans perform the laboratory work.

## What it can do

- Review scientific literature.
- Propose candidate molecules.
- Run in-silico simulations.
- Plan clinical trials.

## Agent roster

| Agent | Responsibility |
| --- | --- |
| Literature Reviewer | Review and synthesize scientific literature. |
| Molecule Discovery | Propose candidate molecules. |
| Simulation Agent | Run in-silico simulations and screening. |
| Trial Planner | Design and plan clinical trials. |

## Human in the loop

Humans keep **laboratory work**; the agents run everything else. The output of this harness is **candidate molecules and trial plans**.

## Tool servers

Published models and datasets for target and candidate work; the protocol documents a programme is run from live in a shared workspace.

Declared in [`mcp.json`](mcp.json) and merged with anything the install
ships and anything an operator adds from the console. A server marked
*needs a token* is declared but off: write its credential from
Settings → Connections, then enable it there.

| Server | What it is for | Ships |
| --- | --- | --- |
| `huggingface` | Models, datasets and papers on the Hugging Face Hub. Public and no-auth. | on |
| `notion` | The workspace this company's documents already live in. Needs a token. | off — needs a token |

## Run it

```sh
cargo run --bin opencompany -- serve --company companies/agentic_pharma_startup
```
