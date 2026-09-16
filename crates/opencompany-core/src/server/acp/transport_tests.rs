use super::*;
use async_trait::async_trait;
use serde_json::json;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use crate::company::CompanyManifest;
use crate::ports::EventSeq;
use crate::ports::types::{ApprovalId, CompressedTrace, CycleRequest, CycleResult, TokenUsage};
use crate::ports::users::{UserRecord, UserRole, UserStatus};
use crate::ports::{Brain, CompanyStore, CycleHost, SessionKind, SessionRecord};
use crate::server::graphql::auth::UserPrincipal;
use crate::server::platform_auth::PlatformClaims;
use crate::server::users::cookie::session_cookie_name;
use crate::server::users::token::{OsTokens, mint_session_token, sha256_hex};
use crate::store::FsCompanyStore;
use crate::{AppConfig, ports::types::CompanyRecord};

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

#[test]
fn owner_keys_a_user_by_company_as_well_as_id() {
    // `user_id` is only guaranteed unique within a company, so two
    // companies minting the same id must not collide into one owner.
    let same_id_in_acme = GqlAuth::User(UserPrincipal {
        company: CompanyId::new("acme"),
        user_id: "u1".to_string(),
        email: "a@example.test".to_string(),
        role: UserRole::Admin,
        must_change_password: false,
        session_token_hash: "hash".to_string(),
        credential: crate::ports::SessionKind::Browser,
    });
    let same_id_in_globex = GqlAuth::User(UserPrincipal {
        company: CompanyId::new("globex"),
        user_id: "u1".to_string(),
        email: "b@example.test".to_string(),
        role: UserRole::Admin,
        must_change_password: false,
        session_token_hash: "hash".to_string(),
        credential: crate::ports::SessionKind::Browser,
    });
    assert_ne!(owner(&same_id_in_acme), owner(&same_id_in_globex));
}

#[test]
fn owner_canonicalizes_the_platform_tenant() {
    // `authorize_address` compares tenants in canonical_tenant form, so a
    // raw-string owner key would treat `tenant:acme` and `acme` as two
    // different owners even though they name the same tenant.
    let prefixed = GqlAuth::Platform(PlatformClaims {
        tenant: "tenant:acme".to_string(),
        scopes: Default::default(),
        companies: None,
    });
    let bare = GqlAuth::Platform(PlatformClaims {
        tenant: "acme".to_string(),
        scopes: Default::default(),
        companies: None,
    });
    assert_eq!(owner(&prefixed), owner(&bare));
}

/// A brain that answers a cycle with nothing, so the ACP `prompt` turn
/// completes without an inference credential. The notification this suite
/// asserts on is filed before the turn runs, so the empty answer is fine.
struct SilentBrain;

#[async_trait]
impl Brain for SilentBrain {
    async fn run_cycle(
        &self,
        req: CycleRequest,
        _host: &dyn CycleHost,
    ) -> crate::Result<CycleResult> {
        Ok(CycleResult {
            channel_responses: Vec::new(),
            new_traces: vec![CompressedTrace::now(req.cycle_id, "silent test brain")],
            ledger_deltas: Vec::new(),
            token_usage: TokenUsage::default(),
        })
    }
}

async fn seed_user(state: &AppState, company: &CompanyId, id: &str, display: &str) -> String {
    let runtime = state.registry().get(company).expect("company");
    let now = crate::ports::now_millis();
    runtime
        .users()
        .upsert_user(
            company,
            &UserRecord {
                id: id.to_string(),
                email: format!("{id}@example.test"),
                display_name: Some(display.to_string()),
                avatar: None,
                role: UserRole::Member,
                status: UserStatus::Active,
                password_hash: None,
                must_change_password: false,
                created_at_millis: now,
                last_seen_at_millis: None,
                updated_at_millis: now,
            },
        )
        .await
        .expect("seed_user: upsert");
    id.to_string()
}

/// A host whose registry runtime answers cycles with [`SilentBrain`], on
/// the `acp,runner,tinymemory` lane — the one that executes `server::acp`.
async fn acp_state(home: &std::path::Path) -> AppState {
    let manifest: CompanyManifest = toml::from_str(
        r#"
[company]
name = "Acme"

[[agent]]
id = "product_manager"
role = "Product Manager"

[[agent]]
id = "writer"
role = "Writer"

[[group_chat]]
id = "engineering"
name = "Engineering"
members = []

[[group_chat]]
id = "writer"
name = "Writer desk"
members = []

[policy]
mode = "full"
"#,
    )
    .unwrap();
    let store = FsCompanyStore::new(home.to_path_buf());
    let id = CompanyId::new("acme");
    store
        .save(&CompanyRecord {
            overlay_desk_hive: Vec::new(),
            overlay_retired_agents: Vec::new(),
            overlay_agent_edits: Vec::new(),
            id: id.clone(),
            manifest: manifest.clone(),
            ledger: Vec::new(),
            lifecycle: "running".to_string(),
            overlay_agents: Vec::new(),
            overlay_desk_members: Vec::new(),
            overlay_desk_order: Vec::new(),
            overlay_desks: Vec::new(),
            overlay_workflows: Vec::new(),
            overlay_budgets: Vec::new(),
            overlay_policy: None,
            overlay_tool_grants: None,
            overlay_desk_tools: Default::default(),
            disabled_workflows: Vec::new(),
            template_provenance: None,
            setup: None,
            name_confirmed: false,
            activation_completed_at: None,
            created_at_millis: None,
        })
        .await
        .unwrap();
    let runtime = crate::runtime::RuntimeBuilder::new(home.to_path_buf(), manifest)
        .with_id(id.clone())
        .with_brain(std::sync::Arc::new(SilentBrain))
        .build()
        .await
        .unwrap();
    let state = AppState::new(AppConfig::default());
    state.registry().insert(id, std::sync::Arc::new(runtime));
    state
}

