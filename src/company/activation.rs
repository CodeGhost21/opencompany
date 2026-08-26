//! Activation-signal derivation (issue #1843): the shared "is this company
//! activated" answer the onboarding gate (#1844) and the week-1 nudge (#1845)
//! both read, so neither has to re-derive the funnel or invent its own step
//! vocabulary.
//!
//! Activation is three steps, each named by [`OnboardingStep`]:
//!
//! 1. **`NameConfirmed`** — [`CompanyRecord::name_confirmed`], written by a
//!    future console write path (#1844's confirm-name route). This module only
//!    reads it.
//! 2. **`IntegrationConnected`** — waived (vacuously complete) when the
//!    company's `[tools].allow` never grants the `composio` namespace at all
//!    (per
//!    [`grants_composio_explicit`](crate::company::grants_composio_explicit)) —
//!    several bundled companies drop `composio` on purpose
//!    (`companies/agentic_math_lab`, `companies/agentic_product_team`,
//!    `companies/agentic_research_lab`, `companies/openhuman_demo`,
//!    `companies/signals_opportunity_studio`), and requiring a connection no
//!    agent in that company could ever make would permanently block
//!    activation for every one of them (issue #1850 review). When the
//!    namespace *is* grantable, the step still requires an actual live
//!    Composio connection — a connection nobody granted the namespace for
//!    cannot be used by any agent, so it does not complete the step on its
//!    own either.
//! 3. **`WorkflowRunSucceeded`** — at least one real (non-dry) workflow run
//!    reached [`RunStatus::Succeeded`](crate::ports::runs::RunStatus::Succeeded).
//!    Answered by scanning the journal for a
//!    [`CompanyEvent::WorkflowRunFinished`] with no error and not cancelled —
//!    a dry run never journals that event at all
//!    (`WorkflowSpawn::spawn_admitted`, issue #542), so every entry this scan
//!    finds is, by construction, a real run.
//!
//! # The latch is the source of truth once set
//!
//! [`CompanyRecord::activation_completed_at`] is a one-way latch: once every
//! step has been true simultaneously, [`compute_and_latch`] stamps it and
//! [`ActivationStatus::is_activated`] answers `true` forever after — even if a
//! step's live signal later goes false again (a Composio connection gets
//! disconnected). This is deliberate, not an oversight: activation is asking
//! "did this operator ever clear onboarding", not "is onboarding state
//! currently intact", and the two questions have different answers on
//! purpose — the gate this feeds must not re-open for an operator who already
//! got past it.
//!
//! [`compute_and_latch`] short-circuits on an existing latch before doing any
//! IO beyond the record load, which is what keeps a poll from an activated
//! company cheap (no Composio call, no journal scan) and is the other half of
//! the monotonicity guarantee: nothing downstream of the latch can flip it
//! back.

use std::sync::Arc;

use crate::Result;
use crate::company::grants_composio_explicit;
use crate::error::OpenCompanyError;
use crate::ports::events::EventLog;
use crate::ports::now_millis;
use crate::ports::store::{CompanyStore, company_write_lock};
use crate::ports::types::{CompanyEvent, CompanyId, CompanyRecord, EventSeq};
use crate::ports::workflow_verdict::{RunVerdictFacts, WorkflowRunVerdict};

/// The three step answers plus the terminal latch, all as of one moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ActivationStatus {
    /// [`CompanyRecord::name_confirmed`], read verbatim.
    pub name_confirmed: bool,
    /// `true` when the company's manifest cannot grant the `composio`
    /// namespace at all (the step is waived — see the module docs), or when
    /// it can and the company both holds a live Composio connection and has
    /// explicitly granted that namespace.
    pub integration_connected: bool,
    /// Whether the journal shows a real (non-dry) workflow run that reached
    /// `succeeded`.
    pub workflow_run_succeeded: bool,
    /// [`CompanyRecord::activation_completed_at`], read verbatim. `Some` means
    /// the company activated at some point in the past — see
    /// [`Self::is_activated`] for why that does not require every step to
    /// currently read `true`.
    pub activation_completed_at: Option<u64>,
}

impl ActivationStatus {
    /// Whether every step is true **right now**. Not the same question as
    /// [`Self::is_activated`] — see the module docs' latch section. Exposed
    /// mainly for [`compute_and_latch`] to decide whether to stamp the latch;
    /// a reader wanting "is this company activated" should call
    /// [`Self::is_activated`] instead.
    pub fn all_steps_complete(&self) -> bool {
        self.name_confirmed && self.integration_connected && self.workflow_run_succeeded
    }

