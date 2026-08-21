//! The shared feedback board's HTTP surface.
//!
//! Five routes, each mirrored on the single-company `/api/v1/company/...` alias:
//!
//! * `GET  .../feedback/board?sort=&type=&status=&page=&limit=` — one page
//! * `GET  .../feedback/board/{item}` — one item with its comments
//! * `POST .../feedback/board/{item}/vote` — `{ "value": 1 | -1 | 0 }`
//! * `POST .../feedback/board/{item}/comments` — `{ "body": "…" }`
//!
//! None of it is stored here. The board lives on the TinyHumans hub and these
//! handlers proxy it with the instance credential
//! ([`crate::feedback::board`]), which is the whole point: a console in a
//! browser gets a live board — votes, comments, statuses — without ever holding
//! a credential that could reach the hub directly, and without a cross-origin
//! call to a host it cannot authenticate to.
//!
//! An instance with no TinyHumans credential has no board: every route answers
//! `404 tinyhumans_no_board`, which the console reads as "hide the board" (see
//! [`CompanyRuntime::feedback_board`](crate::company::runtime::CompanyRuntime::feedback_board)).

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::AppState;
use crate::company::runtime::CompanyRuntime;
use crate::error::OpenCompanyError;
use crate::feedback::board::{
    BoardComment, BoardDetail, BoardItem, BoardKind, BoardPage, BoardQuery, BoardSort, BoardStatus,
    DEFAULT_LIMIT, VoteValue,
};
use crate::ports::types::CompanyId;
use crate::server::error::ApiError;
use crate::server::feedback::{lookup, sole};
use crate::server::platform_auth::{CompanyAuth, authorize_address};

/// Builds the board route fragment, merged into the main router.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/companies/{id}/feedback/board", get(list))
        .route("/api/v1/companies/{id}/feedback/board/{item}", get(detail))
        .route(
            "/api/v1/companies/{id}/feedback/board/{item}/vote",
            post(vote),
        )
        .route(
            "/api/v1/companies/{id}/feedback/board/{item}/comments",
            post(comment),
        )
        .route("/api/v1/company/feedback/board", get(list_single))
        .route("/api/v1/company/feedback/board/{item}", get(detail_single))
        .route("/api/v1/company/feedback/board/{item}/vote", post(vote_single))
        .route(
            "/api/v1/company/feedback/board/{item}/comments",
            post(comment_single),
        )
}

/// The list query string, in the console's vocabulary.
///
/// Every field is optional and every unrecognized value is *ignored* rather
/// than refused: a filter the console cannot express is a filter the operator
/// did not ask for, and answering the unfiltered board beats a 400 on a
/// hand-edited URL.
#[derive(Debug, Default, Deserialize)]
pub struct BoardParams {
    /// `hot` (default), `top`, or `new`.
    #[serde(default)]
    pub sort: Option<String>,
    /// `feature` or `bug`.
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    /// `open`, `planned`, `completed`, or `closed`.
    #[serde(default)]
    pub status: Option<String>,
    /// 1-based page. Out-of-range values are clamped, not refused.
    #[serde(default)]
    pub page: Option<u32>,
    /// Page size, clamped to the hub's ceiling.
    #[serde(default)]
    pub limit: Option<u32>,
}

impl BoardParams {
    /// The clamped [`BoardQuery`] these params describe.
    pub fn to_query(&self) -> BoardQuery {
        BoardQuery {
            sort: self
                .sort
                .as_deref()
                .and_then(BoardSort::parse)
                .unwrap_or_default(),
            kind: self.kind.as_deref().and_then(BoardKind::parse),
            status: self.status.as_deref().and_then(BoardStatus::parse),
            page: self.page.unwrap_or(1),
            limit: self.limit.unwrap_or(DEFAULT_LIMIT),
        }
        .clamped()
    }
}

/// A vote body.
#[derive(Debug, Deserialize)]
struct VoteRequest {
    /// `1` up, `-1` down, `0` to retract.
    value: VoteValue,
}

/// A comment body.
#[derive(Debug, Deserialize)]
struct CommentRequest {
    /// The comment text.
    body: String,
}

/// Resolves the addressed company, enforcing tenant ownership exactly as the
/// capture routes do — a board call spends this instance's hub credential, so
/// it is no more anonymous than filing is.
fn addressed(
    state: &AppState,
    auth: &crate::server::platform_auth::Principal,
    id: &str,
) -> Result<Arc<CompanyRuntime>, Response> {
    let company = CompanyId::new(id);
    if let Some(resp) = authorize_address(state, auth, &company) {
        return Err(resp);
    }
    lookup(state, id).map_err(IntoResponse::into_response)
}

/// The sole company, authorized the same way.
fn addressed_sole(
    state: &AppState,
    auth: &crate::server::platform_auth::Principal,
) -> Result<Arc<CompanyRuntime>, Response> {
    let runtime = sole(state).map_err(IntoResponse::into_response)?;
    if let Some(resp) = authorize_address(state, auth, runtime.id()) {
        return Err(resp);
    }
    Ok(runtime)
}

/// Refuses an empty comment before it costs a hub round trip.
fn checked_comment(body: &str) -> Result<&str, Response> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Err(
            ApiError(OpenCompanyError::InvalidInput("comment is empty".to_string()))
                .into_response(),
        );
    }
    Ok(trimmed)
}

async fn page(runtime: Arc<CompanyRuntime>, params: &BoardParams) -> Result<Json<BoardPage>, Response> {
    runtime
        .feedback_board(params.to_query())
        .await
        .map(Json)
        .map_err(|e| ApiError(e).into_response())
}

