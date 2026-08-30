# Agentic Recruiting Company

> A recruiting firm of agents that sources, reaches out, screens resumes, runs first-round interviews, schedules, and generates offers — humans make the final call.

## What it can do

- Source candidates against a role spec.
- Run personalized outreach.
- Screen resumes and rank fit.
- Conduct first-round interviews and schedule follow-ups.
- Generate offers.

## Agent roster

| Agent | Responsibility |
| --- | --- |
| Candidate Sourcer | Source candidates for open roles. |
| Outreach Agent | Run personalized candidate outreach. |
| Resume Analyst | Screen and rank resumes against the spec. |
| Interviewer | Conduct first-round interviews. |
| Scheduler | Coordinate interview scheduling. |
| Offer Generator | Draft and generate offers. |

## Human in the loop

Humans keep **final hiring decisions**; the agents run everything else. The output of this harness is **sourced, screened, and scheduled candidates**.

## Tool servers

Search briefs and scorecards live in a shared workspace; an open search is often tracked as work in the client's own tracker.

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
cargo run --bin opencompany -- serve --company companies/agentic_recruiting_company
```
