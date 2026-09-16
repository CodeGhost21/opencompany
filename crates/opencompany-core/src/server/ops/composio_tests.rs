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

/// Issue #397: an **empty** manifest allowlist means "defer to the backend"
/// — allow everything — so the status must report open mode and hand the
/// console a non-empty starting set. The old console gate keyed off
/// `toolkits.length > 0` and therefore rendered nothing in exactly the case
/// where everything is permitted.
#[tokio::test]
async fn an_empty_toolkit_list_is_open_mode_and_still_offers_providers() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    // No `[tools.composio]` section at all — the shape 19 of 20 templates ship.
    let state = state_with_manifest(
        &home,
        "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n[tools]\nallow = [\"composio\"]\n",
    )
    .await;

    let (status, dto, _) = send(&state, "GET", "/api/v1/company/composio", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(dto["granted"], true);
    assert_eq!(dto["toolkits"], json!([]), "the manifest list is verbatim");
    assert_eq!(dto["openMode"], true);
    let offered = dto["effectiveToolkits"]
        .as_array()
        .expect("open mode carries an effective toolkit list");
    assert!(
        !offered.is_empty(),
        "open mode means allow-everything, so the console must be offered providers"
    );
    assert!(
        offered.iter().any(|t| t == "github"),
        "the fallback set covers the common providers: {offered:?}"
    );
    // No credential and (in the default build) no client, so the catalog
    // cannot be read. The list is still offered — but it says so.
    assert_eq!(
        dto["catalogSource"], "fallback",
        "an unfetchable catalog must never be reported as the backend's: {dto}"
    );
    assert!(
        dto["catalogNotice"]
            .as_str()
            .is_some_and(|n| n.contains("incomplete")),
        "a fallback tells the operator it may be incomplete: {dto}"
    );
}

/// The company registered under `company` in `state`.
fn runtime_of(state: &AppState, company: &str) -> std::sync::Arc<super::CompanyRuntime> {
    state
        .registry()
        .get(&CompanyId::new(company))
        .expect("company is registered")
}

/// A hundred-provider catalog, the shape the backend actually returns —
/// each entry carrying the display metadata #600 stopped discarding.
fn hundred_entries() -> Vec<CatalogEntry> {
    (0..100)
        .map(|i| CatalogEntry {
            slug: format!("provider{i:03}"),
            name: format!("Provider {i:03}"),
            description: format!("Does provider-{i:03} things."),
            logo: Some(format!("https://logos.example.test/provider{i:03}")),
            categories: vec!["productivity".to_string()],
        })
        .collect()
}

/// Just the slugs of [`hundred_entries`], for asserting on the slug list
/// the wire has always carried.
fn hundred_slugs() -> Vec<String> {
    hundred_entries().into_iter().map(|e| e.slug).collect()
}

/// Serialises this test against `an_admin_is_unaffected`'s env mutation.
///
/// In composio builds that test repoints `TINYHUMANS_API_URL_ENV` at a
/// loopback backend for its whole body (Composio's own backend-URL override,
/// `OPENCOMPANY_COMPOSIO_BACKEND_URL`, was removed in phase 6a of #2306; the
/// tenant's shared API base is the only repoint left). The cache-seeding
/// tests below derive a cache key that embeds that URL, seed the cache under
/// it, then re-derive the key on the request path — a process-wide override
/// landing between the two reads would change the key and strand the seeded
/// entry, failing the `catalogSource == "backend"` assertion. `EnvVarGuard`
/// serialises guard users against each other, so taking it here closes the
/// race the way the crate documents (unguarded `std::env::var` readers are
/// otherwise fair game). Gated to composio builds, where the mutation
/// exists.
#[cfg(feature = "composio")]
fn composio_backend_env_guard() -> crate::test_support::EnvVarGuard {
    crate::test_support::EnvVarGuard::capture(&[
        crate::company::composio::TINYHUMANS_API_URL_ENV,
    ])
}

/// The heart of the reopened issue: in open mode the console is offered the
/// **backend's** catalog, not a list maintained by hand in this repo.
///
/// The cache is seeded rather than a backend stood up, because what is under
/// test is the decision — "open mode serves the fetched catalog" — and not
/// the HTTP call, which has its own test beside
/// `list_catalog_toolkits`. Seeding proves the thing that regressed: that a
/// fetched answer actually reaches the response instead of being discarded
/// in favour of the constant.
#[tokio::test]
async fn open_mode_serves_the_fetched_catalog_rather_than_a_hardcoded_list() {
    #[cfg(feature = "composio")]
    let _env = composio_backend_env_guard();
    let home_dir = home();
    let state = state_with_manifest_id(
        home_dir.path(),
        "catalogco",
        "[company]\nname = \"Catalog Co\"\n[policy]\nmode = \"full\"\n[tools]\nallow = [\"composio\"]\n",
    )
    .await;
    let catalog = hundred_entries();
    composio_toolkits::cache().store(
        &super::catalog_cache_key(runtime_of(&state, "catalogco").as_ref()),
        Ok(catalog.clone()),
        std::time::Instant::now(),
    );

    let (status, dto, raw) =
        send_for(&state, "catalogco", "GET", "/api/v1/company/composio", None).await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(dto["openMode"], true);
    assert_eq!(
        dto["catalogSource"], "backend",
        "a real catalog is reported as the backend's answer: {dto}"
    );
    assert_eq!(
        dto["catalogNotice"],
        Value::Null,
        "nothing to apologise for"
    );
    assert_eq!(
        dto["effectiveToolkits"],
        json!(hundred_slugs()),
        "open mode must serve what the backend permits, verbatim"
    );
    assert!(
        dto["effectiveToolkits"].as_array().unwrap().len()
            > composio_toolkits::FALLBACK_TOOLKITS.len(),
        "the fetched catalog is far longer than the built-in list — serving the \
         constant here is the exact regression this test exists to catch: {dto}"
    );
}

/// The important one. A company that deliberately narrowed its belt must
/// **not** be silently widened by the catalog.
///
/// The same hundred-slug catalog is in the cache; this company asked for
/// `gmail` and gets `gmail`. Anything that unioned, defaulted-to, or fell
/// back to the catalog for an explicit manifest would hand the company
/// ninety-nine providers it decided against.
#[tokio::test]
async fn an_explicit_allowlist_is_never_widened_by_the_catalog() {
    #[cfg(feature = "composio")]
    let _env = composio_backend_env_guard();
    let home_dir = home();
    let state = state_with_manifest_id(
        home_dir.path(),
        "narrowco",
        "[company]\nname = \"Narrow Co\"\n[policy]\nmode = \"full\"\n[tools]\nallow = [\"composio\"]\n[tools.composio]\ntoolkits = [\"gmail\"]\n",
    )
    .await;
    composio_toolkits::cache().store(
        &super::catalog_cache_key(runtime_of(&state, "narrowco").as_ref()),
        Ok(hundred_entries()),
        std::time::Instant::now(),
    );

    let (status, dto, raw) =
        send_for(&state, "narrowco", "GET", "/api/v1/company/composio", None).await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(dto["openMode"], false);
    assert_eq!(dto["effectiveToolkits"], json!(["gmail"]));
    assert_eq!(dto["toolkits"], json!(["gmail"]));
    assert_eq!(
        dto["catalogSource"], "manifest",
        "the company's own list is the source, and the catalog is not consulted: {dto}"
    );
}

