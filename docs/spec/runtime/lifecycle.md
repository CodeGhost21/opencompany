# Company Lifecycle

The state machine every [Company](../glossary.md) moves through, and the
durability guarantees the runtime makes at each step.

## States

```text
drafted ──▶ onboarding ──▶ live ◀──▶ paused
                            │
                            ├──▶ suspended (platform-initiated)
                            └──▶ archived  (terminal; export retained)
```

| State | Meaning | Who triggers transitions |
| --- | --- | --- |
| `drafted` | Manifest exists and validates; nothing runs. | Operator/platform creates |
| `onboarding` | The brain runs the interview; Charter filling in. | Operator completes or skips |
| `live` | Events flow, cycles run, effects execute. | — |
| `paused` | Intake stopped by the Operator or a tripped budget cap; state preserved. | Operator, or kernel on `[budget]` breach |
| `suspended` | Platform-forced pause (quota, billing, abuse). | Platform operator only |
| `archived` | Terminal. Bundle exported and retained; handle renewal stops. | Operator/platform |

All transitions are recorded as events in the `EventLog` with the acting
`Actor`.

## Boot sequence (`opencompany serve --company <dir>`)

1. **Parse + validate** the manifest; materialize or refresh the
   `CompanyRecord` in `CompanyStore`. The manifest is the source of truth for
   charter/roster seeds; runtime state layers on top
   ([manifest.md](manifest.md), provenance).
2. **Open stores** (fs defaults unless the builder swapped them); **replay
   the `EventLog` tail** to rebuild in-flight tasks and the approval queue.
3. **Economy (optional)**: if `[place].discoverable`, load or generate the
   Ed25519 keypair, then `AgentEconomy::ensure_registered` and
   `publish_card`. Failures degrade to "not discoverable" with a warning —
   they MUST NOT block boot.
4. **Start** channel adapters, the cron scheduler, the feedback poller; mount
   routes ([api.md](api.md)).

Platform mode (`--companies-root <dir>` or provisioning API) runs this
sequence per company under the `CompanyRegistry`.

## Event → cycle loop

All stimuli normalize to `CompanyEvent { source, kind, payload, correlation }`
(variants in [events.md](events.md)). Per company there is **one serial cycle
queue** — one cycle at a time, events batched/debounced between cycles;
distinct companies run concurrently. A cycle
([company-brain/README.md](../company-brain/README.md)):

1. Drain pending events for the company.
2. Load working memory (`MemoryStore::recent_traces`), context index, roster
   + charter.
3. `Brain::run_cycle`, servicing callbacks: tool calls → `ToolProvider`
   (grant-checked), context ops → `ContextStore`, effects →
   `ApprovalGate::evaluate`.
4. Effects: `Allow` executes; `RequireApproval` parks
   (`EffectDisposition::PendingApproval`) and surfaces in the operator's
   approvals inbox; `Deny` returns to the brain as a refusal it can plan
   around.
5. Persist: compressed traces → `MemoryStore`, events/effects → `EventLog`,
   ledger deltas → `CompanyStore`. Cycle results stream to API subscribers
   over SSE.

Resolving an approval emits `ApprovalResolved`, which schedules a follow-up
cycle so the brain learns the verdict — approve executes the parked effect,
deny makes the brain replan.

## Durability guarantees

Must survive a crash/restart with no operator-visible loss:

- Charter and roster (CompanyStore)
- The event log and everything derivable from replaying it (in-flight tasks,
  approval queue)
- The ledger (append-only; never rewritten)
- Compressed memory traces and context chunks
- The company keypair and secrets

In-flight cycle work is **not** guaranteed: a crash mid-cycle loses that
cycle's partial passes; the unhandled events remain queued and the next boot
re-runs them. Effects are executed at-most-once by the kernel — an effect is
journaled *before* execution and marked after, so replay never re-fires a
completed effect.

### Which crash a journal record survives (issue #392)

The runtime journal's durability is chosen **per record kind**, not once for the
file. Each `JournalRecord` declares it, and `RuntimeJournal::append` — the single
choke point every record goes through — honours the declaration:

