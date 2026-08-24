//! HTTP transport for the host-side ACP session model.
//!
//! ACP's native transports are stdio and WebSocket JSON-RPC. OpenCompany uses
//! HTTP JSON-RPC at its public edge: one request carries one RPC call, while
//! the returned `updates` array preserves the protocol's ordered session
//! updates for callers that cannot hold a socket open. The endpoint is always
//! authenticated with the same company authorization as the operator API.

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{Value, json};

use crate::AppState;
use crate::ports::types::{Actor, ActorKind, CompanyEvent, CompanyId};
use crate::server::graphql::auth::GqlAuth;
use crate::server::platform_auth::{CompanyAuth, authorize_address, refuse_until_password_changed};

/// The ACP protocol version this host speaks.
const PROTOCOL_VERSION: u64 = 1;

pub(super) fn router() -> Router<AppState> {
    Router::new().route("/acp", post(call))
}

async fn call(
    State(state): State<AppState>,
    CompanyAuth(auth): CompanyAuth,
    Json(request): Json<Value>,
) -> Response {
    // A temporary password is a boundary, not a suggestion: a user who has not
    // replaced it may not run company cycles over any surface, ACP included.
    // The same check `ScopedCompany` applies to the operator API.
    if let Some(resp) = refuse_until_password_changed(&auth) {
        return resp;
    }
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
    let result = match method {
        "initialize" => Ok(initialize_result()),
        "session/new" => open_session(&state, &auth, &params).await,
        "session/list" => list_sessions(&state, &auth, &params),
        "session/prompt" => prompt(&state, &auth, &params).await,
        "session/delete" => delete_session(&state, &auth, &params),
        // The HTTP edge has no socket whose closure sweeps a connection, so
        // the client ends its own connection explicitly.
        "session/disconnect" => disconnect(&state, &auth, &params),
        // There is no safe generic interruption point inside an arbitrary
        // company cycle. Say so rather than claiming a cancel that cannot stop
        // provider work or tools already in flight.
        "session/cancel" => Err(
            "OpenCompany does not yet support cancelling an in-flight company cycle".to_string(),
        ),
        _ => Err(format!("unsupported ACP method `{method}`")),
    };
    match result {
        Ok(result) => Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response(),
        Err(message) => Json(
            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32602, "message": message } }),
        )
        .into_response(),
    }
}

/// What this host answers an `initialize` with.
///
/// ACP, not MCP: `protocolVersion` is the integer ACP version and the result
/// carries `agentCapabilities` and `agentInfo`, where MCP's shape has a
/// date-valued `protocolVersion`, `capabilities` and `serverInfo`. A standard
/// ACP client (Zed, `acpx`) deserializes the ACP shape, so an MCP-shaped
/// answer would fail the handshake before any session could open.
///
/// Capabilities are what this host actually implements — `session/delete`
/// only. Everything omitted defaults to unsupported, which is the honest
/// answer rather than a promise a later turn would have to keep.
fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "agentCapabilities": { "session": { "delete": {} } },
        "agentInfo": {
            "name": "opencompany",
            "title": "OpenCompany",
            "version": env!("CARGO_PKG_VERSION"),
        },
    })
}

fn connection(params: &Value) -> Result<&str, String> {
    params
        .get("_meta")
        .and_then(|m| m.get("opencompany/connectionId"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "`_meta.opencompany/connectionId` is required".to_string())
}

fn target(params: &Value) -> Result<(&str, String, Option<String>), String> {
    let meta = params
        .get("_meta")
        .and_then(|m| m.get("opencompany"))
        .ok_or_else(|| "`_meta.opencompany` is required".to_string())?;
    let company = meta
        .get("company")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "`_meta.opencompany.company` is required".to_string())?;
    let chat = meta
        .get("chat")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .unwrap_or(crate::server::ops::language::DEFAULT_DESK)
        .to_string();
    let agent = meta
        .get("agentId")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok((company, chat, agent))
}

