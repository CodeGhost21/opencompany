# The vending world

The simulated business behind `companies/vending_machine_co`, in three files
that deliberately do not know about each other's concerns.

| File | What it is | What it must not know |
| --- | --- | --- |
| `world.py` | The business: fleet, warehouse, clients, incidents, news, and the clock that moves them | What an agent is; anything about HTTP |
| `mcp_server.py` | The only way to touch the world: an MCP Streamable-HTTP server | How the world computes anything |
| `../vending-sim.py` | The scenario driver: advances the clock and posts each day's triggers into a running company | How either of the above works internally |

## The world

`World.seeded()` builds an eight-machine operator across five host sites —
office, gym, hospital, campus, factory — each with its own demand profile, so
the right assortment at one is wrong at another.

`World.advance(days)` is the whole point. In a fixed order per day it lands
deliveries (so the day's restock can use them), draws demand, spoils
perishables, draws and ages incidents, and rolls news. It returns the
**triggers** the period produced: a machine that went empty, a jam, a contract
clock. The world says what happened; it never says what to do about it.

Three couplings make the decisions non-trivial, and none of them is decorative:

- **The warehouse is finite and the supplier takes four days.** A restock
  decision is therefore also a purchasing decision made four days earlier, and
  a plan that ignores the lead time buys stock for a week that is already over.
- **A jam is total.** A jammed machine takes no money at all, so restocking one
  stocks a machine that cannot sell — the single most expensive mistake
  available, and the easiest to make from a stock report alone.
- **Operational failure becomes commercial failure on a delay.** Stockouts and
  incidents left open past three days erode the host's satisfaction, which is
  what a competitor quotes against at renewal. By the time satisfaction is
  visibly low, the damage was done a fortnight ago.

Runs are seeded and the RNG state round-trips through `--state`, so a scenario
replays exactly: a run that produced an interesting deliberation can be re-run
against a changed prompt and the difference attributed to the prompt.

## The MCP server

Fourteen tools, split by **decision** rather than by table. `fleet_status`
answers "where should the van go today" and returns `days_cover` per slot —
because "how full is it" is the wrong question and "when does it run out" is the
right one. `margin_report` computes margin against the supplier cost *at the
time of sale*, so a line whose cost rose shows the squeeze rather than hiding
it. An agent that had to assemble either from primitives would spend its turn on
arithmetic instead of on the judgement its desk was seated for.

Writes refuse rather than partially succeed: `restock_machine` rejects the whole
call if any line exceeds warehouse stock or slot capacity, because a
quietly-shortened plan hides the order that should have been placed first.

Transport is MCP Streamable HTTP — one `POST /mcp` carrying JSON-RPC, answering
`application/json`. That is what OpenCompany dials, and the only transport
hosted OpenCompany supports (a stdio `command` is a validation error there). SSE
is accepted by the client but not required, so this stays standard-library only.

**There is no auth, deliberately.** It binds loopback and holds an invented
fleet; a token would only mean the bundle had to ship a secret to make its own
demo work. Do not put it on a public interface.

```bash
python3 scripts/vending/mcp_server.py --port 7801 --state /tmp/vending.json
```

Register it with a running company — runtime is the only layer where an
`http://` endpoint is accepted, which is why the bundle ships its `vending`
entry disabled rather than pointing it here:

```bash
curl -X POST localhost:8080/api/v1/company/mcp/servers \
  -H 'content-type: application/json' \
  -d '{"name":"vending","endpoint":"http://127.0.0.1:7801/mcp"}'
```

## The driver

`scripts/vending-sim.py` does all of the above and then runs the loop: advance a
day, route the day's triggers to the desk that owns the *decision* (not the desk
that holds the most facts — the desks can ask each other, and whether they do is
what the run is measuring), wait for the episodes to close, and report.

It also pumps the approvals queue, because `place_order` and
`renegotiate_contract` are on the bundle's `always_approve` list and an
unattended run would otherwise deadlock with a desk parked on a tool call.

Exit status is the number of days on which no desk produced a decision, so a
CI-style caller can treat zero as "the company was awake the whole time".