/// A catalog that cannot be fetched degrades to the built-in list — and the
/// response says so, both in `catalogSource` and in words the console can
/// show the operator. Silently serving eight slugs that look like the whole
/// catalog is the failure this pins shut.
#[tokio::test]
async fn an_unfetchable_catalog_is_marked_degraded_not_passed_off_as_real() {
    #[cfg(feature = "composio")]
    let _env = composio_backend_env_guard();
    let home_dir = home();
    let state = state_with_manifest_id(
        home_dir.path(),
        "degradedco",
        "[company]\nname = \"Degraded Co\"\n[policy]\nmode = \"full\"\n[tools]\nallow = [\"composio\"]\n",
    )
    .await;
    composio_toolkits::cache().store(
        &super::catalog_cache_key(runtime_of(&state, "degradedco").as_ref()),
        Err("connection refused".to_string()),
        std::time::Instant::now(),
    );

    let (status, dto, raw) = send_for(
        &state,
        "degradedco",
        "GET",
        "/api/v1/company/composio",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(dto["openMode"], true);
    assert_eq!(
        dto["catalogSource"], "fallback",
        "the built-in list must never claim to be the backend's: {dto}"
    );
    let notice = dto["catalogNotice"]
        .as_str()
        .expect("a degraded list must explain itself");
    assert!(notice.contains("connection refused"), "{notice}");
    assert!(notice.contains("may be incomplete"), "{notice}");
    // Still usable: the operator gets something to click, just not a claim.
    assert_eq!(
        dto["effectiveToolkits"],
        json!(composio_toolkits::FALLBACK_TOOLKITS)
    );
}

/// A credential change drops the cached catalog: a rotated BYO token can
/// resolve to a different Composio account, and serving the previous
/// account's list for the rest of the TTL is the stale-by-construction
/// answer this issue is about.
///
/// Driven through a *clear* rather than a set so the status re-read at the
/// end of the write has no credential to dial with — the assertion is about
/// the eviction, and a test that reached the network to make it would be a
/// different test.
#[tokio::test]
async fn a_credential_change_evicts_the_cached_catalog() {
    #[cfg(feature = "composio")]
    let _env = composio_backend_env_guard();
    let home_dir = home();
    let state = state_with_manifest_id(
        home_dir.path(),
        "rotateco",
        "[company]\nname = \"Rotate Co\"\n[policy]\nmode = \"full\"\n[tools]\nallow = [\"composio\"]\n",
    )
    .await;
    let key = super::catalog_cache_key(runtime_of(&state, "rotateco").as_ref());
    composio_toolkits::cache().store(&key, Ok(hundred_entries()), std::time::Instant::now());

    let (status, resp, raw) = send_for(
        &state,
        "rotateco",
        "PUT",
        "/api/v1/company/composio/token",
        // The managed token clear is guarded while the company is on the
        // managed route (in-use-guards.md §2) — this test is about
        // catalog eviction, not the guard, so it confirms.
        Some(json!({ "token": "", "confirmInUse": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_ne!(
        resp["status"]["catalogSource"], "backend",
        "the pre-change catalog must not survive the change: {resp}"
    );
}

/// The other half of #397: a company that deliberately narrowed its belt
/// sees exactly what it chose, and is NOT in open mode. A fix that offered
/// the curated set to everyone would silently widen every restrictive
/// manifest.
#[tokio::test]
async fn an_explicit_toolkit_list_is_offered_verbatim_and_is_not_open_mode() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(
        &home,
        "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n[tools]\nallow = [\"composio\"]\n[tools.composio]\ntoolkits = [\"gmail\"]\n",
    )
    .await;

    let (status, dto, _) = send(&state, "GET", "/api/v1/company/composio", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(dto["openMode"], false);
    assert_eq!(dto["effectiveToolkits"], json!(["gmail"]));
    assert_eq!(dto["toolkits"], json!(["gmail"]));
}

/// The `credentialSource` matrix as the console consumes it, plus the
/// write-only contract. Runs with no platform identity in the environment, so
/// the instance contributes nothing and the company's own token is the only
/// credential in play: `none` → `static` → `none`.
#[tokio::test]
async fn credential_source_tracks_the_byo_token_and_never_carries_it() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(
        &home,
        "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n[tools]\nallow = [\"composio\"]\n[tools.composio]\ntoolkits = [\"gmail\", \"github\"]\n",
    )
    .await;

    // Initial status: granted, toolkits surfaced, no credential obtainable.
    let (status, dto, _) = send(&state, "GET", "/api/v1/company/composio", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(dto["granted"], true);
    assert_eq!(dto["credentialSource"], "none");
    assert_eq!(dto["toolkits"], json!(["gmail", "github"]));
    assert!(dto.get("backendUrl").is_some());
    assert!(
        dto.get("token").is_none(),
        "status must never carry a token"
    );
    assert!(
        dto.get("tokenConfigured").is_none(),
        "the boolean surface is replaced by credentialSource"
    );

    // Set the write-only BYO token → the static tier.
    let (status, resp, raw) = send(
        &state,
        "PUT",
        "/api/v1/company/composio/token",
        Some(json!({ "token": TOKEN })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(resp["status"]["credentialSource"], "static");
    assert!(!raw.contains(TOKEN), "PUT response leaked the token: {raw}");

    // GET reflects it and still never carries the token.
    let (_, dto, raw) = send(&state, "GET", "/api/v1/company/composio", None).await;
    assert_eq!(dto["credentialSource"], "static");
    assert!(!raw.contains(TOKEN), "GET status leaked the token: {raw}");

    // Clearing with "" reverts to whatever the instance offers — here
    // nothing. Guarded while the company is on the managed route
    // (in-use-guards.md §2); this test is about credentialSource, not
    // the guard, so it confirms.
    let (_, resp, _) = send(
        &state,
        "PUT",
        "/api/v1/company/composio/token",
        Some(json!({ "token": "", "confirmInUse": true })),
    )
    .await;
    assert_eq!(resp["status"]["credentialSource"], "none");
}

/// BYOK end to end over the route: storing a Composio API key moves the
/// company onto its own account, the read plane says so without ever
/// carrying the key, and clearing it hands the managed route back.
#[tokio::test]
async fn a_company_can_bring_its_own_composio_account_and_give_it_back() {
    const API_KEY: &str = "ak_not_a_real_key_0123456789";
    let home_dir = home();
    let state = state_with_manifest(home_dir.path(), GRANTED).await;
    // This test is about storing and clearing, not about the check. Pin the
    // probe clean so a `composio` build runs it against the override rather
    // than against `backend.composio.dev`.
    super::probe_override::set("acme", Ok(()));

    // Managed is the default and needs nothing stored.
    let (_, dto, _) = send(&state, "GET", "/api/v1/company/composio", None).await;
    assert_eq!(dto["mode"], "managed");

    let (status, resp, raw) = send(
        &state,
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": API_KEY })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(resp["status"]["mode"], "byok");
    assert_eq!(
        resp["status"]["credentialSource"], "static",
        "the key is this company's own, which is the static tier"
    );
    assert!(
        !raw.contains(API_KEY),
        "the PUT response leaked the key: {raw}"
    );

    // The read plane reports the route and the host it reaches, and still
    // carries no credential.
    let (_, dto, raw) = send(&state, "GET", "/api/v1/company/composio", None).await;
    assert_eq!(dto["mode"], "byok");
    assert_eq!(
        dto["backendUrl"],
        crate::company::composio::DIRECT_BASE_URL,
        "a status still naming the managed backend would read as though nothing changed"
    );
    assert!(
        !raw.contains(API_KEY),
        "the GET status leaked the key: {raw}"
    );

    // Clearing it returns the company to the managed route. Guarded
    // while the company is on BYOK (in-use-guards.md §2); this test is
    // about the route round-trip, not the guard, so it confirms.
    let (_, resp, _) = send(
        &state,
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": "", "confirmInUse": true })),
    )
    .await;
    assert_eq!(resp["status"]["mode"], "managed");
    assert_ne!(
        resp["status"]["backendUrl"],
        crate::company::composio::DIRECT_BASE_URL
    );
}

// ── The managed tier, reported independently of the mode (#2275) ────

/// The field's whole reason to exist: under BYOK, `credentialSource` names
/// the Composio key the agents present, and `managedCredentialSource` still
/// names the tier the **managed** route would answer with.
///
/// Driven over the real route rather than over the resolver, because what
/// regressed before is not the chain — it is whether the chain's answer
/// survives the short-circuit `resolve_access` takes under BYOK and reaches
/// the DTO at all.
#[tokio::test]
async fn the_managed_tier_is_reported_while_byok_is_selected() {
    const BYOK_KEY: &str = "ak_not_a_real_key_0123456789";
    use crate::company::company_key;
    use crate::company::composio::store_token;

    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "managedtier", GRANTED).await;
    // The check is not what this test is about; pin it clean so a
    // `composio` build answers from the override instead of the network.
    super::probe_override::set("managedtier", Ok(()));
    let runtime = runtime_of(&state, "managedtier");
    let secrets = runtime.secrets();

    async fn status(state: &AppState) -> (Value, String) {
        let (code, dto, raw) = send_for(
            state,
            "managedtier",
            "GET",
            "/api/v1/company/composio",
            None,
        )
        .await;
        assert_eq!(code, StatusCode::OK, "{raw}");
        (dto, raw)
    }

    // Nothing stored, and (in the test process) no instance identity: both
    // fields agree, and agree on nothing.
    let (dto, _) = status(&state).await;
    assert_eq!(dto["credentialSource"], "none");
    assert_eq!(dto["managedCredentialSource"], "none");

    // A pasted `composio/tinyhumans/key` (formerly `composio/token`) is the
    // managed chain's first tier.
    store_token(runtime.id(), secrets.as_ref(), "byo-managed-bearer")
        .await
        .unwrap();
    let (dto, _) = status(&state).await;
    assert_eq!(dto["credentialSource"], "static");
    assert_eq!(
        dto["managedCredentialSource"], "static",
        "under managed the two fields come from the same resolver and cannot differ: {dto}"
    );

    // Withdraw it over the company's own TinyHumans key: the chain falls
    // exactly one tier, and both fields follow it.
    store_token(runtime.id(), secrets.as_ref(), "")
        .await
        .unwrap();
    company_key::store_key(runtime.id(), secrets.as_ref(), "th_company")
        .await
        .unwrap();
    let (dto, _) = status(&state).await;
    assert_eq!(dto["credentialSource"], "company");
    assert_eq!(dto["managedCredentialSource"], "company");

    // The point. BYOK is selected, so the agents present the Composio key —
    // `static`. The managed route is untouched underneath, and the console
    // can now say so without making the operator clear the key to find out.
    let (code, resp, raw) = send_for(
        &state,
        "managedtier",
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": BYOK_KEY })),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{raw}");
    assert_eq!(resp["status"]["mode"], "byok");
    assert_eq!(resp["status"]["credentialSource"], "static");
    assert_eq!(
        resp["status"]["managedCredentialSource"], "company",
        "BYOK must not be able to hide which tier the managed route would answer with: {resp}"
    );
    assert!(!raw.contains(BYOK_KEY), "the PUT leaked the key: {raw}");

    let (dto, raw) = status(&state).await;
    assert_eq!(dto["credentialSource"], "static");
    assert_eq!(dto["managedCredentialSource"], "company");
    assert!(!raw.contains(BYOK_KEY), "the GET leaked the key: {raw}");
    assert!(
        dto.get("token").is_none(),
        "status must never carry a token"
    );
    assert!(
        dto.get("tokenConfigured").is_none(),
        "the boolean surface stays gone (#886): {dto}"
    );
}

/// The `attested` row of the matrix, which the route cannot reach without
/// mutating the process environment.
///
/// Drives the **exact pair of calls** `effective_status` makes — `access_for`
/// for the effective tier, `resolve_credential` for the managed one — against
/// a company already switched to BYOK, through the env seam. A company whose
/// only credential is the instance identity presents its own Composio key and
/// still reports `attested` as what managed would fall back to.
#[tokio::test]
async fn the_managed_tier_reads_the_instance_identity_under_byok() {
    use crate::app::config::MapEnv;
    use crate::company::composio::{resolve_credential, store_api_key};

    let dir = tempfile::Builder::new()
        .prefix("oc-managed-tier-")
        .tempdir()
        .expect("tempdir");
    let path = dir.path().join("token");
    std::fs::write(&path, "projected-instance-token").unwrap();
    let projected = MapEnv::new([(
        crate::company::credentials::TOKEN_FILE_ENV,
        path.display().to_string(),
    )]);
    let source = super::TinyhumansTokenSource::from_env(&projected).map(std::sync::Arc::new);

    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "attestedbyok", GRANTED).await;
    let runtime = runtime_of(&state, "attestedbyok");

    // Managed, with only the instance identity: both answers are `attested`.
    assert_eq!(
        access_for(&runtime, source.clone()).await.unwrap().1,
        CredentialSource::Attested
    );
    assert_eq!(
        resolve_credential(runtime.id(), runtime.secrets().as_ref(), source.clone())
            .await
            .unwrap()
            .source(),
        CredentialSource::Attested
    );

    // Switch to BYOK. The effective tier becomes the company's own Composio
    // key; the managed chain is unchanged and still answers `attested`.
    store_api_key(
        runtime.id(),
        runtime.secrets().as_ref(),
        "ak_not_a_real_key_0123456789",
    )
    .await
    .unwrap();
    let (mode, effective) = access_for(&runtime, source.clone()).await.unwrap();
    assert_eq!(mode, super::ComposioMode::Byok);
    assert_eq!(effective, CredentialSource::Static);
    assert_eq!(
        resolve_credential(runtime.id(), runtime.secrets().as_ref(), source)
            .await
            .unwrap()
            .source(),
        CredentialSource::Attested,
        "selecting BYOK must not change what the managed route would resolve to"
    );

    std::fs::remove_dir_all(&dir).ok();
}

// ── The draft-key check on `PUT …/composio/api-key` (#2275) ──────────

/// A key Composio rejects is **not stored**, and the refusal says so in the
/// classifier's own words.
///
/// There is no rollback here because there is no write: the probe runs on
/// the draft, before the store, which is the deliberate departure from the
/// inference connect flow (that one writes first because its probe resolves
/// the key by slug). The proof is the GET afterwards — the company is still
/// on the managed route.
#[tokio::test]
async fn a_rejected_key_is_refused_and_nothing_is_stored() {
    const BYOK_KEY: &str = "ak_not_a_real_key_0123456789";
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "probeauth", GRANTED).await;
    super::probe_override::set(
        "probeauth",
        Err("Composio answered 401 Unauthorized".to_string()),
    );

    let (code, body, raw) = send_for(
        &state,
        "probeauth",
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": BYOK_KEY })),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "{raw}");
    assert_eq!(body["code"], "invalid_request", "{body}");
    assert!(
        body["error"]
            .as_str()
            .is_some_and(
                |message| message.contains(crate::company::composio_probe::describe(
                    crate::company::composio_probe::ComposioProbeClass::Auth
                ))
            ),
        "the refusal is the classifier's copy, not an interpolated upstream string: {body}"
    );
    assert!(
        !body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("Unauthorized"),
        "the upstream reason belongs in the debug log, not in the refusal: {body}"
    );
    assert!(
        !raw.contains(BYOK_KEY),
        "a refusal must not echo the key back: {raw}"
    );

    let (_, dto, raw) =
        send_for(&state, "probeauth", "GET", "/api/v1/company/composio", None).await;
    assert_eq!(
        dto["mode"], "managed",
        "a refused key must leave the company exactly where it was: {dto}"
    );
    assert!(!raw.contains(BYOK_KEY), "the GET leaked the key: {raw}");
}

/// A check that fails for any reason **other** than the credential keeps the
/// key: it is plausibly fine and only the connection is in question. The
/// write lands, and the response carries the class and the advisory beside
/// it.
///
/// The case in the fixture is the one the ordering rule exists for — a
/// gateway in the path — which must never read as a bad key.
#[tokio::test]
async fn a_key_that_could_not_be_checked_is_stored_with_an_advisory() {
    const BYOK_KEY: &str = "ak_not_a_real_key_0123456789";
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "probeadvise", GRANTED).await;
    super::probe_override::set("probeadvise", Err("502 Bad Gateway".to_string()));

    let (code, resp, raw) = send_for(
        &state,
        "probeadvise",
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": BYOK_KEY })),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{raw}");
    assert_eq!(resp["status"]["mode"], "byok", "the key was stored: {resp}");
    assert_eq!(resp["probeClass"], "unknown", "{resp}");
    assert_eq!(
        resp["advisory"],
        crate::company::composio_probe::describe(
            crate::company::composio_probe::ComposioProbeClass::Unknown
        )
    );
    assert!(
        !raw.contains("Bad Gateway"),
        "the upstream text belongs in the debug log, not in operator copy: {raw}"
    );
    assert!(!raw.contains(BYOK_KEY), "the PUT leaked the key: {raw}");
}

/// A clean check stores the key and says nothing extra — the two additive
/// fields are **omitted**, not null, so a consumer that predates them reads
/// the body it always did.
#[tokio::test]
async fn a_clean_check_stores_the_key_and_adds_nothing_to_the_response() {
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "probeclean", GRANTED).await;
    super::probe_override::set("probeclean", Ok(()));

    let (code, resp, raw) = send_for(
        &state,
        "probeclean",
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": "ak_not_a_real_key_0123456789" })),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{raw}");
    assert_eq!(resp["status"]["mode"], "byok");
    assert!(resp.get("advisory").is_none(), "{resp}");
    assert!(resp.get("probeClass").is_none(), "{resp}");
}

/// `skipVerify` is the "add anyway" escape: a key whose check the route
/// would have refused is stored, unexamined, when the caller says so.
///
/// Defaulting matters as much as the behaviour — every test above sends no
/// `skipVerify` at all and gets the checked path, which is what pins the
/// `#[serde(default)]` false.
#[tokio::test]
async fn skip_verify_stores_a_key_the_check_would_have_refused() {
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "probeskip", GRANTED).await;
    super::probe_override::set(
        "probeskip",
        Err("Composio answered 401 Unauthorized".to_string()),
    );

    let (code, resp, raw) = send_for(
        &state,
        "probeskip",
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": "ak_not_a_real_key_0123456789", "skipVerify": true })),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{raw}");
    assert_eq!(resp["status"]["mode"], "byok", "{resp}");
    assert!(
        resp.get("advisory").is_none(),
        "nothing was checked, so there is nothing to advise about: {resp}"
    );
}

/// Clearing is **never** checked. Withdrawing a credential is always
/// allowed, and a probe that could refuse a clear would strand a company on
/// a key it had already decided against.
#[tokio::test]
async fn clearing_a_key_is_never_checked() {
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "probeclear", GRANTED).await;
    super::probe_override::set("probeclear", Ok(()));
    let (code, _, raw) = send_for(
        &state,
        "probeclear",
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": "ak_not_a_real_key_0123456789" })),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{raw}");

    // Now make any probe destructive. The clear must still go through.
    // Guarded while the company is on BYOK (in-use-guards.md §2); this
    // test is about the probe never blocking a clear, not the guard, so
    // it confirms.
    super::probe_override::set(
        "probeclear",
        Err("Composio answered 401 Unauthorized".to_string()),
    );
    let (code, resp, raw) = send_for(
        &state,
        "probeclear",
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": "", "confirmInUse": true })),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{raw}");
    assert_eq!(resp["status"]["mode"], "managed", "{resp}");
    assert!(resp.get("advisory").is_none(), "{resp}");
}

// ── `POST …/composio/api-key/test` — check, never change ─────────────

/// A company on the managed route has no key of its own to check, and a
/// company in BYOK with a blank slot has none either. Both answer the same
/// permanent `not_configured`, which is what lets the console disable the
/// control instead of offering a check that can only fail.
#[tokio::test]
async fn testing_a_key_that_is_not_there_says_so_rather_than_probing() {
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "testnokey", GRANTED).await;
    // Destructive if it ever ran. It must not run.
    super::probe_override::set(
        "testnokey",
        Err("Composio answered 401 Unauthorized".to_string()),
    );

    // Managed — the default, nothing stored.
    let (code, body, raw) = send_for(
        &state,
        "testnokey",
        "POST",
        "/api/v1/company/composio/api-key/test",
        None,
    )
    .await;
    assert_eq!(code, StatusCode::CONFLICT, "{raw}");
    assert_eq!(body["code"], "not_configured", "{body}");

    // BYOK selected with an empty slot — the same answer, because the same
    // thing is true: there is no key here to check.
    let runtime = runtime_of(&state, "testnokey");
    runtime
        .secrets()
        .set(
            runtime.id(),
            crate::company::composio::MODE_KEY,
            crate::ports::types::SecretValue(crate::company::composio::BYOK_MODE.to_string()),
        )
        .await
        .unwrap();
    let (code, body, raw) = send_for(
        &state,
        "testnokey",
        "POST",
        "/api/v1/company/composio/api-key/test",
        None,
    )
    .await;
    assert_eq!(code, StatusCode::CONFLICT, "{raw}");
    assert_eq!(body["code"], "not_configured", "{body}");
}

/// The assertion the route exists to keep honest: a **failed** check —
/// including the destructive class — leaves the stored key and the stored
/// mode byte-identical. Testing a credential and withdrawing it are
/// separate acts, and nothing on this path may conflate them.
#[tokio::test]
async fn a_failed_check_changes_absolutely_nothing() {
    const BYOK_KEY: &str = "ak_not_a_real_key_0123456789";
    use crate::company::composio::{BYOK_KEY_KEY, LEGACY_API_KEY_KEY, MODE_KEY};

    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "testkeeps", GRANTED).await;
    super::probe_override::set("testkeeps", Ok(()));
    let (code, _, raw) = send_for(
        &state,
        "testkeeps",
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": BYOK_KEY })),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{raw}");

    let runtime = runtime_of(&state, "testkeeps");
    async fn slots(
        runtime: &super::CompanyRuntime,
    ) -> (Option<String>, Option<String>, Option<String>) {
        let read = |key: &'static str| async move {
            runtime
                .secrets()
                .get(runtime.id(), key)
                .await
                .unwrap()
                .map(|crate::ports::types::SecretValue(v)| v)
        };
        (
            read(MODE_KEY).await,
            read(BYOK_KEY_KEY).await,
            read(LEGACY_API_KEY_KEY).await,
        )
    }
    let before = slots(&runtime).await;

    // Now make the check fail in the ONE class that is destructive on the
    // write route, and check again.
    super::probe_override::set(
        "testkeeps",
        Err("Composio answered 401 Unauthorized".to_string()),
    );
    let (code, body, raw) = send_for(
        &state,
        "testkeeps",
        "POST",
        "/api/v1/company/composio/api-key/test",
        None,
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{raw}");
    assert_eq!(body["ok"], false, "{body}");
    assert_eq!(body["probeClass"], "auth", "{body}");
    assert_eq!(
        body["message"],
        crate::company::composio_probe::describe_verdict(
            crate::company::composio_probe::ComposioProbeClass::Auth
        ),
        "a check that stored nothing must not report that it saved: {body}"
    );
    assert!(
        !raw.contains(BYOK_KEY),
        "the check leaked the stored key: {raw}"
    );

    assert_eq!(
        slots(&runtime).await,
        before,
        "a failed check must leave the stored mode and key byte-identical"
    );

    // And the company is still on its own account, as the status says.
    let (_, dto, raw) =
        send_for(&state, "testkeeps", "GET", "/api/v1/company/composio", None).await;
    assert_eq!(dto["mode"], "byok", "{dto}");
    assert!(!raw.contains(BYOK_KEY), "the GET leaked the key: {raw}");
}

