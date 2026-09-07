#!/usr/bin/env python3
"""Run Northgate Vending as a live scenario: a world, a clock, and three desks.

This is the vending-machine counterpart to ``scripts/hive-euler.py``. Where that
one states a problem and grades the answer, this one runs a **business**: it
starts the simulated world behind an MCP server, registers that server with a
running company, then advances the clock a day at a time — posting each day's
triggers into the desk that owns them and waiting for the room to answer.

What it is for is the thing a unit test cannot show: whether real models seated
on three desks, given a fleet that is genuinely too big for the van, actually
**talk to each other**. The run reports every cross-desk referral it saw, which
is the mechanism this bundle exists to exercise.

Stdlib only, so it runs wherever ``python3`` does. Talks to a running
``opencompany serve`` (default ``http://127.0.0.1:8080``) whose auth mode is
``none``, or signs in as the manifest admin through the dev-code flow.

    # the whole thing, defaults
    python3 scripts/vending-sim.py --days 14

    # against an already-running simulator, and a different company
    python3 scripts/vending-sim.py --mcp-url http://127.0.0.1:7801/mcp --no-spawn-mcp

Exit status is the number of days on which no desk produced a decision, so a
CI-style caller can treat zero as "the company was awake the whole time".
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent / "vending"))

from mcp_server import build_server  # noqa: E402

SCOPE = "/api/v1/company"
ADMIN_EMAIL = "harness-e2e@tinyhumans.ai"
HIVE_REPORT_AUTHOR = "hive-report"
HIVE_REFERRAL_AUTHOR = "hive-referral"

# Which desk owns which kind of trigger. The routing is deliberately dumb: a
# trigger goes to the desk that owns the *decision*, not to the desk that holds
# the most facts about it, because the desks can ask each other for facts and
# the whole point of the run is to see whether they do.
TRIGGER_DESK = {
    "machine_down": "ops",
    "chiller_fault": "ops",
    "stockout": "ops",
    "spoilage": "ops",
    "delivery_received": "ops",
    "incident_stale": "ops",
    "complaint": "commercial",
    "contract_renewal": "commercial",
    "news": "intel",
}

# A day can produce dozens of triggers and a desk can hold one episode at a
# time. Batching per desk per day is both cheaper and truer: an operator does
# not page a team once per empty spiral, they hand over the morning's list.
MAX_TRIGGERS_PER_MESSAGE = 12


class Client:
    """The company's REST surface, with the dev-code sign-in the harness needs."""

    def __init__(self, base: str, timeout: float = 30.0) -> None:
        self.base = base.rstrip("/")
        self.timeout = timeout
        self.opener = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor()
        )

    def _call(self, method: str, path: str, body: Any = None) -> Any:
        url = f"{self.base}{path}"
        data = json.dumps(body).encode() if body is not None else None
        req = urllib.request.Request(url, data=data, method=method)
        req.add_header("accept", "application/json")
        if data:
            req.add_header("content-type", "application/json")
        try:
            with self.opener.open(req, timeout=self.timeout) as resp:
                raw = resp.read()
                return json.loads(raw) if raw else None
        except urllib.error.HTTPError as err:
            detail = err.read().decode(errors="replace")[:400]
            raise RuntimeError(f"{method} {path} -> {err.code}: {detail}") from err

    def get(self, path: str) -> Any:
        return self._call("GET", path)

    def post(self, path: str, body: Any = None) -> Any:
        return self._call("POST", path, body)

    # -- auth -------------------------------------------------------------

    def sign_in_if_needed(self) -> None:
        """No-op under `OPENCOMPANY_AUTH_MODE=none`; dev-code flow otherwise.

        The host echoes a dev code on loopback with no public URL, which is the
        only reason this can be unattended. If neither path works, the run
        fails here rather than a hundred confusing 401s later.
        """
        try:
            self.get(f"{SCOPE}/health")
            return
        except RuntimeError:
            pass
        started = self.post("/api/v1/auth/dev/start", {"email": ADMIN_EMAIL})
        code = (started or {}).get("code")
        if not code:
            raise RuntimeError(
                "the host did not echo a dev sign-in code; run it with "
                "OPENCOMPANY_AUTH_MODE=none or on loopback with no public URL"
            )
        self.post("/api/v1/auth/dev/verify", {"email": ADMIN_EMAIL, "code": code})

    # -- the surfaces this driver uses ------------------------------------

    def register_mcp(self, name: str, endpoint: str) -> Any:
        """Add the simulator as a *runtime* MCP server.

        Runtime is the only layer that accepts an `http://` endpoint — a server
        declared in a bundle's `mcp.json` must be `https` (`content_test`).
        That is why the bundle ships `vending` disabled and this registers the
        loopback one instead of enabling it.
        """
        return self.post(f"{SCOPE}/mcp/servers", {"name": name, "endpoint": endpoint})

    def say(self, desk: str, text: str) -> Any:
        return self.post(f"{SCOPE}/chat", {"chat": desk, "text": text})

    def events(self, after: int = 0, limit: int = 500) -> list[dict[str, Any]]:
        got = self.get(f"{SCOPE}/events?after={after}&limit={limit}")
        if isinstance(got, dict):
            return got.get("events") or got.get("items") or []
        return got or []

    def approvals(self) -> list[dict[str, Any]]:
        got = self.get(f"{SCOPE}/approvals")
        if isinstance(got, dict):
            return got.get("approvals") or got.get("items") or []
        return got or []

    def decide(self, approval_id: str, approve: bool = True) -> Any:
        return self.post(
            f"{SCOPE}/approvals/{approval_id}",
            {"decision": "approve" if approve else "deny", "reason": "vending-sim: unattended run"},
        )


