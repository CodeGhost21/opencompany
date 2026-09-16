use super::{
    CatalogEntry, CatalogSource, ComposioMode, ComposioStatusDto, CredentialSource,
    TinyhumansTokenSource, access_for,
};
use crate::company::runtime::CompanyRuntime;
use crate::server::error::ApiError;
use crate::server::ops::composio_toolkits;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
#[cfg(feature = "composio")]
use axum::routing::post;
#[cfg(feature = "composio")]
use axum::{Json, Router};
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::company::CompanyManifest;
use crate::ports::types::{CompanyId, CompanyRecord};
use crate::runtime::RuntimeBuilder;
use crate::server::router;
use crate::store::FsCompanyStore;
use crate::{AppConfig, AppState};

const TOKEN: &str = "composio-tenant-bearer-SECRET-xyz";

fn home() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("oc-composio-")
        .tempdir()
        .expect("tempdir")
}

/// A loopback Composio authorize endpoint for the feature-enabled route
/// test. Keeping the HTTP boundary real proves the handler reaches the
/// client after the admin guard, without allowing a unit test to dial the
/// production backend.
#[cfg(feature = "composio")]
async fn spawn_authorize_backend() -> String {
    let app = Router::new().route(
        "/agent-integrations/composio/authorize",
        post(|Json(body): Json<Value>| async move {
            assert_eq!(body["toolkit"], "gmail");
            Json(json!({
                "success": true,
                "data": {
                    "connectUrl": "https://composio.test/connect/gmail",
                    "connectionId": "gmail-connection"
                }
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

async fn state_with_manifest(home: &std::path::Path, manifest_toml: &str) -> AppState {
    state_with_manifest_id(home, "acme", manifest_toml).await
}

/// The same, under an explicit company id.
///
/// The catalog cache is process-wide and keyed by company, so the tests that
/// seed it need ids of their own — two tests sharing `acme` would share an
/// entry and race.
async fn state_with_manifest_id(
    home: &std::path::Path,
    company: &str,
    manifest_toml: &str,
) -> AppState {
    use crate::ports::CompanyStore;
    let manifest: CompanyManifest = toml::from_str(manifest_toml).unwrap();
    let store = FsCompanyStore::new(home.to_path_buf());
    let id = CompanyId::new(company);
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
    let runtime = RuntimeBuilder::new(home.to_path_buf(), manifest)
        .with_id(id.clone())
        .build()
        .await
        .unwrap();
    let state = AppState::new(AppConfig::default());
    state.registry().insert(id, std::sync::Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, company).await;
    state
}

async fn send(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value, String) {
    send_for(state, "acme", method, uri, body).await
}

/// [`send`] against a named company's session.
async fn send_for(
    state: &AppState,
    company: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value, String) {
    send_as(
        state,
        method,
        uri,
        body,
        Auth::Cookie(crate::server::test_support::fixed_cookie(company)),
    )
    .await
}

/// How a request presents itself, so the role boundary can be driven with
/// an admin session, a member session, or a machine credential.
enum Auth {
    Cookie(String),
    Bearer(String),
}

async fn send_as(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
    auth: Auth,
) -> (StatusCode, Value, String) {
    let request = Request::builder().method(method).uri(uri);
    let request = match auth {
        Auth::Cookie(cookie) => request.header("cookie", cookie),
        Auth::Bearer(token) => request.header("authorization", format!("Bearer {token}")),
    };
    let request = match body {
        Some(body) => request
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => request.body(Body::empty()).unwrap(),
    };
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

/// The hosted shape, driven through the env seam (no process mutation): a
/// company that pasted nothing reads `attested` from the instance identity,
/// its own TinyHumans key outranks that, its own Composio token outranks
/// both, and with none of the three the answer is `none`.
///
/// Drives the **real** resolver through a real secret store rather than a
/// restatement of its precedence. A pure function that merely mirrored the
/// rule would keep passing after the resolver lost a tier — which is exactly
/// what the negative control for issue #586 caught.
#[tokio::test]
async fn credential_source_matrix_follows_the_resolver_precedence() {
    use crate::app::config::MapEnv;
    use crate::company::company_key;
    use crate::company::composio::store_token;

    let dir = tempfile::Builder::new()
        .prefix("oc-dto-")
        .tempdir()
        .expect("tempdir");
    let path = dir.path().join("token");
    std::fs::write(&path, "projected-instance-token").unwrap();
    let projected = MapEnv::new([(
        crate::company::credentials::TOKEN_FILE_ENV,
        path.display().to_string(),
    )]);

    /// The instance identity a given environment resolves to.
    fn source_of(
        env: &dyn crate::app::config::EnvSource,
    ) -> Option<std::sync::Arc<super::TinyhumansTokenSource>> {
        super::TinyhumansTokenSource::from_env(env).map(std::sync::Arc::new)
    }

    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "matrix", GRANTED).await;
    let runtime = state
        .registry()
        .get(&CompanyId::new("matrix"))
        .expect("registered");
    let secrets = runtime.secrets();
    let id = runtime.id().clone();

    // Nothing stored + a projected instance identity → attested.
    assert_eq!(
        credential_source_for(&runtime, source_of(&projected))
            .await
            .unwrap(),
        CredentialSource::Attested
    );
    // Nothing stored at all → nothing obtainable, so no tools.
    assert_eq!(
        credential_source_for(&runtime, source_of(&MapEnv::default()))
            .await
            .unwrap(),
        CredentialSource::None
    );
    // A static instance key is the static tier.
    assert_eq!(
        credential_source_for(
            &runtime,
            source_of(&MapEnv::new([(
                crate::company::credentials::API_KEY_ENV,
                "th_static"
            )]))
        )
        .await
        .unwrap(),
        CredentialSource::Static
    );

    // The company's own TinyHumans key outranks the instance identity — a
    // company with a key set connects providers as *itself*, not as the pod
    // it happens to run in (issue #586)…
    company_key::store_key(&id, secrets.as_ref(), "th_company")
        .await
        .unwrap();
    assert_eq!(
        credential_source_for(&runtime, source_of(&projected))
            .await
            .unwrap(),
        CredentialSource::Company
    );
    // …and is the whole credential when the instance carries none, which is
    // the case this issue exists to fix.
    assert_eq!(
        credential_source_for(&runtime, source_of(&MapEnv::default()))
            .await
            .unwrap(),
        CredentialSource::Company
    );

    // The company's own Composio token outranks everything.
    store_token(&id, secrets.as_ref(), "byo-composio")
        .await
        .unwrap();
    assert_eq!(
        credential_source_for(&runtime, source_of(&projected))
            .await
            .unwrap(),
        CredentialSource::Static
    );
    assert_eq!(
        credential_source_for(&runtime, source_of(&MapEnv::default()))
            .await
            .unwrap(),
        CredentialSource::Static
    );

    // Clearing it falls back exactly one tier, not all the way to nothing.
    store_token(&id, secrets.as_ref(), "").await.unwrap();
    assert_eq!(
        credential_source_for(&runtime, source_of(&MapEnv::default()))
            .await
            .unwrap(),
        CredentialSource::Company
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// The DTO's whole surface, serialized: a tier name and non-secret routing.
/// No token, and no token-file path either — the console never needs to know
/// where on disk the instance's identity lives.
#[test]
fn the_dto_carries_a_tier_name_and_nothing_secret() {
    let dto = ComposioStatusDto {
        in_build: true,
        granted: true,
        credential_source: CredentialSource::Attested,
        managed_credential_source: CredentialSource::Attested,
        mode: ComposioMode::Managed,
        backend_url: "https://api.tinyhumans.ai".to_string(),
        toolkits: vec!["gmail".to_string()],
        open_mode: false,
        effective_toolkits: vec!["gmail".to_string()],
        effective_catalog: vec![CatalogEntry::from_slug("gmail")],
        catalog_source: CatalogSource::Manifest,
        catalog_notice: None,
    };
    let json = serde_json::to_value(&dto).unwrap();
    assert_eq!(json["credentialSource"], "attested");
    assert_eq!(json["managedCredentialSource"], "attested");
    assert_eq!(json["catalogSource"], "manifest");
    assert_eq!(json["mode"], "managed");
    let mut keys: Vec<&String> = json.as_object().unwrap().keys().collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "backendUrl",
            "catalogNotice",
            "catalogSource",
            "credentialSource",
            "effectiveCatalog",
            "effectiveToolkits",
            "granted",
            "inBuild",
            "managedCredentialSource",
            "mode",
            "openMode",
            "toolkits",
        ],
        "the read shape must stay exactly this: {keys:?}"
    );
}

/// A `*` wildcard grant must NOT count as a composio grant on the status
/// route (mirrors the harness build gate).
#[tokio::test]
async fn wildcard_grant_does_not_count_as_composio() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(
        &home,
        "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n[tools]\nallow = [\"*\"]\n",
    )
    .await;
    let (_, dto, _) = send(&state, "GET", "/api/v1/company/composio", None).await;
    assert_eq!(dto["granted"], false, "{dto}");
}

/// The OAuth sign-in plane is always wired into the route table (like the
/// status route), so `POST …/composio/authorize` is never a 404. Without a
/// usable Composio client it conflicts (`409`): on the default build because
/// the feature is not compiled in; under the `composio` feature because no
/// per-tenant token is configured yet. Either way the console gets a clear,
/// non-404 signal rather than a missing route.
#[tokio::test]
async fn authorize_route_conflicts_without_build_or_token() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(
        &home,
        "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n[tools]\nallow = [\"composio\"]\n[tools.composio]\ntoolkits = [\"gmail\"]\n",
    )
    .await;

    let (status, body, raw) = send(
        &state,
        "POST",
        "/api/v1/company/composio/authorize",
        Some(json!({ "toolkit": "gmail" })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{raw}");
    #[cfg(not(feature = "composio"))]
    assert_eq!(body["code"], "not_in_build", "{body}");
    #[cfg(feature = "composio")]
    assert_eq!(body["code"], "not_configured", "{body}");
}

// --- Who may change what the company connects through (issue #403) -------

/// A company granting composio with one provider — the shape every
/// authorization test below drives.
const GRANTED: &str = "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n\
     [tools]\nallow = [\"composio\"]\n[tools.composio]\ntoolkits = [\"gmail\"]\n";

/// The regression this issue is about: a signed-in member who is not an
/// admin cannot change what the company's agents connect through — neither
/// by replacing the credential they present, nor by starting an OAuth
/// handoff that would make their own account the company's connection.
///
/// Both halves matter. The reported symptom was the connect flow, but the
/// token route is the sharper one: it repoints the company's entire tool
/// surface at whatever account the caller controls.
#[tokio::test]
async fn a_member_cannot_change_what_the_company_connects_through() {
    let home_dir = home();
    let state = state_with_manifest(home_dir.path(), GRANTED).await;
    let member =
        crate::server::test_support::seed_session(&state, "acme", crate::ports::UserRole::Member)
            .await;

    let (status, body, raw) = send_as(
        &state,
        "PUT",
        "/api/v1/company/composio/token",
        Some(json!({ "token": TOKEN })),
        Auth::Cookie(member.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{raw}");
    assert_eq!(body["code"], "forbidden", "{body}");
    assert!(
        body["error"].as_str().unwrap_or_default().contains("admin"),
        "the refusal has to say why it was refused: {body}"
    );

    let (status, body, raw) = send_as(
        &state,
        "POST",
        "/api/v1/company/composio/authorize",
        Some(json!({ "toolkit": "gmail" })),
        Auth::Cookie(member.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{raw}");
    assert_eq!(body["code"], "forbidden", "{body}");

    // And the refusal is real, not merely a different status: the token
    // never landed, so the company's credential is untouched.
    let (_, dto, _) = send(&state, "GET", "/api/v1/company/composio", None).await;
    assert_eq!(
        dto["credentialSource"], "none",
        "a refused write must not have stored anything: {dto}"
    );
}

/// The other side of the boundary, and the reason this is a role check
/// rather than a removal: an admin still does both things. Without the
/// feature, `authorize` answers `409` at the build boundary; with it, this
/// test supplies a loopback backend and proves the admin reaches the real
/// authorization call without dialling production (issue #801).
#[tokio::test]
async fn an_admin_is_unaffected() {
    #[cfg(feature = "composio")]
    let backend = spawn_authorize_backend().await;
    #[cfg(feature = "composio")]
    let env = crate::test_support::EnvVarGuard::capture(&[
        crate::company::composio::TINYHUMANS_API_URL_ENV,
    ]);
    #[cfg(feature = "composio")]
    env.set(crate::company::composio::TINYHUMANS_API_URL_ENV, &backend);

    let home_dir = home();
    let state = state_with_manifest(home_dir.path(), GRANTED).await;

    let (status, resp, raw) = send(
        &state,
        "PUT",
        "/api/v1/company/composio/token",
        Some(json!({ "token": TOKEN })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(resp["status"]["credentialSource"], "static");

    let (status, _body, raw) = send(
        &state,
        "POST",
        "/api/v1/company/composio/authorize",
        Some(json!({ "toolkit": "gmail" })),
    )
    .await;
    #[cfg(feature = "composio")]
    assert_eq!(
        status,
        StatusCode::OK,
        "an admin reaches authorization: {raw}"
    );
    #[cfg(feature = "composio")]
    assert_eq!(
        _body["connectUrl"], "https://composio.test/connect/gmail",
        "the handler returns the loopback backend's authorization URL: {_body}"
    );
    #[cfg(not(feature = "composio"))]
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "an admin reaches the build check, not the role check: {raw}"
    );
}

/// Reads stay open to any member, and this pins that decision either way.
///
/// Knowing *that* Gmail is connected is what lets a member understand why
/// an agent can read mail; it carries no credential. Only the ability to
/// change it needed an owner. If a future change decides reads are
/// sensitive too, this test is the thing that has to be edited on purpose.
#[tokio::test]
async fn a_member_may_still_read_the_composio_status_and_connections() {
    let home_dir = home();
    let state = state_with_manifest(home_dir.path(), GRANTED).await;
    let member =
        crate::server::test_support::seed_session(&state, "acme", crate::ports::UserRole::Member)
            .await;

    let (status, dto, raw) = send_as(
        &state,
        "GET",
        "/api/v1/company/composio",
        None,
        Auth::Cookie(member.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(dto["granted"], true);
    assert!(
        dto.get("token").is_none(),
        "a member's read must not carry a credential either: {dto}"
    );

    // 409 (no client in this build), NOT 403 — the read is not role-gated.
    let (status, _, raw) = send_as(
        &state,
        "GET",
        "/api/v1/company/composio/connections",
        None,
        Auth::Cookie(member),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{raw}");
}

/// A machine credential may set the company's token — and is **named** in
/// the trail when it does.
///
/// This is the deliberate half of the issue's "cannot, or if it may, the
/// write is attributed". Refusing the hosting control plane a route it
/// already sits above (it provisions the tenant and holds its database
/// credentials) would be ceremony, not a boundary, and it would contradict
/// the two-principal model in `docs/spec/runtime/config.md`. What it does
/// not get is anonymity: the entry names the tenant, so a machine-made
/// change is as reviewable afterwards as a human one.
#[tokio::test]
async fn a_machine_credential_is_named_when_it_sets_the_token() {
    use crate::server::platform_auth::{
        PlatformAuthConfig, PlatformClaims, UnsignedTenantVerifier,
    };

    let home_dir = home();
    let state = state_with_manifest(home_dir.path(), GRANTED)
        .await
        .with_platform_auth(PlatformAuthConfig::new(std::sync::Arc::new(
            UnsignedTenantVerifier::new("test-platform-secret"),
        )));
    let token = UnsignedTenantVerifier::tenant_token(&PlatformClaims {
        tenant: "tenant:platform".to_string(),
        scopes: std::collections::HashSet::from(["platform".to_string()]),
        companies: None,
    });

    let (status, _, raw) = send_as(
        &state,
        "PUT",
        "/api/v1/company/composio/token",
        Some(json!({ "token": TOKEN })),
        Auth::Bearer(token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");

    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let events = runtime
        .events()
        .read_from(runtime.id(), crate::ports::types::EventSeq::new(0), 100)
        .await
        .unwrap();
    let by = events
        .iter()
        .find_map(|stored| match &stored.event {
            crate::ports::types::CompanyEvent::ToolAccessChanged { by, .. } => by.clone(),
            _ => None,
        })
        .expect("the machine write is journaled");
    assert_eq!(
        by.kind,
        crate::ports::types::ActorKind::System,
        "a machine is a machine, not a person: {by:?}"
    );
    assert_eq!(by.id, "tenant:platform", "the tenant is named: {by:?}");
}

/// Every accepted change to the company's tool access names the person who
/// made it, so "how did we come to be connected through that account" is
/// answerable afterwards.
///
/// Set and clear are journaled as distinct words — one grants access and
/// the other withdraws it, and an audit trail that conflated them would be
/// worth little.
#[tokio::test]
async fn a_credential_change_records_who_made_it() {
    use crate::ports::types::{ActorKind, CompanyEvent, EventSeq};

    let home_dir = home();
    let state = state_with_manifest(home_dir.path(), GRANTED).await;

    // `confirmInUse` is ignored on the set and needed on the clear (the
    // company is on the managed route throughout, in-use-guards.md §2)
    // — this test is about the journaled actor, not the guard, so both
    // iterations confirm.
    for token in [TOKEN, ""] {
        let (status, _, raw) = send(
            &state,
            "PUT",
            "/api/v1/company/composio/token",
            Some(json!({ "token": token, "confirmInUse": true })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{raw}");
    }

    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let events = runtime
        .events()
        .read_from(runtime.id(), EventSeq::new(0), 100)
        .await
        .unwrap();
    let changes: Vec<(String, Option<String>)> = events
        .iter()
        .filter_map(|stored| match &stored.event {
            CompanyEvent::ToolAccessChanged {
                change,
                toolkit,
                by,
            } => {
                let by = by.as_ref().expect("an admin write is always attributed");
                assert_eq!(by.kind, ActorKind::User, "attributed to a person");
                assert!(!by.id.is_empty(), "the person is named");
                Some((change.clone(), toolkit.clone()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        changes,
        vec![
            ("credential_set".to_string(), None),
            ("credential_cleared".to_string(), None),
        ],
        "a set and a clear are distinct entries in the trail"
    );

    // The trail is an audit record, not a second place a secret lives.
    let raw = serde_json::to_string(&events).unwrap();
    assert!(!raw.contains(TOKEN), "the journal leaked the token: {raw}");
}

/// `GET …/composio/connections` is likewise always wired and conflicts
/// (`409`) when there is no usable client — no build feature or no token.
#[tokio::test]
async fn connections_route_conflicts_without_build_or_token() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(
        &home,
        "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n[tools]\nallow = [\"composio\"]\n[tools.composio]\ntoolkits = [\"gmail\"]\n",
    )
    .await;

    let (status, body, raw) =
        send(&state, "GET", "/api/v1/company/composio/connections", None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{raw}");
    #[cfg(not(feature = "composio"))]
    assert_eq!(body["code"], "not_in_build", "{body}");
    #[cfg(feature = "composio")]
    assert_eq!(body["code"], "not_configured", "{body}");
}

/// The disconnect added for #404 is wired on the same terms as the rest of
/// the OAuth plane: present in the route table whatever the build, and a
/// `409` — never a `404` — when there is no usable client. A `404` here
/// would read as "no such connection", which is a claim about the company's
/// accounts that this build cannot make.
#[tokio::test]
async fn disconnect_route_conflicts_without_build_or_token() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(
        &home,
        "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n[tools]\nallow = [\"composio\"]\n[tools.composio]\ntoolkits = [\"gmail\"]\n",
    )
    .await;

    let (status, body, raw) = send(
        &state,
        "DELETE",
        "/api/v1/company/composio/connections/conn-1",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{raw}");
    #[cfg(not(feature = "composio"))]
    assert_eq!(body["code"], "not_in_build", "{body}");
    #[cfg(feature = "composio")]
    assert_eq!(body["code"], "not_configured", "{body}");
}
