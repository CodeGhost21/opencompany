#!/usr/bin/env python3
"""Replay tau2-bench tasks through an OpenCompany company, and grade the result.

This is the tau2 counterpart to ``scripts/vending-sim.py``. Where that one runs a
business forward on a clock, this one replays **tau2-bench tasks** — a customer's
opening message, and the database end-state tau2 says a correct handling
produces — through a company whose desks each hold one remedy.

What it is for is the thing tau2 itself cannot ask. Its orchestrator wires
exactly one agent to one user simulator, with no agent-to-agent path, so it can
score whether an agent called the right tool but not whether an ORGANISATION
routed the work to the seat that owns it. Here a task only completes if the case
reaches the right desk and that desk settles which remedy applies.

Domains
-------

``retail`` and ``airline`` are supported. ``telecom`` is NOT, and the reason is
in the task set rather than in this script: 2,048 of its expected actions are
``grant_app_permission``, 1,127 ``toggle_airplane_mode``, 1,040 ``reboot_device``
— actions performed by tau2's **user simulator** on its own simulated handset,
not by the agent. Nothing in an agent-side database records them, so a run here
could not be graded even if every desk behaved perfectly. Telecom needs the user
simulator wired in as a participant first.

The servers are NOT in this repo. They live in `opencompany-tau2`, which vendors
tau2-bench (~850 MB, mostly benchmark data) and needs its own venv — which is why
these bundles ship DISABLED placeholder entries in ``mcp.json`` and this script
repoints them at loopback. Start them first, one per seat:

    cd ../opencompany-tau2
    uv run tau2-mcp --roles roles/retail.yaml --role triage --http 8801 &
    ...                                                                 # etc

Then, against a running ``opencompany serve --company companies/<bundle>``:

    python3 scripts/tau2-sim.py --domain retail  --task 0
    python3 scripts/tau2-sim.py --domain airline --tasks 7,12 --out run.json

Stdlib only, so it runs wherever ``python3`` does. Exit status is the number of
tasks whose end state did not match tau2's ``evaluation_criteria``, so a
CI-style caller can treat zero as "the company handled every case correctly".
"""

from __future__ import annotations

import argparse
import http.cookiejar
import json
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any

# Per-domain wiring. `seats` maps a seat to the loopback port its role server
# listens on, in the order the servers are started; `entry` is the desk a task's
# opening message is posted to, the way a customer would arrive.
#
# `writes` is the grading table: for each mutating action tau2 asserts, how to
# read the same fact back out of the shared database. Reads are deliberately not
# graded — they are how an agent gets there, and two correct handlings can read
# different things.
DOMAINS = {
    "retail": {
        "company": "retail-co",
        # Enter at the desk that owns delivered orders: two seats holding
        # competing remedies. Small enough to read, and it still has to
        # reach outside itself for what only another seat can answer —
        # which is the crossing worth watching.
        "entry": "returns",
        "state": ".state/retail.json",
        # seat -> (port, tools in scope, of which mutating). The counts are the
        # contract this whole design rests on: `triage` holding a write tool, or
        # `exchanges` reaching the refund tool, means the scoping silently broke
        # and every later result is meaningless. `--check` asserts them.
        "seats": {
            "triage": (8801, 9, 0),
            "exchanges": (8802, 9, 1),
            "refunds": (8803, 8, 1),
            "cancellations": (8804, 8, 1),
            "amendments": (8805, 11, 3),
        },
        "desks": {
            "order_ops": ["cancellations", "amendments"],
            "returns": ["exchanges", "refunds"],
        },
        "collection": "orders",
        "key": "order_id",
        "writes": {
            "exchange_delivered_order_items": {
                "status": "exchange requested",
                "fields": {
                    "exchange_items": "item_ids",
                    "exchange_new_items": "new_item_ids",
                    "exchange_payment_method_id": "payment_method_id",
                },
            },
            "return_delivered_order_items": {
                "status": "return requested",
                "fields": {
                    "return_items": "item_ids",
                    "return_payment_method_id": "payment_method_id",
                },
            },
            "cancel_pending_order": {"status": "cancelled", "fields": {}},
            # The modify_* tools are once-per-order, so the end state is the
            # comparison — not which call produced it.
            "modify_pending_order_items": {"status": "pending", "fields": {}},
            "modify_pending_order_address": {"status": "pending", "fields": {}},
            "modify_pending_order_payment": {"status": "pending", "fields": {}},
        },
    },
    "airline": {
        "company": "airline-co",
        "entry": "triage",
        "state": ".state/airline.json",
        "seats": {
            "triage": (8811, 8, 0),
            "booking": (8812, 9, 1),
            "changes": (8813, 11, 3),
            "refunds": (8814, 10, 2),
        },
        "desks": {
            "triage": ["triage"],
            "booking": ["booking"],
            "post_booking": ["changes", "refunds"],
        },
        "collection": "reservations",
        "key": "reservation_id",
        "writes": {
            # A cancelled reservation is REMOVED from the collection rather than
            # flagged, so absence is the assertion.
            "cancel_reservation": {"absent": True, "fields": {}},
            "update_reservation_flights": {"fields": {"cabin": "cabin"}, "flights": "flights"},
            "update_reservation_baggages": {
                "fields": {"total_baggages": "total_baggages",
                           "nonfree_baggages": "nonfree_baggages"},
            },
            "update_reservation_passengers": {"fields": {"passengers": "passengers"}},
            # A booking creates the row; presence under the asserted id is the
            # assertion, since tau2 does not fix the generated id in advance.
            "book_reservation": {"present": True, "fields": {}},
        },
    },
}