async fn open_session(state: &AppState, auth: &GqlAuth, params: &Value) -> Result<Value, String> {
    // Refused, not ignored: silently dropping `mcpServers` or
    // `additionalDirectories` would tell a client its tools and extra roots
    // were active when they never were. `session::refuse_unsupported` is the
    // single place that decision lives.
    if let Some(refusal) = super::session::refuse_unsupported(
        params
            .get("mcpServers")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]),
        params
            .get("additionalDirectories")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]),
    ) {
        return Err(refusal.message().to_string());
    }
    let (company, requested_chat, agent_id) = target(params)?;
    let company = CompanyId::new(company);
    if authorize_address(state, auth, &company).is_some() {
        return Err("not authorized for this company".to_string());
    }
    let runtime = state
        .registry()
        .get(&company)
        .ok_or_else(|| format!("company `{company}` was not found"))?;
    // A pin that names nobody must be refused now, not answered by the
    // orchestrator for the life of the session. `resolve_roster_agent_id` is
    // the same resolver the cycle's routing uses, so what passes here is
    // exactly what routes later.
    if let Some(id) = &agent_id {
        let record = runtime
            .store()
            .load(&company)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("company `{company}` was not found"))?;
        if record.resolve_roster_agent_id(id).is_none() {
            return Err(format!("`agentId` `{id}` is not a roster member"));
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    state.acp_sessions().insert(
        connection(params)?,
        super::AcpSession {
            id: id.clone(),
            company,
            // A pinned session answers as its member; see `AcpSession::thread_key`.
            chat: super::AcpSession::thread_key(&requested_chat, agent_id.as_deref()),
            agent_id,
        },
    );
    // ACP's result requires a `cwd`. On this host the workspace is server-side
    // and the client's own path is never used — the same truth `cwd_meta`
    // reports in `_meta` is stated as `cwd` so a strict client deserializes.
    let workspace = "server-side company workspace";
    Ok(json!({
        "sessionId": id,
        "cwd": workspace,
        "_meta": super::session::cwd_meta(workspace),
    }))
}