    /// Whether this company is activated: the latch if one is already set,
    /// otherwise whether every step currently reads true (the moment the latch
    /// itself would be set, for a caller computing this without persisting).
    pub fn is_activated(&self) -> bool {
        self.activation_completed_at.is_some() || self.all_steps_complete()
    }
}

/// Derives the three step booleans from already-fetched inputs. Pure and
/// synchronous on purpose: neither a live Composio connection lookup nor a
/// journal scan belongs in a function a unit test should be able to call with
/// a bare [`CompanyRecord`] and two booleans — see this module's own `test`
/// submodule for the permutation coverage that buys.
///
/// `has_composio_connection` is the caller's own answer to "does this company
/// hold at least one active Composio connection" (from
/// `list_connection_states`/`list_connections_detailed` on the live path, or a
/// fixture in a test) — fetching it is IO this function deliberately does not
/// perform.
pub(crate) fn derive_steps(
    record: &CompanyRecord,
    has_composio_connection: bool,
    workflow_run_succeeded: bool,
) -> ActivationStatus {
    // A company whose manifest never grants `composio` has no lever to ever
    // make this step true (issue #1850 review) — waive it rather than
    // permanently blocking activation for every bundled company that
    // deliberately omits the namespace. When it IS grantable, a connection
    // still has to actually exist — see the module docs' second bullet.
    let composio_grantable = grants_composio_explicit(&record.manifest.tools.allow);
    ActivationStatus {
        name_confirmed: record.name_confirmed,
        integration_connected: !composio_grantable || has_composio_connection,
        workflow_run_succeeded,
        activation_completed_at: record.activation_completed_at,
    }
}

/// Whether the company's journal shows at least one real (non-dry) workflow
/// run reaching `succeeded` — see the module docs for why a dry run can never
/// answer this `true`.
///
/// "Succeeded" means [`WorkflowRunVerdict::Ok`], not merely `error: None,
/// cancelled: false`: a run that paused for approval or blocked on a human
/// carries neither an error nor a cancellation, but
/// [`record_run_finished`](crate::runtime::record_run_finished) still journals
/// it with `error: None, cancelled: false` — the same shape as a run that
/// actually finished. Checking only those two fields let a run parked on
/// `pending_approvals`/`blocked_nodes` (verdict `AwaitingApproval` or
/// `Blocked`) complete this activation step and permanently latch it,
/// alongside the other two, before anything had actually run to completion.
/// Routing through the shared verdict ladder is what the console's Steps
/// panel and every other run reader already use to draw this exact line — see
/// [`crate::ports::workflow_verdict`]'s module docs for why a bespoke
/// derivation here would be a second, driftable transcription of the same
/// rule.
///
/// `stranded_approvals` is passed as `0`: a journal replay has no live
/// approval queue to reconcile pending approvals against, which is the
/// documented degrade [`RunVerdictFacts::stranded_approvals`] describes for
/// exactly this kind of caller. That can only under-count `Stranded` in favor
/// of `AwaitingApproval` — both outrank `Ok` — so it never turns a
/// not-yet-succeeded run into a succeeded one.
///
/// Reads the whole journal (`EventSeq::new(0)..`, matching the fallback
/// [`EventLog::read_before`] itself uses) rather than an indexed query,
/// because none exists — acceptable because [`compute_and_latch`] only ever
/// calls this while [`CompanyRecord::activation_completed_at`] is still
/// `None`, i.e. at most once per company between "created" and "activated",
/// never again after.
pub(crate) async fn any_workflow_run_succeeded(
    company: &CompanyId,
    events: &Arc<dyn EventLog>,
) -> Result<bool> {
    let stored = events
        .read_from(company, EventSeq::new(0), usize::MAX)
        .await?;
    Ok(stored.iter().any(|entry| {
        let CompanyEvent::WorkflowRunFinished {
            deliveries,
            pending_approvals,
            error,
            cancelled,
            blocked_nodes,
            ..
        } = &entry.event
        else {
            return false;
        };
        WorkflowRunVerdict::of(RunVerdictFacts {
            running: false,
            error: error.as_deref(),
            cancelled: *cancelled,
            blocked_nodes: blocked_nodes.len(),
            deliveries,
            pending_approvals: pending_approvals.len(),
            stranded_approvals: 0,
        }) == WorkflowRunVerdict::Ok
    }))
}