UNSUPPORTED = {
    "telecom": (
        "telecom tasks are graded on the USER's device, not the agent's database: "
        "2,048 expected actions are `grant_app_permission`, 1,127 "
        "`toggle_airplane_mode`, 1,040 `reboot_device`. Those are performed by "
        "tau2's user simulator on its own simulated handset, so no agent-side "
        "state records them and a run here cannot be scored. Wire the user "
        "simulator in as a participant before enabling this domain."
    ),
}

ADMIN_EMAIL = "harness-e2e@tinyhumans.ai"


class Host:
    """The running company, over its HTTP API."""

    def __init__(self, base: str, company: str) -> None:
        self.base = base.rstrip("/")
        self.scope = f"/api/v1/companies/{company}"
        jar = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(jar)
        )

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
        except (urllib.error.URLError, OSError) as err:
            # Nothing listening, or the host died mid-run. Reported as 0 rather
            # than raising, so `--check` can say "unreachable" instead of dying
            # with a traceback — and so it is never mistaken for a 404 from a
            # host that IS answering.
            return 0, f"unreachable: {err}"

    def sign_in(self) -> None:
        """No-op when auth is `none`; otherwise the loopback dev-code flow."""
        status, _ = self.call("GET", f"{self.scope}/chat/history?limit=1")
        if status == 200:
            return
        status, body = self.call("POST", f"{self.scope}/auth/request", {"email": ADMIN_EMAIL})
        code = (body or {}).get("dev_code") if isinstance(body, dict) else None
        if not code:
            raise SystemExit(f"sign-in: no dev_code from auth/request ({status}: {body})")
        status, body = self.call("POST", f"{self.scope}/auth/verify", {"code": code})
        if status >= 300:
            raise SystemExit(f"sign-in: verify refused ({status}: {body})")

    def register_mcp(self, name: str, endpoint: str) -> tuple[int, Any]:
        """Point one declared server at this run's loopback role server.

        Runtime is the only layer that accepts an ``http://`` endpoint — a
        server declared in a bundle's ``mcp.json`` must be ``https``. That is
        why this bundle ships all five disabled and pointing at placeholders.

        **PUT, not POST.** The name is already declared, so adding it is a 409
        telling you to override instead — and a POST that 409s leaves the desks
        holding the shipped, disabled placeholder and deliberating confidently
        with no tools at all.
        """
        status, body = self.call(
            "PUT",
            f"{self.scope}/mcp/servers/{urllib.parse.quote(name)}",
            {"endpoint": endpoint, "enabled": True},
        )
        if status < 300:
            return status, body
        return self.call(
            "POST", f"{self.scope}/mcp/servers", {"name": name, "endpoint": endpoint}
        )

    def activity(self, desk: str) -> tuple[int, bool]:
        """How many rows this channel holds, and whether a room has closed on it.

        The settle loop's event source. A closing report is journaled under the
        reserved `hive-report` author when an episode ends — whatever it ended
        as — so its arrival means the room is done and further waiting buys
        nothing. The count catches the other shape: a single-responder desk, or
        a referral answering on a channel with no room at all.
        """
        # `desk=`, not `chat=`. The handler's query struct is `#[serde(default)]`,
        # so an unknown parameter is dropped in silence and `desk` falls back to
        # the General line: `?chat=anything`, including a desk that does not
        # exist, returns General's transcript with a 200. This loop was polling
        # General on every tick while claiming to watch the desk it settles on.
        status, body = self.call(
            "GET", f"{self.scope}/chat/history?desk={urllib.parse.quote(desk)}"
        )
        if status != 200 or not isinstance(body, list):
            return (0, False)
        closed = any((m.get("channel") or m.get("from") or "") == "hive-report" for m in body)
        return (len(body), closed)

    def say(self, desk: str, text: str, timeout: float = 3600, parent: str | None = None):
        """Put one message to `desk`, holding the POST open for the turn.

        `parent` threads this message onto an earlier one. Without it every turn
        is a new line in the channel, and `reply_thread` roots each one on
        itself — "N messages would mean N threads instead of N *topics*". A
        follow-up then opens its OWN episode, deriving a fresh topic id from its
        own words (observed: a confirmation produced the topic `#yes-i`, a room
        deliberating about the word "yes"), and the prior exchange is demoted to
        the cross-thread index, which the agent is told not to read: "do NOT
        read or answer from them unless this message explicitly refers to one".
        So "go ahead as you described" pointed at a conversation the room was
        barred from consulting.

        Threaded, the follow-up lands inside the room that asked, that room
        keeps its own transcript in view, and the seat that made the decision is
        the one that acts on the answer.
        """
        body = {"text": text, "chat": desk}
        if parent is not None:
            body["parent"] = str(parent)
        return self.call("POST", f"{self.scope}/chat", body, timeout=timeout)


