use super::*;

// Used by the resolver tests below; the module body itself resolves its
// credential through `company::composio::resolve_credential`.
use crate::company::company_key;
use crate::ports::types::SecretValue;

/// The bearer a config would present right now.
async fn token_of(config: &TenantComposio) -> Option<String> {
    config.current_token().await.expect("resolves")
}

fn config_with(credential: Credential) -> Option<TenantComposio> {
    Some(TenantComposio::new(
        "https://api.tinyhumans.ai",
        credential,
        vec!["gmail".to_string()],
    ))
}

use std::net::SocketAddr;

use crate::company::composio::CatalogEntry;

use axum::Router;
use axum::routing::{get, post};
use serde_json::{Value, json};

/// Mock `POST /agent-integrations/composio/authorize` — returns a hosted
/// connect URL inside the backend's `{success,data}` envelope.
async fn authorize_handler() -> axum::Json<Value> {
    axum::Json(json!({
        "success": true,
        "data": { "connectUrl": "https://connect.composio.dev/abc", "connectionId": "conn-xyz" }
    }))
}

/// Mock `GET /agent-integrations/composio/connections` — gmail has one
/// active + one pending row (→ connected), slack only pending (→ not
/// connected), notion active (filtered out unless allowlisted).
///
/// The identity fields exercise each arm of the account-label precedence
/// (issue #404): `c1` publishes an email, `c2` only a blank one plus a
/// workspace, `c3` only a username, `c4` nothing at all.
async fn connections_handler() -> axum::Json<Value> {
    axum::Json(json!({
        "success": true,
        "data": { "connections": [
            {
                "id": "c1", "toolkit": "gmail", "status": "ACTIVE",
                "createdAt": "2026-08-01T10:00:00Z",
                "accountEmail": " ops@acme.test ",
                "username": "ignored-when-an-email-is-present"
            },
            {
                "id": "c2", "toolkit": "gmail", "status": "INITIATED",
                "accountEmail": "   ",
                "workspace": "Acme Workspace"
            },
            { "id": "c3", "toolkit": "slack", "status": "INITIATED", "username": "acme-bot" },
            { "id": "c4", "toolkit": "notion", "status": "ACTIVE" }
        ] }
    }))
}

/// Mock `GET /agent-integrations/composio/toolkits` — the dynamic catalog
/// shape (backend #1012): a `toolkits` allowlist plus a `catalog[]` whose
/// entries carry an `enabled` gate. `zendesk` is present but not connectable
/// and must not be advertised; the casing and whitespace on `HubSpot` must
/// normalise.
///
/// Entries carry the display metadata (`logo`, `description`, `categories`)
/// the backend actually publishes — issue #600 is that all of it was dropped
/// on the way through, so a mock that omitted it could not have caught the
/// bug.
async fn toolkits_handler() -> axum::Json<Value> {
    axum::Json(json!({
        "success": true,
        "data": {
            "toolkits": ["gmail", "slack"],
            "catalog": [
                {
                    "slug": " HubSpot ",
                    "name": "HubSpot",
                    "enabled": true,
                    "logo": " https://logos.composio.dev/api/hubspot ",
                    "description": "  CRM and marketing automation.  ",
                    "categories": ["crm", " marketing ", ""]
                },
                {
                    "slug": "gmail",
                    "name": "Gmail",
                    "enabled": true,
                    "description": "Send and read email.",
                    "categories": ["email"]
                },
                { "slug": "zendesk", "name": "Zendesk", "enabled": false },
                { "slug": "gmail", "name": "Gmail (dup)", "enabled": true }
            ]
        }
    }))
}

/// Mock toolkits route for a backend predating the dynamic catalog: the
/// plain slug allowlist and no `catalog[]` at all.
async fn legacy_toolkits_handler() -> axum::Json<Value> {
    axum::Json(json!({
        "success": true,
        "data": { "toolkits": ["Notion", "gmail", ""] }
    }))
}

async fn spawn_backend() -> String {
    spawn_backend_with(get(toolkits_handler)).await
}

