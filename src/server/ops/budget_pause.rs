//! `GET {scope}/agents/{agent_id}/budget-pause` and
//! `POST {scope}/agents/{agent_id}/budget-pause/redeem` (issue #1846): the
//! console's Add-Credits CTA. A turn that paused for lack of inference
//! budget/credits parks a durable marker
//! ([`crate::runtime::grants::BudgetPauseMarker`]) naming the original
//! message; this route lets the operator read that marker back and trigger
//! its re-issue once the account is topped up.
//!
//! **Not true resume** (issue #561): redeeming re-enters the SAME cycle path
//! an ordinary chat message takes
//! ([`CompanyEvent::OperatorMessage`](crate::ports::types::CompanyEvent::OperatorMessage)
//! through [`CompanyRuntime::run_cycle`](crate::company::runtime::CompanyRuntime::run_cycle)),
//! addressed to the same chat thread the original message was. Whatever the
//! paused attempt had already done stays done; the redeemed turn runs fresh
//! from the top and can repeat a non-idempotent side effect the first attempt
//! already performed.

use std::sync::Arc;

use axum::extract::{Path, Query};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::error::OpenCompanyError;
use crate::ports::types::CompanyEvent;
use crate::runtime::grants::{BudgetPauseMarker, RedeemMatch, budget_pauses_for};
use crate::server::error::ApiError;
use crate::server::ops::{ScopedCompany, scoped};

/// Builds the budget-pause route fragment.
pub fn router() -> Router<AppState> {
    scoped("/agents/{agent_id}/budget-pause", get(get_budget_pause)).merge(scoped(
        "/agents/{agent_id}/budget-pause/redeem",
        post(redeem_budget_pause),
    ))
}

#[derive(Debug, Deserialize)]
struct AgentPath {
    agent_id: String,
}

/// The redeem route's `?id=` — the marker id the console last read via
/// `GET`, so the reservation below can be matched rather than blind (issue
/// #1846 review, Codex #3866418876). Absent for a caller with no prior read
/// to compare against, in which case redemption falls back to the
/// unconditional pre-fix behaviour.
#[derive(Debug, Deserialize)]
struct RedeemQuery {
    #[serde(default)]
    id: Option<String>,
}

/// The console's read of a parked budget pause.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BudgetPauseDto {
    id: String,
    agent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    chat_id: Option<String>,
    message: String,
    summary: String,
    at_millis: u64,
}

impl From<BudgetPauseMarker> for BudgetPauseDto {
    fn from(marker: BudgetPauseMarker) -> Self {
        Self {
            id: marker.id,
            agent: marker.agent,
            chat_id: marker.chat_id,
            message: marker.message,
            summary: marker.summary,
            at_millis: marker.at_millis,
        }
    }
}

/// `GET {scope}/agents/{agent_id}/budget-pause` — the parked marker for this
/// agent, or `null` when nothing is paused. Read-only: does not consume the
/// marker, so the console can poll/render it (the "approaching"/"exhausted"
/// banner) without accidentally triggering a redeem.
async fn get_budget_pause(
    company: ScopedCompany,
    Path(AgentPath { agent_id }): Path<AgentPath>,
) -> Json<Option<BudgetPauseDto>> {
    let marker = budget_pauses_for(company.id()).peek(&agent_id);
    Json(marker.map(BudgetPauseDto::from))
}

