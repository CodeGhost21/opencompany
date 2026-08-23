//! HTTP transport for the host-side ACP session model.
//!
//! ACP's native transports are stdio and WebSocket JSON-RPC. OpenCompany uses
//! HTTP JSON-RPC at its public edge: one request carries one RPC call, while
//! the returned `updates` array preserves the protocol's ordered session
//! updates for callers that cannot hold a socket open. The endpoint is always
//! authenticated with the same company authorization as the operator API.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{Value, json};

use crate::AppState;
use crate::ports::types::{CompanyEvent, CompanyId};
use crate::server::platform_auth::{CompanyAuth, authorize_address};

pub(super) fn router() -> Router<AppState> {
    Router::new().route("/acp", post(call))
}

async fn call(
    State(state): State<AppState>,
    CompanyAuth(auth): CompanyAuth,
    Json(request): Json<Value>,
) -> Response {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": "2025-06-18",
            "serverInfo": { "name": "opencompany", "version": env!("CARGO_PKG_VERSION") },
            "capabilities": { "session": {} }
        })),
        "session/new" => open_session(&state, &auth, &params).await,
        "session/list" => list_sessions(&state, &params),
        "session/prompt" => prompt(&state, &auth, &params).await,
        // There is no safe generic interruption point inside an arbitrary
        // company cycle. Say so rather than claiming a cancel that cannot stop
        // provider work or tools already in flight.
        "session/cancel" => Err("OpenCompany does not yet support cancelling an in-flight company cycle".to_string()),
        _ => Err(format!("unsupported ACP method `{method}`")),
    };
    match result {
        Ok(result) => Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response(),
        Err(message) => Json(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32602, "message": message } })).into_response(),
    }
}

fn connection(params: &Value) -> Result<&str, String> {
    params.get("_meta").and_then(|m| m.get("opencompany/connectionId")).and_then(Value::as_str)
        .filter(|id| !id.is_empty()).ok_or_else(|| "`_meta.opencompany/connectionId` is required".to_string())
}

fn target(params: &Value) -> Result<(&str, String, Option<String>), String> {
    let meta = params.get("_meta").and_then(|m| m.get("opencompany")).ok_or_else(|| "`_meta.opencompany` is required".to_string())?;
    let company = meta.get("company").and_then(Value::as_str).filter(|v| !v.is_empty()).ok_or_else(|| "`_meta.opencompany.company` is required".to_string())?;
    let chat = meta.get("chat").and_then(Value::as_str).filter(|v| !v.is_empty()).unwrap_or(crate::server::ops::language::DEFAULT_DESK).to_string();
    let agent = meta.get("agentId").and_then(Value::as_str).map(str::to_string);
    Ok((company, chat, agent))
}

async fn open_session(state: &AppState, auth: &crate::server::graphql::auth::GqlAuth, params: &Value) -> Result<Value, String> {
    let (company, chat, agent_id) = target(params)?;
    let company = CompanyId::new(company);
    if authorize_address(state, auth, &company).is_some() { return Err("not authorized for this company".to_string()); }
    if state.registry().get(&company).is_none() { return Err(format!("company `{company}` was not found")); }
    let id = uuid::Uuid::new_v4().to_string();
    state.acp_sessions().insert(connection(params)?, super::AcpSession { id: id.clone(), company, chat, agent_id });
    Ok(json!({ "sessionId": id, "_meta": super::session::cwd_meta("server-side company workspace") }))
}

fn list_sessions(state: &AppState, params: &Value) -> Result<Value, String> {
    let sessions = state.acp_sessions().list(connection(params)?).into_iter().map(|s| json!({ "sessionId": s.id, "_meta": { "opencompany": { "company": s.company, "chat": s.chat, "agentId": s.agent_id } } })).collect::<Vec<_>>();
    Ok(json!({ "sessions": sessions }))
}

async fn prompt(state: &AppState, auth: &crate::server::graphql::auth::GqlAuth, params: &Value) -> Result<Value, String> {
    let session_id = params.get("sessionId").and_then(Value::as_str).ok_or_else(|| "`sessionId` is required".to_string())?;
    let session = state.acp_sessions().get(connection(params)?, session_id).ok_or_else(|| "unknown ACP session".to_string())?;
    if authorize_address(state, auth, &session.company).is_some() { return Err("not authorized for this company".to_string()); }
    let text = params.get("prompt").and_then(|p| p.get("text")).and_then(Value::as_str).ok_or_else(|| "`prompt.text` is required".to_string())?;
    let runtime = state.registry().get(&session.company).ok_or_else(|| format!("company `{}` was not found", session.company))?;
    let report = runtime.run_cycle(vec![CompanyEvent::OperatorMessage { text: text.to_string(), by: None, chat: Some(session.chat.clone()), parent: None, deliverable: None }]).await.map_err(|e| e.to_string())?;
    let updates = report.responses.into_iter().map(|reply| json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": reply.text } })).collect::<Vec<_>>();
    Ok(json!({ "stopReason": "end_turn", "updates": updates }))
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn target_requires_an_explicit_company() {
        assert!(target(&json!({ "_meta": { "opencompany": {} } })).is_err());
    }
    #[test]
    fn target_defaults_to_general() {
        let (_, chat, _) = target(&json!({ "_meta": { "opencompany": { "company": "acme" } } })).unwrap();
        assert_eq!(chat, crate::server::ops::language::DEFAULT_DESK);
    }
}
