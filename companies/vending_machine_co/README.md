# Northgate Vending

A vending-machine operator: eight machines at five host sites, a finite
warehouse behind them, and host contracts that renew whether or not anybody
prepared for it. Three hive-mind desks run it.

Every other hive bundle here answers a **question** — `hive_math_lab` solves a
stated Project Euler problem and stops. This one runs an **operation**. Work
arrives on its own, through `[[schedule]]` cadences and the
`POST /hooks/{company}/{channel}` webhook path, and nothing in the world
restocks itself.

## What this bundle is for: agent-to-agent communication

The decisions worth watching here are the ones no single desk can make
correctly:

- *Which machines does the van visit today?* is an **ops** question whose right
  answer depends on which host site is about to renew badly — which is
  **commercial's** knowledge.
- *Do we raise the price of the energy line at the gym?* is a **commercial**
  question whose right answer depends on whether that machine's chiller is
  reliable — which is **ops'** knowledge.

So all three desks run with cross-desk referral on. A member mid-episode may put
a question to a peer desk, that desk takes one real turn on it, and the answer
comes home under `hive-referral`.

The `ops` desk also turns on the other seam, the one that stays *inside* a desk:
**private asides**. Its fleet technician and stock controller may compare notes
in a line the rest of the room cannot read — "is VM-301's chiller reliable
enough to put sandwiches back in it" is a question those two settle in two lines,
and settling it on the floor costs the room two of its twelve turns watching a
conversation with no bearing on the route until it has an answer. The row is
**elided, never removed**: everyone still sees that the exchange happened, who
was in it, and where it settled, and a `^N` citation naming it still resolves.
The pair then owes the room a `!surface` in the open.

Asides are auditable, **not confidential** — an operator and every person reads
one in full. They are on for this one desk and off everywhere else in the repo
on purpose: upstream measured the mechanism and it *lost* on answer quality, so
enabling it is a decision about this desk rather than a default anybody
inherits. See [`hivemind-asides.md`](../../docs/spec/runtime/hivemind-asides.md).

One rule governs both seams, and it is what makes this sound rather than merely
chatty: **what crosses a visibility boundary carries information, never
support.** A referred answer and a private line each add no supporter and move
no option toward a decision — the asking desk still has to convince itself. A
desk that could import a quorum from elsewhere, or assemble one where the room
cannot see it, would be a desk that never had to be convinced. See
[`hivemind-referral.md`](../../docs/spec/runtime/hivemind-referral.md).

## The three desks

| Desk | Members | Quorum / budget | Owns |
| --- | --- | --- | --- |
| `ops` | `route_planner`, `fleet_tech`, `stock_controller`, `field_realist` | 2 of 4, 12 turns | The van's finite day: route, shelves, faults |
| `commercial` | `account_manager`, `pricing_analyst`, `contract_counsel` | 2 of 3, 9 turns | Margin, prices, host sites, contracts |
| `intel` | `market_scout`, `demand_analyst` | 2 of 2, 6 turns | What the market did, and whether it changes a plan |

Each desk restricts its seats' moves (`[group_chat.hive.moves]`), and the
restrictions carry the design:

- On `ops` only the **route planner** may `!propose`. Three seats proposing
  three near-identical routes is a room that votes rather than deliberates —
  the live failure `docs/spec/runtime/hivemind.md` records. The fleet
  technician and stock controller ground or `!refute` the plan from the two
  constraints that actually bind, and the **field realist** may never
  `!support` at all: a seat that both objects and supports drifts into being a
  second planner, and the room loses the one member whose incentive is purely
  to find the flaw.
- On `commercial` **two** seats may propose, because a commercial question has
  two genuinely different framings — what we charge, and what we sign — and
  forcing both through one seat produces a plan that is only ever one of them.
  **Counsel proposes nothing and may `!refute` anything**: its whole job is to
  be the seat that says no with a citation.
- `intel` is two members, the smallest room that is still a room, at a quorum
  of two — unanimity. A room of two that could carry on one supporter is a
  single responder with extra steps.

`require_evidential` is on for all three, so a `!support` with no `^citation`
adds nothing to quorum. In this company that bites hardest on the stock
controller: "we have enough stock" is a claim about a number
`warehouse_status` will actually print.

## Tool servers

The company's entire work environment is one MCP server, `vending`, declared in
[`mcp.json`](mcp.json). It exposes the fleet, the warehouse and lead times,
margin, host clients, incidents and the market feed — plus the writes that
change any of them: restock, service, order, price, renegotiate. Every read an
agent makes and every change it causes is a tool call, which is what makes a
run auditable and replayable.

`vending` **ships disabled**, pointing at a placeholder hosted URL and naming an
`authSecret`. That is deliberate: a bundle-declared server must be `https` and
must not ship enabled while it needs a credential nobody has provisioned. For a
local run you do not enable it — you run the simulator on loopback and register
it at **runtime**, which is the only layer where an `http://` endpoint is
accepted. `scripts/vending-sim.py` does that for you.

The grant that reaches it is `mcp:vending`, named explicitly in `[tools].allow`.
A wildcard cannot reach an MCP server by design (`grants_cover_server`), so a
company that wildcarded its belt does not silently acquire every server an
operator later installs.

## Running it

```bash
# 1. Memory. CortexDB is a standalone service, not the removed in-pod engine;
#    the script prints the OPENCOMPANY_MEMORY_* exports to use.
./scripts/cortexdb-up.sh

# 2. The company.
OPENCOMPANY_INFERENCE_URL=http://127.0.0.1:6969/v1 \
OPENCOMPANY_INFERENCE_KEY=$LADDER_API_KEY \
OPENCOMPANY_AUTH_MODE=none \
  cargo run --features openhuman,mcp --bin opencompany -- \
    serve --company companies/vending_machine_co

# 3. The world, the MCP server, and the trigger loop — one command.
python3 scripts/vending-sim.py --days 14
```

`vending-sim.py` starts the simulator, registers it as a runtime MCP server,
then advances the clock a day at a time, posting each day's triggers into the
company and waiting for the desks to answer. It reports what each episode
decided, which desks asked each other questions, and what the fleet was worth
at the end.

## The ledgers

The operational truth lives in the MCP server; duplicating it here would produce
two records that disagree by the end of the week. These four hold what the
server cannot:

- **`decisions`** — the *reasoning*. A transcript scrolls out of a
  thirty-message window and nobody re-reads one to learn why the van skipped
  Vulcan for a fortnight. The `not_doing` field is the most useful and the most
  likely to be left empty.
- **`clients`** — what each host was *promised*, and by whom. A promise nobody
  wrote down is one this company will break by accident.
- **`incidents`** — the *pattern*. One jam is an event; VM-301 jamming four
  times in a month is a machine that needs replacing, and no list of open
  incidents will ever say so.
- **`signals`** — observation kept apart from inference, because a correct
  observation with a wrong inference and a wrong observation with a lucky one
  look identical afterwards.

## What the operator decides

`[policy]` is `supervised`, and the desks act on the fleet themselves —
restocking and pricing are the job. What stays with a human is money and
commitments: `place_order` and `renegotiate_contract` are on `always_approve`,
so a headless run has to pump the approvals queue (the driver does).