/// Same as [`acp_state`], but the sole company is `[users] mode = "none"`
/// — the packaged-desktop shape with no sign-in, reachable by anyone who
/// can reach the loopback bind at all.
async fn acp_state_none_mode(home: &std::path::Path) -> AppState {
    let manifest: CompanyManifest = toml::from_str(
        r#"
[company]
name = "Acme"

[[agent]]
id = "product_manager"
role = "Product Manager"

[[group_chat]]
id = "engineering"
name = "Engineering"
members = []

[policy]
mode = "full"

[users]
mode = "none"
"#,
    )
    .unwrap();
    let store = FsCompanyStore::new(home.to_path_buf());
    let id = CompanyId::new("acme");
    store
        .save(&CompanyRecord {
            overlay_retired_agents: Vec::new(),
            overlay_agent_edits: Vec::new(),
            id: id.clone(),
            manifest: manifest.clone(),
            ledger: Vec::new(),
            lifecycle: "running".to_string(),
            overlay_agents: Vec::new(),
            overlay_desk_members: Vec::new(),
            overlay_desk_order: Vec::new(),
            overlay_desk_hive: Vec::new(),
            overlay_desks: Vec::new(),
            overlay_workflows: Vec::new(),
            overlay_budgets: Vec::new(),
            overlay_policy: None,
            overlay_tool_grants: None,
            overlay_desk_tools: Default::default(),
            disabled_workflows: Vec::new(),
            template_provenance: None,
            setup: None,
            name_confirmed: false,
            activation_completed_at: None,
            created_at_millis: None,
        })
        .await
        .unwrap();
    let runtime = crate::runtime::RuntimeBuilder::new(home.to_path_buf(), manifest)
        .with_id(id.clone())
        .with_brain(std::sync::Arc::new(SilentBrain))
        .build()
        .await
        .unwrap();
    let state = AppState::new(AppConfig::default());
    state.registry().insert(id, std::sync::Arc::new(runtime));
    state
}