def load_tasks(data_dir: Path, domain: str) -> list[dict]:
    path = data_dir / "tau2" / "domains" / domain / "tasks.json"
    if not path.exists():
        raise SystemExit(
            f"no tau2 task file at {path}\n"
            "Pass --tau2 pointing at the opencompany-tau2 checkout's "
            "vendor/tau2-bench/data directory."
        )
    raw = json.loads(path.read_text())
    return raw if isinstance(raw, list) else raw.get("tasks", [])


# tau2 writes `user_scenario.instructions` in the second person, because they are
# directions to ITS user simulator — "You are Yusuf Rossi", "you wish to
# exchange". Handed to an agent verbatim they read as stage directions, and the
# agent answers them as such: the first run of this script had `triage` reply
# "You said: You are Yusuf Rossi in zip code 19122…" and nothing else.
#
# This flips them to first person so the desk receives something a customer
# could plausibly have written. It is a crude stand-in for the user simulator,
# and only for the OPENING message — a task needing genuine back-and-forth (a
# confirmation, a preference the desk has to ask for) still needs the simulator
# wired in as a participant. See `--help`.
_PERSON = [
    (r"\byou'd\b", "I'd"), (r"\bYou'd\b", "I'd"),
    (r"\byou're\b", "I'm"), (r"\bYou're\b", "I'm"),
    (r"\byou've\b", "I've"), (r"\bYou've\b", "I've"),
    (r"\byou are\b", "I am"), (r"\bYou are\b", "I am"),
    (r"\byou have\b", "I have"), (r"\bYou have\b", "I have"),
    (r"\byou wish\b", "I wish"), (r"\bYou wish\b", "I wish"),
    (r"\byou want\b", "I want"), (r"\bYou want\b", "I want"),
    (r"\byourself\b", "myself"), (r"\byours\b", "mine"),
    (r"\byour\b", "my"), (r"\bYour\b", "My"),
    (r"\byou\b", "I"), (r"\bYou\b", "I"),
]