/// A key Composio accepts answers `ok` and nothing else — no class, no
/// sentence, and no credential.
#[tokio::test]
async fn a_clean_check_answers_ok_and_carries_no_credential() {
    const BYOK_KEY: &str = "ak_not_a_real_key_0123456789";
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "testclean", GRANTED).await;
    super::probe_override::set("testclean", Ok(()));
    let (code, _, raw) = send_for(
        &state,
        "testclean",
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": BYOK_KEY })),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{raw}");

    let (code, body, raw) = send_for(
        &state,
        "testclean",
        "POST",
        "/api/v1/company/composio/api-key/test",
        None,
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{raw}");
    assert_eq!(body["ok"], true, "{body}");
    assert!(body.get("probeClass").is_none(), "{body}");
    assert!(body.get("message").is_none(), "{body}");
    assert!(!raw.contains(BYOK_KEY), "the check leaked the key: {raw}");
}

// ── Storage addresses and the legacy fallback (#2306) ──────────────

/// A token `PUT` writes both addresses, and a legacy-only value still reads
/// as configured before that first write.
#[tokio::test]
async fn a_token_put_mirrors_to_the_legacy_slot() {
    use crate::company::composio::{LEGACY_TOKEN_KEY, TINYHUMANS_KEY_KEY};

    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "legacytoken", GRANTED).await;
    let runtime = runtime_of(&state, "legacytoken");
    runtime
        .secrets()
        .set(
            runtime.id(),
            LEGACY_TOKEN_KEY,
            crate::ports::types::SecretValue("th-not-a-real-key".into()),
        )
        .await
        .unwrap();

    let (_, dto, raw) = send_for(
        &state,
        "legacytoken",
        "GET",
        "/api/v1/company/composio",
        None,
    )
    .await;
    assert_eq!(dto["credentialSource"], "static", "{raw}");

    let (code, _, raw) = send_for(
        &state,
        "legacytoken",
        "PUT",
        "/api/v1/company/composio/token",
        Some(json!({ "token": "th-not-a-real-key-2" })),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{raw}");

    assert_eq!(
        read_slot(&runtime, TINYHUMANS_KEY_KEY).await.as_deref(),
        Some("th-not-a-real-key-2")
    );
    assert_eq!(
        read_slot(&runtime, LEGACY_TOKEN_KEY).await.as_deref(),
        Some("th-not-a-real-key-2")
    );
    assert!(
        !raw.contains("th-not-a-real-key"),
        "the PUT leaked the token: {raw}"
    );

    // Guarded while the company is on the managed route
    // (in-use-guards.md §2); this test is about the mirror write, not
    // the guard, so it confirms.
    let (code, _, raw) = send_for(
        &state,
        "legacytoken",
        "PUT",
        "/api/v1/company/composio/token",
        Some(json!({ "token": "", "confirmInUse": true })),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{raw}");
    assert_eq!(
        read_slot(&runtime, TINYHUMANS_KEY_KEY).await.as_deref(),
        Some("")
    );
    assert_eq!(
        read_slot(&runtime, LEGACY_TOKEN_KEY).await.as_deref(),
        Some("")
    );
}

/// Raw slot contents for a company's runtime, blank-or-absent collapsed to
/// `None` only when truly absent (a stored `""` reads back as `Some("")`).
async fn read_slot(runtime: &super::CompanyRuntime, key: &'static str) -> Option<String> {
    runtime
        .secrets()
        .get(runtime.id(), key)
        .await
        .unwrap()
        .map(|crate::ports::types::SecretValue(v)| v)
}

/// A legacy-only BYOK key still passes the check route, and a subsequent
/// `PUT` mirrors the rotated value to both addresses.
#[tokio::test]
async fn the_api_key_test_route_reads_a_legacy_byok_key() {
    use crate::company::composio::{BYOK_KEY_KEY, LEGACY_API_KEY_KEY, MODE_KEY};

    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "legacybyok", GRANTED).await;
    let runtime = runtime_of(&state, "legacybyok");
    runtime
        .secrets()
        .set(
            runtime.id(),
            MODE_KEY,
            crate::ports::types::SecretValue(crate::company::composio::BYOK_MODE.to_string()),
        )
        .await
        .unwrap();
    runtime
        .secrets()
        .set(
            runtime.id(),
            LEGACY_API_KEY_KEY,
            crate::ports::types::SecretValue("ak-not-a-real-key".into()),
        )
        .await
        .unwrap();
    super::probe_override::set("legacybyok", Ok(()));

    let (code, body, raw) = send_for(
        &state,
        "legacybyok",
        "POST",
        "/api/v1/company/composio/api-key/test",
        None,
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{raw}");
    assert_eq!(body["ok"], true, "{body}");

    let (code, _, raw) = send_for(
        &state,
        "legacybyok",
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": "ak-not-a-real-key-2" })),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{raw}");

    assert_eq!(
        read_slot(&runtime, BYOK_KEY_KEY).await.as_deref(),
        Some("ak-not-a-real-key-2")
    );
    assert_eq!(
        read_slot(&runtime, LEGACY_API_KEY_KEY).await.as_deref(),
        Some("ak-not-a-real-key-2")
    );
    assert_eq!(
        read_slot(&runtime, MODE_KEY).await.as_deref(),
        Some(crate::company::composio::BYOK_MODE)
    );
    assert!(
        !raw.contains("ak-not-a-real-key"),
        "the PUT leaked the key: {raw}"
    );
}

/// Spending the company's Composio credential against a third party is a
/// decision taken on the company's behalf, so a member cannot trigger it —
/// the same boundary the writes on this surface carry (issue #403).
#[tokio::test]
async fn a_member_cannot_test_the_company_s_composio_key() {
    let home_dir = home();
    let state = state_with_manifest(home_dir.path(), GRANTED).await;
    let member = crate::server::test_support::seed_session(
        &state,
        "acme",
        crate::ports::UserRole::Member,
    )
    .await;
    let (code, body, raw) = send_as(
        &state,
        "POST",
        "/api/v1/company/composio/api-key/test",
        None,
        Auth::Cookie(member),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN, "{raw}");
    assert_eq!(body["code"], "forbidden", "{body}");
}

/// `POST …/composio/tinyhumans/key/from-account` copies the account key
/// into the Composio slot, evicts the cached catalog and reports the one
/// slot it touched (keys rework #2306, slice 4c).
#[tokio::test]
async fn the_from_account_route_fills_the_composio_key_and_reports_one_slot() {
    use crate::company::composio::TINYHUMANS_KEY_KEY;

    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "reuse-composio", GRANTED).await;
    let runtime = runtime_of(&state, "reuse-composio");
    runtime
        .secrets()
        .set(
            runtime.id(),
            crate::company::company_key::KEY_KEY,
            crate::ports::types::SecretValue("th-not-a-real-account-key".into()),
        )
        .await
        .unwrap();

    let (code, body, raw) = send_for(
        &state,
        "reuse-composio",
        "POST",
        "/api/v1/company/composio/tinyhumans/key/from-account",
        None,
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{raw}");
    assert_eq!(
        read_slot(&runtime, TINYHUMANS_KEY_KEY).await.as_deref(),
        Some("th-not-a-real-account-key")
    );
    let slots = body["slots"].as_array().expect("slots array");
    assert_eq!(slots.len(), 1, "{body}");
    assert_eq!(slots[0]["slot"], "composio", "{body}");
    assert_eq!(slots[0]["outcome"], "filled", "{body}");
    assert!(
        body["note"]
            .as_str()
            .is_some_and(|n| n.contains("Composio now uses your account key")),
        "{body}"
    );
    assert!(
        !raw.contains("th-not-a-real-account-key"),
        "the response leaked the key: {raw}"
    );
}

/// P3-3 (keys rework #2306 review): a `Filled` copy is journaled — the
/// counterpart to `copying_an_already_current_key_does_not_journal`
/// below, which proves the opposite for `Kept`.
#[tokio::test]
async fn copying_a_new_composio_key_journals_the_fill() {
    use crate::ports::types::{CompanyEvent, CompanyId, EventSeq};

    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "reuse-journal-fill", GRANTED).await;
    let runtime = runtime_of(&state, "reuse-journal-fill");
    runtime
        .secrets()
        .set(
            runtime.id(),
            crate::company::company_key::KEY_KEY,
            crate::ports::types::SecretValue("th-not-a-real-account-key".into()),
        )
        .await
        .unwrap();

    let (code, _, raw) = send_for(
        &state,
        "reuse-journal-fill",
        "POST",
        "/api/v1/company/composio/tinyhumans/key/from-account",
        None,
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{raw}");

    let events = runtime
        .events()
        .read_from(&CompanyId::new("reuse-journal-fill"), EventSeq::new(0), 100)
        .await
        .unwrap();
    let journaled = events.iter().any(|stored| {
        matches!(
            &stored.event,
            CompanyEvent::ToolAccessChanged { change, .. }
                if change == "company_key_composio_filled"
        )
    });
    assert!(journaled, "a Filled copy must be journaled: {events:?}");
}

/// P3-3 (keys rework #2306 review): a copy whose outcome is
/// `Kept(AlreadyCurrent)` — the Composio slot already held exactly the
/// account key's value — changes no stored state, so it must not add a
/// journal entry either, matching 4a §3.5's "no entry for kept" rule.
#[tokio::test]
async fn copying_an_already_current_key_does_not_journal() {
    use crate::ports::types::{CompanyEvent, CompanyId, EventSeq};

    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "reuse-journal-kept", GRANTED).await;
    let runtime = runtime_of(&state, "reuse-journal-kept");
    const ACCOUNT_KEY: &str = "th-not-a-real-account-key";
    runtime
        .secrets()
        .set(
            runtime.id(),
            crate::company::company_key::KEY_KEY,
            crate::ports::types::SecretValue(ACCOUNT_KEY.into()),
        )
        .await
        .unwrap();
    // The Composio slot already agrees with the account key, so the copy
    // is a no-op (`Kept(AlreadyCurrent)`), not a fill.
    runtime
        .secrets()
        .set(
            runtime.id(),
            crate::company::composio::TINYHUMANS_KEY_KEY,
            crate::ports::types::SecretValue(ACCOUNT_KEY.into()),
        )
        .await
        .unwrap();

    let (code, body, raw) = send_for(
        &state,
        "reuse-journal-kept",
        "POST",
        "/api/v1/company/composio/tinyhumans/key/from-account",
        None,
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{raw}");
    assert_eq!(body["slots"][0]["outcome"], "kept", "{body}");

    let events = runtime
        .events()
        .read_from(&CompanyId::new("reuse-journal-kept"), EventSeq::new(0), 100)
        .await
        .unwrap();
    let journaled = events.iter().any(|stored| {
        matches!(
            &stored.event,
            CompanyEvent::ToolAccessChanged { change, .. }
                if change == "company_key_composio_filled"
        )
    });
    assert!(
        !journaled,
        "a Kept(AlreadyCurrent) copy changed nothing and must not journal: {events:?}"
    );
}

/// Copying with no account key on file is refused before any write.
#[tokio::test]
async fn the_from_account_route_refuses_without_an_account_key() {
    use crate::company::composio::TINYHUMANS_KEY_KEY;

    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "reuse-no-key", GRANTED).await;
    let runtime = runtime_of(&state, "reuse-no-key");

    let (code, body, raw) = send_for(
        &state,
        "reuse-no-key",
        "POST",
        "/api/v1/company/composio/tinyhumans/key/from-account",
        None,
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "{raw}");
    assert_eq!(body["code"], "invalid_request", "{body}");
    assert_eq!(
        read_slot(&runtime, TINYHUMANS_KEY_KEY).await,
        None,
        "nothing is written on a refusal"
    );
}