/// Builds a bare JSON-RPC `POST /acp` request with no auth headers.
fn acp_call_request(body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/acp")
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

/// An `@alice-smith` ACP prompt must badge alice exactly as a console
/// message would: the ACP surface is just another operator ingress, and the
/// durable notification is what lets an offline person see the mention at
/// all.
#[tokio::test]
async fn a_prompt_mention_files_the_durable_notification() {
    let home = tempfile::Builder::new()
        .prefix("oc-acp-mention-")
        .tempdir()
        .expect("tempdir");
    let state = acp_state(home.path()).await;
    let company = CompanyId::new("acme");
    let runtime = state.registry().get(&company).expect("company");

    // Two people: the operator driving the prompt, and the person it names.
    let admin = seed_user(&state, &company, "u-admin", "Admin Person").await;
    let alice = seed_user(&state, &company, "u-alice", "Alice Smith").await;
    let auth = GqlAuth::User(UserPrincipal {
        company: company.clone(),
        user_id: admin,
        email: "admin@example.test".to_string(),
        role: UserRole::Admin,
        must_change_password: false,
        session_token_hash: "hash".to_string(),
        credential: crate::ports::SessionKind::Browser,
    });

    state
        .acp_sessions()
        .open(
            "conn-1",
            &owner(&auth),
            crate::server::acp::AcpSession {
                id: "s-1".to_string(),
                company: company.clone(),
                chat: "engineering".to_string(),
                agent_id: None,
            },
            crate::ports::now_millis(),
        )
        .expect("open session");

    let result = prompt(
        &state,
        &auth,
        &json!({
            "sessionId": "s-1",
            "prompt": [
                { "type": "text", "text": "@alice-smith please review the invoice" },
            ],
            "_meta": { "opencompany/connectionId": "conn-1" },
        }),
    )
    .await;
    assert!(result.is_ok(), "prompt failed: {result:?}");

    // The durable half of the mention: the person named gets a row they can
    // badge, placed in the channel the prompt ran in.
    let rows = runtime
        .notifications()
        .list(&company, &alice)
        .await
        .expect("list");
    assert_eq!(rows.len(), 1, "an @alice prompt must badge alice");
    assert_eq!(rows[0].notification.kind, "mention");
    assert_eq!(rows[0].notification.context.as_deref(), Some("engineering"));
    // And the author is not badged for their own prompt.
    let admin_rows = runtime
        .notifications()
        .list(&company, "u-admin")
        .await
        .expect("list");
    assert!(admin_rows.is_empty(), "{admin_rows:?}");
}

/// B-101, on the ACP ingress (codex P2): a `@writer` that names both the
/// `writer` teammate and the `writer` desk must be refused and reported,
/// exactly as the REST chat path's `accept_chat_turn` does it. Before this
/// fix the ACP `session/prompt` handler called the plain `resolve_mentions`
/// and never posted the ambiguity note, so an ACP client (e.g. Zed) sending
/// an ambiguous `@name` pinged nobody with no durable refusal anywhere.
#[tokio::test]
async fn a_prompt_ambiguous_mention_is_reported_in_the_channel() {
    let home = tempfile::Builder::new()
        .prefix("oc-acp-ambiguous-")
        .tempdir()
        .expect("tempdir");
    let state = acp_state(home.path()).await;
    let company = CompanyId::new("acme");
    let runtime = state.registry().get(&company).expect("company");
    let admin = seed_user(&state, &company, "u-admin", "Admin Person").await;
    let auth = GqlAuth::User(UserPrincipal {
        company: company.clone(),
        user_id: admin,
        email: "admin@example.test".to_string(),
        role: UserRole::Admin,
        must_change_password: false,
        session_token_hash: "hash".to_string(),
        credential: crate::ports::SessionKind::Browser,
    });

    state
        .acp_sessions()
        .open(
            "conn-1",
            &owner(&auth),
            crate::server::acp::AcpSession {
                id: "s-1".to_string(),
                company: company.clone(),
                chat: "engineering".to_string(),
                agent_id: None,
            },
            crate::ports::now_millis(),
        )
        .expect("open session");

    let result = prompt(
        &state,
        &auth,
        &json!({
            "sessionId": "s-1",
            "prompt": [
                { "type": "text", "text": "@writer can you draft the autumn brief?" },
            ],
            "_meta": { "opencompany/connectionId": "conn-1" },
        }),
    )
    .await;
    assert!(result.is_ok(), "prompt failed: {result:?}");

    // The durable refusal notice: the same `AgentReply` the REST chat path
    // posts, attributed to the runtime and landing in the channel the
    // prompt ran in.
    let events = runtime
        .events()
        .read_from(&company, EventSeq::new(0), usize::MAX)
        .await
        .expect("read events");
    let advisories: Vec<(String, String)> = events
        .into_iter()
        .filter_map(|stored| match stored.event {
            CompanyEvent::AgentReply {
                agent_id,
                chat_id,
                text,
                ..
            } if agent_id == crate::ports::SYSTEM_AUTHOR => Some((chat_id, text)),
            _ => None,
        })
        .collect();
    assert_eq!(
        advisories.len(),
        1,
        "exactly one ambiguity note, cross-ingress: {advisories:?}"
    );
    let (chat, text) = &advisories[0];
    assert_eq!(chat, "engineering");
    assert!(text.contains("@writer"), "names the literal typed: {text}");
    assert!(
        text.contains("pinged nobody"),
        "states what happened: {text}"
    );
}

/// A runtime being replaced refuses the prompt *before* it is journaled
/// (codex P2): a message appended and then rejected would stay in the
/// transcript with nothing that will ever answer it — the ordering the
/// REST chat path holds via `accept_chat_turn`.
#[tokio::test]
async fn a_quiesced_runtime_refuses_prompt_before_journaling() {
    let home = tempfile::Builder::new()
        .prefix("oc-acp-quiesce-")
        .tempdir()
        .expect("tempdir");
    let state = acp_state(home.path()).await;
    let company = CompanyId::new("acme");
    let runtime = state.registry().get(&company).expect("company");
    let admin = seed_user(&state, &company, "u-admin", "Admin Person").await;
    let auth = GqlAuth::User(UserPrincipal {
        company: company.clone(),
        user_id: admin,
        email: "admin@example.test".to_string(),
        role: UserRole::Admin,
        must_change_password: false,
        session_token_hash: "hash".to_string(),
        credential: crate::ports::SessionKind::Browser,
    });

    state
        .acp_sessions()
        .open(
            "conn-1",
            &owner(&auth),
            crate::server::acp::AcpSession {
                id: "s-1".to_string(),
                company: company.clone(),
                chat: "engineering".to_string(),
                agent_id: None,
            },
            crate::ports::now_millis(),
        )
        .expect("open session");

    runtime.quiesce().await;

    let result = prompt(
        &state,
        &auth,
        &json!({
            "sessionId": "s-1",
            "prompt": [
                { "type": "text", "text": "please review the invoice" },
            ],
            "_meta": { "opencompany/connectionId": "conn-1" },
        }),
    )
    .await;
    assert!(result.is_err(), "a quiesced runtime must refuse the prompt");

    // And nothing was journaled: the refusal happened before the append.
    let events = runtime
        .events()
        .read_from(&company, EventSeq::new(0), usize::MAX)
        .await
        .expect("read events");
    assert!(
        events
            .iter()
            .all(|stored| !matches!(&stored.event, CompanyEvent::OperatorMessage { .. })),
        "a refused prompt must not leave a message in the journal: {events:?}"
    );
}

/// Issue #1781 review (Codex P1): `prompt` used to journal straight to
/// `runtime.events()`, never through the REST `/chat` route's
/// `chat_and_emit`, so a session opened with `_meta.opencompany.chat =
/// "operator"` could post into the durable, supposedly read-only Operator
/// system feed. `acme` (from `acp_state`) has no real `operator` desk or
/// teammate, so this is the ordinary, non-grandfathered case REST already
/// refuses — the ACP surface must refuse it identically.
#[tokio::test]
async fn an_acp_prompt_addressed_to_the_operator_channel_is_refused() {
    let home = tempfile::Builder::new()
        .prefix("oc-acp-operator-guard-")
        .tempdir()
        .expect("tempdir");
    let state = acp_state(home.path()).await;
    let company = CompanyId::new("acme");
    let runtime = state.registry().get(&company).expect("company");
    let admin = seed_user(&state, &company, "u-admin", "Admin Person").await;
    let auth = GqlAuth::User(UserPrincipal {
        company: company.clone(),
        user_id: admin,
        email: "admin@example.test".to_string(),
        role: UserRole::Admin,
        must_change_password: false,
        session_token_hash: "hash".to_string(),
        credential: crate::ports::SessionKind::Browser,
    });

    // Mirrors what `open_session` stores for an unpinned session whose
    // client requested `_meta.opencompany.chat = "operator"`
    // (`AcpSession::thread_key` passes an unpinned request through
    // verbatim).
    state
        .acp_sessions()
        .open(
            "conn-1",
            &owner(&auth),
            crate::server::acp::AcpSession {
                id: "s-1".to_string(),
                company: company.clone(),
                chat: "operator".to_string(),
                agent_id: None,
            },
            crate::ports::now_millis(),
        )
        .expect("open session");

    let result = prompt(
        &state,
        &auth,
        &json!({
            "sessionId": "s-1",
            "prompt": [
                { "type": "text", "text": "hello from a session pinned to the feed" },
            ],
            "_meta": { "opencompany/connectionId": "conn-1" },
        }),
    )
    .await;
    assert!(
        result.is_err(),
        "a prompt addressed to the read-only Operator channel must be refused"
    );

    // And nothing was journaled: the refusal happened before the append,
    // same ordering the quiesced-runtime test above proves.
    let events = runtime
        .events()
        .read_from(&company, EventSeq::new(0), usize::MAX)
        .await
        .expect("read events");
    assert!(
        events
            .iter()
            .all(|stored| !matches!(&stored.event, CompanyEvent::OperatorMessage { .. })),
        "a refused prompt must not leave a message in the journal: {events:?}"
    );
}

fn admin_auth(company: &CompanyId, user_id: String, session_token_hash: &str) -> GqlAuth {
    GqlAuth::User(UserPrincipal {
        company: company.clone(),
        user_id,
        email: "admin@example.test".to_string(),
        role: UserRole::Admin,
        must_change_password: false,
        session_token_hash: session_token_hash.to_string(),
        credential: crate::ports::SessionKind::Browser,
    })
}

/// A colon is legal in both halves of the owner key, so the join has to be
/// injective on its own rather than by assuming the components are clean.
#[test]
fn two_different_principals_never_share_an_owner_key() {
    let left = owner(&admin_auth(&CompanyId::new("a:b"), "c".to_string(), "h"));
    let right = owner(&admin_auth(&CompanyId::new("a"), "b:c".to_string(), "h"));
    assert_ne!(
        left, right,
        "a company and a user id that split differently must not collide"
    );

    // The same principal still resolves to one stable key, or an operator
    // would lose their own connection between calls.
    let again = owner(&admin_auth(&CompanyId::new("a:b"), "c".to_string(), "h"));
    assert_eq!(left, again);
}

#[tokio::test]
async fn a_caller_cannot_open_a_session_on_a_connection_it_does_not_own() {
    let home = tempfile::Builder::new()
        .prefix("oc-acp-conn-owner-")
        .tempdir()
        .expect("tempdir");
    let state = acp_state(home.path()).await;
    let company = CompanyId::new("acme");
    let alice = seed_user(&state, &company, "u-alice", "Alice").await;
    let bob = seed_user(&state, &company, "u-bob", "Bob").await;
    let auth_alice = admin_auth(&company, alice, "hash-alice");
    let auth_bob = admin_auth(&company, bob, "hash-bob");
    let params = json!({
        "_meta": {
            "opencompany": { "company": "acme" },
            "opencompany/connectionId": "conn-shared",
        }
    });

    let first = open_session(&state, &auth_alice, &params).await;
    assert!(first.is_ok(), "alice opens the connection first: {first:?}");

    let second = open_session(&state, &auth_bob, &params).await;
    assert!(
        second.is_err(),
        "bob must not be able to open a session on alice's connection"
    );

    // And bob gets no view into what alice has, by any surface.
    let listed = list_sessions(&state, &auth_bob, &params);
    assert!(listed.is_err(), "bob cannot list alice's connection either");
}

#[tokio::test]
async fn open_session_refuses_once_the_per_connection_cap_is_hit() {
    let home = tempfile::Builder::new()
        .prefix("oc-acp-conn-cap-")
        .tempdir()
        .expect("tempdir");
    let state = acp_state(home.path()).await;
    let company = CompanyId::new("acme");
    let admin = seed_user(&state, &company, "u-admin", "Admin Person").await;
    let auth = admin_auth(&company, admin, "hash");
    let params = json!({
        "_meta": {
            "opencompany": { "company": "acme" },
            "opencompany/connectionId": "conn-cap",
        }
    });

    for _ in 0..crate::server::acp::session::MAX_SESSIONS_PER_CONNECTION {
        let result = open_session(&state, &auth, &params).await;
        assert!(result.is_ok(), "{result:?}");
    }
    let refusal = open_session(&state, &auth, &params).await;
    assert!(refusal.is_err(), "the cap must refuse the next open");
}

#[tokio::test]
async fn disconnect_closes_every_session_on_the_connection_at_once() {
    let home = tempfile::Builder::new()
        .prefix("oc-acp-disconnect-")
        .tempdir()
        .expect("tempdir");
    let state = acp_state(home.path()).await;
    let company = CompanyId::new("acme");
    let admin = seed_user(&state, &company, "u-admin", "Admin Person").await;
    let auth = admin_auth(&company, admin, "hash");
    let params = json!({
        "_meta": {
            "opencompany": { "company": "acme" },
            "opencompany/connectionId": "conn-close",
        }
    });

    open_session(&state, &auth, &params)
        .await
        .expect("first session");
    open_session(&state, &auth, &params)
        .await
        .expect("second session");

    let closed = disconnect(&state, &auth, &params);
    assert!(closed.is_ok());

    // The whole connection is gone, not merely emptied of sessions:
    // `list_sessions` on an unknown connection is the same refusal as one
    // this caller never owned.
    assert!(
        list_sessions(&state, &auth, &params).is_err(),
        "the connection must not survive its own disconnect"
    );
}

#[test]
fn a_parked_turn_carries_an_approval_notification_but_still_ends_the_turn() {
    let report = crate::runtime::CycleReport {
        cycle_id: "c1".to_string(),
        responses: Vec::new(),
        executed_effects: Vec::new(),
        parked: vec![ApprovalId::from("appr-1".to_string())],
        persisted_seq: None,
        input_seqs: Vec::new(),
    };
    let result = prompt_result(report);
    assert_eq!(result["stopReason"], "end_turn");
    let updates = result["updates"].as_array().expect("updates array");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0]["_meta"]["opencompany/approval"]["id"], "appr-1");
}

