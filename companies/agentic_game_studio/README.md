# Agentic Game Studio

> A studio of agents that designs worlds and stories, writes code, generates assets, tests, and balances — shipping games under human creative direction.

## What it can do

- Build worlds, lore, and narrative.
- Implement gameplay systems and code.
- Generate art, audio, and other assets.
- Run QA and balance testing.
- Market the launch.

## Agent roster

| Agent | Responsibility |
| --- | --- |
| World Builder | Design worlds, lore, and settings. |
| Narrative Designer | Story, characters, and dialogue. |
| Gameplay Engineer | Implement gameplay systems and code. |
| Asset Generator | Generate art, audio, and 3D assets. |
| QA Tester | Find and report bugs. |
| Balance Designer | Tune difficulty and game balance. |
| Marketer | Market and promote the launch. |

## Human in the loop

Humans keep **creative and design direction**; the agents run everything else. The output of this harness is **shippable games**.

## Tool servers

Engine and middleware documentation, plus the issue tracker a shipping game's feature and balance work is planned in.

Declared in [`mcp.json`](mcp.json) and merged with anything the install
ships and anything an operator adds from the console. A server marked
*needs a token* is declared but off: write its credential from
Settings → Connections, then enable it there.

| Server | What it is for | Ships |
| --- | --- | --- |
| `deepwiki` | Documentation and Q&A for any public GitHub repository. Public and no-auth. | on |
| `context7` | Version-accurate API and library documentation, so answers match the release in use. | on |
| `linear` | Issues and cycles, when the work is tracked outside this board. Needs a token. | off — needs a token |

## Run it

```sh
cargo run --bin opencompany -- serve --company companies/agentic_game_studio
```