/// Whose account the company's Composio calls present is an admin's
/// decision, exactly as the token and API-key writes on this surface are
/// (issue #403).
#[tokio::test]
async fn a_member_cannot_copy_the_account_key_to_composio() {
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "reuse-member", GRANTED).await;
    let runtime = runtime_of(&state, "reuse-member");
    runtime
        .secrets()
        .set(
            runtime.id(),
            crate::company::company_key::KEY_KEY,
            crate::ports::types::SecretValue("th-not-a-real-account-key".into()),
        )
        .await
        .unwrap();
    let member = crate::server::test_support::seed_session(
        &state,
        "reuse-member",
        crate::ports::UserRole::Member,
    )
    .await;

    let (code, body, raw) = send_as(
        &state,
        "POST",
        "/api/v1/company/composio/tinyhumans/key/from-account",
        None,
        Auth::Cookie(member),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN, "{raw}");
    assert_eq!(body["code"], "forbidden", "{body}");
}

/// A build with no Composio client cannot check a key — and treats that as
/// **un-probeable**, not as a rejected credential.
///
/// No override: this drives the real `probe_transport`, which in this build
/// is the honest "not compiled in" error. Refusing the write here would make
/// BYOK unconfigurable on the default build; classifying an absent client as
/// `auth` would throw away a key nothing ever looked at.
#[cfg(not(feature = "composio"))]
#[tokio::test]
async fn a_build_without_the_composio_client_stores_the_key_and_says_it_could_not_check() {
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "probenobuild", GRANTED).await;

    let (code, resp, raw) = send_for(
        &state,
        "probenobuild",
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": "ak_not_a_real_key_0123456789" })),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{raw}");
    assert_eq!(resp["status"]["mode"], "byok", "{resp}");
    assert_eq!(resp["probeClass"], "unknown", "{resp}");
}