#[test]
fn a_clean_turn_carries_no_approval_notification() {
    let report = crate::runtime::CycleReport {
        cycle_id: "c1".to_string(),
        responses: Vec::new(),
        executed_effects: Vec::new(),
        parked: Vec::new(),
        persisted_seq: None,
        input_seqs: Vec::new(),
    };
    let result = prompt_result(report);
    assert_eq!(result["stopReason"], "end_turn");
    assert!(
        result["updates"]
            .as_array()
            .expect("updates array")
            .is_empty()
    );
}

// -----------------------------------------------------------------
// PLAT-057 / PLAT-060: the `call` HTTP handler itself. Every test above
// this point calls `open_session`/`prompt`/etc. directly, bypassing the
// axum extraction, method dispatch and JSON-RPC envelope that only `call`
// (mounted by `router()`) actually implements.
// -----------------------------------------------------------------

/// PLAT-060 (AUTH): a `none`-mode company's local owner is reachable over
/// the real HTTP `call` handler with zero credentials — no cookie, no
/// bearer — same as every other credential-less surface that mode grants.
#[tokio::test]
async fn call_handler_authenticates_a_credential_less_none_mode_request() {
    let home = tempfile::Builder::new()
        .prefix("oc-acp-call-none-")
        .tempdir()
        .expect("tempdir");
    let state = acp_state_none_mode(home.path()).await;
    let app = router().with_state(state);

    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} });
    let response = app.oneshot(acp_call_request(body)).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["jsonrpc"], "2.0");
    assert_eq!(value["id"], 1);
    assert_eq!(value["result"]["protocolVersion"], 1);
}

