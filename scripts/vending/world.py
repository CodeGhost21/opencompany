#!/usr/bin/env python3
"""The vending-machine world: state, the clock that moves it, and the triggers it fires.

This is the *simulated business* the ``vending_machine_co`` bundle manages. It is
deliberately separate from the MCP layer (``mcp_server.py``) and from the driver
that runs a scenario (``../vending-sim.py``): the world does not know what an
agent is, and nothing here talks HTTP.

What it models, and why each piece earns its place:

* **A fleet.** Machines at host sites, each with slots holding a SKU at a par
  level. Stock falls as the day's footfall draws it down, so "which machine to
  restock" is a real question with a wrong answer.
* **A warehouse.** Finite stock with supplier lead times, so a restock decision
  is coupled to a purchasing decision — the two desks cannot each be right on
  their own.
* **Clients.** The host sites, on commission contracts with renewal dates and a
  satisfaction level that moves with stockouts and unresolved incidents. This
  is what makes an operational failure a commercial one a week later.
* **Incidents.** Jams, spoilage, cash faults, complaints. They age, and ageing
  is what makes a deferred decision cost something.
* **News.** Supplier price moves, weather, local events, a competitor. Signals
  that should change a plan and that nobody is told to act on.

The clock is the whole point. ``World.advance(days)`` draws demand, spoils
stock, ages incidents, lands deliveries, and returns the **triggers** the day
produced. A trigger is an event the company should notice — a machine that went
empty, a contract about to renew badly, a delivery that slipped. The driver
turns those into messages into the company; the world just says what happened.

Determinism is on purpose: seed the RNG and a scenario replays exactly, so a
run that produced an interesting deliberation can be re-run against a changed
prompt and the difference attributed to the prompt.
"""

from __future__ import annotations

import json
import random
from dataclasses import dataclass, field, asdict
from pathlib import Path
from typing import Any

# --------------------------------------------------------------------------
# The catalogue.
#
# Margin is per unit, in cents, and varies enough between lines that a pricing
# or assortment decision has a defensible answer. `perishable` is what makes
# over-restocking a chilled slot cost money rather than only tying it up.
# --------------------------------------------------------------------------

CATALOGUE: dict[str, dict[str, Any]] = {
    "COLA-330": {"name": "Cola 330ml", "cost": 42, "price": 150, "perishable": False, "chilled": True},
    "WATER-500": {"name": "Still water 500ml", "cost": 18, "price": 120, "perishable": False, "chilled": True},
    "ENERGY-250": {"name": "Energy drink 250ml", "cost": 78, "price": 250, "perishable": False, "chilled": True},
    "CRISPS-40": {"name": "Crisps 40g", "cost": 31, "price": 120, "perishable": False, "chilled": False},
    "CHOC-45": {"name": "Chocolate bar 45g", "cost": 38, "price": 140, "perishable": False, "chilled": False},
    "PROTEIN-60": {"name": "Protein bar 60g", "cost": 95, "price": 280, "perishable": False, "chilled": False},
    "SANDWICH-1": {"name": "Sandwich", "cost": 165, "price": 395, "perishable": True, "chilled": True},
    "SALAD-1": {"name": "Salad pot", "cost": 180, "price": 450, "perishable": True, "chilled": True},
    "COFFEE-1": {"name": "Canned coffee", "cost": 55, "price": 190, "perishable": False, "chilled": True},
    "NUTS-50": {"name": "Mixed nuts 50g", "cost": 62, "price": 200, "perishable": False, "chilled": False},
}