/// Whose Composio account a company acts through — and therefore who pays
/// for the calls — is an admin's decision, exactly as the backend token is.
#[tokio::test]
async fn a_member_cannot_bring_its_own_composio_account() {
    let home_dir = home();
    let state = state_with_manifest(home_dir.path(), GRANTED).await;
    let member = crate::server::test_support::seed_session(
        &state,
        "acme",
        crate::ports::UserRole::Member,
    )
    .await;

    let (status, body, raw) = send_as(
        &state,
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": "ak_live" })),
        Auth::Cookie(member),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{raw}");
    assert_eq!(body["code"], "forbidden", "{body}");

    // The refusal is real: the route did not move.
    let (_, dto, _) = send(&state, "GET", "/api/v1/company/composio", None).await;
    assert_eq!(
        dto["mode"], "managed",
        "a refused write stored nothing: {dto}"
    );
}

/// A managed-route token stored by a company that is **on BYOK** must not
/// claim agents have started using it.
///
/// The state is newly reachable: a BYOK company whose managed chain resolves
/// to nothing cannot be offered "Use this" — that would be switching into an
/// outage — so the console offers the token first and the switch second, and
/// that is the only order that works. `SWITCH_NOTE` at the end of the first
/// step would report the second as already done, while `resolve_access` is
/// still reading the BYOK key.
#[tokio::test]
async fn a_token_for_the_route_a_company_is_not_on_does_not_claim_effect() {
    use crate::company::composio::store_api_key;

    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "inactivetoken", GRANTED).await;
    let runtime = runtime_of(&state, "inactivetoken");
    // Straight to the store rather than through the route: the route probes
    // the draft key, and this test is about the note, not the probe.
    store_api_key(
        runtime.id(),
        runtime.secrets().as_ref(),
        "ak_not_a_real_key_0123456789",
    )
    .await
    .unwrap();

    let (_, resp, _) = send_for(
        &state,
        "inactivetoken",
        "PUT",
        "/api/v1/company/composio/token",
        Some(json!({ "token": TOKEN })),
    )
    .await;
    let note = resp["note"].as_str().expect("a note").to_string();
    assert!(
        !note.contains("next turn"),
        "the company is on BYOK, so agents do not pick this up next turn: {note}"
    );
    assert!(
        note.contains("still on its own Composio account"),
        "the note says why the token is not in effect: {note}"
    );
}

