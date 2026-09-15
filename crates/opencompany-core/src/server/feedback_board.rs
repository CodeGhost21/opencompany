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
use axum::response::IntoResponse;
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
use crate::server::graphql::auth::GqlAuth;
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
        .route(
            "/api/v1/company/feedback/board/{item}/vote",
            post(vote_single),
        )
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
    auth: &GqlAuth,
    id: &str,
) -> Result<Arc<CompanyRuntime>, crate::server::Rejection> {
    let company = CompanyId::new(id);
    if let Some(resp) = authorize_address(state, auth, &company) {
        return Err(resp.into());
    }
    lookup(state, id).map_err(|error| IntoResponse::into_response(error).into())
}

/// The sole company, authorized the same way.
fn addressed_sole(
    state: &AppState,
    auth: &GqlAuth,
) -> Result<Arc<CompanyRuntime>, crate::server::Rejection> {
    let runtime = sole(state)?;
    if let Some(resp) = authorize_address(state, auth, runtime.id()) {
        return Err(resp.into());
    }
    Ok(runtime)
}

/// Refuses an empty comment before it costs a hub round trip.
fn checked_comment(body: &str) -> Result<&str, crate::server::Rejection> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Err(ApiError(OpenCompanyError::InvalidRequest(
            "comment is empty".to_string(),
        ))
        .into_response()
        .into());
    }
    Ok(trimmed)
}

async fn page(
    runtime: Arc<CompanyRuntime>,
    params: &BoardParams,
) -> Result<Json<BoardPage>, crate::server::Rejection> {
    runtime
        .feedback_board(params.to_query())
        .await
        .map(Json)
        .map_err(|e| ApiError(e).into_response().into())
}

/// `GET /api/v1/companies/{id}/feedback/board`.
async fn list(
    CompanyAuth(auth): CompanyAuth,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<BoardParams>,
) -> Result<Json<BoardPage>, crate::server::Rejection> {
    page(addressed(&state, &auth, &id)?, &params).await
}

/// `GET /api/v1/company/feedback/board` (single-company alias).
async fn list_single(
    CompanyAuth(auth): CompanyAuth,
    State(state): State<AppState>,
    Query(params): Query<BoardParams>,
) -> Result<Json<BoardPage>, crate::server::Rejection> {
    page(addressed_sole(&state, &auth)?, &params).await
}

/// `GET /api/v1/companies/{id}/feedback/board/{item}`.
async fn detail(
    CompanyAuth(auth): CompanyAuth,
    State(state): State<AppState>,
    Path((id, item)): Path<(String, String)>,
) -> Result<Json<BoardDetail>, crate::server::Rejection> {
    addressed(&state, &auth, &id)?
        .feedback_board_item(&item)
        .await
        .map(Json)
        .map_err(|e| ApiError(e).into_response().into())
}

/// `GET /api/v1/company/feedback/board/{item}` (single-company alias).
async fn detail_single(
    CompanyAuth(auth): CompanyAuth,
    State(state): State<AppState>,
    Path(item): Path<String>,
) -> Result<Json<BoardDetail>, crate::server::Rejection> {
    addressed_sole(&state, &auth)?
        .feedback_board_item(&item)
        .await
        .map(Json)
        .map_err(|e| ApiError(e).into_response().into())
}

/// `POST /api/v1/companies/{id}/feedback/board/{item}/vote`.
async fn vote(
    CompanyAuth(auth): CompanyAuth,
    State(state): State<AppState>,
    Path((id, item)): Path<(String, String)>,
    Json(body): Json<VoteRequest>,
) -> Result<Json<BoardItem>, crate::server::Rejection> {
    addressed(&state, &auth, &id)?
        .vote_feedback_board(&item, body.value)
        .await
        .map(Json)
        .map_err(|e| ApiError(e).into_response().into())
}

/// `POST /api/v1/company/feedback/board/{item}/vote` (single-company alias).
async fn vote_single(
    CompanyAuth(auth): CompanyAuth,
    State(state): State<AppState>,
    Path(item): Path<String>,
    Json(body): Json<VoteRequest>,
) -> Result<Json<BoardItem>, crate::server::Rejection> {
    addressed_sole(&state, &auth)?
        .vote_feedback_board(&item, body.value)
        .await
        .map(Json)
        .map_err(|e| ApiError(e).into_response().into())
}