def as_customer(text: str) -> str:
    """tau2's second-person directions, rewritten as the customer's own words."""
    for pattern, repl in _PERSON:
        text = re.sub(pattern, repl, text)
    # "to I" / "for I" — the object case the blunt swap above gets wrong.
    text = re.sub(r"\b(to|for|with|at|from|of) I\b", r"\1 me", text)
    return text


def opening_message(task: dict) -> str:
    """The customer's first line, as tau2 states it.

    `known_info` carries the identity the agent has to establish (name, zip),
    which a real customer would volunteer; `reason_for_call` is what they want.
    """
    ui = (task.get("user_scenario") or {}).get("instructions") or {}
    known = as_customer((ui.get("known_info") or "").strip())
    reason = as_customer((ui.get("reason_for_call") or "").strip())
    return f"{known}\n\n{reason}".strip() if known else reason


# tau2's retail and airline policies require the agent to state the action and
# get an explicit "yes" before any write, so a one-turn replay cannot complete
# those tasks however well the desks behave: the room correctly stops and asks.
# tau2 answers that with its user simulator; this is the bounded stand-in.
#
# It is deliberately dumb — it confirms what the desk proposed, and says nothing
# the task did not already state. A task needing a genuine CHOICE from the user
# (which of two variants, refund or exchange) is not completable this way and
# will fail here; that is the honest result, not something to paper over with a
# cleverer script.
FOLLOW_UP = (
    "Yes — I confirm, go ahead exactly as you described. "
    "Use the original payment method on the order. I have nothing to add."
)


def communicated(task: dict, replies: list[str]) -> tuple[list[str], list[str]]:
    """Which `communicate_info` strings actually reached the customer.

    tau2 grades most tasks on ``reward_basis = ["DB", "NL_ASSERTION"]``: the
    database end state AND natural-language assertions about what the agent
    said, the latter judged by an LLM in tau2's own harness. This checks neither
    — it is a literal substring test over what the desks replied, which is a
    deterministic PROXY for the communication half and nothing more. A task can
    pass here and still fail tau2's judge.
    """
    want = [str(x) for x in ((task.get("evaluation_criteria") or {}).get("communicate_info") or [])]
    blob = "\n".join(r or "" for r in replies)
    return want, [w for w in want if w not in blob]


def grade(task: dict, state: dict, spec: dict) -> tuple[bool, str]:
    """Compare the shared database against tau2's expected end state.

    Only the write actions are graded. Reads are how an agent gets there, and
    tau2's own scoring does not require a particular path through them — two
    correct handlings can read different things.
    """
    wants = [a for a in ((task.get("evaluation_criteria") or {}).get("actions") or [])
             if a.get("name") in spec["writes"]]
    if not wants:
        return True, "no gradeable write expected"

    rows = state.get(spec["collection"]) or {}
    problems = []
    for want in wants:
        name = want["name"]
        rule = spec["writes"][name]
        args = want.get("arguments") or {}
        rid = args.get(spec["key"])
        row = rows.get(rid)

        if rule.get("absent"):
            if row is not None:
                problems.append(f"{name}: {rid} is still present")
            continue
        if row is None:
            problems.append(f"{name}: {rid} not in {spec['collection']}")
            continue
        if rule.get("present"):
            continue

        want_status = rule.get("status")
        if want_status is not None and row.get("status") != want_status:
            problems.append(f"{name}: status={row.get('status')!r}, wanted {want_status!r}")
        for field, arg in (rule.get("fields") or {}).items():
            if arg in args and row.get(field) != args[arg]:
                problems.append(f"{name}: {field} does not match {arg}")
        # `flights` is asserted as a list of {flight_number, date} pairs; the row
        # stores richer objects, so compare only the keys tau2 named.
        if "flights" in rule and "flights" in args:
            got = [{k: f.get(k) for k in ("flight_number", "date")}
                   for f in (row.get("flights") or [])]
            want_f = [{k: f.get(k) for k in ("flight_number", "date")} for f in args["flights"]]
            if got != want_f:
                problems.append(f"{name}: flights do not match")

    return (not problems), "; ".join(problems) or "matches"


