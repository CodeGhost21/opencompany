//! The MCP protocol surface: JSON-RPC 2.0 over a single HTTP POST route.
//!
//! This is the whole contract OpenCompany's client speaks — `initialize`,
//! `tools/list`, `tools/call`, plus an ack for notifications. There is no SSE
//! and no stdio: `McpServer::endpoint` is documented as `http(s)://`-only and
//! a `command` is a validation error, because tenant agents run in a shared
//! container where per-tenant subprocesses are out of scope.
//!
//! # Tool failures are results, not JSON-RPC errors
//!
//! A JSON-RPC `error` means the call could not be dispatched. A Chargebee
//! rejection ("customer not found") dispatched fine and produced an answer the
//! agent can act on, so it comes back as a normal result carrying `isError`.
//! Collapsing the two would leave the agent unable to distinguish "this server
//! is broken" from "that customer does not exist".

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde_json::{Value, json};
use std::sync::Arc;

use super::client::ChargebeeClient;
use super::tools;

/// The protocol revision this server implements, matching the revision
/// OpenCompany's client announces.
pub const PROTOCOL_VERSION: &str = "2025-11-25";

/// JSON-RPC "method not found".
const METHOD_NOT_FOUND: i64 = -32601;

/// Shared handler state.
#[derive(Clone)]
pub struct ServerState {
    client: Arc<ChargebeeClient>,
    /// When set, every request must present this exact bearer token.
    ///
    /// Optional because of a constraint in the consuming runtime: a server
    /// listed in `[[default_mcp_server]]` is rejected if it names an
    /// `auth_secret`, on the grounds that a default ships to every company
    /// unattended and must carry no secret. A demo deployment therefore runs
    /// this unset and relies on network placement; a production deployment sets
    /// it and is registered per company from the console instead, where the
    /// token lives in that company's own secret store.
    bearer: Option<String>,
}

impl ServerState {
    /// Builds handler state.
    pub fn new(client: ChargebeeClient, bearer: Option<String>) -> Self {
        Self {
            client: Arc::new(client),
            bearer: bearer.filter(|b| !b.trim().is_empty()),
        }
    }
}

/// The router: `POST /mcp` for the protocol, `GET /healthz` for liveness.
pub fn router(state: ServerState) -> Router {
    Router::new()
        .route("/mcp", post(handle))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(state)
}

/// Whether `headers` carries the configured bearer, if one is configured.
fn authorized(state: &ServerState, headers: &HeaderMap) -> bool {
    let Some(expected) = &state.bearer else {
        return true;
    };
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|token| token == expected)
}