def pump_approvals(client: Client, stop: threading.Event, log: list[str]) -> None:
    """Answer the approvals queue for as long as the run lasts.

    `place_order` and `renegotiate_contract` are on the bundle's
    `always_approve` list, so an unattended run deadlocks without this — the
    desk commits, reaches for the tool, and parks forever. Approving everything
    is right for a simulation and wrong for anything else.
    """
    while not stop.is_set():
        try:
            for card in client.approvals():
                ident = card.get("id") or card.get("approval_id")
                if not ident:
                    continue
                client.decide(str(ident), True)
                log.append(f"approved {card.get('tool') or card.get('title') or ident}")
        except Exception as err:  # a transient read must not kill the pump
            log.append(f"approval pump: {err}")
        stop.wait(3.0)


def describe(triggers: list[dict[str, Any]], day: int) -> str:
    """Render one desk's share of a day as a message an operator would send."""
    shown = triggers[:MAX_TRIGGERS_PER_MESSAGE]
    lines = [f"- {t['detail']}" for t in shown]
    extra = len(triggers) - len(shown)
    if extra > 0:
        lines.append(f"- (and {extra} more of the same kind today)")
    return (
        f"Day {day}. What came in overnight:\n\n"
        + "\n".join(lines)
        + "\n\nDecide what to do about it. Read the fleet before you plan, and "
        "say what you are choosing not to do."
    )


def episode_rows(events: list[dict[str, Any]]) -> dict[str, list[dict[str, Any]]]:
    """Split journal rows into the three things this run reports on."""
    out: dict[str, list[dict[str, Any]]] = {"reports": [], "referrals": [], "turns": []}
    for ev in events:
        body = ev.get("event") if isinstance(ev.get("event"), dict) else ev
        kind = body.get("kind") or body.get("type") or ""
        if "AgentReply" not in str(kind) and "agent_reply" not in str(kind):
            continue
        author = body.get("agent_id") or body.get("agentId") or ""
        if author == HIVE_REPORT_AUTHOR:
            out["reports"].append(body)
        elif author == HIVE_REFERRAL_AUTHOR:
            out["referrals"].append(body)
        else:
            out["turns"].append(body)
    return out