async fn spawn_backend_with(toolkits: axum::routing::MethodRouter) -> String {
    let app = Router::new()
        .route(
            "/agent-integrations/composio/authorize",
            post(authorize_handler),
        )
        .route(
            "/agent-integrations/composio/connections",
            get(connections_handler),
        )
        .route("/agent-integrations/composio/toolkits", toolkits);
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn config(url: &str, toolkits: Vec<String>) -> TenantComposio {
    TenantComposio::new(
        url.to_string(),
        Credential::from_value("tenant-token"),
        toolkits,
    )
}

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::get;
use oh::tools::traits::Tool;
use openhuman_core as oh;
use serde_json::{Value, json};

/// Shared recorder for every `Authorization` header the mock backend saw.
type AuthLog = Arc<Mutex<Vec<String>>>;

/// The mock `/agent-integrations/composio/connections` handler: records the
/// bearer it received and returns a `{success,data}` envelope whose single
/// connection's `account_email` is derived from that bearer — so a caller can
/// prove it only ever sees *its own* tenant's data.
async fn connections(State(log): State<AuthLog>, headers: HeaderMap) -> axum::Json<Value> {
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    log.lock().unwrap().push(auth.clone());
    // Derive tenant identity purely from the presented bearer.
    let email = if auth.contains("token-a") {
        "a@example.com"
    } else if auth.contains("token-b") {
        "b@example.com"
    } else {
        "unknown@example.com"
    };
    axum::Json(json!({
        "success": true,
        "data": {
            "connections": [
                { "id": "conn-1", "toolkit": "gmail", "status": "ACTIVE", "accountEmail": email }
            ]
        }
    }))
}

/// Spawn the mock backend on an ephemeral port; returns its base URL + the
/// auth-header recorder.
async fn spawn_backend() -> (String, AuthLog) {
    let log: AuthLog = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/agent-integrations/composio/connections", get(connections))
        .with_state(log.clone());
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), log)
}

fn config(url: &str, token: &str) -> TenantComposio {
    TenantComposio::new(url, Credential::from_value(token), Vec::new())
}

fn list_connections_tool(config: &TenantComposio) -> Box<dyn Tool> {
    let metering = ComposioMetering {
        company: CompanyId::new("acme"),
        agent: "ceo".to_string(),
        meter: None,
    };
    composio_tools(config, metering)
        .into_iter()
        .find(|t| t.name() == "composio_list_connections")
        .expect("composio_list_connections tool present")
}

// --- FAIL-axis: what each Composio tool does when the backend fails ------

use axum::http::StatusCode;
use axum::routing::post;

/// A handler that always 5xxs, recording each hit so a caller can count the
/// requests a single tool call actually made.
async fn always_500(State(log): State<AuthLog>) -> (StatusCode, axum::Json<Value>) {
    log.lock().unwrap().push("hit".to_string());
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        axum::Json(json!({ "success": false, "error": "upstream exploded" })),
    )
}

/// Spawn a backend whose every Composio route 5xxs.
async fn spawn_failing_backend() -> (String, AuthLog) {
    let log: AuthLog = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/agent-integrations/composio/toolkits", get(always_500))
        .route("/agent-integrations/composio/tools", get(always_500))
        .route("/agent-integrations/composio/connections", get(always_500))
        .with_state(log.clone());
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), log)
}

fn tool_named(config: &TenantComposio, name: &str) -> Box<dyn Tool> {
    let metering = ComposioMetering {
        company: CompanyId::new("acme"),
        agent: "ceo".to_string(),
        meter: None,
    };
    composio_tools(config, metering)
        .into_iter()
        .find(|t| t.name() == name)
        .unwrap_or_else(|| panic!("`{name}` tool present"))
}


#[path = "composio_tests_part1.rs"]
mod tests_part1;
#[path = "composio_tests_part2.rs"]
mod tests_part2;
#[path = "composio_tests_part3.rs"]
mod tests_part3;