# Site archetypes. The demand profile is what makes one machine's right
# assortment wrong at another — a gym does not buy what an office buys, and a
# hospital buys at night when nobody is there to restock it.
SITE_PROFILES: dict[str, dict[str, float]] = {
    "office": {
        "COLA-330": 1.0, "WATER-500": 1.2, "ENERGY-250": 0.6, "CRISPS-40": 0.9,
        "CHOC-45": 1.0, "PROTEIN-60": 0.4, "SANDWICH-1": 1.1, "SALAD-1": 0.7,
        "COFFEE-1": 1.4, "NUTS-50": 0.5,
    },
    "gym": {
        "COLA-330": 0.3, "WATER-500": 2.0, "ENERGY-250": 1.6, "CRISPS-40": 0.2,
        "CHOC-45": 0.3, "PROTEIN-60": 1.8, "SANDWICH-1": 0.4, "SALAD-1": 0.5,
        "COFFEE-1": 0.6, "NUTS-50": 1.1,
    },
    "hospital": {
        "COLA-330": 0.9, "WATER-500": 1.3, "ENERGY-250": 1.1, "CRISPS-40": 1.0,
        "CHOC-45": 1.2, "PROTEIN-60": 0.5, "SANDWICH-1": 1.4, "SALAD-1": 0.6,
        "COFFEE-1": 1.7, "NUTS-50": 0.4,
    },
    "campus": {
        "COLA-330": 1.3, "WATER-500": 1.1, "ENERGY-250": 1.5, "CRISPS-40": 1.4,
        "CHOC-45": 1.3, "PROTEIN-60": 0.6, "SANDWICH-1": 0.9, "SALAD-1": 0.3,
        "COFFEE-1": 1.0, "NUTS-50": 0.5,
    },
    "factory": {
        "COLA-330": 1.4, "WATER-500": 1.0, "ENERGY-250": 1.3, "CRISPS-40": 1.2,
        "CHOC-45": 1.1, "PROTEIN-60": 0.4, "SANDWICH-1": 1.0, "SALAD-1": 0.2,
        "COFFEE-1": 1.5, "NUTS-50": 0.4,
    },
}


@dataclass
class Slot:
    """One spiral in one machine: what it holds, how much fits, what is in it."""

    sku: str
    capacity: int
    stock: int
    price: int  # cents, per unit, at this machine


@dataclass
class Machine:
    id: str
    site: str
    client_id: str
    profile: str
    footfall: int          # people past the machine per day
    slots: list[Slot]
    cash_cents: int = 0
    chiller_ok: bool = True
    coin_jam: bool = False
    last_serviced_day: int = 0

    def slot(self, sku: str) -> Slot | None:
        for s in self.slots:
            if s.sku == sku:
                return s
        return None


@dataclass
class Client:
    """A host site. Commission is what the client earns on our gross."""

    id: str
    name: str
    profile: str
    commission_pct: float
    contract_renews_day: int
    satisfaction: float = 0.8   # 0..1; falls on stockouts and stale incidents
    notes: list[str] = field(default_factory=list)


@dataclass
class Incident:
    id: str
    day: int
    machine_id: str
    kind: str          # jam | chiller | spoilage | complaint | vandalism | cash
    detail: str
    severity: str      # low | medium | high
    resolved_day: int | None = None
    resolution: str | None = None

    @property
    def open(self) -> bool:
        return self.resolved_day is None


@dataclass
class PurchaseOrder:
    id: str
    sku: str
    qty: int
    unit_cost: int
    placed_day: int
    arrives_day: int
    received: bool = False


@dataclass
class Sale:
    day: int
    machine_id: str
    sku: str
    units: int
    unit_price: int
    unit_cost: int

    @property
    def revenue(self) -> int:
        return self.units * self.unit_price

    @property
    def margin(self) -> int:
        return self.units * (self.unit_price - self.unit_cost)


@dataclass
class NewsItem:
    day: int
    headline: str
    detail: str
    tags: list[str]


# --------------------------------------------------------------------------
# The world
# --------------------------------------------------------------------------

# Supplier lead time in days. A restock decision that ignores this orders stock
# that arrives after the week it was needed for, which is the single most
# common way this business loses margin.
LEAD_TIME_DAYS = 4

# What a bad day costs a host site, in satisfaction.
#
# Charged **once per machine per day**, not once per empty spiral. Per-spiral was
# the first cut and it made satisfaction a countdown rather than a signal: eight
# machines with four empty lines each is thirty-two charges a day, so every site
# in the fleet sat at 0.00 within a fortnight and the number could no longer
# distinguish a badly-run site from a catastrophically-run one — which is the
# only thing it is for.
STOCKOUT_SATISFACTION_HIT = 0.02
STALE_INCIDENT_SATISFACTION_HIT = 0.015
# An incident is "stale" once it has been open this long.
INCIDENT_STALE_DAYS = 3

# What a *good* day is worth. A site whose machines all sold without running dry,
# and that is carrying no stale incident, recovers a little.
#
# Recovery is what makes satisfaction a feedback loop instead of a ratchet: a
# desk that fixes a site should be able to see it come back, and a contract
# renewal negotiated after a good fortnight should be a different conversation
# from one negotiated after a bad one. It is deliberately slower than the
# damage — goodwill is easier to lose than to earn, and a fleet cannot neglect a
# site for a month and repair it in a week.
SATISFACTION_RECOVERY = 0.008
# Recovery never carries a site past this on its own. Getting above it takes
# something a host notices — a renegotiation, or a commitment kept.
SATISFACTION_RECOVERY_CEILING = 0.85