fn list_sessions(state: &AppState, auth: &GqlAuth, params: &Value) -> Result<Value, String> {
    // `connectionId` is caller-supplied and shared state, not a credential: an
    // authenticated tenant who learns another tenant's connection id must not
    // be able to enumerate its company, thread, agent and session ids. Each
    // entry is therefore filtered through the same `authorize_address` every
    // other company-scoped read gets.
    let sessions = state
        .acp_sessions()
        .list(connection(params)?)
        .into_iter()
        .filter(|s| authorize_address(state, auth, &s.company).is_none())
        .map(|s| {
            json!({
                "sessionId": s.id,
                "_meta": {
                    "opencompany": {
                        "company": s.company,
                        "chat": s.chat,
                        "agentId": s.agent_id,
                    }
                }
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({ "sessions": sessions }))
}

fn delete_session(state: &AppState, auth: &GqlAuth, params: &Value) -> Result<Value, String> {
    let session_id = params
        .get("sessionId")
        .and_then(Value::as_str)
        .ok_or_else(|| "`sessionId` is required".to_string())?;
    let conn = connection(params)?;
    let registry = state.acp_sessions();
    // Authorize against the session when it exists. A never-existing session
    // deletes silently — ACP says so, and an id is opaque enough that saying
    // "I never had that" leaks nothing useful.
    if let Some(session) = registry.get(conn, session_id)
        && authorize_address(state, auth, &session.company).is_some()
    {
        return Err("not authorized for this company".to_string());
    }
    registry.remove(conn, session_id);
    Ok(json!({}))
}

/// Closes the caller's connection: every session it holds whose company the
/// caller may address.
///
/// The HTTP edge has no socket whose closure sweeps a connection, so the
/// client ends its connection explicitly. Each session is authorized
/// individually, matching `session/list` — a caller may only close sessions it
/// could have listed, so one tenant cannot sweep another's by guessing its
/// connection id.
fn disconnect(state: &AppState, auth: &GqlAuth, params: &Value) -> Result<Value, String> {
    let conn = connection(params)?;
    let registry = state.acp_sessions();
    let ours: Vec<String> = registry
        .list(conn)
        .into_iter()
        .filter(|s| authorize_address(state, auth, &s.company).is_none())
        .map(|s| s.id.clone())
        .collect();
    for id in ours {
        registry.remove(conn, &id);
    }
    Ok(json!({}))
}

async fn prompt(state: &AppState, auth: &GqlAuth, params: &Value) -> Result<Value, String> {
    let session_id = params
        .get("sessionId")
        .and_then(Value::as_str)
        .ok_or_else(|| "`sessionId` is required".to_string())?;
    let session = state
        .acp_sessions()
        .get(connection(params)?, session_id)
        .ok_or_else(|| "unknown ACP session".to_string())?;
    if authorize_address(state, auth, &session.company).is_some() {
        return Err("not authorized for this company".to_string());
    }
    let text = prompt_text(params)?;
    let runtime = state
        .registry()
        .get(&session.company)
        .ok_or_else(|| format!("company `{}` was not found", session.company))?;
    // A paused or archived company refuses work on every other surface before
    // any cycle runs (chat, A2A, webhooks); the ACP prompt must hold the same
    // line. `run_cycle` checks only the process-local quiescing window, so
    // without this an operator's explicit pause/archive would still pay for
    // provider and tool work driven here.
    runtime.ensure_running().await.map_err(|e| e.to_string())?;
    // Keep the person, drop the credential, exactly as `ScopedCompany` does: a
    // human-authored ACP prompt is attributed to that user in the journal and
    // the audit trail. Only platform credentials stay anonymous.
    let by = match auth {
        GqlAuth::User(user) => Some(Actor {
            kind: ActorKind::User,
            id: user.user_id.clone(),
        }),
        GqlAuth::Platform(_) => None,
    };
    // A pinned session is answered by its member because the thread key stored
    // at session-open was already that member's DM channel (`dm:<member>`) —
    // the one chat key the cycle's routing (`responder_for`) resolves to a
    // specific roster member. The text is sent as-is, exactly as a console DM
    // is; no synthetic `@`-mention is needed, and one would only be dropped by
    // revalidation against a body that does not contain it.
    let mentions = runtime.resolve_mentions(&text, None, by.as_ref()).await;
    let report = runtime
        .run_cycle(vec![CompanyEvent::OperatorMessage {
            text,
            by,
            chat: Some(session.chat.clone()),
            parent: None,
            deliverable: None,
            mentions,
            attachments: Vec::new(),
        }])
        .await
        .map_err(|e| e.to_string())?;
    let updates = report
        .responses
        .into_iter()
        .map(|reply| {
            json!({
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": reply.text }
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({ "stopReason": "end_turn", "updates": updates }))
}

/// The text of an ACP `session/prompt`, from its content-block array.
///
/// ACP represents `params.prompt` as a `ContentBlock[]`, so a plain text
/// prompt arrives as `[{"type":"text","text":"hello"}]` — never as a `text`
/// field on the prompt itself. Only `text` blocks are consumed; every other
/// block type is rejected with its name, because this host advertises no
/// image, audio or embedded-context capability, and silently dropping a block
/// would send a turn without content the client believed it had sent.
fn prompt_text(params: &Value) -> Result<String, String> {
    let blocks = params
        .get("prompt")
        .and_then(Value::as_array)
        .ok_or_else(|| "`prompt` must be an array of content blocks".to_string())?;
    let mut text = String::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                let value = block
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "a text content block must carry a `text` string".to_string())?;
                text.push_str(value);
            }
            Some(other) => {
                return Err(format!(
                    "unsupported prompt content block type `{other}`; this host accepts text \
                     blocks only"
                ));
            }
            None => {
                return Err("a prompt content block must carry a `type`".to_string());
            }
        }
    }
    if text.is_empty() {
        return Err("`prompt` carried no text".to_string());
    }
    Ok(text)
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
        let (_, chat, _) =
            target(&json!({ "_meta": { "opencompany": { "company": "acme" } } })).unwrap();
        assert_eq!(chat, crate::server::ops::language::DEFAULT_DESK);
    }

    #[test]
    fn target_reads_an_agent_pin() {
        let (_, _, agent) =
            target(&json!({ "_meta": { "opencompany": { "company": "acme", "agentId": "ceo" } } }))
                .unwrap();
        assert_eq!(agent.as_deref(), Some("ceo"));
    }

    #[test]
    fn initialize_result_is_acp_shaped() {
        let result = initialize_result();
        // Numeric ACP version, not MCP's date-valued protocolVersion.
        assert_eq!(result["protocolVersion"], json!(1));
        assert!(result.get("capabilities").is_none(), "no MCP capabilities");
        assert!(result.get("serverInfo").is_none(), "no MCP serverInfo");
        // The two ACP-required result fields.
        assert!(result.get("agentCapabilities").is_some());
        assert!(result.get("agentInfo").is_some());
        assert!(result["agentInfo"]["name"].is_string());
        assert!(result["agentInfo"]["version"].is_string());
    }

    #[test]
    fn prompt_blocks_concatenate_text() {
        let params = json!({
            "prompt": [
                { "type": "text", "text": "hello " },
                { "type": "text", "text": "world" },
            ]
        });
        assert_eq!(prompt_text(&params).unwrap(), "hello world");
    }

    #[test]
    fn prompt_must_be_an_array_of_blocks() {
        assert!(prompt_text(&json!({ "prompt": "hello" })).is_err());
        assert!(prompt_text(&json!({ "prompt": { "text": "hello" } })).is_err());
    }

    #[test]
    fn unsupported_prompt_blocks_are_named() {
        let err =
            prompt_text(&json!({ "prompt": [ { "type": "image", "data": "..." } ] })).unwrap_err();
        assert!(
            err.contains("image"),
            "rejected by type, not generically: {err}"
        );
    }

    #[test]
    fn an_empty_prompt_is_refused() {
        assert!(prompt_text(&json!({ "prompt": [] })).is_err());
    }
}