/// Loads the record, derives the current step answers, and — the moment every
/// step is true for the first time — durably latches
/// [`CompanyRecord::activation_completed_at`] and journals the terminal
/// [`CompanyEvent::OnboardingCompleted`]. Short-circuits on an existing latch
/// (see the module docs): no Composio call, no journal scan, once activated.
///
/// `has_composio_connection` is a **lazy** answer to "does this company hold a
/// live Composio connection" — a closure rather than a pre-fetched `bool`, so
/// an already-activated company's poll never pays for the round trip that
/// answer costs. Callers without the `composio` feature simply pass a closure
/// that always resolves to `false`; this function does no Composio IO itself
/// either way.
pub(crate) async fn compute_and_latch(
    company: &CompanyId,
    store: &Arc<dyn CompanyStore>,
    events: &Arc<dyn EventLog>,
    has_composio_connection: impl AsyncFnOnce() -> bool,
) -> Result<ActivationStatus> {
    let record = store
        .load(company)
        .await?
        .ok_or_else(|| OpenCompanyError::CompanyNotFound(company.to_string()))?;

    // The latch, once set, is the whole answer — see the module docs. Every
    // IO below this point, Composio included, exists only to decide whether
    // to *set* it, so an already-activated company skips all of it —
    // `integration_connected` is hardcoded `true` here for the same reason
    // `workflow_run_succeeded` already was: once latched, every step displays
    // as complete without re-deriving it live.
    if record.activation_completed_at.is_some() {
        return Ok(ActivationStatus {
            name_confirmed: record.name_confirmed,
            integration_connected: true,
            workflow_run_succeeded: true,
            activation_completed_at: record.activation_completed_at,
        });
    }

    let has_composio_connection = has_composio_connection().await;
    let workflow_run_succeeded = any_workflow_run_succeeded(company, events).await?;
    let status = derive_steps(&record, has_composio_connection, workflow_run_succeeded);

    if !status.all_steps_complete() {
        return Ok(status);
    }

    // Every step just read true for the first time. Re-load and re-check under
    // the per-company write lock before latching: two concurrent callers (two
    // console polls racing a webhook, say) could both observe every step
    // complete above, and without this the second would both clobber whichever
    // other write landed between its `load` and `save` AND double-journal
    // `OnboardingCompleted`. The lock + re-check makes the second caller's
    // latch a no-op that returns the first caller's timestamp instead.
    let write_lock = company_write_lock(company);
    let _lock = write_lock.lock().await;
    let mut record = store
        .load(company)
        .await?
        .ok_or_else(|| OpenCompanyError::CompanyNotFound(company.to_string()))?;
    if let Some(at_millis) = record.activation_completed_at {
        return Ok(ActivationStatus {
            activation_completed_at: Some(at_millis),
            ..status
        });
    }

    // The record is the source of truth ([`ActivationStatus::is_activated`]'s
    // contract) — latch it before journaling, so the event is only ever the
    // audit trail of a fact the record already carries.
    let at_millis = now_millis();
    record.activation_completed_at = Some(at_millis);
    store.save(&record).await?;
    drop(_lock);

    // Best-effort: the latch above already landed, so a journal failure here
    // never leaves the company un-activated — only the audit trail thinner.
    if let Err(err) = events
        .append(company, CompanyEvent::OnboardingCompleted { at_millis })
        .await
    {
        tracing::warn!(
            %company,
            %err,
            "company completed activation but the OnboardingCompleted audit event could not be journaled"
        );
    }

    Ok(ActivationStatus {
        activation_completed_at: Some(at_millis),
        ..status
    })
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::company::CompanyManifest;
    use crate::store::fs::{FsCompanyStore, FsEventLog};

    /// A fresh filesystem-backed store + journal pair, rooted at a throwaway
    /// tempdir — the same real [`CompanyStore`]/[`EventLog`] implementations
    /// the running app uses, not a hand-rolled fake, so `read_from`/`append`
    /// behave exactly as [`compute_and_latch`] will see them in production.
    fn stores() -> (Arc<dyn CompanyStore>, Arc<dyn EventLog>, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store: Arc<dyn CompanyStore> = Arc::new(FsCompanyStore::new(dir.path()));
        let events: Arc<dyn EventLog> = Arc::new(FsEventLog::new(dir.path()));
        (store, events, dir)
    }

    fn manifest(allow: &[&str]) -> CompanyManifest {
        let allow_line = allow
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(", ");
        toml::from_str(&format!(
            "[company]\nname = \"Acme\"\n[tools]\nallow = [{allow_line}]\n"
        ))
        .expect("valid manifest")
    }

    fn record(id: &CompanyId, allow: &[&str]) -> CompanyRecord {
        CompanyRecord {
            id: id.clone(),
            manifest: manifest(allow),
            ledger: Vec::new(),
            lifecycle: "running".to_string(),
            overlay_agents: Vec::new(),
            overlay_desk_members: Vec::new(),
            overlay_desk_order: Vec::new(),
            overlay_desks: Vec::new(),
            overlay_workflows: Vec::new(),
            overlay_agent_edits: Vec::new(),
            overlay_retired_agents: Vec::new(),
            overlay_policy: None,
            overlay_desk_tools: Default::default(),
            overlay_budgets: Vec::new(),
            disabled_workflows: Vec::new(),
            template_provenance: None,
            setup: None,
            name_confirmed: false,
            activation_completed_at: None,
        }
    }

    // --- derive_steps: every permutation of the three inputs -----------------

    #[test]
    fn no_steps_complete_when_nothing_is_true() {
        // `composio` granted so the integration step is a genuine unmet
        // requirement here rather than a waiver — see
        // `no_composio_grant_at_all_waives_the_integration_step` for that case.
        let r = record(&CompanyId::new("acme"), &["composio"]);
        let status = derive_steps(&r, false, false);
        assert!(!status.name_confirmed);
        assert!(!status.integration_connected);
        assert!(!status.workflow_run_succeeded);
        assert!(!status.all_steps_complete());
        assert!(!status.is_activated());
    }

    #[test]
    fn name_confirmed_alone_is_not_activation() {
        let mut r = record(&CompanyId::new("acme"), &[]);
        r.name_confirmed = true;
        let status = derive_steps(&r, false, false);
        assert!(status.name_confirmed);
        assert!(!status.all_steps_complete());
    }

    #[test]
    fn no_composio_grant_at_all_waives_the_integration_step() {
        // Issue #1850 review: several bundled companies deliberately never
        // grant `composio` at all (`companies/agentic_math_lab`,
        // `companies/agentic_product_team`, `companies/agentic_research_lab`,
        // `companies/openhuman_demo`, `companies/signals_opportunity_studio`)
        // — requiring a connection nobody in those companies could ever make
        // would permanently block activation for every one of them. When the
        // manifest cannot grant the namespace, the step waives (reads
        // vacuously complete) regardless of connection state.
        let r = record(&CompanyId::new("acme"), &["files", "docs", "shell"]);
        let status = derive_steps(&r, /* has_composio_connection */ false, false);
        assert!(
            status.integration_connected,
            "a company that can never grant composio has no lever to complete this step — it must waive, not permanently block"
        );
    }

    #[test]
    fn wildcard_grant_also_waives_the_step_since_it_never_confers_composio() {
        // `*` deliberately excludes `composio` (see `grants_composio_explicit`),
        // so a wildcard-only company is in exactly the same "can never grant
        // composio" position as a narrow allow-list that omits it. The live
        // connection here is a red herring — it still cannot be used by any
        // agent (namespace never granted), so it neither blocks nor is
        // required; the waiver applies the same as with no connection at all.
        let r = record(&CompanyId::new("acme"), &["*"]);
        let status = derive_steps(&r, /* has_composio_connection */ true, false);
        assert!(status.integration_connected);
    }

    #[test]
    fn grant_without_a_connection_is_not_integration_connected() {
        let r = record(&CompanyId::new("acme"), &["composio"]);
        let status = derive_steps(&r, /* has_composio_connection */ false, false);
        assert!(!status.integration_connected);
    }

    #[test]
    fn connection_and_explicit_grant_together_complete_the_step() {
        let r = record(&CompanyId::new("acme"), &["composio"]);
        let status = derive_steps(&r, true, false);
        assert!(status.integration_connected);
    }

    #[test]
    fn dotted_composio_subgrant_also_counts() {
        let r = record(&CompanyId::new("acme"), &["composio.gmail"]);
        let status = derive_steps(&r, true, false);
        assert!(status.integration_connected);
    }

    #[test]
    fn all_three_steps_true_is_activation() {
        let mut r = record(&CompanyId::new("acme"), &["composio"]);
        r.name_confirmed = true;
        let status = derive_steps(&r, true, true);
        assert!(status.all_steps_complete());
        assert!(status.is_activated());
    }

    // --- latch monotonicity ---------------------------------------------------

    #[test]
    fn latched_company_reads_activated_even_with_every_live_step_false() {
        // Issue #1843: a Composio connection disconnected AFTER activation must
        // not un-activate the company. `is_activated` must answer from the
        // latch, not by re-deriving the three steps. `composio` stays granted
        // here so `integration_connected` is a genuine live "false" (a real
        // disconnect) rather than a waiver — see the `_waives_` tests above
        // for that case.
        let mut r = record(&CompanyId::new("acme"), &["composio"]);
        r.activation_completed_at = Some(1_700_000_000_000);
        let status = derive_steps(&r, false, false);
        assert!(
            !status.all_steps_complete(),
            "the live steps really are false"
        );
        assert!(
            status.is_activated(),
            "the latch alone must be enough — monotonicity"
        );
    }

    // --- compute_and_latch: the async orchestration ---------------------------

    #[tokio::test]
    async fn compute_and_latch_stamps_the_record_and_journals_once_all_steps_complete() {
        let id = CompanyId::new("acme");
        let (store, events, _dir) = stores();

        let mut r = record(&id, &["composio"]);
        r.name_confirmed = true;
        store.save(&r).await.unwrap();

        // The workflow-run-succeeded signal comes from the journal, not the
        // record — append a successful, non-cancelled `WorkflowRunFinished`.
        events
            .append(
                &id,
                CompanyEvent::WorkflowRunFinished {
                    workflow_id: "digest".to_string(),
                    scheduled: false,
                    run_id: Some("run-1".to_string()),
                    deliveries: Vec::new(),
                    pending_approvals: Vec::new(),
                    error: None,
                    cancelled: false,
                    notices: Vec::new(),
                    board: Vec::new(),
                    blocked_nodes: Vec::new(),
                    approvals: Vec::new(),
                },
            )
            .await
            .unwrap();

        let status = compute_and_latch(&id, &store, &events, async || true)
            .await
            .unwrap();
        assert!(status.is_activated());
        assert!(status.activation_completed_at.is_some());

        let reloaded = store.load(&id).await.unwrap().unwrap();
        assert!(
            reloaded.activation_completed_at.is_some(),
            "the latch must be durably persisted, not just returned"
        );
    }

    /// The end-to-end shape of the issue #1850 review finding: a company
    /// whose manifest never grants `composio` at all (the
    /// `agentic_math_lab`/`agentic_product_team` pattern) must still be able
    /// to latch activation once its other two steps are true — the waived
    /// integration step must not block `compute_and_latch` from ever
    /// stamping the record for these company types.
    #[tokio::test]
    async fn compute_and_latch_activates_a_company_that_never_grants_composio() {
        let id = CompanyId::new("acme");
        let (store, events, _dir) = stores();

        let mut r = record(&id, &["files", "docs", "shell"]);
        r.name_confirmed = true;
        store.save(&r).await.unwrap();

        events
            .append(
                &id,
                CompanyEvent::WorkflowRunFinished {
                    workflow_id: "digest".to_string(),
                    scheduled: false,
                    run_id: Some("run-1".to_string()),
                    deliveries: Vec::new(),
                    pending_approvals: Vec::new(),
                    error: None,
                    cancelled: false,
                    notices: Vec::new(),
                    board: Vec::new(),
                    blocked_nodes: Vec::new(),
                    approvals: Vec::new(),
                },
            )
            .await
            .unwrap();

        // The composio closure always answers `false` (no client, no
        // connection) — matching the `#[cfg(not(feature = "composio"))]`
        // fallback and every company that never grants the namespace.
        let status = compute_and_latch(&id, &store, &events, async || false)
            .await
            .unwrap();
        assert!(
            status.is_activated(),
            "a company that structurally cannot grant composio must still be able to activate"
        );
        assert!(status.activation_completed_at.is_some());

        let reloaded = store.load(&id).await.unwrap().unwrap();
        assert!(
            reloaded.activation_completed_at.is_some(),
            "the latch must be durably persisted, not just returned"
        );
    }

    // --- any_workflow_run_succeeded: verdict, not just error/cancelled ------

    /// The exact regression this guards (issue #1850 review): a run that
    /// blocked on a human carries `error: None, cancelled: false` — the same
    /// shape as a run that actually finished — so checking only those two
    /// fields let a blocked run complete this activation step. Routing
    /// through [`WorkflowRunVerdict::of`] instead reads `blocked_nodes` and
    /// scores it `Blocked`, not `Ok`.
    #[tokio::test]
    async fn a_blocked_run_does_not_count_as_succeeded() {
        let id = CompanyId::new("acme");
        let (_store, events, _dir) = stores();

        events
            .append(
                &id,
                CompanyEvent::WorkflowRunFinished {
                    workflow_id: "digest".to_string(),
                    scheduled: false,
                    run_id: Some("run-1".to_string()),
                    deliveries: Vec::new(),
                    pending_approvals: Vec::new(),
                    error: None,
                    cancelled: false,
                    notices: Vec::new(),
                    board: Vec::new(),
                    blocked_nodes: vec![crate::ports::WorkflowBlockedNode {
                        node_id: "fetch_invoice".to_string(),
                        tools: vec!["gmail".to_string()],
                        approval_ids: Vec::new(),
                        unparkable: 0,
                        stranded: 0,
                    }],
                    approvals: Vec::new(),
                },
            )
            .await
            .unwrap();

        assert!(!any_workflow_run_succeeded(&id, &events).await.unwrap());
    }

    /// Same shape, the other trigger: a run parked on an approval gate
    /// (`pending_approvals` non-empty, no blocked node) is `AwaitingApproval`,
    /// not `Ok`.
    #[tokio::test]
    async fn a_run_awaiting_approval_does_not_count_as_succeeded() {
        let id = CompanyId::new("acme");
        let (_store, events, _dir) = stores();

        events
            .append(
                &id,
                CompanyEvent::WorkflowRunFinished {
                    workflow_id: "digest".to_string(),
                    scheduled: false,
                    run_id: Some("run-1".to_string()),
                    deliveries: Vec::new(),
                    pending_approvals: vec!["publish".to_string()],
                    error: None,
                    cancelled: false,
                    notices: Vec::new(),
                    board: Vec::new(),
                    blocked_nodes: Vec::new(),
                    approvals: Vec::new(),
                },
            )
            .await
            .unwrap();

        assert!(!any_workflow_run_succeeded(&id, &events).await.unwrap());
    }

    #[tokio::test]
    async fn compute_and_latch_does_not_latch_when_a_step_is_still_missing() {
        let id = CompanyId::new("acme");
        let (store, events, _dir) = stores();
        store.save(&record(&id, &["composio"])).await.unwrap();

        // No workflow run journaled at all — the third step is missing.
        let status = compute_and_latch(&id, &store, &events, async || true)
            .await
            .unwrap();
        assert!(!status.is_activated());

        let reloaded = store.load(&id).await.unwrap().unwrap();
        assert!(reloaded.activation_completed_at.is_none());
    }

    #[tokio::test]
    async fn compute_and_latch_short_circuits_once_latched_even_if_a_step_regresses() {
        let id = CompanyId::new("acme");
        let (store, events, _dir) = stores();

        let mut r = record(&id, &[]); // no composio grant at all — a regression
        r.activation_completed_at = Some(1_700_000_000_000);
        store.save(&r).await.unwrap();

        // The journal is empty and the composio closure below always answers
        // `false` — every live step reads false, and yet the company must still
        // read as activated. The closure also proves the *other* half of the
        // short-circuit contract: an already-latched company must not pay for a
        // Composio round trip at all, so it counts its own invocations and
        // asserts zero — the regression this test now guards is exactly the one
        // `GET {scope}/activation` shipped (issue #1850 review): the endpoint
        // fetched the live connection state before ever checking the latch.
        let composio_calls = std::sync::atomic::AtomicUsize::new(0);
        let status = compute_and_latch(&id, &store, &events, async || {
            composio_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            false
        })
        .await
        .unwrap();
        assert!(status.is_activated());
        assert_eq!(status.activation_completed_at, Some(1_700_000_000_000));
        assert_eq!(
            composio_calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "an already-latched company must not query Composio"
        );
    }
}