/// `POST /api/v1/companies/{id}/feedback/board/{item}/comments`.
async fn comment(
    CompanyAuth(auth): CompanyAuth,
    State(state): State<AppState>,
    Path((id, item)): Path<(String, String)>,
    Json(body): Json<CommentRequest>,
) -> Result<Json<BoardComment>, crate::server::Rejection> {
    let runtime = addressed(&state, &auth, &id)?;
    let text = checked_comment(&body.body)?;
    runtime
        .comment_feedback_board(&item, text)
        .await
        .map(Json)
        .map_err(|e| ApiError(e).into_response().into())
}

/// `POST /api/v1/company/feedback/board/{item}/comments` (single-company alias).
async fn comment_single(
    CompanyAuth(auth): CompanyAuth,
    State(state): State<AppState>,
    Path(item): Path<String>,
    Json(body): Json<CommentRequest>,
) -> Result<Json<BoardComment>, crate::server::Rejection> {
    let runtime = addressed_sole(&state, &auth)?;
    let text = checked_comment(&body.body)?;
    runtime
        .comment_feedback_board(&item, text)
        .await
        .map(Json)
        .map_err(|e| ApiError(e).into_response().into())
}

#[cfg(test)]
mod test {
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::*;
    use crate::company::CompanyManifest;
    use crate::feedback::MockTinyHumansClient;
    use crate::runtime::RuntimeBuilder;
    use crate::server::router;
    use crate::{AppConfig, AppState};

    fn manifest() -> CompanyManifest {
        toml::from_str(
            r#"
            [company]
            name = "Acme"
            handle = "acme"
            [[agent]]
            id = "dana_roe"
            role = "Analyst"
            [policy]
            mode = "full"
            "#,
        )
        .unwrap()
    }

    /// Two board items so filtering, ordering and paging all have something to
    /// bite on.
    fn seeded_board() -> Vec<BoardItem> {
        vec![
            BoardItem {
                id: "one".to_string(),
                kind: BoardKind::Feature,
                title: "Weekly digest".to_string(),
                body: "Send one on Mondays".to_string(),
                status: BoardStatus::Open,
                author: Some("rin".to_string()),
                upvotes: 3,
                downvotes: 1,
                score: 2,
                comment_count: 0,
                my_vote: VoteValue::None,
                issue_url: None,
                created_at: "2026-01-02T00:00:00.000Z".to_string(),
            },
            BoardItem {
                id: "two".to_string(),
                kind: BoardKind::Bug,
                title: "Totals are wrong".to_string(),
                body: "The invoice doubles tax".to_string(),
                status: BoardStatus::Planned,
                author: None,
                upvotes: 9,
                downvotes: 0,
                score: 9,
                comment_count: 1,
                my_vote: VoteValue::Up,
                issue_url: Some("https://example.test/issues/7".to_string()),
                created_at: "2026-01-01T00:00:00.000Z".to_string(),
            },
        ]
    }

    /// A single-company host, optionally provisioned with a hub serving `board`.
    async fn state_with(
        home: &std::path::Path,
        hub: Option<Arc<MockTinyHumansClient>>,
    ) -> AppState {
        let id = CompanyId::new("acme");
        let mut builder = RuntimeBuilder::new(home.to_path_buf(), manifest()).with_id(id.clone());
        if let Some(hub) = hub {
            builder = builder.with_tinyhumans_feedback(hub);
        }
        let runtime = builder.build().await.unwrap();
        let state = AppState::new(AppConfig::default());
        state.registry().insert(id, Arc::new(runtime));
        crate::server::test_support::seed_fixed_admin(&state, "acme").await;
        state
    }

