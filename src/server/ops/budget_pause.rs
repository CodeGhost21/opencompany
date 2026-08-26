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

use axum::extract::Path;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::error::OpenCompanyError;
use crate::ports::types::CompanyEvent;
use crate::runtime::grants::{BudgetPauseMarker, budget_pauses_for};
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
/// Re-dispatches the original message through the same cycle path an
/// ordinary operator send takes, addressed to the same chat the pause
/// happened on, and only THEN consumes the marker (single-use, like a
/// [`GrantedCall`](crate::runtime::grants::GrantedCall) redemption).
///
/// Deliberately not "consume, then re-dispatch" (issue #1846 review, Codex
/// #3864988181): the marker is [`peek`](crate::runtime::grants::BudgetPauseSet::peek)ed
/// rather than redeemed up front, so a re-dispatch that fails — the event
/// store hiccups, the request is cancelled mid-flight — leaves the marker
/// parked for a retry to find, instead of the CTA silently losing its saved
/// re-issue payload to a `404` on the very next click. The consuming half is
/// [`redeem_matching`](crate::runtime::grants::BudgetPauseSet::redeem_matching),
/// keyed on the marker's own `id`: the re-dispatch can itself pause again on
/// the same agent before this call runs, and matching the id (rather than a
/// plain `redeem(agent)`) is what keeps that fresh marker from being deleted
/// out from under the operator before they ever see it.
///
/// 404 when nothing is parked for this agent — the operator's own "add
/// credits" action beat them to it, the process restarted since the pause
/// (this marker is in-memory only, see
/// [`crate::runtime::grants::BudgetPauseMarker`]'s doc comment), or the
/// `agent_id` never had one.
async fn redeem_budget_pause(
    company: ScopedCompany,
    Path(AgentPath { agent_id }): Path<AgentPath>,
) -> Result<Json<BudgetPauseDto>, ApiError> {
    let marker = budget_pauses_for(company.id())
        .peek(&agent_id)
        .ok_or_else(|| {
            tracing::info!(
                company = %company.id(),
                agent = %agent_id,
                "[budget-pause] redeem requested but nothing is parked — already redeemed, expired with the process, or never paused"
            );
            OpenCompanyError::NotFound(format!("no parked budget pause for agent '{agent_id}'"))
        })?;

    tracing::info!(
        company = %company.id(),
        agent = %agent_id,
        marker_id = %marker.id,
        "[budget-pause] redeeming; re-dispatching the original message from the top"
    );
    let event = CompanyEvent::OperatorMessage {
        text: marker.message.clone(),
        by: company.actor.clone(),
        chat: marker.chat_id.clone(),
        parent: None,
        deliverable: None,
        mentions: Vec::new(),
        attachments: Vec::new(),
    };
    // Propagated with `?` BEFORE the marker is consumed: a failure here must
    // leave the marker parked, not throw away the operator's saved payload
    // over a redispatch that never happened.
    company.runtime.run_cycle(vec![event]).await?;

    let consumed = budget_pauses_for(company.id()).redeem_matching(&agent_id, &marker.id);
    if consumed.is_none() {
        // The re-dispatch itself re-paused this agent before we got here —
        // see `redeem_matching`'s doc. That fresh marker is a different pause
        // the operator has not seen yet, and must be left parked rather than
        // removed.
        tracing::info!(
            company = %company.id(),
            agent = %agent_id,
            marker_id = %marker.id,
            "[budget-pause] redispatch succeeded but re-paused the agent before this marker could be consumed; leaving the fresh marker parked"
        );
    }

    Ok(Json(BudgetPauseDto::from(marker)))
}