/// `GET /api/v1/companies/{id}/feedback/board`.
async fn list(
    CompanyAuth(auth): CompanyAuth,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<BoardParams>,
) -> Result<Json<BoardPage>, Response> {
    page(addressed(&state, &auth, &id)?, &params).await
}

/// `GET /api/v1/company/feedback/board` (single-company alias).
async fn list_single(
    CompanyAuth(auth): CompanyAuth,
    State(state): State<AppState>,
    Query(params): Query<BoardParams>,
) -> Result<Json<BoardPage>, Response> {
    page(addressed_sole(&state, &auth)?, &params).await
}

/// `GET /api/v1/companies/{id}/feedback/board/{item}`.
async fn detail(
    CompanyAuth(auth): CompanyAuth,
    State(state): State<AppState>,
    Path((id, item)): Path<(String, String)>,
) -> Result<Json<BoardDetail>, Response> {
    addressed(&state, &auth, &id)?
        .feedback_board_item(&item)
        .await
        .map(Json)
        .map_err(|e| ApiError(e).into_response())
}

/// `GET /api/v1/company/feedback/board/{item}` (single-company alias).
async fn detail_single(
    CompanyAuth(auth): CompanyAuth,
    State(state): State<AppState>,
    Path(item): Path<String>,
) -> Result<Json<BoardDetail>, Response> {
    addressed_sole(&state, &auth)?
        .feedback_board_item(&item)
        .await
        .map(Json)
        .map_err(|e| ApiError(e).into_response())
}

/// `POST /api/v1/companies/{id}/feedback/board/{item}/vote`.
async fn vote(
    CompanyAuth(auth): CompanyAuth,
    State(state): State<AppState>,
    Path((id, item)): Path<(String, String)>,
    Json(body): Json<VoteRequest>,
) -> Result<Json<BoardItem>, Response> {
    addressed(&state, &auth, &id)?
        .vote_feedback_board(&item, body.value)
        .await
        .map(Json)
        .map_err(|e| ApiError(e).into_response())
}

/// `POST /api/v1/company/feedback/board/{item}/vote` (single-company alias).
async fn vote_single(
    CompanyAuth(auth): CompanyAuth,
    State(state): State<AppState>,
    Path(item): Path<String>,
    Json(body): Json<VoteRequest>,
) -> Result<Json<BoardItem>, Response> {
    addressed_sole(&state, &auth)?
        .vote_feedback_board(&item, body.value)
        .await
        .map(Json)
        .map_err(|e| ApiError(e).into_response())
}

/// `POST /api/v1/companies/{id}/feedback/board/{item}/comments`.
async fn comment(
    CompanyAuth(auth): CompanyAuth,
    State(state): State<AppState>,
    Path((id, item)): Path<(String, String)>,
    Json(body): Json<CommentRequest>,
) -> Result<Json<BoardComment>, Response> {
    let runtime = addressed(&state, &auth, &id)?;
    let text = checked_comment(&body.body)?;
    runtime
        .comment_feedback_board(&item, text)
        .await
        .map(Json)
        .map_err(|e| ApiError(e).into_response())
}

/// `POST /api/v1/company/feedback/board/{item}/comments` (single-company alias).
async fn comment_single(
    CompanyAuth(auth): CompanyAuth,
    State(state): State<AppState>,
    Path(item): Path<String>,
    Json(body): Json<CommentRequest>,
) -> Result<Json<BoardComment>, Response> {
    let runtime = addressed_sole(&state, &auth)?;
    let text = checked_comment(&body.body)?;
    runtime
        .comment_feedback_board(&item, text)
        .await
        .map(Json)
        .map_err(|e| ApiError(e).into_response())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn params_default_to_the_hot_board_and_the_console_page_size() {
        let query = BoardParams::default().to_query();
        assert_eq!(query.sort, BoardSort::Hot);
        assert_eq!(query.kind, None);
        assert_eq!(query.status, None);
        assert_eq!(query.page, 1);
        assert_eq!(query.limit, DEFAULT_LIMIT);
    }

    #[test]
    fn params_read_every_filter_the_console_sends() {
        let params = BoardParams {
            sort: Some("new".to_string()),
            kind: Some("bug".to_string()),
            status: Some("planned".to_string()),
            page: Some(3),
            limit: Some(5),
        };
        let query = params.to_query();
        assert_eq!(query.sort, BoardSort::New);
        assert_eq!(query.kind, Some(BoardKind::Bug));
        assert_eq!(query.status, Some(BoardStatus::Planned));
        assert_eq!(query.page, 3);
        assert_eq!(query.limit, 5);
    }

    #[test]
    fn nonsense_filters_are_ignored_rather_than_refused() {
        let params = BoardParams {
            sort: Some("sideways".to_string()),
            kind: Some("wish".to_string()),
            status: Some("someday".to_string()),
            page: Some(0),
            limit: Some(10_000),
        };
        let query = params.to_query();
        // Falls back to the default board rather than 400ing a hand-edited URL.
        assert_eq!(query.sort, BoardSort::Hot);
        assert_eq!(query.kind, None);
        assert_eq!(query.status, None);
        // And the page bounds are corrected into what the hub accepts.
        assert_eq!(query.page, 1);
        assert_eq!(query.limit, crate::feedback::board::MAX_LIMIT);
    }

    #[test]
    fn an_empty_comment_never_reaches_the_hub() {
        assert!(checked_comment("   \n ").is_err());
        assert_eq!(checked_comment("  hi  ").unwrap(), "hi");
    }
}