| Records | Survives | Why |
|---|---|---|
| `EffectExecuted`, `GrantConsumed`, `StandingGrantRevoked` | **Host crash / power loss.** Flushed to stable storage (`sync_data`, plus every directory the append creates) before the append returns. | These are the only kinds whose loss makes the runtime **repeat an external action**. `EffectExecuted` is written immediately before the side effect, so losing it re-fires that effect on the next boot; losing a `GrantConsumed` keeps the `ApprovalGranted` that minted the grant and drops its redemption, returning a spent single-use grant to the live set where it admits the identical call again with no new approval card; losing a revocation silently re-arms a standing grant an operator took back, letting it keep admitting calls until its own deadline. |
| The other ten | **Process crash.** In the kernel page cache when the append returns; a host crash can lose them. | Losing any of them makes the runtime **re-ask**, never re-fire: an approval is parked again, an operator is prompted again, a cycle bracket reads as interrupted. That is the safe direction, and it is a decision rather than an omission. |

`GrantConsumed`'s flush narrows its window rather than closing it, and is not
claimed to do more. Redemption happens inside a synchronous `ToolPolicy::check`
that holds no journal handle, so the id is buffered on the grant set and written
when the cycle it belongs to ends; a crash inside *that* gap loses the record
before any append is reached. The flush removes the half the journal controls —
a record that was written but only page-cached. Closing the other half means
recording the redemption where it happens, which is a change to the tool-policy
seam rather than to durability.

The split is affordable because the frequency runs the helpful way. The three
flushed kinds are written at operator-decision scale — `EffectExecuted` sits in
front of a network call costing 100ms–2s, so a flush ahead of it is invisible —
while the highest-volume records (`CycleStarted`/`CycleFinished`, a pair per
cycle) are pure observability. A blanket flush would tax the hottest cosmetic
record to protect the rarest dangerous one.

A failed flush **fails the append**, which aborts the effect before it runs: no
record means no effect, so the failure cannot produce the duplicate it guards
against. The flush is never retried, because a failed `fsync` may already have
dropped the dirty pages and a retry would report success over lost data.

**The volume underneath used to bound this guarantee, and no longer does**
(issue #726). The journal was built on the filesystem path unconditionally,
outside the storage ports, so a hosted tenant whose `/data` is ephemeral scratch
— the documented arrangement under `OPENCOMPANY_STORAGE=mongodb` — lost its
journal to a container replacement and gained nothing from these flushes. The
sink is now a port (`JournalStore`), selected from the same backend handles as
every other durable store, and the two levels above travel through it: the fs
backend answers them with `sync_data` and a flushed directory chain, sqlite with
`synchronous=FULL` against `NORMAL`, mongodb with a `j:true` write concern
against the default. See [journal.md](journal.md).

## The fs bundle (default store)

```text
~/.opencompany/companies/<slug>/
├── company.toml        # materialized charter + roster (with provenance)
├── events.jsonl        # append-only event log
├── ledger.jsonl        # append-only money/usage journal
├── memory/             # compressed traces, task results
├── context/            # content-addressed chunks + index
├── keys/agent.ed25519  # company identity (0600)
└── secrets/            # encrypted at rest
```

Human-inspectable and git-friendly by design.

## Export / import

`opencompany export <company>` produces a tar of the bundle. For non-fs
stores, export is defined as *read everything through the four storage ports
and write the fs layout*; import is the inverse. This makes migration between
an end user's laptop and a platform host (or between two platform backends)
total by construction.

## Shutdown

On SIGINT/SIGTERM: stop intake, drain the in-flight cycle with a bounded
timeout, checkpoint stores, exit. The tiny.place Agent Card stays published
(the endpoint simply goes offline); liveness is a directory concern, not a
registration concern.

## Multi-company isolation

- Separate store namespaces per `CompanyId` (separate bundle dirs in fs mode;
  key-prefixing is NOT sufficient for operator-supplied stores — the traits
  take `CompanyId` explicitly so implementations can enforce isolation).
- Separate secrets (`SecretStore` scoping is per-company by signature).
- Separate budgets and ledgers; no cross-company tool grants.
- One brain session per company against the hosted backend
  ([integrations/medulla.md](../integrations/medulla.md), session mapping).
