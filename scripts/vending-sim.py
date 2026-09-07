#!/usr/bin/env python3
"""Run Northgate Vending as a live scenario: a world, a clock, and three desks.

This is the vending-machine counterpart to ``scripts/hive-euler.py``. Where that
one states a problem and grades the answer, this one runs a **business**: it
starts the simulated world behind an MCP server, registers that server with a
running company, then advances the clock a day at a time — posting each day's
triggers into the desk that owns them and waiting for the room to answer.

What it is for is the thing a unit test cannot show: whether real models seated
on three desks, given a fleet that is genuinely too big for the van, actually
**talk to each other**. The run counts both seams this bundle exists to
exercise — cross-desk referrals, and the private asides the ops desk may open —
and prints the text of every referral, because a run in which the desks never
speak to each other looks exactly like a healthy one from the margin alone.

Stdlib only, so it runs wherever ``python3`` does. Talks to a running
``opencompany serve`` (default ``http://127.0.0.1:8080``) whose auth mode is
``none``, or signs in as the manifest admin through the dev-code flow.

    # the whole thing, defaults
    python3 scripts/vending-sim.py --days 14

    # a longer run, persisting the world so it can be resumed
    python3 scripts/vending-sim.py --days 30 --state /tmp/vending.json --out run.json

Exit status is the number of days on which no desk produced a decision, so a
CI-style caller can treat zero as "the company was awake the whole time".
"""

from __future__ import annotations

import argparse
import http.cookiejar
import json
import re
import sys
import threading
import time
import urllib.error
import urllib.parse
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
        """Point the company's `vending` server at this run's simulator.

        Runtime is the only layer that accepts an ``http://`` endpoint — a
        server declared in a bundle's ``mcp.json`` must be ``https``
        (`content_test`). That is why the bundle ships `vending` disabled and
        pointing at a placeholder, and why this repoints it at loopback.

        **It is a PUT, not a POST.** The name is already declared in the bundle,
        so adding it is a 409 telling you to override it instead — and a POST
        that 409s leaves the desks holding the shipped, disabled placeholder and
        deliberating confidently with no tools at all. `PUT` creates a runtime
        override of the manifest entry, which is exactly the layer-4 override
        `docs/spec/runtime/tools.md` describes.
        """
        status, body = self.call(
            "PUT",
            f"{SCOPE}/mcp/servers/{urllib.parse.quote(name)}",
            {"endpoint": endpoint, "enabled": True},
        )
        if status < 300:
            return status, body
        # No such name to override: a company that does not ship the entry at
        # all (someone running this against a hand-rolled bundle). Add it.
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


# The four closes `EpisodeOutcome::summary` can write (`src/hivemind/types.rs`),
# of which exactly one is a decision:
#
#   converged  "The desk settled on #topic after N turns (backed by …)."
#   deadlocked "The desk deadlocked after N turns: #a and #b carried together…"
#   exhausted  "The desk spent its N-turn budget without reaching a decision."
#   idle       "Nobody on the desk had anything to add, so the room did not open."
#
# Anchored on "settled on #" and nothing looser. A first cut matched
# `carried|converged|committed` and was wrong twice over: it missed every real
# decision, because the converged summary says "settled" and never "carried",
# and it would have scored a *deadlock* as a decision, because "carried
# together" is how the deadlock line describes the tie it failed to break.
_CARRIED = re.compile(r"settled on #", re.I)


