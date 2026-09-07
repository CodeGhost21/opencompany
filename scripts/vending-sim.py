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


class Host:
    """A cookie-carrying HTTP client for one OpenCompany host.

    The surfaces and their exact shapes are the ones ``scripts/hive-euler.py``
    already drives against a live host — the auth flow, the approvals verdict
    body and the desk transcript read are copied rather than re-derived,
    because each of them is a place a plausible-looking guess fails only at
    run time against a real server.
    """

    def __init__(self, base: str) -> None:
        self.base = base.rstrip("/")
        self.jar = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(self.jar))

    def call(self, method: str, path: str, body: Any = None, timeout: float = 120):
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(self.base + path, data=data, method=method)
        req.add_header("accept", "application/json")
        if data is not None:
            req.add_header("content-type", "application/json")
        try:
            with self.opener.open(req, timeout=timeout) as resp:
                raw = resp.read()
                return resp.status, (json.loads(raw) if raw else None)
        except urllib.error.HTTPError as err:
            raw = err.read()
            try:
                return err.code, json.loads(raw)
            except ValueError:
                return err.code, raw.decode(errors="replace")

    def sign_in(self) -> None:
        """No-op when auth is `none`; otherwise the loopback dev-code flow."""
        status, _ = self.call("GET", f"{SCOPE}/chat/history?limit=1")
        if status == 200:
            return
        status, body = self.call("POST", f"{SCOPE}/auth/request", {"email": ADMIN_EMAIL})
        code = (body or {}).get("dev_code") if isinstance(body, dict) else None
        if not code:
            raise SystemExit(f"sign-in: no dev_code from auth/request ({status}: {body})")
        status, body = self.call("POST", f"{SCOPE}/auth/verify", {"code": code})
        if status >= 300:
            raise SystemExit(f"sign-in: verify refused ({status}: {body})")

    def register_mcp(self, name: str, endpoint: str) -> tuple[int, Any]:
        """Add the simulator as a *runtime* MCP server.

        Runtime is the only layer that accepts an ``http://`` endpoint — a
        server declared in a bundle's ``mcp.json`` must be ``https``
        (`content_test`). That is why the bundle ships `vending` disabled and
        this registers the loopback one instead of enabling it.
        """
        return self.call(
            "POST", f"{SCOPE}/mcp/servers", {"name": name, "endpoint": endpoint}
        )

    def say(self, desk: str, text: str, timeout: float = 3600) -> None:
        """Put one message to `desk`, holding the POST open for the episode.

        The cycle runs the whole episode synchronously inside this request, so
        this blocks for as long as the room deliberates. The caller has to run
        it on a thread and pump approvals meanwhile — see `run_day`.
        """
        status, body = self.call(
            "POST", f"{SCOPE}/chat", {"text": text, "chat": desk}, timeout=timeout
        )
        if status >= 300:
            raise RuntimeError(f"chat POST to `{desk}` failed ({status}): {body}")

    def approve_all(self) -> list[str]:
        """Answer everything parked.

        `place_order` and `renegotiate_contract` are on this bundle's
        `always_approve` list, so an unattended run deadlocks without this: the
        desk commits, reaches for the tool, and parks inside the still-open
        chat POST. Approving everything is right for a simulation and wrong for
        anything else.
        """
        status, body = self.call("GET", f"{SCOPE}/approvals")
        if status != 200 or not isinstance(body, list):
            return []
        approved = []
        for approval in body:
            aid = approval.get("id")
            status, _ = self.call(
                "POST", f"{SCOPE}/approvals/{aid}", {"verdict": "approve", "detach": True}
            )
            if status < 300:
                approved.append(approval.get("kind", "?"))
        return approved

    def history(self, desk: str, limit: int = 200) -> list[dict]:
        query = urllib.parse.urlencode({"desk": desk, "limit": limit})
        status, body = self.call("GET", f"{SCOPE}/chat/history?{query}")
        if status != 200 or not isinstance(body, list):
            return []
        return body


def last_id(host: Host, desk: str) -> int:
    rows = host.history(desk, limit=1)
    return int(rows[-1]["id"]) if rows else 0


def closing_report(messages: list[dict]) -> dict | None:
    """The episode's close, which is not every `hive-report` row.

    A failed turn is journaled under the same author and begins `@someone's
    turn did not finish` — the room continues after one, so treating it as the
    close would report an episode as over while it was still running.
    """
    return next(
        (
            m
            for m in reversed(messages)
            if m.get("author") == HIVE_REPORT_AUTHOR
            and not m.get("text", "").lstrip().startswith("@")
        ),
        None,
    )


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
