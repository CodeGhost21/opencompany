#!/usr/bin/env python3
"""The vending business, exposed as an MCP server.

This is the *work environment* the ``vending_machine_co`` bundle acts on. The
world (``world.py``) is the business; this file is the only way an agent can
touch it, which is the point: every read an agent makes and every change it
causes is a tool call that can be logged, replayed and argued about.

Transport is MCP **Streamable HTTP** — a single ``POST /mcp`` carrying JSON-RPC,
answering ``application/json``. That is the transport OpenCompany dials
(`vendor/openhuman/vendor/tinymcp/.../transport/http`), and the only one hosted
OpenCompany supports: a stdio ``command`` is a validation error there
(`docs/spec/runtime/tools.md`). SSE is accepted by the client but not required,
and a plain JSON response keeps this to the standard library.

Deliberately no auth. It binds loopback, it holds an invented fleet of vending
machines, and adding a token would only mean the bundle had to ship a secret to
make its own demo work. Do not put it on a public interface.

Run it:

    python3 scripts/vending/mcp_server.py --port 7801 --state /tmp/vending.json

Then register it with a running company (this is why the URL may be ``http://``
— a *runtime* server is allowed loopback, while a server shipped in a bundle's
``mcp.json`` must be ``https``):

    curl -X POST localhost:8080/api/v1/company/mcp/servers \\
      -H 'content-type: application/json' \\
      -d '{"name":"vending","endpoint":"http://127.0.0.1:7801/mcp"}'

``scripts/vending-sim.py`` does all of that for you.
"""

from __future__ import annotations

import argparse
import json
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))

from world import CATALOGUE, LEAD_TIME_DAYS, World  # noqa: E402

PROTOCOL_VERSION = "2025-06-18"
SERVER_INFO = {"name": "vending-ops", "version": "1.0.0"}


# ---------------------------------------------------------------------------
# The tool surface
#
# Split by *decision* rather than by table. `fleet_status` answers "where should
# a van go today", `margin_report` answers "what are we actually making", and
# neither is a thin wrapper over a row store — an agent that had to assemble
# either from primitives would spend its turn on arithmetic instead of on the
# judgement the desk was seated for.
# ---------------------------------------------------------------------------

