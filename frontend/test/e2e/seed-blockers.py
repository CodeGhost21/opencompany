#!/usr/bin/env python3
"""Seed four real parked workflow-node blockers into a host's journal.

One per verdict, each on its own run of the harness's `committed` workflow, and
each with the blocked-node stash the resume needs — so a retry/amend/skip really
does re-enter the node and a cancel really does settle the run.
"""
import json
import sys
import time

data_dir = sys.argv[1]
company = sys.argv[2] if len(sys.argv) > 2 else "e2e-harness-co"
journal = f"{data_dir}/companies/{company}/journal.jsonl"

now = int(time.time() * 1000)
lines = []
for n, verdict in enumerate(["retry", "amend", "skip", "cancel"], start=1):
    run_id = f"live-2028-{verdict}"
    node_id = "draft"
    payload = {
        "kind": "infrastructure",
        "source": "provider",
        "step": {"step": "node", "run_id": run_id, "node_id": node_id},
        "reason": f"the model id `gpt-nope` was rejected ({verdict} case)",
        "needed": "a model id this provider serves",
    }
    effect = {
        "kind": "blocker.infrastructure",
        "group": "other",
        "amount_usd": None,
        "established_thread": False,
        "first_time_counterparty": False,
        "payload": payload,
        "run_id": run_id,
    }
    lines.append({
        "record": "ApprovalParked",
        "id": f"live-2028-{verdict}",
        "effect": effect,
        "at_millis": now - 30_000 - n,
        "task": {"link": "unlinked"},
        # The turn key the park belongs to. Without it the boot reconciler sees
        # a stash with nothing parked against it and retires it as stranded.
        "cycle": f"workflow-node:{run_id}:{node_id}",
    })
    lines.append({
        "record": "BlockedNodeStashed",
        "turn": f"workflow-node:{run_id}:{node_id}",
        "workflow_id": "committed",
        "input": {"topic": "quarterly numbers"},
        "started_by": "operator",
        "at_millis": now - 30_000 - n,
    })

with open(journal, "a") as f:
    for line in lines:
        f.write(json.dumps(line) + "\n")
print(f"seeded {len(lines)} records into {journal}")