/// The note on a write names the operation (issue #1471): a set tells the
/// operator a new token is live, a clear must not claim one exists — the
/// effective credential after a clear is whatever tier remains, which may
/// be nothing at all.
#[tokio::test]
async fn the_clear_note_does_not_claim_a_new_token() {
    let home_dir = home();
    let state = state_with_manifest(home_dir.path(), GRANTED).await;

    let (_, resp, _) = send(
        &state,
        "PUT",
        "/api/v1/company/composio/token",
        Some(json!({ "token": TOKEN })),
    )
    .await;
    let set_note = resp["note"].as_str().expect("a note").to_string();
    assert!(
        set_note.contains("new Composio token"),
        "a set announces the new token: {set_note}"
    );

    // Guarded while the company is on the managed route
    // (in-use-guards.md §2); this test is about the note text, not the
    // guard, so it confirms.
    let (_, resp, _) = send(
        &state,
        "PUT",
        "/api/v1/company/composio/token",
        Some(json!({ "token": "", "confirmInUse": true })),
    )
    .await;
    let clear_note = resp["note"].as_str().expect("a note").to_string();
    assert_ne!(
        clear_note, set_note,
        "set and clear are told apart: {clear_note}"
    );
    assert!(
        !clear_note.contains("new Composio token"),
        "a clear must not invent a token that no longer exists: {clear_note}"
    );
    assert!(
        clear_note.contains("cleared"),
        "the clear names what it did: {clear_note}"
    );
}