/// PLAT-057 (AUTH): a company with real sign-in refuses an unauthenticated
/// `call`, over HTTP — not just at the level of the extractor unit tests.
#[tokio::test]
async fn call_handler_refuses_an_unauthenticated_request() {
    let home = tempfile::Builder::new()
        .prefix("oc-acp-call-auth-")
        .tempdir()
        .expect("tempdir");
    let state = acp_state(home.path()).await;
    let app = router().with_state(state);

    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} });
    let response = app.oneshot(acp_call_request(body)).await.unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// PLAT-057 (STATE): a user who must change their password is refused
/// *before* any ACP method runs, over the real HTTP path — the same
/// boundary `ScopedCompany` enforces for the operator API.
#[tokio::test]
async fn call_handler_refuses_a_temporary_password_user_before_running_any_method() {
    let home = tempfile::Builder::new()
        .prefix("oc-acp-call-temp-pw-")
        .tempdir()
        .expect("tempdir");
    let state = acp_state(home.path()).await;
    let company = CompanyId::new("acme");
    let runtime = state.registry().get(&company).expect("company");
    let now = crate::ports::now_millis();
    runtime
        .users()
        .upsert_user(
            &company,
            &UserRecord {
                id: "u-temp".to_string(),
                email: "temp@example.test".to_string(),
                display_name: None,
                avatar: None,
                role: UserRole::Admin,
                status: UserStatus::Active,
                password_hash: None,
                must_change_password: true,
                created_at_millis: now,
                last_seen_at_millis: None,
                updated_at_millis: now,
            },
        )
        .await
        .expect("seed temp-password user");
    let token = mint_session_token(&OsTokens);
    runtime
        .sessions()
        .create(
            &company,
            &SessionRecord {
                id: "s-temp".to_string(),
                token_hash: sha256_hex(&token),
                user_id: "u-temp".to_string(),
                created_at_millis: now,
                expires_at_millis: now + 60_000,
                user_agent: None,
                kind: SessionKind::Browser,
                label: None,
            },
        )
        .await
        .expect("seed session");
    let cookie_name = session_cookie_name(&company).expect("cookie name");

    let app = router().with_state(state);
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} });
    let mut request = acp_call_request(body);
    request.headers_mut().insert(
        axum::http::header::COOKIE,
        format!("{cookie_name}={token}").parse().unwrap(),
    );
    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