    async fn call(
        app: &axum::Router,
        method: &str,
        uri: &str,
        body: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header("cookie", crate::server::test_support::fixed_cookie("acme"));
        if body.is_some() {
            request = request.header("content-type", "application/json");
        }
        let response = app
            .clone()
            .oneshot(
                request
                    .body(
                        body.map(|b| Body::from(b.to_string()))
                            .unwrap_or(Body::empty()),
                    )
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    fn home() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("opencompany-board-")
            .tempdir()
            .expect("tempdir")
    }

    // The board reaches the console filtered, ordered and paged — and a vote or
    // a comment travels back to the hub and returns the updated row.
    #[tokio::test]
    async fn lists_filters_votes_and_comments() {
        let home_dir = home();
        let hub = Arc::new(MockTinyHumansClient::new().with_board(seeded_board()));
        let state = state_with(home_dir.path(), Some(hub)).await;
        let app = router(state);

        // Default board: both rows, highest score first.
        let (status, value) = call(&app, "GET", "/api/v1/company/feedback/board", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["total"], 2);
        assert_eq!(value["items"][0]["id"], "two");
        assert_eq!(value["items"][0]["my_vote"], 1);
        assert_eq!(value["items"][1]["id"], "one");

        // Filtered to features: one row, and the total follows the filter.
        let (_, value) = call(
            &app,
            "GET",
            "/api/v1/company/feedback/board?type=feature&sort=new",
            None,
        )
        .await;
        assert_eq!(value["total"], 1);
        assert_eq!(value["items"][0]["id"], "one");

        // Paging past the end is an empty page, not an error.
        let (status, value) =
            call(&app, "GET", "/api/v1/company/feedback/board?page=9", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["items"].as_array().unwrap().len(), 0);
        assert_eq!(value["total"], 2);

        // An upvote lands and comes back on the updated row.
        let (status, value) = call(
            &app,
            "POST",
            "/api/v1/company/feedback/board/one/vote",
            Some(r#"{"value":1}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["upvotes"], 4);
        assert_eq!(value["score"], 3);
        assert_eq!(value["my_vote"], 1);

        // Voting again does not double-count: the previous vote is retracted first.
        let (_, value) = call(
            &app,
            "POST",
            "/api/v1/company/feedback/board/one/vote",
            Some(r#"{"value":-1}"#),
        )
        .await;
        assert_eq!(value["upvotes"], 3);
        assert_eq!(value["downvotes"], 2);
        assert_eq!(value["my_vote"], -1);

        // A comment posts and shows up on the item detail with its count bumped.
        let (status, _) = call(
            &app,
            "POST",
            "/api/v1/company/feedback/board/one/comments",
            Some(r#"{"body":"yes please"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (_, value) = call(&app, "GET", "/api/v1/company/feedback/board/one", None).await;
        assert_eq!(value["item"]["comment_count"], 1);
        assert_eq!(value["comments"][0]["body"], "yes please");

        // An empty comment never reaches the hub.
        let (status, _) = call(
            &app,
            "POST",
            "/api/v1/company/feedback/board/one/comments",
            Some(r#"{"body":"   "}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // An unknown item is the hub's 404, surfaced as one.
        let (status, _) = call(&app, "GET", "/api/v1/company/feedback/board/nope", None).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
    }

    // Without a TinyHumans credential there is no board at all — a 404 the
    // console reads as "hide the surface", never an empty-looking board.
    #[tokio::test]
    async fn an_unprovisioned_instance_has_no_board() {
        let home_dir = home();
        let state = state_with(home_dir.path(), None).await;
        let app = router(state);

        let (status, value) = call(&app, "GET", "/api/v1/company/feedback/board", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(value["code"], "tinyhumans_no_board");

        let (status, _) = call(
            &app,
            "POST",
            "/api/v1/company/feedback/board/one/vote",
            Some(r#"{"value":1}"#),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    // The per-company form addresses the same board as the single-company alias.
    #[tokio::test]
    async fn the_company_scoped_route_answers_too() {
        let home_dir = home();
        let hub = Arc::new(MockTinyHumansClient::new().with_board(seeded_board()));
        let state = state_with(home_dir.path(), Some(hub)).await;
        let app = router(state);

        let (status, value) = call(
            &app,
            "GET",
            "/api/v1/companies/acme/feedback/board?status=planned",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["total"], 1);
        assert_eq!(value["items"][0]["id"], "two");
    }

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