TOOLS: list[dict[str, Any]] = [
    {
        "name": "fleet_status",
        "description": (
            "Every machine: site, host client, days since service, faults, and how "
            "empty each slot is. The single read a restock or service decision starts "
            "from. Returns `days_cover` per slot — at current sell-through, how many "
            "days before that line is empty — because 'how full is it' is the wrong "
            "question and 'when does it run out' is the right one."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "machine_id": {
                    "type": "string",
                    "description": "Limit to one machine. Omit for the whole fleet.",
                }
            },
        },
    },
    {
        "name": "warehouse_status",
        "description": (
            "Warehouse stock per SKU, what is on order and when it lands, and the "
            "current supplier cost against the catalogue cost. Read this before "
            "planning a restock: the warehouse is finite and a plan that exceeds it "
            "is refused rather than silently shortened."
        ),
        "inputSchema": {"type": "object", "properties": {}},
    },
    {
        "name": "margin_report",
        "description": (
            "Units, revenue and margin over a window, broken down by SKU and by "
            "machine. Margin is computed against the supplier cost at the time of "
            "sale, so a line whose cost rose shows the squeeze rather than hiding it."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "days": {"type": "integer", "description": "Window, in days back from today. Default 7."},
                "machine_id": {"type": "string", "description": "Limit to one machine."},
            },
        },
    },
    {
        "name": "client_status",
        "description": (
            "The host sites: commission, when the contract renews, satisfaction, and "
            "the notes anyone has left. Satisfaction falls with stockouts and with "
            "incidents left open, which is how an operational failure becomes a "
            "commercial one."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {"client_id": {"type": "string"}},
        },
    },
    {
        "name": "incident_list",
        "description": (
            "Open (or all) incidents: jams, chiller faults, spoilage, complaints, with "
            "how many days each has been open. An incident open three days starts "
            "costing the host's goodwill."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "include_resolved": {"type": "boolean"},
                "machine_id": {"type": "string"},
            },
        },
    },
    {
        "name": "news_feed",
        "description": (
            "Market signals seen so far: supplier cost moves, competitor activity at "
            "named host sites, footfall events, weather. Nothing here is an "
            "instruction — these are things a plan should have reacted to."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {"days": {"type": "integer", "description": "How far back. Default 14."}},
        },
    },
    {
        "name": "restock_machine",
        "description": (
            "Move stock from the warehouse into a machine's slots and mark it "
            "serviced. Refuses the whole call if any line exceeds warehouse stock or "
            "slot capacity — a partially-filled plan would hide the order that should "
            "have been placed first."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "machine_id": {"type": "string"},
                "items": {
                    "type": "object",
                    "description": "SKU to unit count, e.g. {\"COLA-330\": 12}.",
                    "additionalProperties": {"type": "integer"},
                },
            },
            "required": ["machine_id", "items"],
        },
    },
    {
        "name": "service_machine",
        "description": (
            "Send an engineer: clears coin jams and chiller faults, resolves the "
            "matching incidents, and resets the service clock. Faults arrive faster "
            "the longer a machine has gone unserviced."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {"machine_id": {"type": "string"}},
            "required": ["machine_id"],
        },
    },
    {
        "name": "place_order",
        "description": (
            f"Order stock from the supplier at today's cost. Arrives in "
            f"{LEAD_TIME_DAYS} days — a plan that ignores the lead time buys stock for "
            f"a week that is already over."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "sku": {"type": "string"},
                "qty": {"type": "integer"},
            },
            "required": ["sku", "qty"],
        },
    },
    {
        "name": "set_price",
        "description": (
            "Change one SKU's shelf price at one machine, in cents. Prices are per "
            "machine on purpose: the same bar is a different product at a hospital "
            "and at a gym."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "machine_id": {"type": "string"},
                "sku": {"type": "string"},
                "price_cents": {"type": "integer"},
            },
            "required": ["machine_id", "sku", "price_cents"],
        },
    },
    {
        "name": "resolve_incident",
        "description": "Close an incident with a stated resolution. The resolution is the record.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "incident_id": {"type": "string"},
                "resolution": {"type": "string"},
            },
            "required": ["incident_id", "resolution"],
        },
    },
    {
        "name": "note_client",
        "description": "Append a dated note to a host site's record — what was promised, and by whom.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "client_id": {"type": "string"},
                "note": {"type": "string"},
            },
            "required": ["client_id", "note"],
        },
    },
    {
        "name": "renegotiate_contract",
        "description": (
            "Re-sign a host site at a new commission and term. Raising commission "
            "costs margin on every unit that site sells; losing the site costs all of "
            "it. Being asked lifts satisfaction a little."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "client_id": {"type": "string"},
                "commission_pct": {"type": "number", "description": "As a fraction, e.g. 0.18."},
                "term_days": {"type": "integer"},
            },
            "required": ["client_id", "commission_pct", "term_days"],
        },
    },
    {
        "name": "catalogue",
        "description": "Every SKU that can be stocked: cost, list price, whether it is chilled or perishable.",
        "inputSchema": {"type": "object", "properties": {}},
    },
]


