# Agentic Software Company

> A software company of agents that designs, builds, ships, and supports an entire SaaS product — with a human owning product direction.

## What it can do

- Turn a product vision into specs, a roadmap, and prioritized work.
- Design the product and build the frontend and backend.
- Test, secure, and ship releases.
- Write documentation and run developer relations.
- Support customers and feed insight back into the roadmap.

## Agent roster

| Agent | Responsibility |
| --- | --- |
| Product Manager | Own the roadmap, specs, and prioritization. |
| Designer | Product and UX design. |
| Backend Engineer | Build and operate the backend and services. |
| Frontend Engineer | Build the user-facing frontend. |
| QA Engineer | Test features and catch regressions. |
| Security Engineer | Security review, hardening, and response. |
| Documentation Writer | Write and maintain product documentation. |
| Customer Support | Resolve customer issues and feed insight back. |
| Developer Relations | Engage developers with demos, content, and community. |

## Human in the loop

Humans keep **product direction**; the agents run everything else. The output of this harness is **an entire SaaS product**.

## Tool servers

Engineers read other people's code and current API docs all day, and diagnose incidents from production evidence rather than from memory.

Declared in [`mcp.json`](mcp.json) and merged with anything the install
ships and anything an operator adds from the console. A server marked
*needs a token* is declared but off: write its credential from
Settings → Connections, then enable it there.

| Server | What it is for | Ships |
| --- | --- | --- |
| `deepwiki` | Documentation and Q&A for any public GitHub repository. Public and no-auth. | on |
| `context7` | Version-accurate API and library documentation, so answers match the release in use. | on |
| `sentry` | Production errors and their stack traces, for diagnosing an incident from evidence. Needs a token. | off — needs a token |
| `linear` | Issues and cycles, when the work is tracked outside this board. Needs a token. | off — needs a token |

## Run it

```sh
cargo run --bin opencompany -- serve --company companies/agentic_software_company
```