def check(host: "Host", spec: dict, domain: str, state_path: Path) -> int:
    """Preflight: assert every layer this run depends on, spending no model call.

    Each of these has failed silently at least once while this bundle was being
    built, and each looked like a model problem from the transcript alone:

    * a role server reading a state file deleted under it — every tool call came
      back `Error executing tool`, which reads as the agent using them wrong;
    * `mcp.json` declaring server names the runner never registered, so a desk
      deliberated confidently with no tools at all;
    * a scope quietly widening, which makes a pass meaningless rather than loud;
    * no inference key, which surfaces as a 401 on the first turn rather than at
      boot;
    * a desk with one member where two were intended — `deliberates()` needs
      two, so the room never convenes and one seat decides alone.
    """
    bad = 0

    def ok(label: str, good: bool, detail: str = "") -> None:
        nonlocal bad
        bad += 0 if good else 1
        mark = "ok  " if good else "FAIL"
        print(f"  [{mark}] {label}{(' — ' + detail) if detail else ''}")

    print(f"role servers ({domain})")
    for seat, (port, want_tools, want_mut) in spec["seats"].items():
        url = f"http://127.0.0.1:{port}/mcp"
        body = json.dumps({"jsonrpc": "2.0", "id": 1,
                           "method": "tools/list", "params": {}}).encode()
        req = urllib.request.Request(url, data=body, method="POST")
        req.add_header("content-type", "application/json")
        req.add_header("accept", "application/json, text/event-stream")
        try:
            with urllib.request.urlopen(req, timeout=20) as resp:
                raw = resp.read().decode(errors="replace")
        except Exception as err:  # noqa: BLE001 — any failure is the same verdict
            ok(f"{seat} :{port}", False, f"unreachable ({err})")
            continue
        tools = len(re.findall(r'"name":"[a-z_]+"', raw))
        mut = raw.count("write/mutates")
        ok(f"{seat} :{port}", tools == want_tools and mut == want_mut,
           f"{tools} tools / {mut} mutating, wanted {want_tools}/{want_mut}")

    print("company")
    status, desks = host.call("GET", f"{host.scope}/desks")
    ok("desks readable", status == 200, f"HTTP {status}")
    if status == 200 and isinstance(desks, list):
        got = {d.get("id"): sorted(d.get("members") or []) for d in desks}
        for desk, members in spec["desks"].items():
            ok(f"desk {desk}", got.get(desk) == sorted(members),
               f"members={got.get(desk)}, wanted {sorted(members)}")
            if len(members) >= 2:
                ok(f"desk {desk} can deliberate", len(got.get(desk) or []) >= 2,
                   "needs two seats")

    print("mcp wiring")
    status, servers = host.call("GET", f"{host.scope}/mcp/servers")
    if status != 200 or not isinstance(servers, list):
        ok("server list", False, f"HTTP {status}")
    else:
        by_name = {x.get("name"): x for x in servers}
        for seat in spec["seats"]:
            name = f"tau2-{domain}-{seat}"
            row = by_name.get(name)
            live = bool(row and row.get("enabled"))
            ok(f"{name} enabled", live,
               "" if live else ("not registered" if row is None else "declared but disabled"))
            if row and row.get("enabled"):
                st, tools = host.call("GET", f"{host.scope}/mcp/servers/{urllib.parse.quote(name)}/tools")
                ok(f"{name} reachable from the host", st == 200 and isinstance(tools, list),
                   f"HTTP {st}")

    print("inference")
    status, body = host.call("POST", f"{host.scope}/inference/test", {}, timeout=120)
    fine = status == 200 and isinstance(body, dict) and body.get("ok")
    ok("credential probes clean", bool(fine),
       (body or {}).get("error", f"HTTP {status}") if not fine else "")

    print("tau2 state")
    ok(f"{state_path} present", state_path.exists(),
       "delete it AND restart the servers to reseed — they seed at boot")

    print(f"\n{'all checks passed' if not bad else str(bad) + ' check(s) failed'}")
    return bad


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("--domain", default="retail", help="tau2 domain: " + ", ".join(DOMAINS))
    ap.add_argument("--base", default="http://127.0.0.1:8080", help="running `opencompany serve`")
    ap.add_argument("--tau2", type=Path, default=Path("../opencompany-tau2"),
                    help="the opencompany-tau2 checkout (tasks + shared state)")
    ap.add_argument("--check", action="store_true",
                    help="verify servers, desks, mcp wiring, credential and state, "
                         "then exit — spends no model call")
    ap.add_argument("--task", help="one task id")
    ap.add_argument("--tasks", help="comma-separated task ids")
    ap.add_argument("--host", default="127.0.0.1", help="host the role servers bound to")
    ap.add_argument("--out", type=Path, help="write the run as JSON")
    ap.add_argument("--settle", type=float, default=240,
                    help="seconds to wait after each turn for a detached referral "
                         "to land before grading (the far desk runs after the POST "
                         "returns)")
    ap.add_argument("--quiet", type=float, default=60,
                    help="stop settling once the channel has been silent this "
                         "long — the work is not coming")
    ap.add_argument("--turns", type=int, default=8,
                    help="MAX turns per task, not a target — the loop stops as soon "
                         "as the end state matches. Turn 2+ sends the canned "
                         "confirmation (a stand-in for tau2's user simulator). A "
                         "low cap silently fails every task whose policy demands "
                         "confirmation before a write, which is most of them: the "
                         "desk asks, nobody answers, and it reads as a refusal to act")
    ap.add_argument("--timeout", type=float, default=3600, help="seconds to hold one turn open")
    args = ap.parse_args()

    if args.domain in UNSUPPORTED:
        raise SystemExit(f"{args.domain}: {UNSUPPORTED[args.domain]}")
    if args.domain not in DOMAINS:
        ap.error(f"unknown domain {args.domain!r}; known: {', '.join(DOMAINS)}")
    spec = DOMAINS[args.domain]

    state_path = args.tau2 / spec["state"]
    host = Host(args.base, spec["company"])

    if args.check:
        host.sign_in()
        for seat, (port, _t, _m) in spec["seats"].items():
            host.register_mcp(f"tau2-{args.domain}-{seat}", f"http://{args.host}:{port}/mcp")
        return check(host, spec, args.domain, state_path)

    ids = []
    if args.task:
        ids = [args.task]
    if args.tasks:
        ids += [t.strip() for t in args.tasks.split(",") if t.strip()]
    if not ids:
        ap.error("pass --task or --tasks")

    tasks = {str(t["id"]): t
             for t in load_tasks(args.tau2 / "vendor" / "tau2-bench" / "data", args.domain)}
    missing = [i for i in ids if i not in tasks]
    if missing:
        raise SystemExit(f"no such {args.domain} task(s): {', '.join(missing)}")

    host.sign_in()
    for seat, (port, _tools, _mut) in spec["seats"].items():
        name = f"tau2-{args.domain}-{seat}"
        status, body = host.register_mcp(name, f"http://{args.host}:{port}/mcp")
        if status >= 300:
            raise SystemExit(f"could not register {name}: {status} {body}")
    print(f"registered {len(spec['seats'])} role servers for {args.domain}", file=sys.stderr)

    results = []
    failed = 0
    for tid in ids:
        task = tasks[tid]
        text = opening_message(task)
        print(f"\n=== {args.domain} task {tid} ===\n{text}\n", file=sys.stderr)

        turns = []
        thread = None
        ok, why = False, "no turn ran"
        for turn in range(1, args.turns + 1):
            status, body = host.say(spec["entry"], text, timeout=args.timeout, parent=thread)
            # The first message roots the thread; every follow-up joins it.
            if thread is None and isinstance(body, dict):
                thread = body.get("messageId")
            replies = ([r.get("text") for r in (body or {}).get("responses", [])]
                       if isinstance(body, dict) else [])
            for r in replies:
                print(f"  [{spec['entry']}] {r}", file=sys.stderr)
            turns.append({"turn": turn, "status": status, "sent": text,
                          "thread": thread, "replies": replies})

            # A referral is DETACHED — `spawn_referred_turn` puts the question on
            # the other desk's channel and returns; the POST answering here does
            # not wait for that room to finish. Grading the instant the POST
            # returns therefore races the work it is grading.
            #
            # Event-driven rather than a fixed wait. Sitting out the whole
            # `--settle` budget is only correct when the work is still coming;
            # when it is not, it is dead time, and it dominated the wall clock
            # of every failing run — two turns of a 600s budget is twenty
            # minutes spent waiting for something that already was not going to
            # happen. So stop on the first of: the state matching, the room
            # closing (a `hive-report` row), or the channel going quiet.
            ok, why = False, "no state yet"
            deadline = time.monotonic() + args.settle
            last_seen, quiet_since = None, time.monotonic()
            while True:
                state = json.loads(state_path.read_text()) if state_path.exists() else {}
                ok, why = grade(task, state, spec)
                if ok:
                    break
                rows, closed = host.activity(spec["entry"])
                if rows != last_seen:
                    last_seen, quiet_since = rows, time.monotonic()
                if closed:
                    why += " (the room closed)"
                    break
                if time.monotonic() - quiet_since >= args.quiet:
                    why += f" (nothing journaled for {args.quiet:.0f}s)"
                    break
                if time.monotonic() >= deadline:
                    why += " (settle budget spent)"
                    break
                time.sleep(5)
            if ok:
                break
            if turn < args.turns:
                # The desk is most likely holding for the confirmation its policy
                # demands. Answer it once and let it act.
                print(f"  … not settled ({why}); confirming", file=sys.stderr)
                text = FOLLOW_UP

        said = [r for t in turns for r in (t["replies"] or [])]
        want_info, missing_info = communicated(task, said)
        basis = (task.get("evaluation_criteria") or {}).get("reward_basis") or []

        failed += 0 if ok else 1
        print(f"  -> DB {'PASS' if ok else 'FAIL'} ({why}) in {len(turns)} turn(s)", file=sys.stderr)
        if want_info:
            print(f"     communicate_info {len(want_info) - len(missing_info)}/{len(want_info)}"
                  + (f", missing {missing_info}" if missing_info else ""), file=sys.stderr)
        if "NL_ASSERTION" in basis:
            print("     note: tau2 also scores NL_ASSERTION here, which needs its judge",
                  file=sys.stderr)

        results.append({
            "id": tid,
            # DB only. Named so a reader cannot mistake it for tau2's reward.
            "db_passed": ok,
            "db_detail": why,
            "reward_basis": basis,
            "communicate_info": {"expected": want_info, "missing": missing_info},
            "scored_here": "DB end state, plus a literal substring proxy for "
                           "communicate_info. NL_ASSERTION is NOT scored — it needs "
                           "tau2's LLM judge over the transcript.",
            "turns": turns,
        })

    if args.out:
        args.out.write_text(json.dumps({"domain": args.domain, "results": results}, indent=2) + "\n")
        print(f"\nwrote {args.out}", file=sys.stderr)
    print(f"\n{len(ids) - failed}/{len(ids)} passed", file=sys.stderr)
    return failed


if __name__ == "__main__":
    raise SystemExit(main())