/// `POST {scope}/agents/{agent_id}/budget-pause/redeem` — the Add-Credits CTA.
/// Reserves the marker (single-use, like a
/// [`GrantedCall`](crate::runtime::grants::GrantedCall) redemption), THEN
/// re-dispatches the original message through the same cycle path an
/// ordinary operator send takes, addressed to the same chat the pause
/// happened on.
///
/// Deliberately "reserve, then re-dispatch", not "peek, re-dispatch, then
/// consume" (issue #1846 review, Codex #3865395849, replacing the shape
/// Codex #3864988181 first added): peeking first left a window between two
/// concurrent redeem requests — say, clicks from two browser tabs — where
/// BOTH could read the same marker before either had re-dispatched, so both
/// re-dispatched it, and only one of the two later consume calls actually
/// won while the loser still reported success to its own caller, silently
/// repeating whatever non-idempotent side effect the original attempt
/// performed. [`redeem`](crate::runtime::grants::BudgetPauseSet::redeem)
/// takes the marker atomically up front, so the SECOND request's own
/// reservation finds nothing — the first already took it — and 404s before
/// it ever re-dispatches.
///
/// A reservation that never redispatches (this call errors before
/// `run_cycle` returns) is restored via
/// [`restore_if_absent`](crate::runtime::grants::BudgetPauseSet::restore_if_absent)
/// rather than left gone: a re-dispatch failure — the event store hiccups,
/// the request is cancelled mid-flight — must not silently lose the CTA's
/// saved re-issue payload to a `404` on the very next click. Guarded on
/// absence rather than a plain re-insert: the re-dispatch can itself pause
/// again on the same agent before the restore runs, and restoring only when
/// nothing is parked is what keeps that fresh marker from being clobbered by
/// the stale one being put back.
///
/// 404 when nothing is parked for this agent — the operator's own "add
/// credits" action beat them to it, the process restarted since the pause
/// (this marker is in-memory only, see
/// [`crate::runtime::grants::BudgetPauseMarker`]'s doc comment), or the
/// `agent_id` never had one.
///
/// 409 when `?id=` names a marker that is no longer the one parked (issue
/// #1846 review, Codex #3866418876): a background turn (a workflow node, an
/// unstreamed task) pausing for the SAME agent re-parks with no chat
/// destination and overwrites the marker the console's chat card was reading
/// from, with no signal the transcript-based staleness check can observe.
/// The console re-reads the live marker (`GET` above) immediately before
/// every redeem and sends its `id` here so this mismatch is caught
/// server-side, atomically, rather than the CTA silently re-dispatching
/// whatever is parked NOW under the assumption it is still what the
/// operator clicked. See [`RedeemMatch`]'s doc for the full reasoning.
async fn redeem_budget_pause(
    company: ScopedCompany,
    Path(AgentPath { agent_id }): Path<AgentPath>,
    Query(RedeemQuery { id }): Query<RedeemQuery>,
) -> Result<Json<BudgetPauseDto>, ApiError> {
    let pauses = budget_pauses_for(company.id());
    // Reserved (atomically removed) up front, not merely peeked — see this
    // function's doc comment. A concurrent second request's own `redeem`
    // below finds nothing and 404s before it ever re-dispatches.
    //
    // `?id=` present (every console call site sends it, having just read the
    // marker back via `GET`): reserve only if that id is STILL what's
    // parked — `RedeemMatch::Stale` means a background turn overwrote it
    // since the console last read it, and must not silently redispatch the
    // wrong marker. `?id=` absent: unconditional `redeem`, unchanged from
    // before this fix — for any caller with nothing to compare against.
    let marker = match id {
        Some(expected_id) => match pauses.redeem_matching(&agent_id, &expected_id) {
            RedeemMatch::Reserved(marker) => marker,
            RedeemMatch::Absent => {
                tracing::info!(
                    company = %company.id(),
                    agent = %agent_id,
                    "[budget-pause] redeem requested but nothing is parked — already redeemed, expired with the process, or never paused"
                );
                return Err(OpenCompanyError::NotFound(format!(
                    "no parked budget pause for agent '{agent_id}'"
                ))
                .into());
            }
            RedeemMatch::Stale => {
                tracing::info!(
                    company = %company.id(),
                    agent = %agent_id,
                    expected_id = %expected_id,
                    "[budget-pause] redeem requested a marker that is no longer parked — a newer pause (likely a background turn) has since taken its place; leaving it untouched"
                );
                return Err(OpenCompanyError::Conflict(format!(
                    "the budget pause for agent '{agent_id}' has changed since it was read — refresh and try again"
                ))
                .into());
            }
        },
        None => pauses.redeem(&agent_id).ok_or_else(|| {
            tracing::info!(
                company = %company.id(),
                agent = %agent_id,
                "[budget-pause] redeem requested but nothing is parked — already redeemed, expired with the process, or never paused"
            );
            OpenCompanyError::NotFound(format!("no parked budget pause for agent '{agent_id}'"))
        })?,
    };

    tracing::info!(
        company = %company.id(),
        agent = %agent_id,
        marker_id = %marker.id,
        "[budget-pause] redeeming; re-dispatching the original message from the top"
    );
    // Issue #1846 review (Codex #3865812419/#3865812423/#3865812432): replay
    // the ORIGINAL message's thread parent, composer intent, and resolved
    // mentions from the marker, rather than the empty defaults that used to
    // sit here. See `BudgetPauseMarker`'s field docs for what each default
    // silently broke.
    let event = CompanyEvent::OperatorMessage {
        text: marker.message.clone(),
        by: company.actor.clone(),
        chat: marker.chat_id.clone(),
        parent: marker.parent,
        deliverable: marker.deliverable,
        mentions: marker.mentions.clone(),
        attachments: Vec::new(),
    };
    // Issue #1846 review (Codex #3865812411): spawned, not awaited directly
    // in this handler's own future. This host is plain
    // `axum::serve(listener, router(state))`; hyper drops a handler's future
    // the moment the peer disconnects, and a reverse proxy in front of a
    // hosted tenant closes it the moment it decides the upstream is too
    // slow. A direct `.await` here left `restore_if_absent` below
    // unreachable on a drop: the reservation `redeem` took above is gone,
    // `run_cycle` is abandoned mid-flight — tokens spent, side effects
    // possibly already applied, nothing ever re-dispatched to completion —
    // and the operator's saved re-issue payload is lost for good instead of
    // restored for their next click. Same shape and same fix as
    // `spawn_chat_turn` (`src/server/operator.rs`) and
    // `CompanyRuntime::resolve_approval_spawned` use for the ordinary chat
    // and approval paths.
    //
    // Awaiting the `JoinHandle` is drop-safe: dropping it abandons only the
    // *waiting*, so the redispatch always runs to completion — including
    // this function's own restore-on-failure below — no matter what happens
    // to the request that triggered it.
    let runtime = Arc::clone(&company.runtime);
    let redispatch = tokio::spawn(async move { runtime.run_cycle(vec![event]).await });
    match redispatch.await {
        Ok(Ok(_report)) => Ok(Json(BudgetPauseDto::from(marker))),
        // The redispatch ran to completion but returned an error — restore
        // the reservation so the operator's saved payload survives for a
        // retry, not thrown away over a redispatch that never happened.
        Ok(Err(err)) => {
            pauses.restore_if_absent(marker);
            Err(err.into())
        }
        // The spawned task itself panicked (not `run_cycle` returning
        // `Err`) — exactly as "never happened" as an `Err`, so the same
        // restore applies.
        Err(join_err) => {
            pauses.restore_if_absent(marker);
            Err(OpenCompanyError::BackgroundTask(format!(
                "budget-pause redeem's redispatch did not finish: {join_err}"
            ))
            .into())
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;
    use crate::company::CompanyManifest;
    use crate::ports::CompanyStore;
    use crate::ports::types::{
        CompanyId, CompanyRecord, EventSeq, Mention, MentionTarget, MessageIntent,
    };
    use crate::runtime::RuntimeBuilder;
    use crate::runtime::grants::RedeemContext;
    use crate::server::router;
    use crate::store::FsCompanyStore;
    use crate::{AppConfig, AppState};

    fn home() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("opencompany-budget-pause-")
            .tempdir()
            .expect("tempdir")
    }

    fn manifest() -> CompanyManifest {
        toml::from_str("[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n").unwrap()
    }

    /// Builds an [`AppState`] for a single company, its lone registered
    /// runtime running on `brain`. Same shape as `operator.rs`'s
    /// `build_state_with_brain` — a fresh `company` id per test, never the
    /// shared `"acme"` other files' budget-pause tests use, so this file's
    /// `BudgetPauseSet` (keyed globally by company id) never collides with a
    /// concurrently-running test elsewhere in the same binary.
    async fn state_with_brain(
        home: &std::path::Path,
        company: &str,
        brain: Arc<dyn crate::ports::brain::Brain>,
    ) -> AppState {
        let m = manifest();
        let store = FsCompanyStore::new(home.to_path_buf());
        let id = CompanyId::new(company);
        store
            .save(&CompanyRecord {
                overlay_retired_agents: Vec::new(),
                overlay_agent_edits: Vec::new(),
                id: id.clone(),
                manifest: m.clone(),
                ledger: Vec::new(),
                lifecycle: "running".to_string(),
                overlay_agents: Vec::new(),
                overlay_desk_members: Vec::new(),
                overlay_desk_order: Vec::new(),
                overlay_desks: Vec::new(),
                overlay_workflows: Vec::new(),
                overlay_budgets: Vec::new(),
                overlay_policy: None,
                overlay_desk_tools: Default::default(),
                disabled_workflows: Vec::new(),
                template_provenance: None,
                setup: None,
                overlay_tool_grants: None,
            })
            .await
            .unwrap();

        let runtime = RuntimeBuilder::new(home.to_path_buf(), m)
            .with_id(id.clone())
            .with_brain(brain)
            .build()
            .await
            .unwrap();
        let state = AppState::new(AppConfig::default());
        state.registry().insert(id, Arc::new(runtime));
        crate::server::test_support::seed_fixed_admin(&state, company).await;
        state
    }

    async fn send(
        state: &AppState,
        company: &str,
        method: &str,
        uri: &str,
    ) -> (StatusCode, Value, String) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header("cookie", crate::server::test_support::fixed_cookie(company))
            .body(Body::empty())
            .unwrap();
        let response = router(state.clone()).oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let raw = String::from_utf8_lossy(&bytes).to_string();
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, value, raw)
    }

    /// A brain that records the last `OperatorMessage` event any cycle it
    /// runs carries, and otherwise reports an uneventful cycle.
    #[derive(Default)]
    struct RecordingBrain {
        last: std::sync::Mutex<Option<CompanyEvent>>,
    }

    #[async_trait::async_trait]
    impl crate::ports::brain::Brain for RecordingBrain {
        async fn run_cycle(
            &self,
            req: crate::ports::types::CycleRequest,
            _host: &dyn crate::ports::brain::CycleHost,
        ) -> crate::Result<crate::ports::types::CycleResult> {
            if let Some(event) = req
                .events
                .into_iter()
                .find(|e| matches!(e, CompanyEvent::OperatorMessage { .. }))
            {
                *self.last.lock().unwrap() = Some(event);
            }
            Ok(crate::ports::types::CycleResult {
                channel_responses: Vec::new(),
                new_traces: Vec::new(),
                ledger_deltas: Vec::new(),
                token_usage: crate::ports::types::TokenUsage::default(),
            })
        }
    }

    /// Issue #1846 review (Codex #3865812419/#3865812423/#3865812432): a
    /// redeem replays the marker's parent/deliverable/mentions onto the
    /// redispatched `OperatorMessage` instead of the empty defaults this
    /// route used to fall back to.
    #[tokio::test]
    async fn redeem_replays_the_markers_parent_deliverable_and_mentions() {
        let home = home();
        let company = "acme-redeem-fields";
        let recording = Arc::new(RecordingBrain::default());
        let state = state_with_brain(home.path(), company, recording.clone()).await;
        let id = CompanyId::new(company);

        let parent = EventSeq::new(11);
        let deliverable = MessageIntent::Workflow;
        let mentions = vec![Mention {
            target: MentionTarget::Agent {
                id: "researcher".to_string(),
            },
            text: "@researcher".to_string(),
            offset: 0,
            quiet: false,
        }];
        budget_pauses_for(&id).park(
            "ceo",
            Some("general".to_string()),
            "ship the API",
            "paused",
            1_000,
            RedeemContext {
                parent: Some(parent),
                deliverable: Some(deliverable),
                mentions: mentions.clone(),
            },
        );

        let (status, _resp, raw) = send(
            &state,
            company,
            "POST",
            "/api/v1/company/agents/ceo/budget-pause/redeem",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{raw}");

        let recorded = recording
            .last
            .lock()
            .unwrap()
            .clone()
            .expect("the redispatch reached the brain");
        match recorded {
            CompanyEvent::OperatorMessage {
                text,
                parent: got_parent,
                deliverable: got_deliverable,
                mentions: got_mentions,
                ..
            } => {
                // Not `assert_eq!`: a `Workflow`-deliverable message picks up
                // the builder-pass briefing (`cycle_conversation`'s
                // `inject_workflow_builder_awareness`) between the redispatch
                // and the brain seeing it — itself proof `deliverable`
                // actually reached the live cycle, not just this route's own
                // event construction.
                assert!(
                    text.starts_with("ship the API"),
                    "the operator's original words must lead the redispatched text: {text}"
                );
                assert_eq!(got_parent, Some(parent));
                assert_eq!(got_deliverable, Some(deliverable));
                assert_eq!(got_mentions, mentions);
            }
            other => panic!("expected an OperatorMessage, got {other:?}"),
        }
    }

    /// Issue #1846 review (Codex #3866418876) — the keystone test for the
    /// background-overwrite fix. A chat-visible pause parks a marker for
    /// `ceo` with a chat destination; a background turn (a workflow node or
    /// an unstreamed task) for the SAME agent then pauses too and
    /// overwrites it with a marker that has none. The console's stale-card
    /// check never sees this happen — a chat-less park never touches the
    /// transcript it watches — so the OLD chat card is still what the
    /// operator clicks. Redeeming with that card's (now stale) `?id=` must
    /// be refused with 409 and must NOT redispatch anything — proven here
    /// by the recording brain seeing no `OperatorMessage` at all, not merely
    /// the "wrong" one. Redeeming with the CURRENT marker's id then succeeds
    /// and redispatches the background pause's own message.
    #[tokio::test]
    async fn a_stale_marker_id_is_refused_without_redispatching_the_background_pause() {
        let home = home();
        let company = "acme-redeem-stale-id";
        let recording = Arc::new(RecordingBrain::default());
        let state = state_with_brain(home.path(), company, recording.clone()).await;
        let id = CompanyId::new(company);

        let chat_marker = budget_pauses_for(&id).park(
            "ceo",
            Some("general".to_string()),
            "ship the API",
            "paused for the chat turn",
            1_000,
            RedeemContext::default(),
        );
        let background_marker = budget_pauses_for(&id).park(
            "ceo",
            None,
            "run the nightly workflow node",
            "paused for the background turn",
            2_000,
            RedeemContext::default(),
        );

        // The console clicks the OLD chat card, so it sends the OLD id.
        let (status, _resp, raw) = send(
            &state,
            company,
            "POST",
            &format!(
                "/api/v1/company/agents/ceo/budget-pause/redeem?id={}",
                chat_marker.id
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "a stale marker id must be refused, not silently honoured: {raw}"
        );
        assert!(
            recording.last.lock().unwrap().is_none(),
            "a refused stale redeem must never reach the brain — the background pause's \
             message must not be silently redispatched under a click meant for the chat one"
        );
        // Left completely untouched — still there, still the background one.
        let still_parked = budget_pauses_for(&id)
            .peek("ceo")
            .expect("survives the refusal");
        assert_eq!(still_parked.id, background_marker.id);

        // The console re-reads the live marker and redeems with ITS id.
        let (status, _resp, raw) = send(
            &state,
            company,
            "POST",
            &format!(
                "/api/v1/company/agents/ceo/budget-pause/redeem?id={}",
                background_marker.id
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{raw}");
        let recorded = recording
            .last
            .lock()
            .unwrap()
            .clone()
            .expect("the redispatch reached the brain");
        match recorded {
            CompanyEvent::OperatorMessage { text, .. } => {
                assert!(
                    text.starts_with("run the nightly workflow node"),
                    "the background pause's own message must be what gets resent: {text}"
                );
            }
            other => panic!("expected an OperatorMessage, got {other:?}"),
        }
    }

    /// A caller that sends no `?id=` at all falls back to the pre-fix,
    /// unconditional redeem — the escape hatch for anything that has no
    /// prior marker read to compare against.
    #[tokio::test]
    async fn omitting_the_id_query_param_redeems_unconditionally() {
        let home = home();
        let company = "acme-redeem-no-id-param";
        let recording = Arc::new(RecordingBrain::default());
        let state = state_with_brain(home.path(), company, recording.clone()).await;
        let id = CompanyId::new(company);

        budget_pauses_for(&id).park(
            "ceo",
            None,
            "ship the API",
            "paused",
            1_000,
            RedeemContext::default(),
        );

        let (status, _resp, raw) = send(
            &state,
            company,
            "POST",
            "/api/v1/company/agents/ceo/budget-pause/redeem",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{raw}");
    }

    /// A brain whose `run_cycle` always refuses — the redispatch never
    /// completes successfully.
    struct FailingRedispatchBrain;

    #[async_trait::async_trait]
    impl crate::ports::brain::Brain for FailingRedispatchBrain {
        async fn run_cycle(
            &self,
            _req: crate::ports::types::CycleRequest,
            _host: &dyn crate::ports::brain::CycleHost,
        ) -> crate::Result<crate::ports::types::CycleResult> {
            Err(OpenCompanyError::InvalidRequest(
                "redispatch refused".to_string(),
            ))
        }
    }

    /// Issue #1846 review (Codex #3865812411): a redispatch that returns
    /// `Err` must restore the reservation `redeem` took, not leave the
    /// operator's saved payload gone for good — the failure branch the
    /// spawn-based fix has to keep reachable.
    #[tokio::test]
    async fn a_failed_redispatch_restores_the_reservation() {
        let home = home();
        let company = "acme-redeem-restore";
        let state = state_with_brain(home.path(), company, Arc::new(FailingRedispatchBrain)).await;
        let id = CompanyId::new(company);

        budget_pauses_for(&id).park(
            "ceo",
            None,
            "ship the API",
            "paused",
            1_000,
            RedeemContext::default(),
        );

        let (status, _resp, raw) = send(
            &state,
            company,
            "POST",
            "/api/v1/company/agents/ceo/budget-pause/redeem",
        )
        .await;
        assert!(
            status.is_client_error() || status.is_server_error(),
            "a refused redispatch must not report success: {raw}"
        );

        assert!(
            budget_pauses_for(&id).peek("ceo").is_some(),
            "a failed redispatch must restore the reservation so the operator's saved \
             payload survives for a retry, rather than being thrown away over a redispatch \
             that never happened"
        );
    }

    /// A brain that stalls mid-cycle so the test can drop the connection
    /// while the redispatch is still in flight, then release it and prove
    /// the redispatch ran to completion anyway. Same shape as
    /// `operator.rs`'s `StalledContinuationBrain`.
    struct StalledRedispatchBrain {
        /// Fires once the redispatch cycle is under way — the moment a
        /// dropped connection would have cancelled it under the pre-fix
        /// direct `.await`.
        entered: std::sync::Arc<tokio::sync::Notify>,
        /// The test's permission for the cycle to finish.
        release: std::sync::Arc<tokio::sync::Notify>,
    }

    #[async_trait::async_trait]
    impl crate::ports::brain::Brain for StalledRedispatchBrain {
        async fn run_cycle(
            &self,
            req: crate::ports::types::CycleRequest,
            _host: &dyn crate::ports::brain::CycleHost,
        ) -> crate::Result<crate::ports::types::CycleResult> {
            if req
                .events
                .iter()
                .any(|e| matches!(e, CompanyEvent::OperatorMessage { .. }))
            {
                self.entered.notify_one();
                self.release.notified().await;
            }
            Ok(crate::ports::types::CycleResult {
                channel_responses: Vec::new(),
                new_traces: Vec::new(),
                ledger_deltas: Vec::new(),
                token_usage: crate::ports::types::TokenUsage::default(),
            })
        }
    }

    /// Issue #1846 review (Codex #3865812411) — the keystone test for the
    /// cancellation-safety fix. This host is plain
    /// `axum::serve(listener, router(state))`; hyper drops a handler's
    /// future the moment the peer disconnects, and a reverse proxy in front
    /// of a hosted tenant closes it the moment it decides the upstream is
    /// too slow. Before this fix, `redeem_budget_pause` awaited
    /// `run_cycle` directly in its own future, so that drop cancelled the
    /// redispatch mid-flight: `restore_if_absent` never ran, the reservation
    /// `redeem` took was gone for good, and the operator's saved payload
    /// vanished with no redispatch ever having completed.
    ///
    /// `Router::oneshot` reproduces that drop faithfully rather than by
    /// analogy — same mechanism hyper uses, since the handler future is
    /// owned by the future the caller polls.
    #[tokio::test]
    async fn a_dropped_connection_does_not_cancel_the_redispatch() {
        let home = home();
        let company = "acme-redeem-drop";
        let entered = std::sync::Arc::new(tokio::sync::Notify::new());
        let release = std::sync::Arc::new(tokio::sync::Notify::new());
        let state = state_with_brain(
            home.path(),
            company,
            Arc::new(StalledRedispatchBrain {
                entered: entered.clone(),
                release: release.clone(),
            }),
        )
        .await;
        let id = CompanyId::new(company);

        budget_pauses_for(&id).park(
            "ceo",
            None,
            "ship the API",
            "paused",
            1_000,
            RedeemContext::default(),
        );

        let uri = "/api/v1/company/agents/ceo/budget-pause/redeem";
        let request = Request::builder()
            .method("POST")
            .uri(uri)
            .header("cookie", crate::server::test_support::fixed_cookie(company))
            .body(Body::empty())
            .unwrap();
        let mut redeeming = Box::pin(router(state.clone()).oneshot(request));
        tokio::select! {
            _ = &mut redeeming => panic!("the redeem answered before the redispatch began"),
            _ = entered.notified() => {}
        }
        drop(redeeming);

        // The reservation is gone — exactly the state a client sees the
        // instant a real proxy gives up mid-redispatch.
        assert!(
            budget_pauses_for(&id).peek("ceo").is_none(),
            "the marker was reserved before the connection dropped"
        );

        // So the redispatch the reservation exists for must still run to
        // completion, not die with the dropped connection.
        release.notify_one();
        let recorded = recording_settles(&id, "ceo").await;
        assert!(
            recorded,
            "the redispatch died with the dropped connection: the reservation is spent and \
             the redispatch never ran to completion"
        );
    }

    /// Polls until the marker for `agent` is gone-and-stays-gone (redeemed
    /// and never restored) or the timeout expires, so the drop test above
    /// does not need a bespoke completion channel through `StalledRedispatchBrain`.
    /// `run_cycle` returns `Ok` on release, so a marker that is STILL absent
    /// after a settle window means the background redispatch ran to
    /// completion without erroring — an error would have restored it.
    async fn recording_settles(id: &CompanyId, agent: &str) -> bool {
        for _ in 0..200 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            if budget_pauses_for(id).peek(agent).is_none() {
                // Give the spawned task's own `Ok` branch a moment past the
                // notify to finish; then confirm it stayed absent rather than
                // having been an in-between read racing a restore.
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                return budget_pauses_for(id).peek(agent).is_none();
            }
        }
        false
    }
}