async fn handle(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    if !authorized(&state, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response();
    }

    let method = body.get("method").and_then(Value::as_str).unwrap_or("");
    let id = body.get("id").cloned();

    // No `id` means a notification (`notifications/initialized` is the one the
    // client actually sends). It expects an ack with no `result` and no `error`.
    let Some(id) = id else {
        return Json(json!({"jsonrpc": "2.0"})).into_response();
    };

    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {"tools": {}},
            "serverInfo": {
                "name": "opencompany-chargebee",
                "version": env!("CARGO_PKG_VERSION"),
            },
        })),
        "tools/list" => Ok(json!({
            "tools": tools::descriptors()
                .into_iter()
                .map(|d| json!({
                    "name": d.name,
                    "description": d.description,
                    "inputSchema": d.input_schema,
                }))
                .collect::<Vec<_>>()
        })),
        "tools/call" => {
            let params = body.get("params").cloned().unwrap_or_else(|| json!({}));
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));

            Ok(match tools::call(&state.client, &name, args).await {
                Ok(value) => json!({
                    "content": [{
                        "type": "text",
                        "text": serde_json::to_string_pretty(&value)
                            .unwrap_or_else(|_| value.to_string()),
                    }],
                    "isError": false,
                }),
                Err(e) => json!({
                    "content": [{"type": "text", "text": e.to_string()}],
                    "isError": true,
                }),
            })
        }
        other => Err(format!("unknown method `{other}`")),
    };

    match result {
        Ok(result) => Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response(),
        Err(message) => Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": METHOD_NOT_FOUND, "message": message},
        }))
        .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chargebee::types::ChargebeeConfig;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn state(bearer: Option<&str>) -> ServerState {
        let client = ChargebeeClient::with_base_url(
            ChargebeeConfig {
                site: "test".to_string(),
                api_key: "key".to_string(),
            },
            "http://127.0.0.1:0".to_string(),
        )
        .expect("client builds");
        ServerState::new(client, bearer.map(str::to_string))
    }

    async fn rpc(state: ServerState, body: Value, auth: Option<&str>) -> (StatusCode, Value) {
        let mut req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json");
        if let Some(token) = auth {
            req = req.header("authorization", format!("Bearer {token}"));
        }
        let response = router(state)
            .oneshot(
                req.body(Body::from(body.to_string()))
                    .expect("request builds"),
            )
            .await
            .expect("router responds");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, value)
    }

    #[tokio::test]
    async fn initialize_announces_the_clients_protocol_revision() {
        let (status, body) = rpc(
            state(None),
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"}),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(body["id"], 1);
        // Without this the client discovers zero tools despite a healthy probe.
        assert!(body["result"]["capabilities"]["tools"].is_object());
    }

    #[tokio::test]
    async fn tools_list_advertises_all_five_with_schemas() {
        let (_, body) = rpc(
            state(None),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
            None,
        )
        .await;
        let tools = body["result"]["tools"].as_array().expect("tools array");
        assert_eq!(tools.len(), 5);
        for tool in tools {
            assert!(tool["name"].is_string());
            assert_eq!(tool["inputSchema"]["type"], "object");
        }
    }

    #[tokio::test]
    async fn a_notification_is_acked_with_neither_result_nor_error() {
        let (status, body) = rpc(
            state(None),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.get("result").is_none(), "got: {body}");
        assert!(body.get("error").is_none(), "got: {body}");
    }

    #[tokio::test]
    async fn an_unknown_method_is_a_jsonrpc_error() {
        let (_, body) = rpc(
            state(None),
            json!({"jsonrpc": "2.0", "id": 3, "method": "resources/list"}),
            None,
        )
        .await;
        assert_eq!(body["error"]["code"], METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn a_rejected_tool_call_is_a_result_not_an_error() {
        // Local validation rejects this before any network call, so the test
        // exercises the error *shape* without reaching Chargebee.
        let (_, body) = rpc(
            state(None),
            json!({
                "jsonrpc": "2.0", "id": 4, "method": "tools/call",
                "params": {
                    "name": "chargebee_record_payment",
                    "arguments": {
                        "invoice_id": "inv_1",
                        "amount_in_minor_units": 50_000,
                        "payment_method": "credit_card"
                    }
                }
            }),
            None,
        )
        .await;
        assert!(body.get("error").is_none(), "dispatch succeeded: {body}");
        assert_eq!(body["result"]["isError"], true);
        assert!(
            body["result"]["content"][0]["text"]
                .as_str()
                .expect("text content")
                .contains("bank_transfer")
        );
    }

    #[tokio::test]
    async fn a_configured_bearer_is_enforced() {
        let (status, _) = rpc(
            state(Some("s3cret")),
            json!({"jsonrpc": "2.0", "id": 5, "method": "tools/list"}),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (status, _) = rpc(
            state(Some("s3cret")),
            json!({"jsonrpc": "2.0", "id": 6, "method": "tools/list"}),
            Some("wrong"),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (status, body) = rpc(
            state(Some("s3cret")),
            json!({"jsonrpc": "2.0", "id": 7, "method": "tools/list"}),
            Some("s3cret"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["tools"].as_array().expect("tools").len(), 5);
    }

    #[tokio::test]
    async fn an_unset_bearer_leaves_the_server_open() {
        // The demo deployment depends on this: a default MCP server may carry
        // no credential, so an unset bearer must not fail closed.
        let (status, _) = rpc(
            state(None),
            json!({"jsonrpc": "2.0", "id": 8, "method": "tools/list"}),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }
}