/// PLAT-057 (FAIL): an unsupported ACP method comes back as a `-32602`
/// JSON-RPC error envelope, over the real HTTP path — not a raw error, not
/// an HTTP-level 4xx/5xx.
#[tokio::test]
async fn call_handler_reports_an_unsupported_method_as_a_json_rpc_error() {
    let home = tempfile::Builder::new()
        .prefix("oc-acp-call-fail-")
        .tempdir()
        .expect("tempdir");
    let state = acp_state_none_mode(home.path()).await;
    let app = router().with_state(state);

    let body = json!({
        "jsonrpc": "2.0",
        "id": "req-9",
        "method": "session/frobnicate",
        "params": {},
    });
    let response = app.oneshot(acp_call_request(body)).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["id"], "req-9");
    assert_eq!(value["error"]["code"], -32602);
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("session/frobnicate")
    );
}

/// PLAT-057 (BOUND): the JSON-RPC `id` round-trips exactly, including the
/// boundary case of a request that omits it entirely (must answer `null`,
/// not fail or invent one).
#[tokio::test]
async fn call_handler_echoes_the_request_id_including_when_absent() {
    let home = tempfile::Builder::new()
        .prefix("oc-acp-call-bound-")
        .tempdir()
        .expect("tempdir");
    let state = acp_state_none_mode(home.path()).await;
    let app = router().with_state(state);

    let body = json!({ "jsonrpc": "2.0", "id": 42, "method": "initialize", "params": {} });
    let response = app.clone().oneshot(acp_call_request(body)).await.unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["id"], 42);

    let body = json!({ "jsonrpc": "2.0", "method": "initialize", "params": {} });
    let response = app.oneshot(acp_call_request(body)).await.unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["id"], Value::Null);
    assert_eq!(value["result"]["protocolVersion"], 1);
}

/// PLAT-057 (INPUT): a body that is not valid JSON at all must not panic
/// or hang the handler — it is rejected before `call`'s body even runs.
#[tokio::test]
async fn call_handler_rejects_a_body_that_is_not_valid_json() {
    let home = tempfile::Builder::new()
        .prefix("oc-acp-call-input-")
        .tempdir()
        .expect("tempdir");
    let state = acp_state_none_mode(home.path()).await;
    let app = router().with_state(state);

    let request = Request::builder()
        .method("POST")
        .uri("/acp")
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(b"{ this is not json".to_vec()))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();

    assert!(
        response.status().is_client_error(),
        "a malformed JSON body must be rejected, got {:?}",
        response.status()
    );
}

/// Mints a real, HTTP-carriable admin session for `acp_state`'s "acme"
/// company, returning its `Cookie` header value.
async fn seed_admin_session_cookie(
    state: &AppState,
    company: &CompanyId,
    user_id: &str,
) -> String {
    let runtime = state.registry().get(company).expect("company");
    let now = crate::ports::now_millis();
    runtime
        .users()
        .upsert_user(
            company,
            &UserRecord {
                id: user_id.to_string(),
                email: format!("{user_id}@example.test"),
                display_name: None,
                avatar: None,
                role: UserRole::Admin,
                status: UserStatus::Active,
                password_hash: None,
                must_change_password: false,
                created_at_millis: now,
                last_seen_at_millis: None,
                updated_at_millis: now,
            },
        )
        .await
        .expect("seed admin");
    let token = mint_session_token(&OsTokens);
    runtime
        .sessions()
        .create(
            company,
            &SessionRecord {
                id: format!("s-{user_id}"),
                token_hash: sha256_hex(&token),
                user_id: user_id.to_string(),
                created_at_millis: now,
                expires_at_millis: now + 60_000,
                user_agent: None,
                kind: SessionKind::Browser,
                label: None,
            },
        )
        .await
        .expect("seed session");
    let cookie_name = session_cookie_name(company).expect("cookie name");
    format!("{cookie_name}={token}")
}

fn session_new_request(cookie: &str, connection_id: &str, request_id: u64) -> Request<Body> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "session/new",
        "params": {
            "_meta": {
                "opencompany": { "company": "acme" },
                "opencompany/connectionId": connection_id,
            }
        }
    });
    let mut request = acp_call_request(body);
    request
        .headers_mut()
        .insert(axum::http::header::COOKIE, cookie.parse().unwrap());
    request
}