class VendingOps:
    """Dispatch from tool name to the world, under one lock.

    The lock matters: a hive desk authorizes one speaker at a time, but several
    desks run concurrently, and two agents restocking the same machine from the
    same warehouse read is exactly the race this business has in real life.
    Serializing here means the world's own refusals are the only way a plan
    fails, rather than a torn read.
    """

    def __init__(self, world: World, state_path: Path | None) -> None:
        self.world = world
        self.state_path = state_path
        self.lock = threading.Lock()

    def _persist(self) -> None:
        if self.state_path:
            self.world.save(self.state_path)

    def call(self, name: str, args: dict[str, Any]) -> Any:
        with self.lock:
            result = self._dispatch(name, args)
            self._persist()
            return result

    def _dispatch(self, name: str, args: dict[str, Any]) -> Any:
        w = self.world
        if name == "fleet_status":
            return self._fleet(args.get("machine_id"))
        if name == "warehouse_status":
            return self._warehouse()
        if name == "margin_report":
            days = int(args.get("days", 7))
            return w.sales_report(max(0, w.day - days), args.get("machine_id"))
        if name == "client_status":
            return self._clients(args.get("client_id"))
        if name == "incident_list":
            return self._incidents(bool(args.get("include_resolved")), args.get("machine_id"))
        if name == "news_feed":
            days = int(args.get("days", 14))
            return [
                {"day": n.day, "headline": n.headline, "detail": n.detail, "tags": n.tags}
                for n in w.news
                if n.day > w.day - days
            ]
        if name == "restock_machine":
            items = {str(k): int(v) for k, v in (args.get("items") or {}).items()}
            return w.restock(str(args["machine_id"]), items)
        if name == "service_machine":
            return w.service(str(args["machine_id"]))
        if name == "place_order":
            return w.order(str(args["sku"]), int(args["qty"]))
        if name == "set_price":
            return w.set_price(str(args["machine_id"]), str(args["sku"]), int(args["price_cents"]))
        if name == "resolve_incident":
            return w.resolve_incident(str(args["incident_id"]), str(args["resolution"]))
        if name == "note_client":
            return w.note_client(str(args["client_id"]), str(args["note"]))
        if name == "renegotiate_contract":
            return w.renegotiate(
                str(args["client_id"]), float(args["commission_pct"]), int(args["term_days"])
            )
        if name == "catalogue":
            return CATALOGUE
        raise KeyError(name)

    # -- read shaping -----------------------------------------------------

    def _recent_daily_rate(self, machine_id: str, sku: str, window: int = 7) -> float:
        w = self.world
        rows = [
            s for s in w.sales
            if s.machine_id == machine_id and s.sku == sku and s.day > w.day - window
        ]
        if not rows:
            return 0.0
        return sum(s.units for s in rows) / float(min(window, max(1, w.day)))

    def _fleet(self, machine_id: str | None) -> list[dict[str, Any]]:
        w = self.world
        out = []
        for m in w.machines.values():
            if machine_id and m.id != machine_id:
                continue
            slots = []
            for s in m.slots:
                rate = self._recent_daily_rate(m.id, s.sku)
                slots.append({
                    "sku": s.sku,
                    "name": CATALOGUE[s.sku]["name"],
                    "stock": s.stock,
                    "capacity": s.capacity,
                    "price_cents": s.price,
                    "daily_rate": round(rate, 2),
                    # `None` reads as "no recent sales", which is a different
                    # thing from "will never run out" and must not be rendered
                    # as a large number.
                    "days_cover": round(s.stock / rate, 1) if rate > 0 else None,
                })
            out.append({
                "machine_id": m.id,
                "site": m.site,
                "client_id": m.client_id,
                "client": w.clients[m.client_id].name,
                "profile": m.profile,
                "footfall": m.footfall,
                "days_since_service": w.day - m.last_serviced_day,
                "coin_jam": m.coin_jam,
                "chiller_ok": m.chiller_ok,
                "cash_cents": m.cash_cents,
                "slots": slots,
            })
        return out

    def _warehouse(self) -> dict[str, Any]:
        w = self.world
        on_order: dict[str, list[dict[str, Any]]] = {}
        for po in w.orders:
            if po.received:
                continue
            on_order.setdefault(po.sku, []).append(
                {"order_id": po.id, "qty": po.qty, "arrives_day": po.arrives_day}
            )
        return {
            "day": w.day,
            "lead_time_days": LEAD_TIME_DAYS,
            "stock": w.warehouse,
            "on_order": on_order,
            "supplier_cost_cents": {
                sku: int(CATALOGUE[sku]["cost"] * w.cost_index[sku]) for sku in CATALOGUE
            },
            "catalogue_cost_cents": {sku: CATALOGUE[sku]["cost"] for sku in CATALOGUE},
        }

    def _clients(self, client_id: str | None) -> list[dict[str, Any]]:
        w = self.world
        out = []
        for c in w.clients.values():
            if client_id and c.id != client_id:
                continue
            out.append({
                "client_id": c.id,
                "name": c.name,
                "profile": c.profile,
                "commission_pct": c.commission_pct,
                "contract_renews_day": c.contract_renews_day,
                "days_to_renewal": c.contract_renews_day - w.day,
                "satisfaction": round(c.satisfaction, 3),
                "machines": [m.id for m in w.machines.values() if m.client_id == c.id],
                "notes": c.notes,
            })
        return out

    def _incidents(self, include_resolved: bool, machine_id: str | None) -> list[dict[str, Any]]:
        w = self.world
        out = []
        for i in w.incidents:
            if not include_resolved and not i.open:
                continue
            if machine_id and i.machine_id != machine_id:
                continue
            out.append({
                "incident_id": i.id,
                "opened_day": i.day,
                "days_open": (w.day - i.day) if i.open else (i.resolved_day - i.day),
                "machine_id": i.machine_id,
                "kind": i.kind,
                "severity": i.severity,
                "detail": i.detail,
                "open": i.open,
                "resolution": i.resolution,
            })
        return out


