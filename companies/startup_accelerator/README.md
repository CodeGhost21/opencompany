# Startup Accelerator

> Runs a cohort-based accelerator end to end — sourcing founders, screening applications, matching mentors, running the curriculum, and staging demo day.

## What it can do

- Source and screen inbound and outbound startup applications.
- Match each startup to relevant mentors and resources.
- Run a structured curriculum and track weekly progress.
- Prepare cohorts for demo day and warm investor introductions.
- Provide ongoing portfolio support after the program.

## Agent roster

| Agent | Responsibility |
| --- | --- |
| Startup Scout | Source promising founders and startups into the pipeline. |
| Application Screener | Score and shortlist applications against the thesis. |
| Mentor Matcher | Pair startups with the right mentors and resources. |
| Curriculum Designer | Design and schedule the program curriculum. |
| Progress Coach | Track weekly milestones and unblock founders. |
| Demo Day Producer | Prepare pitches and stage demo day. |
| Investor Liaison | Make warm, targeted investor introductions. |
| Portfolio Support | Support alumni after the program. |

## Human in the loop

Humans keep **investment and demo-day decisions**; the agents run everything else. The output of this harness is **a funded, mentored startup cohort**.

## Tool servers

Applications and cohort records live in a shared workspace; a cohort company's own work is tracked in its own tracker.

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
cargo run --bin opencompany -- serve --company companies/startup_accelerator
```