/// PLAT-057 (LIMIT): the per-connection session cap
/// (`MAX_SESSIONS_PER_CONNECTION`) is enforced through the real HTTP `call`
/// handler, not just the `open_session` inner function every test above
/// this point calls directly — the handler's own JSON-RPC dispatch and
/// envelope sit between a caller and that cap in production, and neither
/// was ever exercised together with it.
#[tokio::test]
async fn call_handler_refuses_session_new_once_the_per_connection_cap_is_hit() {
    let home = tempfile::Builder::new()
        .prefix("oc-acp-call-limit-")
        .tempdir()
        .expect("tempdir");
    let state = acp_state(home.path()).await;
    let company = CompanyId::new("acme");
    let cookie = seed_admin_session_cookie(&state, &company, "u-cap-admin").await;
    let app = router().with_state(state);

    for i in 0..crate::server::acp::session::MAX_SESSIONS_PER_CONNECTION {
        let response = app
            .clone()
            .oneshot(session_new_request(&cookie, "conn-http-cap", i as u64))
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(
            value.get("result").is_some(),
            "open #{i} must succeed: {value:?}"
        );
    }

    let response = app
        .oneshot(session_new_request(&cookie, "conn-http-cap", 999))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "still a JSON-RPC 200");
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        value.get("error").is_some(),
        "the session past the cap must come back a JSON-RPC error: {value:?}"
    );
}

/// PLAT-057 (CONC): two different principals racing `session/new` on the
/// same `connectionId`, over the real HTTP handler concurrently rather than
/// sequentially. `a_caller_cannot_open_a_session_on_a_connection_it_does_
/// not_own` proves the inner function's ownership rule one call at a time;
/// this proves the lock the handler sits on top of actually serializes two
/// requests that land at the same time, rather than both racing past a
/// check-then-insert and one silently losing its own connection.
#[tokio::test]
async fn call_handler_lets_only_one_of_two_concurrent_openers_win_a_connection() {
    let home = tempfile::Builder::new()
        .prefix("oc-acp-call-conc-")
        .tempdir()
        .expect("tempdir");
    let state = acp_state(home.path()).await;
    let company = CompanyId::new("acme");
    let cookie_a = seed_admin_session_cookie(&state, &company, "u-conc-a").await;
    let cookie_b = seed_admin_session_cookie(&state, &company, "u-conc-b").await;
    let app = router().with_state(state);

    let request_a = session_new_request(&cookie_a, "conn-http-race", 1);
    let request_b = session_new_request(&cookie_b, "conn-http-race", 2);
    let app_a = app.clone();
    let app_b = app.clone();
    let (response_a, response_b) =
        tokio::join!(app_a.oneshot(request_a), app_b.oneshot(request_b));

    let value_a: Value = serde_json::from_slice(
        &axum::body::to_bytes(response_a.unwrap().into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    let value_b: Value = serde_json::from_slice(
        &axum::body::to_bytes(response_b.unwrap().into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();

    let winners = [
        value_a.get("result").is_some(),
        value_b.get("result").is_some(),
    ]
    .into_iter()
    .filter(|ok| *ok)
    .count();
    assert_eq!(
        winners, 1,
        "exactly one of two concurrent openers may win a shared connection: \
         {value_a:?} / {value_b:?}"
    );
}

/// PLAT-060 (STATE): `local_owner` falls back to `registry().sole()` when
/// `/acp` cannot name an addressed company (it has no `{id}` path param).
/// Once a second company is registered, `sole()` no longer resolves — the
/// credential-less request must come back unauthorized, not silently
/// authorized against whichever company happens to be `none`-mode.
#[tokio::test]
async fn call_handler_none_mode_owner_is_unreachable_once_a_second_company_exists() {
    let home = tempfile::Builder::new()
        .prefix("oc-acp-call-state-")
        .tempdir()
        .expect("tempdir");
    let state = acp_state_none_mode(home.path()).await;

    // Control, proven first on this exact state: with only the sole
    // `none`-mode company registered, the credential-less request
    // succeeds — the same claim `call_handler_authenticates_a_credential_
    // less_none_mode_request` makes, repeated here so the 401 asserted
    // below is shown to depend on the second company, not on some other
    // difference between the two tests' fixtures.
    let control_app = router().with_state(state.clone());
    let control_body =
        json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {} });
    let control_response = control_app
        .oneshot(acp_call_request(control_body))
        .await
        .unwrap();
    assert_eq!(
        control_response.status(),
        StatusCode::OK,
        "control: the sole none-mode company must still answer credential-less \
         before a second company is registered"
    );

    let manifest: CompanyManifest = toml::from_str("[company]\nname = \"Globex\"\n").unwrap();
    let store = FsCompanyStore::new(home.path().to_path_buf());
    let globex = CompanyId::new("globex");
    store
        .save(&CompanyRecord {
            overlay_retired_agents: Vec::new(),
            overlay_agent_edits: Vec::new(),
            id: globex.clone(),
            manifest: manifest.clone(),
            ledger: Vec::new(),
            lifecycle: "running".to_string(),
            overlay_agents: Vec::new(),
            overlay_desk_members: Vec::new(),
            overlay_desk_order: Vec::new(),
            overlay_desk_hive: Vec::new(),
            overlay_desks: Vec::new(),
            overlay_workflows: Vec::new(),
            overlay_budgets: Vec::new(),
            overlay_policy: None,
            overlay_tool_grants: None,
            overlay_desk_tools: Default::default(),
            disabled_workflows: Vec::new(),
            template_provenance: None,
            setup: None,
            name_confirmed: false,
            activation_completed_at: None,
            created_at_millis: None,
        })
        .await
        .unwrap();
    let globex_runtime =
        crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest)
            .with_id(globex.clone())
            .with_brain(std::sync::Arc::new(SilentBrain))
            .build()
            .await
            .unwrap();
    state
        .registry()
        .insert(globex, std::sync::Arc::new(globex_runtime));

    let app = router().with_state(state);
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} });
    let response = app.oneshot(acp_call_request(body)).await.unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "a none-mode company sharing a host with a second company must not \
         silently authorize a credential-less /acp request against either one"
    );
}

