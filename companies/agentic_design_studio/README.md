# Agentic Design Studio

> A design studio of agents delivering branding, UI, motion, and illustration — validated with user testing, signed off by a human creative director.

## What it can do

- Develop brand identity systems.
- Design product UI and design systems.
- Produce motion and illustration assets.
- Run user testing and iterate on findings.

## Agent roster

| Agent | Responsibility |
| --- | --- |
| Brand Designer | Identity systems, logos, and guidelines. |
| UI Designer | Product UI and design systems. |
| Motion Designer | Animation and motion graphics. |
| Illustrator | Custom illustration and iconography. |
| User Researcher | User testing and design validation. |

## Human in the loop

Humans keep **creative direction sign-off**; the agents run everything else. The output of this harness is **brand and product design systems**.

## Tool servers

Design systems ship as code against a real component API, and the brand documents a studio works from live in a shared workspace.

Declared in [`mcp.json`](mcp.json) and merged with anything the install
ships and anything an operator adds from the console. A server marked
*needs a token* is declared but off: write its credential from
Settings → Connections, then enable it there.

| Server | What it is for | Ships |
| --- | --- | --- |
| `context7` | Version-accurate API and library documentation, so answers match the release in use. | on |
| `notion` | The workspace this company's documents already live in. Needs a token. | off — needs a token |

## Run it

```sh
cargo run --bin opencompany -- serve --company companies/agentic_design_studio
```