class Handler(BaseHTTPRequestHandler):
    """One JSON-RPC endpoint. `ops` and `path` are set on the server object."""

    protocol_version = "HTTP/1.1"
    server_version = "vending-mcp/1.0"

    def log_message(self, fmt: str, *args: Any) -> None:  # noqa: A003
        if self.server.verbose:  # type: ignore[attr-defined]
            sys.stderr.write("[vending-mcp] " + (fmt % args) + "\n")

    # A GET on the MCP path is how a client opens the optional server->client
    # SSE stream. This server never initiates anything, so declining the stream
    # is both honest and within the spec.
    def do_GET(self) -> None:  # noqa: N802
        if self.path.rstrip("/") == self.server.mcp_path.rstrip("/"):  # type: ignore[attr-defined]
            self._send(405, {"error": "this server does not open a server-initiated stream"})
        else:
            self._send(404, {"error": "not found"})

    def do_DELETE(self) -> None:  # noqa: N802
        # Session teardown. Stateless server, so there is nothing to tear down.
        self._send(204, None)

    def do_POST(self) -> None:  # noqa: N802
        if self.path.rstrip("/") != self.server.mcp_path.rstrip("/"):  # type: ignore[attr-defined]
            self._send(404, {"error": "not found"})
            return
        length = int(self.headers.get("content-length") or 0)
        raw = self.rfile.read(length) if length else b""
        try:
            message = json.loads(raw or b"{}")
        except json.JSONDecodeError as err:
            self._send(400, {"jsonrpc": "2.0", "id": None,
                             "error": {"code": -32700, "message": f"parse error: {err}"}})
            return

        batch = isinstance(message, list)
        requests = message if batch else [message]
        responses = [r for r in (self._handle(m) for m in requests) if r is not None]

        if not responses:
            # Every message was a notification. The protocol wants 202 and no body.
            self._send(202, None)
            return
        self._send(200, responses if batch else responses[0])

    def _handle(self, msg: dict[str, Any]) -> dict[str, Any] | None:
        method = msg.get("method")
        msg_id = msg.get("id")
        is_notification = "id" not in msg

        def ok(result: Any) -> dict[str, Any] | None:
            return None if is_notification else {"jsonrpc": "2.0", "id": msg_id, "result": result}

        def err(code: int, message: str) -> dict[str, Any] | None:
            return None if is_notification else {
                "jsonrpc": "2.0", "id": msg_id, "error": {"code": code, "message": message}
            }

        if method == "initialize":
            return ok({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools": {"listChanged": False}},
                "serverInfo": SERVER_INFO,
                "instructions": (
                    "This is a live vending-machine operation: a fleet of machines at host "
                    "sites, a finite warehouse behind them, and clients whose contracts renew. "
                    "Reads are free; writes change the business and are visible to everyone. "
                    "Nothing here restocks itself."
                ),
            })
        if method in ("notifications/initialized", "notifications/cancelled"):
            return None
        if method == "ping":
            return ok({})
        if method == "tools/list":
            return ok({"tools": TOOLS})
        if method == "tools/call":
            params = msg.get("params") or {}
            name = params.get("name")
            args = params.get("arguments") or {}
            try:
                result = self.server.ops.call(name, args)  # type: ignore[attr-defined]
            except KeyError:
                return err(-32602, f"unknown tool `{name}`")
            except (TypeError, ValueError) as exc:
                # A bad argument is the model's mistake to correct, so it comes
                # back as a tool result rather than a protocol error — the
                # latter is not shown to the agent in most hosts.
                return ok({
                    "content": [{"type": "text", "text": json.dumps(
                        {"ok": False, "error": f"bad arguments for `{name}`: {exc}"})}],
                    "isError": True,
                })
            payload = json.dumps(result, indent=2, default=str)
            is_error = isinstance(result, dict) and result.get("ok") is False
            return ok({
                "content": [{"type": "text", "text": payload}],
                "isError": is_error,
            })
        return err(-32601, f"unknown method `{method}`")

    def _send(self, status: int, body: Any) -> None:
        raw = b"" if body is None else json.dumps(body).encode()
        self.send_response(status)
        if raw:
            self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(raw)))
        self.end_headers()
        if raw:
            self.wfile.write(raw)


class VendingHTTPServer(ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True

    def __init__(self, addr, ops: VendingOps, mcp_path: str, verbose: bool) -> None:
        super().__init__(addr, Handler)
        self.ops = ops
        self.mcp_path = mcp_path
        self.verbose = verbose


def build_server(
    host: str = "127.0.0.1",
    port: int = 7801,
    state: Path | None = None,
    seed: int = 7,
    mcp_path: str = "/mcp",
    verbose: bool = False,
) -> VendingHTTPServer:
    world = World.load(state, seed) if state else World.seeded(seed)
    return VendingHTTPServer((host, port), VendingOps(world, state), mcp_path, verbose)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=7801)
    ap.add_argument("--path", default="/mcp", help="URL path the JSON-RPC endpoint answers on")
    ap.add_argument("--state", type=Path, help="persist the world here; resumes if it exists")
    ap.add_argument("--seed", type=int, default=7, help="RNG seed for a fresh world")
    ap.add_argument("--verbose", action="store_true")
    args = ap.parse_args()

    server = build_server(args.host, args.port, args.state, args.seed, args.path, args.verbose)
    world = server.ops.world
    print(
        f"vending MCP on http://{args.host}:{args.port}{args.path} — "
        f"day {world.day}, {len(world.machines)} machines, {len(world.clients)} clients",
        flush=True,
    )
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("stopping", flush=True)
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