/// PLAT-060 (FAIL): the same `none`-mode gate `local_owner` applies
/// everywhere else — a request carrying a forwarding header is refused
/// outright, never degraded to a session/bearer check — proven here over
/// the real HTTP `call` handler.
#[tokio::test]
async fn call_handler_none_mode_refuses_a_forwarded_request() {
    let home = tempfile::Builder::new()
        .prefix("oc-acp-call-fail2-")
        .tempdir()
        .expect("tempdir");
    let state = acp_state_none_mode(home.path()).await;
    let app = router().with_state(state);

    // Control: the identical request, minus the forwarding header,
    // succeeds on this exact app — so the refusal below is shown to
    // depend on the header, not on some other difference in the fixture.
    let control_body =
        json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {} });
    let control_response = app
        .clone()
        .oneshot(acp_call_request(control_body))
        .await
        .unwrap();
    assert_eq!(
        control_response.status(),
        StatusCode::OK,
        "control: the same none-mode host must answer without the forwarding header"
    );

    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} });
    let mut request = acp_call_request(body);
    request
        .headers_mut()
        .insert("x-forwarded-for", "203.0.113.5".parse().unwrap());
    let response = app.oneshot(request).await.unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "a none-mode company must refuse a forwarded request outright, not \
         treat it as its own local owner"
    );
}

/// PLAT-062 (STATE): a turn that both parked an approval and produced a
/// reply still reports `end_turn` — the mixed case, not just the two
/// single-field variations above.
#[test]
fn a_mixed_turn_with_a_reply_and_a_park_still_ends_the_turn() {
    let report = crate::runtime::CycleReport {
        cycle_id: "c1".to_string(),
        responses: vec![crate::ports::types::OutboundMessage {
            channel: "operator".to_string(),
            agent: None,
            text: "here's what I found".to_string(),
            steps: Vec::new(),
            reply_to: None,
            task_id: None,
            outputs: Vec::new(),
            message_id: None,
            mentions: Vec::new(),
        }],
        executed_effects: Vec::new(),
        parked: vec![ApprovalId::from("appr-1".to_string())],
        persisted_seq: None,
        input_seqs: Vec::new(),
    };
    let result = prompt_result(report);
    assert_eq!(result["stopReason"], "end_turn");
    let updates = result["updates"].as_array().expect("updates array");
    // Counting alone would pass on two chunks and no notification, which is
    // the shape this case exists to refuse.
    let approvals: Vec<&str> = updates
        .iter()
        .filter_map(|u| u["_meta"]["opencompany/approval"]["id"].as_str())
        .collect();
    assert_eq!(
        approvals,
        vec!["appr-1"],
        "the parked approval must carry its own notification: {updates:?}"
    );
    assert!(
        updates.iter().any(|u| u["content"]["text"]
            .as_str()
            .is_some_and(|t| t.contains("here's what I found"))),
        "the reply chunk must survive alongside it: {updates:?}"
    );
    assert_eq!(updates.len(), 2, "and nothing else: {updates:?}");
}

/// PLAT-062 (BOUND): several parked approvals in one turn each get their
/// own notification — none dropped, none deduplicated, at the boundary of
/// "more than one".
#[test]
fn every_parked_approval_in_a_multi_park_turn_gets_its_own_notification() {
    let parked: Vec<ApprovalId> = (0..5)
        .map(|i| ApprovalId::from(format!("appr-{i}")))
        .collect();
    let report = crate::runtime::CycleReport {
        cycle_id: "c1".to_string(),
        responses: Vec::new(),
        executed_effects: Vec::new(),
        parked: parked.clone(),
        persisted_seq: None,
        input_seqs: Vec::new(),
    };
    let result = prompt_result(report);
    assert_eq!(result["stopReason"], "end_turn");
    let updates = result["updates"].as_array().expect("updates array");
    assert_eq!(updates.len(), 5);
    // Five *unique* ids is not the claim — five ids that are the ones we
    // parked is. A set of unrelated ids satisfies the former.
    let mut ids: Vec<&str> = updates
        .iter()
        .map(|u| u["_meta"]["opencompany/approval"]["id"].as_str().unwrap())
        .collect();
    ids.sort_unstable();
    let mut expected: Vec<&str> = parked.iter().map(|id| id.as_ref()).collect();
    expected.sort_unstable();
    assert_eq!(
        ids, expected,
        "every parked id must appear exactly once: {updates:?}"
    );
}