class World:
    """The whole simulated business, and the clock that moves it."""

    def __init__(self, seed: int = 7) -> None:
        self.rng = random.Random(seed)
        self.seed = seed
        self.day = 0
        self.machines: dict[str, Machine] = {}
        self.clients: dict[str, Client] = {}
        self.warehouse: dict[str, int] = {}
        self.incidents: list[Incident] = []
        self.orders: list[PurchaseOrder] = []
        self.sales: list[Sale] = []
        self.news: list[NewsItem] = []
        # Supplier price index per SKU, 1.0 = catalogue cost. News moves it.
        self.cost_index: dict[str, float] = {sku: 1.0 for sku in CATALOGUE}
        self._seq = 0

    # -- identity ---------------------------------------------------------

    def _next_id(self, prefix: str) -> str:
        self._seq += 1
        return f"{prefix}-{self._seq:04d}"

    # -- construction -----------------------------------------------------

    @classmethod
    def seeded(cls, seed: int = 7) -> "World":
        """A plausible eight-machine operator across five host sites.

        Sized so that a single restock run cannot cover everything: the whole
        interest of the ops desk is that it has to choose.
        """
        w = cls(seed)

        sites = [
            ("CL-NORTHGATE", "Northgate Business Park", "office", 0.15, 40),
            ("CL-IRONWORKS", "Ironworks Gym", "gym", 0.20, 22),
            ("CL-STMARY", "St Mary's Hospital", "hospital", 0.12, 61),
            ("CL-BRIDGE", "Bridge Street Campus", "campus", 0.18, 34),
            ("CL-VULCAN", "Vulcan Components", "factory", 0.10, 75),
        ]
        for cid, name, profile, commission, renews in sites:
            w.clients[cid] = Client(
                id=cid, name=name, profile=profile,
                commission_pct=commission, contract_renews_day=renews,
                satisfaction=w.rng.uniform(0.68, 0.88),
            )

        fleet = [
            ("VM-101", "Northgate — atrium", "CL-NORTHGATE", 420),
            ("VM-102", "Northgate — 3rd floor", "CL-NORTHGATE", 180),
            ("VM-201", "Ironworks — main floor", "CL-IRONWORKS", 260),
            ("VM-301", "St Mary's — A&E waiting", "CL-STMARY", 700),
            ("VM-302", "St Mary's — staff room", "CL-STMARY", 150),
            ("VM-401", "Bridge St — library", "CL-BRIDGE", 380),
            ("VM-501", "Vulcan — canteen", "CL-VULCAN", 300),
            ("VM-502", "Vulcan — night gate", "CL-VULCAN", 90),
        ]
        for mid, site, cid, footfall in fleet:
            profile = w.clients[cid].profile
            # Eight slots, drawn from the lines that suit the site, so each
            # machine starts with a defensible-but-improvable assortment.
            ranked = sorted(
                SITE_PROFILES[profile].items(), key=lambda kv: kv[1], reverse=True
            )
            chosen = [sku for sku, _ in ranked[:6]] + [
                sku for sku, _ in ranked[6:]
            ][:2]
            slots = []
            for sku in chosen[:8]:
                # Capacity is sized to this site's own demand for this line —
                # roughly three days of cover when full — rather than being one
                # number for the whole fleet.
                #
                # A flat capacity made the busiest machine structurally
                # impossible: VM-301 draws ~12 sandwiches a day against a
                # 12-slot spiral, so it stocked out on 17 days in 21 even when
                # refilled to the brim every single morning. That turns its host
                # site's satisfaction into a doom clock no decision can affect,
                # which is the opposite of what this simulation is for. The
                # constraint worth modelling is that the van cannot visit
                # everything today — not that one machine can never be right.
                rate = footfall * 0.012 * SITE_PROFILES[profile].get(sku, 0.5)
                cover = 2 if CATALOGUE[sku]["perishable"] else 3
                cap = max(10, int(rate * cover + 0.5))
                slots.append(
                    Slot(
                        sku=sku,
                        capacity=cap,
                        stock=w.rng.randint(int(cap * 0.3), cap),
                        price=CATALOGUE[sku]["price"],
                    )
                )
            w.machines[mid] = Machine(
                id=mid, site=site, client_id=cid, profile=profile,
                footfall=footfall, slots=slots,
                cash_cents=w.rng.randint(2000, 18000),
            )

        for sku in CATALOGUE:
            w.warehouse[sku] = w.rng.randint(40, 200)

        # Two lines whose wholesale cost moved before this company started
        # paying attention. Shelf prices are still the catalogue ones, so the
        # margin on both is already wrong on day one.
        #
        # Seeded rather than left to the news feed: a cost move surfaces on
        # roughly one day in ten, and the commercial desk's opening card asks it
        # to find the prices that are already wrong. A card whose answer is
        # "none yet, come back in a fortnight" teaches the desk that the
        # question is not worth asking.
        w.cost_index["SANDWICH-1"] = 1.24
        w.cost_index["ENERGY-250"] = 1.17

        return w

    # -- the clock --------------------------------------------------------

    def advance(self, days: int = 1) -> list[dict[str, Any]]:
        """Move the world forward and return the triggers the period produced.

        Order within a day matters and is fixed: deliveries land first (so the
        day's restock can use them), then demand draws stock down, then
        perishables spoil, then incidents are drawn and aged, then news. A
        trigger is emitted for anything the company would want to be told.
        """
        triggers: list[dict[str, Any]] = []
        for _ in range(max(1, days)):
            self.day += 1
            # Accumulated separately from `triggers` and reset each iteration:
            # `_recover` asks "did this site have a bad day *today*", and handing
            # it the whole period's list would let one bad Monday block recovery
            # for the rest of the fortnight. That also made `advance(21)` and
            # twenty-one `advance(1)` calls disagree, which would have been a
            # silent difference between a batch run and the day-at-a-time one
            # the scenario driver actually makes.
            today: list[dict[str, Any]] = []
            today.extend(self._receive_deliveries())
            today.extend(self._draw_demand())
            today.extend(self._spoil())
            today.extend(self._draw_incidents())
            today.extend(self._age_incidents())
            today.extend(self._draw_news())
            today.extend(self._recover(today))
            today.extend(self._contract_watch())
            triggers.extend(today)
        return triggers

    def _receive_deliveries(self) -> list[dict[str, Any]]:
        out = []
        for po in self.orders:
            if po.received or po.arrives_day > self.day:
                continue
            po.received = True
            self.warehouse[po.sku] = self.warehouse.get(po.sku, 0) + po.qty
            out.append({
                "kind": "delivery_received",
                "day": self.day,
                "detail": f"PO {po.id} landed: {po.qty} x {po.sku} at {po.unit_cost}c/unit.",
                "sku": po.sku,
                "qty": po.qty,
            })
        return out

    def _draw_demand(self) -> list[dict[str, Any]]:
        """Sell what people wanted and the machine could actually give them.

        A stockout is recorded as lost demand, not as an absence: the whole
        point of the sales ledger is that it should be possible to see revenue
        that was available and not taken.
        """
        out = []
        for m in self.machines.values():
            if m.coin_jam:
                # A jammed machine takes no money at all. This is the most
                # expensive fault in the fleet and the easiest to miss.
                continue
            profile = SITE_PROFILES[m.profile]
            # Stockouts are accumulated per machine per day rather than emitted
            # per slot. A fleet nobody has restocked yet empties every spiral,
            # and one trigger per spiral per day is a few hundred a fortnight —
            # a volume that buries the machine-down and contract triggers that
            # actually need a decision. One row per machine per day is what an
            # operator would be paged with, and it still names every line.
            starved: list[tuple[str, int, int]] = []
            for slot in m.slots:
                weight = profile.get(slot.sku, 0.5)
                if CATALOGUE[slot.sku]["chilled"] and not m.chiller_ok:
                    weight *= 0.15
                # Footfall converts at roughly 1.2% per unit of profile weight.
                expected = m.footfall * 0.012 * weight
                wanted = max(0, int(self.rng.gauss(expected, expected * 0.35)))
                sold = min(wanted, slot.stock)
                lost = wanted - sold
                if sold:
                    slot.stock -= sold
                    unit_cost = int(CATALOGUE[slot.sku]["cost"] * self.cost_index[slot.sku])
                    self.sales.append(Sale(
                        day=self.day, machine_id=m.id, sku=slot.sku,
                        units=sold, unit_price=slot.price, unit_cost=unit_cost,
                    ))
                    m.cash_cents += sold * slot.price
                if lost > 0:
                    starved.append((slot.sku, lost, lost * slot.price))
            if starved:
                client = self.clients[m.client_id]
                client.satisfaction = max(
                    0.0, client.satisfaction - STOCKOUT_SATISFACTION_HIT
                )
                lost_cents = sum(c for _, _, c in starved)
                lines = ", ".join(f"{sku} ({units} units)" for sku, units, _ in starved)
                out.append({
                    "kind": "stockout",
                    "day": self.day,
                    "machine_id": m.id,
                    "client_id": m.client_id,
                    "skus": [sku for sku, _, _ in starved],
                    "lost_units": sum(u for _, u, _ in starved),
                    "lost_revenue_cents": lost_cents,
                    "detail": (
                        f"{m.id} ({m.site}) turned demand away today on {lines} — "
                        f"{lost_cents}c of revenue lost."
                    ),
                })
        return out

    def _spoil(self) -> list[dict[str, Any]]:
        """Perishables die on a warm chiller, and slowly even on a cold one."""
        out = []
        for m in self.machines.values():
            for slot in m.slots:
                if not CATALOGUE[slot.sku]["perishable"] or slot.stock == 0:
                    continue
                rate = 0.45 if not m.chiller_ok else 0.06
                dead = sum(1 for _ in range(slot.stock) if self.rng.random() < rate)
                if dead:
                    slot.stock -= dead
                    cost = dead * CATALOGUE[slot.sku]["cost"]
                    self.incidents.append(Incident(
                        id=self._next_id("INC"), day=self.day, machine_id=m.id,
                        kind="spoilage",
                        detail=f"{dead} x {slot.sku} written off at {m.id}"
                               f"{' (chiller fault)' if not m.chiller_ok else ''}.",
                        severity="high" if not m.chiller_ok else "low",
                    ))
                    out.append({
                        "kind": "spoilage",
                        "day": self.day,
                        "machine_id": m.id,
                        "sku": slot.sku,
                        "units": dead,
                        "cost_cents": cost,
                        "detail": f"{dead} x {slot.sku} spoiled at {m.id}, {cost}c written off.",
                    })
        return out

    def _draw_incidents(self) -> list[dict[str, Any]]:
        """Faults arrive at a rate that rises with use and with time since service."""
        out = []
        for m in self.machines.values():
            since_service = self.day - m.last_serviced_day
            base = 0.006 + 0.0009 * since_service
            if self.rng.random() < base and not m.coin_jam:
                m.coin_jam = True
                inc = Incident(
                    id=self._next_id("INC"), day=self.day, machine_id=m.id,
                    kind="jam",
                    detail=f"{m.id} coin mechanism jammed — the machine is taking no money.",
                    severity="high",
                )
                self.incidents.append(inc)
                out.append({
                    "kind": "machine_down", "day": self.day, "machine_id": m.id,
                    "incident_id": inc.id, "detail": inc.detail,
                })
            if self.rng.random() < base * 0.6 and m.chiller_ok:
                m.chiller_ok = False
                inc = Incident(
                    id=self._next_id("INC"), day=self.day, machine_id=m.id,
                    kind="chiller",
                    detail=f"{m.id} chiller is running warm — chilled lines will spoil.",
                    severity="high",
                )
                self.incidents.append(inc)
                out.append({
                    "kind": "chiller_fault", "day": self.day, "machine_id": m.id,
                    "incident_id": inc.id, "detail": inc.detail,
                })
            if self.rng.random() < 0.010:
                client = self.clients[m.client_id]
                complaint = self.rng.choice([
                    "took my money and gave nothing",
                    "the sandwiches are always gone by 11am",
                    "nothing here is under two pounds",
                    "the card reader declined three times",
                    "everything in the chiller was warm",
                ])
                inc = Incident(
                    id=self._next_id("INC"), day=self.day, machine_id=m.id,
                    kind="complaint",
                    detail=f"Complaint from {client.name} about {m.id}: “{complaint}”.",
                    severity="medium",
                )
                self.incidents.append(inc)
                out.append({
                    "kind": "complaint", "day": self.day, "machine_id": m.id,
                    "client_id": client.id, "incident_id": inc.id, "detail": inc.detail,
                })
        return out

    def _age_incidents(self) -> list[dict[str, Any]]:
        """An open incident costs the host's goodwill once it has been open a while."""
        out = []
        for inc in self.incidents:
            if not inc.open:
                continue
            age = self.day - inc.day
            if age == INCIDENT_STALE_DAYS:
                m = self.machines.get(inc.machine_id)
                if m:
                    client = self.clients[m.client_id]
                    client.satisfaction = max(
                        0.0, client.satisfaction - STALE_INCIDENT_SATISFACTION_HIT
                    )
                    out.append({
                        "kind": "incident_stale",
                        "day": self.day,
                        "incident_id": inc.id,
                        "machine_id": inc.machine_id,
                        "client_id": client.id,
                        "detail": (
                            f"{inc.id} ({inc.kind}) at {inc.machine_id} has been open "
                            f"{age} days. {client.name}'s satisfaction is now "
                            f"{client.satisfaction:.2f}."
                        ),
                    })
        return out

    def _draw_news(self) -> list[dict[str, Any]]:
        """Market signals. Some move costs; all of them are things a plan should react to."""
        if self.rng.random() > 0.35:
            return []
        out = []
        roll = self.rng.random()
        if roll < 0.30:
            sku = self.rng.choice(list(CATALOGUE))
            move = self.rng.uniform(0.06, 0.22)
            self.cost_index[sku] *= 1 + move
            item = NewsItem(
                day=self.day,
                headline=f"Supplier raises {sku} by {move * 100:.0f}%",
                detail=(
                    f"Wholesale cost for {CATALOGUE[sku]['name']} is up "
                    f"{move * 100:.0f}% from today. Our shelf price is unchanged, so the "
                    f"margin on every unit sold from now on is thinner."
                ),
                tags=["supplier", "cost", sku],
            )
        elif roll < 0.50:
            client = self.rng.choice(list(self.clients.values()))
            item = NewsItem(
                day=self.day,
                headline=f"Competitor quoting {client.name}",
                detail=(
                    f"A rival operator is said to be quoting {client.name} at "
                    f"{client.commission_pct * 100 + self.rng.randint(2, 6):.0f}% commission. "
                    f"Their contract renews on day {client.contract_renews_day}."
                ),
                tags=["competitor", "contract", client.id],
            )
        elif roll < 0.70:
            client = self.rng.choice(list(self.clients.values()))
            item = NewsItem(
                day=self.day,
                headline=f"Footfall event at {client.name}",
                detail=(
                    f"{client.name} is running an event next week; expect materially "
                    f"higher traffic past its machines for a few days."
                ),
                tags=["footfall", "event", client.id],
            )
        elif roll < 0.85:
            item = NewsItem(
                day=self.day,
                headline="Heatwave forecast",
                detail=(
                    "A warm spell is forecast. Cold drinks sell through faster and "
                    "chillers work harder; both of those have bitten this fleet before."
                ),
                tags=["weather", "demand"],
            )
        else:
            item = NewsItem(
                day=self.day,
                headline="Card scheme fees changing",
                detail=(
                    "Card acquiring fees rise next month on low-value transactions, "
                    "which is most of what this fleet takes."
                ),
                tags=["fees", "cost"],
            )
        self.news.append(item)
        out.append({
            "kind": "news", "day": self.day,
            "headline": item.headline, "detail": item.detail, "tags": item.tags,
        })
        return out

    def _recover(self, today: list[dict[str, Any]]) -> list[dict[str, Any]]:
        """Sites that had a clean day earn a little goodwill back.

        Clean means: no machine at that site turned demand away, and the site is
        carrying no incident that has been open past `INCIDENT_STALE_DAYS`.
        Both conditions matter — full shelves in front of a chiller that has
        been broken for a week is not a good day for the host, and a site that
        recovered on shelf stock alone would let a desk ignore its faults.

        Emits no trigger. Nothing is owed to anybody when things go right, and a
        daily "still fine" message for five sites would bury the four triggers
        that actually need a decision.
        """
        hurt = {
            t.get("client_id")
            for t in today
            if t["kind"] in ("stockout", "incident_stale")
        }
        # `_draw_incidents` tags complaints with a client too; a complaint today
        # is not a day to recover from either.
        hurt |= {t.get("client_id") for t in today if t["kind"] == "complaint"}
        stale_sites = {
            self.machines[i.machine_id].client_id
            for i in self.incidents
            if i.open
            and self.day - i.day >= INCIDENT_STALE_DAYS
            and i.machine_id in self.machines
        }
        for c in self.clients.values():
            if c.id in hurt or c.id in stale_sites:
                continue
            if c.satisfaction >= SATISFACTION_RECOVERY_CEILING:
                continue
            c.satisfaction = min(
                SATISFACTION_RECOVERY_CEILING, c.satisfaction + SATISFACTION_RECOVERY
            )
        return []

    def _contract_watch(self) -> list[dict[str, Any]]:
        """A renewal is only actionable while there is still time to act."""
        out = []
        for c in self.clients.values():
            days_left = c.contract_renews_day - self.day
            if days_left in (14, 7, 3):
                out.append({
                    "kind": "contract_renewal",
                    "day": self.day,
                    "client_id": c.id,
                    "days_left": days_left,
                    "satisfaction": round(c.satisfaction, 3),
                    "detail": (
                        f"{c.name}'s contract renews in {days_left} days at "
                        f"{c.commission_pct * 100:.0f}% commission. Satisfaction is "
                        f"{c.satisfaction:.2f}."
                    ),
                })
        return out

    # -- actions the company can take -------------------------------------

    def restock(self, machine_id: str, items: dict[str, int]) -> dict[str, Any]:
        """Move stock from the warehouse into a machine's slots.

        Refuses rather than partially inventing stock: a plan that asks for
        more than the warehouse holds is a plan that needed to place an order
        first, and silently shipping less would hide that.
        """
        m = self.machines.get(machine_id)
        if not m:
            return {"ok": False, "error": f"no machine {machine_id}"}
        problems, moved = [], {}
        for sku, qty in items.items():
            slot = m.slot(sku)
            if slot is None:
                problems.append(f"{machine_id} has no slot for {sku}")
                continue
            if qty > self.warehouse.get(sku, 0):
                problems.append(
                    f"warehouse holds {self.warehouse.get(sku, 0)} x {sku}, asked for {qty}"
                )
                continue
            room = slot.capacity - slot.stock
            if qty > room:
                problems.append(f"{machine_id}/{sku} has room for {room}, asked for {qty}")
                continue
            self.warehouse[sku] -= qty
            slot.stock += qty
            moved[sku] = qty
        if problems:
            return {"ok": False, "error": "; ".join(problems), "moved": moved}
        m.last_serviced_day = self.day
        return {"ok": True, "machine_id": machine_id, "moved": moved, "day": self.day}

    def service(self, machine_id: str) -> dict[str, Any]:
        """Clear the mechanical faults on a machine and mark it serviced."""
        m = self.machines.get(machine_id)
        if not m:
            return {"ok": False, "error": f"no machine {machine_id}"}
        cleared = []
        if m.coin_jam:
            m.coin_jam = False
            cleared.append("coin jam")
        if not m.chiller_ok:
            m.chiller_ok = True
            cleared.append("chiller")
        m.last_serviced_day = self.day
        for inc in self.incidents:
            if inc.open and inc.machine_id == machine_id and inc.kind in ("jam", "chiller"):
                inc.resolved_day = self.day
                inc.resolution = "cleared on service visit"
        return {"ok": True, "machine_id": machine_id, "cleared": cleared, "day": self.day}

    def set_price(self, machine_id: str, sku: str, price_cents: int) -> dict[str, Any]:
        m = self.machines.get(machine_id)
        if not m:
            return {"ok": False, "error": f"no machine {machine_id}"}
        slot = m.slot(sku)
        if slot is None:
            return {"ok": False, "error": f"{machine_id} has no slot for {sku}"}
        was, slot.price = slot.price, int(price_cents)
        return {"ok": True, "machine_id": machine_id, "sku": sku,
                "was_cents": was, "now_cents": slot.price}

    def order(self, sku: str, qty: int) -> dict[str, Any]:
        if sku not in CATALOGUE:
            return {"ok": False, "error": f"no such SKU {sku}"}
        unit_cost = int(CATALOGUE[sku]["cost"] * self.cost_index[sku])
        po = PurchaseOrder(
            id=self._next_id("PO"), sku=sku, qty=int(qty), unit_cost=unit_cost,
            placed_day=self.day, arrives_day=self.day + LEAD_TIME_DAYS,
        )
        self.orders.append(po)
        return {"ok": True, "order_id": po.id, "sku": sku, "qty": po.qty,
                "unit_cost_cents": unit_cost, "arrives_day": po.arrives_day,
                "lead_time_days": LEAD_TIME_DAYS}

    def resolve_incident(self, incident_id: str, resolution: str) -> dict[str, Any]:
        for inc in self.incidents:
            if inc.id == incident_id:
                if not inc.open:
                    return {"ok": False, "error": f"{incident_id} was already resolved"}
                inc.resolved_day = self.day
                inc.resolution = resolution
                return {"ok": True, "incident_id": incident_id, "day": self.day}
        return {"ok": False, "error": f"no incident {incident_id}"}

    def note_client(self, client_id: str, note: str) -> dict[str, Any]:
        c = self.clients.get(client_id)
        if not c:
            return {"ok": False, "error": f"no client {client_id}"}
        c.notes.append(f"day {self.day}: {note}")
        return {"ok": True, "client_id": client_id, "notes": len(c.notes)}

    def renegotiate(self, client_id: str, commission_pct: float, term_days: int) -> dict[str, Any]:
        """Re-sign a host site. Goodwill from being asked lifts satisfaction a little."""
        c = self.clients.get(client_id)
        if not c:
            return {"ok": False, "error": f"no client {client_id}"}
        was = c.commission_pct
        c.commission_pct = float(commission_pct)
        c.contract_renews_day = self.day + int(term_days)
        c.satisfaction = min(1.0, c.satisfaction + 0.05)
        return {"ok": True, "client_id": client_id, "was_pct": was,
                "now_pct": c.commission_pct, "renews_day": c.contract_renews_day}

    # -- reads ------------------------------------------------------------

    def sales_report(self, since_day: int = 0, machine_id: str | None = None) -> dict[str, Any]:
        rows = [s for s in self.sales if s.day > since_day
                and (machine_id is None or s.machine_id == machine_id)]
        by_sku: dict[str, dict[str, int]] = {}
        for s in rows:
            b = by_sku.setdefault(s.sku, {"units": 0, "revenue_cents": 0, "margin_cents": 0})
            b["units"] += s.units
            b["revenue_cents"] += s.revenue
            b["margin_cents"] += s.margin
        by_machine: dict[str, dict[str, int]] = {}
        for s in rows:
            b = by_machine.setdefault(s.machine_id, {"units": 0, "revenue_cents": 0, "margin_cents": 0})
            b["units"] += s.units
            b["revenue_cents"] += s.revenue
            b["margin_cents"] += s.margin
        return {
            "day": self.day,
            "since_day": since_day,
            "units": sum(s.units for s in rows),
            "revenue_cents": sum(s.revenue for s in rows),
            "margin_cents": sum(s.margin for s in rows),
            "by_sku": by_sku,
            "by_machine": by_machine,
        }

    def to_json(self) -> str:
        return json.dumps({
            "seed": self.seed,
            "day": self.day,
            "seq": self._seq,
            "machines": {k: asdict(v) for k, v in self.machines.items()},
            "clients": {k: asdict(v) for k, v in self.clients.items()},
            "warehouse": self.warehouse,
            "incidents": [asdict(i) for i in self.incidents],
            "orders": [asdict(o) for o in self.orders],
            "sales": [asdict(s) for s in self.sales],
            "news": [asdict(n) for n in self.news],
            "cost_index": self.cost_index,
            "rng": self.rng.getstate(),
        }, default=list)

    @classmethod
    def from_json(cls, blob: str) -> "World":
        d = json.loads(blob)
        w = cls(d["seed"])
        w.day = d["day"]
        w._seq = d["seq"]
        w.machines = {
            k: Machine(**{**v, "slots": [Slot(**s) for s in v["slots"]]})
            for k, v in d["machines"].items()
        }
        w.clients = {k: Client(**v) for k, v in d["clients"].items()}
        w.warehouse = d["warehouse"]
        w.incidents = [Incident(**i) for i in d["incidents"]]
        w.orders = [PurchaseOrder(**o) for o in d["orders"]]
        w.sales = [Sale(**s) for s in d["sales"]]
        w.news = [NewsItem(**n) for n in d["news"]]
        w.cost_index = d["cost_index"]
        # `getstate` round-trips through JSON as nested lists; `setstate` needs
        # the tuple shape back or a resumed run diverges from an uninterrupted
        # one, which would defeat the point of seeding it.
        state = d["rng"]
        w.rng.setstate((state[0], tuple(state[1]), state[2]))
        return w

    def save(self, path: Path) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(self.to_json())

    @classmethod
    def load(cls, path: Path, seed: int = 7) -> "World":
        if path.exists():
            return cls.from_json(path.read_text())
        return cls.seeded(seed)