/// The credential tier alone.
///
/// The matrix below is about credential *precedence*; which route a company
/// takes is a separate question, asserted separately. Projecting here keeps
/// each test about one of them.
async fn credential_source_for(
    runtime: &CompanyRuntime,
    token_source: Option<std::sync::Arc<TinyhumansTokenSource>>,
) -> Result<CredentialSource, ApiError> {
    Ok(access_for(runtime, token_source).await?.1)
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
    let member = crate::server::test_support::seed_session(
        &state,
        "acme",
        crate::ports::UserRole::Member,
    )
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
    let member = crate::server::test_support::seed_session(
        &state,
        "acme",
        crate::ports::UserRole::Member,
    )
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

/// The ops tests that are decidable only in a build carrying `composio`.
///
/// Gathered under one module so a CI lane can *name* them. A feature-gated
/// test's default fate in this repo is "compiled by `Check
/// (--all-features)`, executed by nothing" (issue #770), and the composio
/// lane has to select by filter rather than run this whole module: with the
/// feature on, `an_admin_is_unaffected` dials `api.tinyhumans.ai` for real
/// (issue #801). One filter on this module runs every gated test here and
/// none of that, and a gated test added later is picked up by joining the
/// module rather than by remembering to edit `ci.yml`.
#[cfg(feature = "composio")]
mod gated_tests {
    use super::*;
    use crate::server::ops::composio::{drop_dangling_defaults, group_by_toolkit};

    /// Issue #404: the per-toolkit shape the tile grid reads is a fold over the
    /// per-connection rows, and the fold must not lose an account or flip a
    /// boolean. Pure — no live backend needed.
    #[test]
    fn grouping_keeps_every_account_and_ors_their_connected_state() {
        use crate::harness::composio::ComposioConnectionRow;

        let row = |id: &str, toolkit: &str, connected: bool, account: Option<&str>| {
            ComposioConnectionRow {
                id: id.to_string(),
                toolkit: toolkit.to_string(),
                status: if connected { "ACTIVE" } else { "INITIATED" }.to_string(),
                connected,
                created_at: None,
                account: account.map(str::to_string),
            }
        };

        let out = group_by_toolkit(
            vec![
                row("c1", "gmail", false, Some("a@acme.test")),
                row("c2", "gmail", true, Some("b@acme.test")),
                row("c3", "slack", false, None),
            ],
            &Default::default(),
        );

        assert_eq!(out.len(), 2, "one entry per toolkit");
        assert_eq!(out[0].toolkit, "gmail");
        assert!(
            out[0].connected,
            "a toolkit is connected when ANY of its accounts is — the second row \
             here, which a first-row-wins fold would have missed"
        );
        assert_eq!(
            out[0]
                .accounts
                .iter()
                .map(|a| (a.id.as_str(), a.connected))
                .collect::<Vec<_>>(),
            vec![("c1", false), ("c2", true)],
            "both accounts survive, in the order the rows arrived"
        );
        assert_eq!(out[1].toolkit, "slack");
        assert!(!out[1].connected, "no active account, so not connected");
        assert_eq!(out[1].accounts.len(), 1);

        // Nothing pinned: nothing is marked, and no default is reported. This
        // is the shape #819 asks the console to render honestly, and it stays
        // the shape until somebody chooses.
        assert!(out.iter().all(|dto| dto.default_connection_id.is_none()));
        assert!(
            out.iter()
                .flat_map(|dto| dto.accounts.iter())
                .all(|account| !account.is_default),
            "an unchosen account is never marked as the default"
        );
    }

    /// Issue #820: once a company has chosen, the choice is reported on the
    /// toolkit **and** marked on the one account it names — the console needs
    /// both to draw a list with one row marked and the rest offering to become
    /// it.
    #[test]
    fn grouping_marks_the_chosen_account_and_only_that_one() {
        use crate::harness::composio::ComposioConnectionRow;

        let row = |id: &str, toolkit: &str| ComposioConnectionRow {
            id: id.to_string(),
            toolkit: toolkit.to_string(),
            status: "ACTIVE".to_string(),
            connected: true,
            created_at: None,
            account: None,
        };
        let defaults: crate::company::composio::ComposioDefaults =
            [("gmail".to_string(), "c2".to_string())]
                .into_iter()
                .collect();

        let out = group_by_toolkit(
            vec![row("c1", "gmail"), row("c2", "gmail"), row("c3", "slack")],
            &defaults,
        );

        assert_eq!(out[0].default_connection_id.as_deref(), Some("c2"));
        assert_eq!(
            out[0]
                .accounts
                .iter()
                .map(|a| (a.id.as_str(), a.is_default))
                .collect::<Vec<_>>(),
            vec![("c1", false), ("c2", true)],
            "exactly one account carries the mark"
        );
        assert!(
            out[1].default_connection_id.is_none(),
            "a choice made for gmail says nothing about slack: {:?}",
            out[1].default_connection_id
        );
    }

    /// Issue #820: an account revoked **at Composio** — not through this console,
    /// so nothing here saw the disconnect — leaves a choice naming a connection
    /// that no longer exists. That choice is not merely stale: it is sent on the
    /// next `composio_execute` and refused, so the toolkit stops working for
    /// every agent for a reason nothing on screen explains. The read the console
    /// polls repairs it.
    ///
    /// Driven through the handler's own helper rather than the route, because
    /// the route needs a live Composio backend and the decision under test is
    /// the one made *after* it answers. What it must not do is as load-bearing
    /// as what it must: a live choice is untouched, and a toolkit whose chosen
    /// account is gone falls back to "Composio picks" rather than being
    /// re-pointed at a sibling account nobody chose.
    #[tokio::test]
    async fn the_connections_read_forgets_a_choice_composio_no_longer_lists() {
        use crate::company::composio::{load_defaults, set_default};
        use crate::harness::composio::ComposioConnectionRow;

        let home_dir = home();
        let state = state_with_manifest(home_dir.path(), GRANTED).await;
        let runtime = runtime_of(&state, "acme");
        let (id, secrets) = (runtime.id(), runtime.secrets());

        // gmail names an account still live; slack names one revoked since.
        for (toolkit, connection) in [("gmail", "c1"), ("slack", "c_revoked")] {
            set_default(id, secrets.as_ref(), toolkit, connection)
                .await
                .unwrap();
        }

        let row = |id: &str, toolkit: &str| ComposioConnectionRow {
            id: id.to_string(),
            toolkit: toolkit.to_string(),
            status: "ACTIVE".to_string(),
            connected: true,
            created_at: None,
            account: None,
        };
        // The company still holds a slack account — just not the chosen one.
        let rows = vec![row("c1", "gmail"), row("c9", "slack")];

        let left = drop_dangling_defaults(
            runtime.as_ref(),
            &rows,
            load_defaults(id, secrets.as_ref()).await.unwrap(),
        )
        .await
        .expect("the cleanup completes");

        assert_eq!(
            left.get("gmail").map(String::as_str),
            Some("c1"),
            "a choice naming a live account is untouched"
        );
        assert!(
            !left.contains_key("slack"),
            "the choice naming a revoked account is dropped: {left:?}"
        );
        assert_eq!(
            load_defaults(id, secrets.as_ref()).await.unwrap(),
            left,
            "the repair is stored, not merely reflected in this one response — \
             otherwise the next agent turn still sends the dead id"
        );

        let out = group_by_toolkit(rows, &left);
        assert_eq!(out[1].toolkit, "slack");
        assert!(
            out[1].default_connection_id.is_none()
                && out[1].accounts.iter().all(|account| !account.is_default),
            "with its choice gone slack is unchosen again — the surviving account \
             is not silently promoted into a decision nobody made: {:?}",
            out[1]
        );
        assert_eq!(out[0].default_connection_id.as_deref(), Some("c1"));
    }

    /// A loopback backend for the `DELETE …/composio/connections/{id}` route
    /// test: `conn-1` is the only account this company knows about, and the
    /// backend's own delete either succeeds or fails depending on
    /// `fail_delete`, so the same mock drives both the 404 and the 502 arm of
    /// the mapping in [`super::super::disconnect_impl`].
    async fn spawn_connections_backend(
        fail_delete: bool,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use axum::extract::Path;
        use axum::response::IntoResponse;

        let deletes: std::sync::Arc<std::sync::Mutex<Vec<String>>> = Default::default();
        let recorded = deletes.clone();
        let app = Router::new()
            .route(
                "/agent-integrations/composio/connections",
                axum::routing::get(|| async {
                    Json(json!({
                        "success": true,
                        "data": { "connections": [
                            { "id": "conn-1", "toolkit": "gmail", "status": "ACTIVE" }
                        ] }
                    }))
                }),
            )
            .route(
                "/agent-integrations/composio/connections/{id}",
                axum::routing::delete(move |Path(id): Path<String>| {
                    let recorded = recorded.clone();
                    async move {
                        recorded.lock().unwrap().push(id);
                        if fail_delete {
                            StatusCode::INTERNAL_SERVER_ERROR.into_response()
                        } else {
                            Json(json!({ "success": true, "data": { "deleted": true } }))
                                .into_response()
                        }
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{addr}"), deletes)
    }

    /// The documented split in [`super::super::disconnect_impl`]: an id this
    /// company's own read cannot see is a `404`, never a `502`, because it is a
    /// claim about the company's accounts rather than about the provider. Route
    /// tests above only reach the "no client at all" `409` arm; this drives the
    /// real mapping with a loopback backend standing in for Composio.
    #[tokio::test]
    async fn disconnect_maps_an_unknown_id_to_404_not_502() {
        let (backend, deletes) = spawn_connections_backend(false).await;
        let env = crate::test_support::EnvVarGuard::capture(&[
            crate::company::composio::TINYHUMANS_API_URL_ENV,
        ]);
        env.set(crate::company::composio::TINYHUMANS_API_URL_ENV, &backend);

        let home_dir = home();
        let state = state_with_manifest(home_dir.path(), GRANTED).await;
        send(
            &state,
            "PUT",
            "/api/v1/company/composio/token",
            Some(json!({ "token": TOKEN })),
        )
        .await;

        let (status, body, raw) = send(
            &state,
            "DELETE",
            "/api/v1/company/composio/connections/conn-does-not-exist",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{raw}");
        assert_eq!(body["code"], "not_found", "{body}");
        assert!(
            deletes.lock().unwrap().is_empty(),
            "an id outside the company's visible connections must never reach a delete call"
        );
    }

    /// The other arm of the same split: an id the company DOES hold, but the
    /// backend refuses to delete, is a `502` naming the provider failure —
    /// never a `404`, which would tell the operator to stop looking for an
    /// account that is right there in the list.
    #[tokio::test]
    async fn disconnect_maps_a_backend_failure_to_502_not_404() {
        let (backend, deletes) = spawn_connections_backend(true).await;
        let env = crate::test_support::EnvVarGuard::capture(&[
            crate::company::composio::TINYHUMANS_API_URL_ENV,
        ]);
        env.set(crate::company::composio::TINYHUMANS_API_URL_ENV, &backend);

        let home_dir = home();
        let state = state_with_manifest(home_dir.path(), GRANTED).await;
        send(
            &state,
            "PUT",
            "/api/v1/company/composio/token",
            Some(json!({ "token": TOKEN })),
        )
        .await;

        let (status, body, raw) = send(
            &state,
            "DELETE",
            "/api/v1/company/composio/connections/conn-1",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{raw}");
        assert_eq!(body["code"], "tinyhumans_composio_disconnect", "{body}");
        assert_eq!(
            deletes.lock().unwrap().as_slice(),
            ["conn-1"],
            "a known id must actually reach the backend's delete before failing"
        );
    }
}

/// The choice plane is wired on the same terms as the rest of the OAuth
/// plane: in the route table whatever the build, and a `409` — never a
/// `404` — when there is no usable client, since "no such connection" is a
/// claim about this company's accounts that a build without Composio cannot
/// make.
#[tokio::test]
async fn set_default_route_conflicts_without_build_or_token() {
    let home_dir = home();
    let state = state_with_manifest(home_dir.path(), GRANTED).await;

    let (status, body, raw) = send(
        &state,
        "PUT",
        "/api/v1/company/composio/connections/conn-1/default",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{raw}");
    #[cfg(not(feature = "composio"))]
    assert_eq!(body["code"], "not_in_build", "{body}");
    #[cfg(feature = "composio")]
    assert_eq!(body["code"], "not_configured", "{body}");
}

/// Clearing is the deliberate exception: it takes no upstream call, so it
/// works in a build without Composio and — the case that matters — when the
/// provider is unreachable or the account is already gone. A clear that
/// needed the network would refuse exactly when it is most needed.
#[tokio::test]
async fn clearing_a_choice_needs_no_client_and_is_idempotent() {
    let home_dir = home();
    let state = state_with_manifest(home_dir.path(), GRANTED).await;

    let (status, body, raw) = send(
        &state,
        "DELETE",
        "/api/v1/company/composio/connections/conn-1/default",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert!(
        body["note"]
            .as_str()
            .unwrap_or_default()
            .contains("nothing changed"),
        "clearing what was never chosen says so rather than claiming a change: {body}"
    );

    // Now with something stored, the same call reports the real change and
    // leaves nothing behind.
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    crate::company::composio::set_default(
        runtime.id(),
        runtime.secrets().as_ref(),
        "gmail",
        "conn-1",
    )
    .await
    .unwrap();

    let (status, body, raw) = send(
        &state,
        "DELETE",
        "/api/v1/company/composio/connections/conn-1/default",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert!(
        body["note"]
            .as_str()
            .unwrap_or_default()
            .contains("Cleared"),
        "{body}"
    );
    assert!(
        crate::company::composio::load_defaults(runtime.id(), runtime.secrets().as_ref())
            .await
            .unwrap()
            .is_empty()
    );
}

/// Choosing the account a company acts as is an admin's decision, like
/// connecting and disconnecting one: every agent in the company acts
/// through the single answer, so it is not a per-operator preference.
#[tokio::test]
async fn a_member_cannot_choose_the_account_the_company_acts_as() {
    let home_dir = home();
    let state = state_with_manifest(home_dir.path(), GRANTED).await;
    let member = crate::server::test_support::seed_session(
        &state,
        "acme",
        crate::ports::UserRole::Member,
    )
    .await;

    for method in ["PUT", "DELETE"] {
        let (status, body, raw) = send_as(
            &state,
            method,
            "/api/v1/company/composio/connections/conn-1/default",
            None,
            Auth::Cookie(member.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method}: {raw}");
        assert_eq!(body["code"], "forbidden", "{method}: {body}");
    }

    // And the refusal is real: nothing was stored by either attempt.
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    assert!(
        crate::company::composio::load_defaults(runtime.id(), runtime.secrets().as_ref())
            .await
            .unwrap()
            .is_empty()
    );
}

// ── in-use guards (#2306): confirmInUse gates a clear/switch ──────
//
// The signal is `composio/mode` (`src/company/composio.rs::MODE_KEY`),
// per in-use-guards.md §2's surfaces table: clearing
// `composio/tinyhumans/key` reports `surfaces: ["composio"]` iff the
// mode currently reads `"managed"`; clearing/switching
// `composio/byok/key` reports it iff the mode currently reads `"byok"`.
// A company connection pin (`composio/defaults`) is no longer the
// signal, so these tests drive the guard by setting the mode — directly,
// or through `store_token`/`store_api_key`, which are what set it in
// practice.

/// A managed-token clear is refused with `409 in_use` while the company
/// is still on the managed route (the default with nothing else
/// stored), and nothing is written.
#[tokio::test]
async fn clearing_the_managed_token_while_mode_is_managed_is_refused_without_confirmation() {
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "clearguard", GRANTED).await;
    let runtime = runtime_of(&state, "clearguard");
    crate::company::composio::store_token(runtime.id(), runtime.secrets().as_ref(), TOKEN)
        .await
        .unwrap();
    assert_eq!(
        crate::company::composio::load_mode(runtime.id(), runtime.secrets().as_ref())
            .await
            .unwrap(),
        crate::company::composio::ComposioMode::Managed,
        "managed is the default mode with nothing else stored"
    );

    let (status, body, raw) = send_for(
        &state,
        "clearguard",
        "PUT",
        "/api/v1/company/composio/token",
        Some(json!({ "token": "" })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{raw}");
    assert_eq!(body["code"], "in_use", "{body}");
    assert_eq!(
        body["error"], "Composio's key is used by Composio.",
        "the message follows in-use-guards.md §2's fixed sentence: {body}"
    );
    assert_eq!(
        body["usedBy"],
        json!({ "surfaces": ["composio"] }),
        "{body}"
    );

    // Refused means nothing changed.
    assert_eq!(
        crate::company::composio::load_tinyhumans_key(runtime.id(), runtime.secrets().as_ref())
            .await
            .unwrap()
            .as_deref(),
        Some(TOKEN),
        "a refused clear must not have written anything"
    );
}

/// The same clear, with `confirmInUse: true`, proceeds and echoes the
/// `usedBy` it would have refused with (in-use-guards.md §3).
#[tokio::test]
async fn a_confirmed_clear_of_the_managed_token_succeeds_and_echoes_used_by() {
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "clearconfirmed", GRANTED).await;
    let runtime = runtime_of(&state, "clearconfirmed");
    crate::company::composio::store_token(runtime.id(), runtime.secrets().as_ref(), TOKEN)
        .await
        .unwrap();

    let (status, body, raw) = send_for(
        &state,
        "clearconfirmed",
        "PUT",
        "/api/v1/company/composio/token",
        Some(json!({ "token": "", "confirmInUse": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(
        body["usedBy"],
        json!({ "surfaces": ["composio"] }),
        "{body}"
    );
    assert_eq!(
        crate::company::composio::load_tinyhumans_key(runtime.id(), runtime.secrets().as_ref())
            .await
            .unwrap(),
        None,
        "the confirmed clear must have landed"
    );
}

/// A managed-token clear needs no confirmation while the company is on
/// BYOK: `composio/mode` no longer selects the managed slot, so this
/// clear is not touching what any live call resolves through, and the
/// response carries no `usedBy` — omitted, not null or empty.
#[tokio::test]
async fn clearing_the_managed_token_while_mode_is_byok_needs_no_confirmation() {
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "clearunguarded", GRANTED).await;
    let runtime = runtime_of(&state, "clearunguarded");
    crate::company::composio::store_token(runtime.id(), runtime.secrets().as_ref(), TOKEN)
        .await
        .unwrap();
    crate::company::composio::store_api_key(
        runtime.id(),
        runtime.secrets().as_ref(),
        "ak_not_a_real_key_0123456789",
    )
    .await
    .unwrap();

    let (status, body, raw) = send_for(
        &state,
        "clearunguarded",
        "PUT",
        "/api/v1/company/composio/token",
        Some(json!({ "token": "" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert!(body.get("usedBy").is_none(), "{body}");
    assert_eq!(
        crate::company::composio::load_tinyhumans_key(runtime.id(), runtime.secrets().as_ref())
            .await
            .unwrap(),
        None
    );
}

/// Setting or rotating a non-empty managed token is never guarded — only
/// a clear can strand anything the mode currently resolves through.
#[tokio::test]
async fn setting_a_managed_token_is_never_guarded_even_while_mode_is_managed() {
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "setnoguard", GRANTED).await;

    let (status, body, raw) = send_for(
        &state,
        "setnoguard",
        "PUT",
        "/api/v1/company/composio/token",
        Some(json!({ "token": TOKEN })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert!(body.get("usedBy").is_none(), "a set is not guarded: {body}");
}

/// The first move onto BYOK needs no confirmation from Composio's own
/// guard: before the write `composio/mode` still reads `"managed"`, so
/// `composio/byok/key` is not the slot the mode currently selects, and
/// setting it strands nothing (in-use-guards.md §2 — the criterion is
/// the mode BEFORE the write, not the one the write is heading to). The
/// probe is forced clean so this does not depend on network access under
/// the `composio` feature.
#[tokio::test]
async fn switching_to_byok_while_mode_is_managed_needs_no_confirmation() {
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "byokswitchunguarded", GRANTED).await;
    let runtime = runtime_of(&state, "byokswitchunguarded");
    super::probe_override::set("byokswitchunguarded", Ok(()));

    let (status, body, raw) = send_for(
        &state,
        "byokswitchunguarded",
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": "ak_not_a_real_key_0123456789" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert!(body.get("usedBy").is_none(), "{body}");
    assert_eq!(
        crate::company::composio::load_mode(runtime.id(), runtime.secrets().as_ref())
            .await
            .unwrap(),
        crate::company::composio::ComposioMode::Byok
    );
}

/// Round-2 review, comment 4012457339 (keys rework #2306): `set_api_key`
/// writes `composio/mode`, the same fact `company_key::fan_out`'s own
/// in-use check reads to decide whether an unconfirmed account-key clear
/// may touch `composio/tinyhumans/key`. Before this fix the route took no
/// lock at all, so a confirmed mode switch landing here and a concurrent,
/// unconfirmed account-key clear could each act on the OTHER's pre-image
/// — the clear sees the old mode and decides the managed slot is
/// inactive, the switch then makes it active, and it is left keyless with
/// neither request ever having confirmed that outcome.
///
/// This proves the route now shares `company_key::fan_out`'s own
/// `slot_guard`: while a test holds that company's guard directly (the
/// same acquisition `fan_out` makes for the whole of an account-key
/// save), the real `PUT …/composio/api-key` handler must not complete —
/// it has to be waiting on the same lock, not racing past it.
#[tokio::test]
async fn set_api_key_blocks_while_the_account_keys_fan_out_lock_is_held() {
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "composio-lock-blocks", GRANTED).await;
    let runtime = runtime_of(&state, "composio-lock-blocks");
    super::probe_override::set("composio-lock-blocks", Ok(()));

    // The exact acquisition `company_key::fan_out` makes for the whole of
    // an account-key save.
    let held = crate::company::company_key::slot_guard(runtime.id()).await;

    let request = send_for(
        &state,
        "composio-lock-blocks",
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": "ak_not_a_real_key_0123456789" })),
    );
    tokio::pin!(request);
    tokio::select! {
        _ = &mut request => panic!(
            "set_api_key must not write composio/mode while the account-key \
             fan-out's own lock is held elsewhere"
        ),
        _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
    }

    drop(held);
    let (status, _body, raw) =
        tokio::time::timeout(std::time::Duration::from_millis(1000), request)
            .await
            .expect("set_api_key proceeds once the lock is released");
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(
        crate::company::composio::load_mode(runtime.id(), runtime.secrets().as_ref())
            .await
            .unwrap(),
        crate::company::composio::ComposioMode::Byok
    );
}

/// Giving the managed route back while BYOK is active IS refused without
/// confirmation: `composio/mode` already reads `"byok"`, which is
/// exactly the slot `composio/byok/key` being cleared belongs to.
#[tokio::test]
async fn switching_back_to_managed_while_mode_is_byok_is_refused_without_confirmation() {
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "managedswitchguard", GRANTED).await;
    let runtime = runtime_of(&state, "managedswitchguard");
    crate::company::composio::store_api_key(
        runtime.id(),
        runtime.secrets().as_ref(),
        "ak_not_a_real_key_0123456789",
    )
    .await
    .unwrap();

    let (status, body, raw) = send_for(
        &state,
        "managedswitchguard",
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": "" })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{raw}");
    assert_eq!(body["code"], "in_use", "{body}");
    assert_eq!(
        body["usedBy"],
        json!({ "surfaces": ["composio"] }),
        "{body}"
    );

    // Refused: the company is still on BYOK, key untouched.
    assert_eq!(
        crate::company::composio::load_mode(runtime.id(), runtime.secrets().as_ref())
            .await
            .unwrap(),
        crate::company::composio::ComposioMode::Byok
    );
}

/// The same switch back to managed, confirmed, lands and echoes
/// `usedBy` (in-use-guards.md §3).
#[tokio::test]
async fn a_confirmed_switch_back_to_managed_succeeds_and_echoes_used_by() {
    let home_dir = home();
    let state =
        state_with_manifest_id(home_dir.path(), "managedswitchconfirmed", GRANTED).await;
    let runtime = runtime_of(&state, "managedswitchconfirmed");
    crate::company::composio::store_api_key(
        runtime.id(),
        runtime.secrets().as_ref(),
        "ak_not_a_real_key_0123456789",
    )
    .await
    .unwrap();

    let (status, body, raw) = send_for(
        &state,
        "managedswitchconfirmed",
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": "", "confirmInUse": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(
        body["usedBy"],
        json!({ "surfaces": ["composio"] }),
        "{body}"
    );
    assert_eq!(
        crate::company::composio::load_mode(runtime.id(), runtime.secrets().as_ref())
            .await
            .unwrap(),
        crate::company::composio::ComposioMode::Managed
    );
}

/// Rotating a key while staying on the SAME route is never guarded — a
/// rotate is not a switch at all (in-use-guards.md §2), so the mode
/// match is never even consulted.
#[tokio::test]
async fn rotating_an_already_byok_key_is_never_guarded() {
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "byokrotate", GRANTED).await;
    let runtime = runtime_of(&state, "byokrotate");
    crate::company::composio::store_api_key(
        runtime.id(),
        runtime.secrets().as_ref(),
        "ak_not_a_real_key_0123456789",
    )
    .await
    .unwrap();
    super::probe_override::set("byokrotate", Ok(()));

    let (status, body, raw) = send_for(
        &state,
        "byokrotate",
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": "ak_not_a_real_key_9999999999" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert!(
        body.get("usedBy").is_none(),
        "a same-route rotation is not a switch: {body}"
    );
}