def run_desk(host: Host, desk: str, text: str, settle: float, log) -> dict[str, Any]:
    """State one day's triggers in one desk and wait for the room to close.

    The chat POST holds open for the whole episode, and an approval parks
    *inside* it, so the operator's two jobs have to run concurrently: state the
    message on a thread, and pump approvals from here while it runs. This is the
    same shape `hive-euler.py` uses, for the same reason — a driver that posted
    synchronously would deadlock on its own approval the first time a desk
    reached for `place_order`.
    """
    after = last_id(host, desk)
    failure: list[BaseException] = []

    def state() -> None:
        try:
            # A little past the reader's own deadline, so the POST outlives the
            # wait rather than racing it.
            host.say(desk, text, timeout=settle + 60)
        except BaseException as err:  # noqa: BLE001 — surfaced below
            failure.append(err)

    poster = threading.Thread(target=state, daemon=True)
    poster.start()

    deadline = time.time() + settle
    approved: list[str] = []
    messages: list[dict] = []
    report = None
    while time.time() < deadline:
        if failure:
            log(f"[sim]     !! chat POST failed: {failure[0]}")
            break
        approved.extend(host.approve_all())
        messages = [m for m in host.history(desk) if int(m.get("id", "0")) > after]
        report = closing_report(messages)
        if report:
            break
        time.sleep(3)

    # `poster` is a daemon thread and never otherwise joined, so without this
    # the next desk's POST could start while this one is still in flight and
    # both would touch the same company concurrently. Bounded, not indefinite:
    # a tool still parked could keep the POST open well past our own deadline.
    poster.join(timeout=30)
    if poster.is_alive():
        log(f"[sim]     !! the POST is still in flight past the settle window")

    referrals = [m for m in messages if m.get("author") == HIVE_REFERRAL_AUTHOR]
    turns = [
        m
        for m in messages
        if m.get("author") not in (HIVE_REPORT_AUTHOR, HIVE_REFERRAL_AUTHOR)
        and not m.get("mine")
    ]
    # An aside is journaled as an ordinary desk turn carrying a narrower
    # audience, and the transcript renders its marker — which is what these
    # count. An operator is admitted to every aside, so this reader sees them
    # all; a peer agent's own projection would have elided the ones it is not in.
    asides = [m for m in turns if m.get("text", "").lstrip().startswith("!aside")]
    surfaced = [m for m in turns if m.get("text", "").lstrip().startswith("!surface")]

    return {
        "desk": desk,
        "turns": len(turns),
        "referrals": [m.get("text", "") for m in referrals],
        "asides": len(asides),
        "surfaced": len(surfaced),
        "approved": approved,
        "report": (report or {}).get("text"),
        "carried": bool(report and _CARRIED.search(report.get("text", ""))),
    }


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
    ap.add_argument("--state", type=Path, default=None, help="persist the world here")
    ap.add_argument(
        "--settle",
        type=float,
        default=600.0,
        help="seconds to wait for one desk's episode before moving on",
    )
    ap.add_argument("--out", type=Path, default=None, help="write a JSON transcript here")
    args = ap.parse_args()

    def log(line: str) -> None:
        print(line, flush=True)

    server = build_server(args.mcp_host, args.mcp_port, args.state, args.seed)
    world = server.ops.world
    threading.Thread(target=server.serve_forever, daemon=True).start()
    log(f"[sim] vending MCP on http://{args.mcp_host}:{args.mcp_port}/mcp — day {world.day}")

    mcp_url = args.mcp_url or f"http://{args.mcp_host}:{args.mcp_port}/mcp"
    host = Host(args.base)
    host.sign_in()
    status, body = host.register_mcp("vending", mcp_url)
    if status >= 300:
        # Already registered from a previous run is fine and common; anything
        # else is worth seeing, because the desks have no tools at all without
        # it and would otherwise deliberate confidently about nothing.
        log(f"[sim] register `vending` -> {mcp_url}: {status} {body}")
    else:
        log(f"[sim] registered `vending` -> {mcp_url}")

    transcript: list[dict[str, Any]] = []
    silent_days = 0

    for _ in range(args.days):
        day_triggers = world.advance(1)
        by_desk: dict[str, list[dict[str, Any]]] = {}
        for t in day_triggers:
            desk = TRIGGER_DESK.get(t["kind"])
            if desk:
                by_desk.setdefault(desk, []).append(t)

        log(f"\n[sim] === day {world.day} — {len(day_triggers)} triggers ===")
        outcomes = []
        for desk, items in by_desk.items():
            kinds = ", ".join(sorted({t["kind"] for t in items}))
            log(f"[sim]  -> {desk}: {len(items)} ({kinds})")
            outcome = run_desk(host, desk, describe(items, world.day), args.settle, log)
            outcomes.append(outcome)
            log(
                f"[sim]     {outcome['turns']} turns, {outcome['asides']} asides "
                f"({outcome['surfaced']} surfaced), {len(outcome['referrals'])} referrals, "
                f"{len(outcome['approved'])} approvals"
            )
            for text in outcome["referrals"]:
                log(f"[sim]     ~~ referral: {text[:150]}")
            if outcome["report"]:
                log(f"[sim]     == {outcome['report'][:150]}")
            else:
                log("[sim]     == no close within the settle window")

        if not any(o["carried"] for o in outcomes):
            silent_days += 1
        transcript.append(
            {
                "day": world.day,
                "triggers": day_triggers,
                "routed": {k: len(v) for k, v in by_desk.items()},
                "outcomes": outcomes,
            }
        )

    report = world.sales_report(max(0, world.day - args.days))
    referrals = sum(len(o["referrals"]) for t in transcript for o in t["outcomes"])
    asides = sum(o["asides"] for t in transcript for o in t["outcomes"])
    surfaced = sum(o["surfaced"] for t in transcript for o in t["outcomes"])
    approvals = sum(len(o["approved"]) for t in transcript for o in t["outcomes"])

    log("\n[sim] ===== the run =====")
    log(f"[sim] days simulated        {args.days}")
    log(f"[sim] revenue               {report['revenue_cents'] / 100:.2f}")
    log(f"[sim] margin                {report['margin_cents'] / 100:.2f}")
    log(f"[sim] units                 {report['units']}")
    log(f"[sim] incidents still open  {sum(1 for i in world.incidents if i.open)}")
    log(f"[sim] cross-desk referrals  {referrals}")
    log(f"[sim] private asides        {asides} ({surfaced} surfaced)")
    log(f"[sim] approvals answered    {approvals}")
    log(f"[sim] days with no decision {silent_days}")
    for c in world.clients.values():
        log(
            f"[sim]   {c.name:<26} satisfaction {c.satisfaction:.2f}  "
            f"renews day {c.contract_renews_day}"
        )

    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(json.dumps(transcript, indent=2, default=str))
        log(f"[sim] transcript -> {args.out}")

    server.shutdown()
    server.server_close()
    return silent_days


if __name__ == "__main__":
    raise SystemExit(main())
