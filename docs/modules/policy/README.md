# Policy Module

The policy module owns the default `ApprovalGate` and the durable approval
queue (`park` / `resolve`). Policy-generated HITL is currently disabled:
production evaluation allows effects that previously became
`RequireApproval`, while hard emergency-stop denials remain enforced.

Approval cards now come from explicit `park_effect` calls. For agent work, that
means the intrinsic `request_approval` tool; ordinary tool calls are not
silently converted into operator prompts.

There are two ways onto that queue, both landing in `CycleHostImpl::park` (so a
parked effect is journaled one way and survives a restart with its original id):

- `CycleHost::emit_effect` still evaluates an effect, but production policy no
  longer returns `RequireApproval`.
- `CycleHost::park_effect` explicitly puts a request in the inbox. The harness
  uses it only after an agent called `request_approval`. See
  [the OpenHuman module](../openhuman/README.md#explicit-approval-requests).

The historical checkpoint classification remains available for audit and
irreversible-effect reporting, but no longer creates HITL. Explicitly parked
approvals **default-deny on silence**: they expire to `deny` after a configurable window
(default 7 days) measured against an injectable clock. The operator may **edit**
a parked effect's payload and approve the amended version; the follow-up cycle
shows the brain both the original and the edit.

## Emergency stop

`ManifestApprovalGate` carries an `AtomicBool` kill switch, checked by
`evaluate` **before** every policy rule including `always_approve`. While it is
engaged, any effect outside `EffectGroup::Other` is `Deny` — not
`RequireApproval`, so the approval queue cannot be used to work around the
switch. `Other` is exempt so chat keeps working.

The durable state is the event log, not a record field: `replayed_emergency`
scans for the last `CompanyEvent::EmergencyPauseChanged` at boot and
`CompanyRuntime::hydrate_emergency` seeds the flag from it, **failing safe to
stopped** if the log cannot be read. The switch is untouched by `sweep_expired`
— it has no TTL and never auto-releases.

Full normative rules, including the asymmetric confirmation on the two REST
routes, are in
[`docs/spec/company-brain/approvals.md`](../../spec/company-brain/approvals.md#emergency-stop-the-governance-kill-switch).