_DECIDED = re.compile(r"carried|converged|committed", re.I)


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("--base", default="http://127.0.0.1:8080", help="the running opencompany serve")
    ap.add_argument("--days", type=int, default=14, help="simulated days to run")
    ap.add_argument("--seed", type=int, default=7)
    ap.add_argument("--mcp-host", default="127.0.0.1")
    ap.add_argument("--mcp-port", type=int, default=7801)
    ap.add_argument("--mcp-url", default=None, help="override the URL registered with the company")
    ap.add_argument("--no-spawn-mcp", action="store_true", help="use an already-running simulator")
    ap.add_argument("--state", type=Path, default=None, help="persist the world here")
    ap.add_argument(
        "--settle",
        type=float,
        default=90.0,
        help="seconds to wait for a day's episodes before moving the clock",
    )
    ap.add_argument("--out", type=Path, default=None, help="write a JSON transcript here")
    args = ap.parse_args()

    server = None
    world = None
    if not args.no_spawn_mcp:
        server = build_server(args.mcp_host, args.mcp_port, args.state, args.seed)
        world = server.ops.world
        threading.Thread(target=server.serve_forever, daemon=True).start()
        print(f"[sim] vending MCP on http://{args.mcp_host}:{args.mcp_port}/mcp", flush=True)

    mcp_url = args.mcp_url or f"http://{args.mcp_host}:{args.mcp_port}/mcp"

    client = Client(args.base)
    client.sign_in_if_needed()
    try:
        client.register_mcp("vending", mcp_url)
        print(f"[sim] registered `vending` -> {mcp_url}", flush=True)
    except RuntimeError as err:
        # Already registered from a previous run is fine and common.
        print(f"[sim] register `vending`: {err}", flush=True)

    stop = threading.Event()
    approval_log: list[str] = []
    pump = threading.Thread(target=pump_approvals, args=(client, stop, approval_log), daemon=True)
    pump.start()

    cursor = 0
    try:
        cursor = max((e.get("seq") or 0) for e in client.events(0, 1000)) if True else 0
    except Exception:
        cursor = 0

    transcript: list[dict[str, Any]] = []
    silent_days = 0

    for _ in range(args.days):
        if world is None:
            print("[sim] --no-spawn-mcp: this driver cannot advance somebody else's clock")
            break
        day_triggers = world.advance(1)
        by_desk: dict[str, list[dict[str, Any]]] = {}
        for t in day_triggers:
            desk = TRIGGER_DESK.get(t["kind"])
            if desk:
                by_desk.setdefault(desk, []).append(t)

        print(f"\n[sim] === day {world.day} — {len(day_triggers)} triggers ===", flush=True)
        for desk, items in by_desk.items():
            kinds = ", ".join(sorted({t["kind"] for t in items}))
            print(f"[sim]   -> {desk}: {len(items)} ({kinds})", flush=True)
            try:
                client.say(desk, describe(items, world.day))
            except RuntimeError as err:
                print(f"[sim]   !! {desk}: {err}", flush=True)

        deadline = time.time() + args.settle
        seen_reports = 0
        rows: dict[str, list[dict[str, Any]]] = {"reports": [], "referrals": [], "turns": []}
        while time.time() < deadline:
            try:
                fresh = client.events(cursor, 500)
            except RuntimeError:
                time.sleep(2.0)
                continue
            if fresh:
                cursor = max(cursor, max((e.get("seq") or 0) for e in fresh))
                split = episode_rows(fresh)
                for key in rows:
                    rows[key].extend(split[key])
            # Every desk that was asked has closed its episode.
            if len(rows["reports"]) >= len(by_desk) and by_desk:
                break
            time.sleep(2.0)

        decided = [r for r in rows["reports"] if _DECIDED.search(str(r.get("text", "")))]
        if not decided:
            silent_days += 1
        print(
            f"[sim]   {len(rows['turns'])} turns, {len(rows['referrals'])} cross-desk referrals, "
            f"{len(rows['reports'])} episodes closed ({len(decided)} carried)",
            flush=True,
        )
        for ref in rows["referrals"]:
            print(f"[sim]   ~~ referral: {str(ref.get('text', ''))[:160]}", flush=True)
        for rep in rows["reports"]:
            print(f"[sim]   == {str(rep.get('text', ''))[:160]}", flush=True)

        transcript.append({
            "day": world.day,
            "triggers": day_triggers,
            "routed": {k: len(v) for k, v in by_desk.items()},
            "turns": len(rows["turns"]),
            "referrals": [r.get("text") for r in rows["referrals"]],
            "reports": [r.get("text") for r in rows["reports"]],
        })

    stop.set()

    if world is not None:
        report = world.sales_report(max(0, world.day - args.days))
        open_incidents = [i for i in world.incidents if i.open]
        print("\n[sim] ===== the run =====")
        print(f"[sim] days simulated      {args.days}")
        print(f"[sim] revenue             {report['revenue_cents'] / 100:.2f}")
        print(f"[sim] margin              {report['margin_cents'] / 100:.2f}")
        print(f"[sim] units               {report['units']}")
        print(f"[sim] incidents still open {len(open_incidents)}")
        print(f"[sim] approvals answered  {len(approval_log)}")
        print(f"[sim] cross-desk referrals {sum(len(d['referrals']) for d in transcript)}")
        print(f"[sim] days with no decision {silent_days}")
        for c in world.clients.values():
            print(f"[sim]   {c.name:<26} satisfaction {c.satisfaction:.2f}  "
                  f"renews day {c.contract_renews_day}")

    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(json.dumps(transcript, indent=2, default=str))
        print(f"[sim] transcript -> {args.out}")

    if server is not None:
        server.shutdown()
        server.server_close()

    return silent_days


if __name__ == "__main__":
    raise SystemExit(main())
