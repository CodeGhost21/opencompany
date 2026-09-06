# Hive-mind integration: handoff (2026-09-06)

State of the `hivemind` branch for whoever picks this up on another machine.
Draft PR: https://github.com/tinyhumansai/opencompany/pull/2097 (fork branch
`senamakel:hivemind` → `tinyhumansai/opencompany` `main`). The PR body is the
long-form summary of what was built and why; this file is the operational
state.

## Where things stand

- **Merge of `upstream/main` (69324110b, 764 commits) is in progress and
  uncommitted** in the worktree `worktrees/hivemind` on the original box.
  The five source conflicts (`src/harness/built_in/brain.rs`,
  `src/harness/built_in/mod.rs`, `src/runtime/delegation.rs`,
  `src/server/ops/memory_engine.rs`, `src/store/memory/driver.rs`) and
  `Cargo.lock` were resolved by a merge-conflict-resolver agent; the merge
  commit was not made before the session ran out. The last pushed commit is
  `b17625a27` (pre-merge). Upstream moved the `vendor/openhuman` gitlink to
  `b7f4e9490`; `vendor/tinyhivemind` must stay at `e272c84` (tinyhivemind
  `main`).
- If you start from a fresh clone: `git fetch upstream && git merge
  upstream/main` on `hivemind` and redo the five conflicts. What must survive
  from our side is listed in the PR body under "Summary"; in short: the hive
  hook and `room_answered` flag in brain.rs, `ChatTarget::deliberating` +
  `history_seed` in delegation.rs and the `seed_chat` gate in mod.rs, the
  `cortexdb` arm / `SUPPORTED_REMOTE_DRIVERS` / actor helpers in driver.rs and
  the CortexDB tile in memory_engine.rs (order must match the driver list),
  `output_cap` / `OPENCOMPANY_INFERENCE_MAX_TOKENS` in provider.rs.
- After the merge: run `git submodule update --init --recursive`, then the
  gates below. An OpenHuman bump is a migration (harness APIs move); fix our
  side, never `vendor/`.

## Gates (all green on the pre-merge tree)

```
cargo fmt --all -- --check
cargo clippy --all-targets --features openhuman -- -D warnings
cargo clippy --all-targets -- -D warnings
cargo test --features openhuman            # 6394 lib + every tests/ target
scripts/ci/assert-feature-lanes.sh
scripts/ci/assert-integration-targets-run.sh openhuman
```

Focused targets: `cargo test --features openhuman hivemind` (40),
`--test hivemind_e2e` (8), `brain::` (208), `store::memory` (cortexdb 10),
`memory_engine`, `manifest`, `--lib content_test`.

Long foreground commands get reaped by the harness on that box; detach with
`setsid nohup … > file &` and poll the file. `pkill -f <pattern>` matches the
shell's own command line and kills it (exit 144); kill by pid instead. The
shell cwd resets to the primary checkout between commands, so `cd` into the
worktree first.

## Live run recipe

```
scripts/cortexdb-up.sh                                  # container opencompany-cortexdb on 127.0.0.1:3141
set -a; . ~/.config/opencompany/cortexdb.env; set +a    # CORTEX_API_KEY
OPENCOMPANY_MEMORY=remote OPENCOMPANY_MEMORY_DRIVER=cortexdb \
OPENCOMPANY_MEMORY_URL=http://127.0.0.1:3141 OPENCOMPANY_MEMORY_API_KEY=$CORTEX_API_KEY \
OPENCOMPANY_MEMORY_ACTOR=service:opencompany \
OPENCOMPANY_INFERENCE_URL=http://127.0.0.1:6969/v1 OPENCOMPANY_INFERENCE_KEY=$LADDER_API_KEY \
OPENCOMPANY_INFERENCE_MAX_TOKENS=65536 OPENHUMAN_AGENT_TURN_TIMEOUT_SECS=1500 \
OPENCOMPANY_AUTH_MODE=none OPENCOMPANY_DATA_DIR=$HOME/.opencompany/hive-live-2 \
OPENCOMPANY_BIND=127.0.0.1:8080 \
  ./target/debug/opencompany serve --company companies/hive_math_lab
python3 scripts/hive-euler.py --problems 233,249,301,345 --timeout 5400 --out euler.json
```

The inference endpoint is the local llm-ladder-router (`ladder` container,
`~/.config/ladder/config.toml`), which now serves `chat-v1`, `reasoning-v1`,
`agentic-v1`, `vision-v1` as aliases of `flash` / `reasoning` /
`max-reasoning`, so per-tier models work without `OPENCOMPANY_INFERENCE_MODEL`.
A BYOK `[inference] provider` in the manifest would need its key in the secret
store (`inference/key`); the env path is simpler for a lab.

Approvals: `shell` never parked on this policy (`mode = "full"`) during any
run; the driver still pumps `GET/POST /api/v1/company/approvals` concurrently
with the blocking chat POST.

## Live results so far

| run | seats | problems | result |
|---|---|---|---|
| 1 | 3, one model, anyone may propose | 1,5,12,14,31,60,92,100,145 | 8 correct in 4 turns each; 145 lost to the 10-min turn ceiling. Three independent proposals + vote every time, no cross-agent moves. |
| 2 | 6, `moves`, `require_evidential`, quorum 3, per-tier models | 12 | Exhausted: quorum reached at the 3rd evidential support, then the fold gave the Commit floor to seats barred from `!commit` for 8 turns. Fixed: commit is never gated. |
| 3 | same + commit fix + topic-id/citation discipline | 12,145,206,214 | all correct, 9 turns each (4 evidence, 2 support, 1 defer, 1 commit; supports cite evidence; verifier corrected a wrong figure; archivist defers with memory). |
| 3 | | 233 | strong seats lost turns: theorist and verifier to the 16k output cap (`finish_reason: length`), programmer to the 25-min ceiling; room continued; run was cut by session restarts before a close. |
| 4 | + `OPENCOMPANY_INFERENCE_MAX_TOKENS=65536` | 233,249,301,345 | started 2026-09-06 ~12:10 local on the original box; check `~/.opencompany/hive-live-2` journal / the PR for outcome. |

Transcripts are readable with
`GET /api/v1/company/chat/history?desk=solvers&limit=200` on a running serve
over the same data dir.

## What the hive does and does not do yet

Does: one shared transcript per desk; salience-bid floor; blind opening round;
evidence/support/object/defer/pin grammar with per-member gating; evidential
quorum; commit phase open to all; failed turns tolerated; desk memory recalled
before and written after each episode via CortexDB; per-seat model tiers.

Does not: mention dispatch (`MentionTurnQueue`) or cross-desk referral
(`ReferralQueue`) — an `@mention` is text the room reads, not a dispatched
turn. `!object` traffic has not appeared live because every seat solved the
rungs alone; it needs problems where seats disagree.

## Suggested next steps

1. Commit the merge (or redo it), rebuild, rerun `--problems 233,249,301,345`
   and paste the rows into the PR body; mark the PR ready.
2. Wire `MentionTurnQueue` and `ReferralQueue` so `@#records` / `@#desk`
   leaves the room with one answer carried back (tinyhivemind wiki:
   Cross-desk-referral, Mentions).
3. Add a "trap" problem set where a literal misreading gives a different
   integer, to exercise the skeptic's `!object` path.
4. Consider `reasoning_effort` on the ladder's `agentic-v1` alias if
   `finish_reason: length` recurs even at 65536.
