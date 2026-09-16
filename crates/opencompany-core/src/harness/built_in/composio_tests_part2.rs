use super::*;

/// A BYOK company still resolves the managed chain — not to act through, but
/// to ask OpenHuman which providers to offer. The two credentials are kept
/// apart: the Composio key is what calls present, the managed bearer is only
/// ever the curated list's.
#[tokio::test]
async fn byok_keeps_the_managed_credential_for_the_curated_catalog_only() {
    use crate::company::company_key;
    use crate::company::composio::store_api_key;
    use crate::store::FsSecretStore;

    let dir = tempfile::Builder::new()
        .prefix("oc-composio-catalog-")
        .tempdir()
        .expect("tempdir");
    let secrets = FsSecretStore::new(dir.path());
    let company = CompanyId::new("acme");

    company_key::store_key(&company, &secrets, "th_company")
        .await
        .unwrap();
    store_api_key(&company, &secrets, "ak_live").await.unwrap();

    let config = TenantComposio::resolve(&company, &secrets, vec![], None, None)
        .await
        .expect("a BYOK company has a config");

    assert_eq!(config.mode(), ComposioMode::Byok);
    assert_eq!(
        config.current_token().await.unwrap().as_deref(),
        Some("ak_live"),
        "calls present the company's own Composio key"
    );
    assert_eq!(
        config.catalog_token().await.unwrap().as_deref(),
        Some("th_company"),
        "the curated list is fetched with the managed credential, not the Composio key"
    );
}

/// With no managed tier at all — a standalone host carrying no TinyHumans
/// identity — there is no curated list to fetch, and the config says so
/// rather than presenting the Composio key to the OpenHuman backend.
#[tokio::test]
async fn byok_without_a_managed_tier_has_no_curated_catalog_credential() {
    use crate::company::composio::store_api_key;
    use crate::store::FsSecretStore;

    let dir = tempfile::Builder::new()
        .prefix("oc-composio-standalone-")
        .tempdir()
        .expect("tempdir");
    let secrets = FsSecretStore::new(dir.path());
    let company = CompanyId::new("acme");
    store_api_key(&company, &secrets, "ak_live").await.unwrap();

    let config = TenantComposio::resolve(&company, &secrets, vec![], None, None)
        .await
        .expect("a BYOK company has a config");
    assert_eq!(
        config.current_token().await.unwrap().as_deref(),
        Some("ak_live")
    );
    assert!(
        config.catalog_token().await.unwrap().is_none(),
        "no managed tier means no curated list — never the Composio key standing in for one"
    );
}

/// The roster path honours the stored route: a company that brought its own
/// Composio account resolves to a BYOK config carrying that key, and one
/// that selected BYOK without storing a key resolves to **no tools** rather
/// than to the platform identity standing in for it.
#[tokio::test]
async fn resolve_follows_the_stored_route() {
    use crate::company::composio::{BYOK_MODE, MODE_KEY, store_api_key};
    use crate::store::FsSecretStore;

    let dir = tempfile::Builder::new()
        .prefix("oc-composio-byok-")
        .tempdir()
        .expect("tempdir");
    let secrets = FsSecretStore::new(dir.path());
    let company = CompanyId::new("acme");
    store_api_key(&company, &secrets, "ak_live").await.unwrap();

    let config = TenantComposio::resolve(&company, &secrets, vec![], None, None)
        .await
        .expect("a BYOK company has a config");
    assert_eq!(config.mode(), ComposioMode::Byok);
    assert_eq!(
        config.current_token().await.unwrap().as_deref(),
        Some("ak_live")
    );

    // BYOK selected with nothing stored: fail closed.
    let bare = CompanyId::new("bare");
    secrets
        .set(&bare, MODE_KEY, SecretValue(BYOK_MODE.into()))
        .await
        .unwrap();
    assert!(
        TenantComposio::resolve(&bare, &secrets, vec![], None, None)
            .await
            .is_none(),
        "an operator who asked for their own account must never silently get the platform's"
    );
}
}

/// The console-facing ops helpers ([`authorize_connect_url`],
/// [`list_connection_states`]) over a mock Composio backend: proves the connect
/// URL is surfaced, the allowlist is enforced before any network call, and
/// connection rows aggregate to per-toolkit `connected` state filtered to the
/// tenant grant.
#[cfg(all(test, feature = "composio"))]
mod ops_helper_tests {
use super::*;

#[tokio::test]
async fn authorize_returns_hosted_connect_url() {
    let url = spawn_backend().await;
    let out = authorize_connect_url(&config(&url, vec!["gmail".into()]), "gmail")
        .await
        .expect("authorize returns a connect URL");
    assert_eq!(out, "https://connect.composio.dev/abc");
}

#[tokio::test]
async fn authorize_rejects_toolkit_outside_allowlist_before_any_network_call() {
    // Backend URL is unreachable — the allowlist rejection must fire first.
    let out =
        authorize_connect_url(&config("http://127.0.0.1:1", vec!["gmail".into()]), "slack")
            .await;
    let err = out.expect_err("a toolkit outside the allowlist must be refused");
    assert!(err.to_string().contains("allowlist"), "{err}");
}

#[tokio::test]
async fn list_connection_states_aggregates_active_and_filters_to_allowlist() {
    let url = spawn_backend().await;
    // gmail + slack allowed; notion is active upstream but not in the grant.
    let states = list_connection_states(&config(&url, vec!["gmail".into(), "slack".into()]))
        .await
        .expect("list connections");
    assert_eq!(
        states,
        vec![("gmail".to_string(), true), ("slack".to_string(), false)],
        "gmail active (one ACTIVE row), slack pending only, notion filtered out"
    );
}

/// Issue #404: the detail view needs the account behind a connection, not
/// just that one exists. Pins the whole projection — per-connection rows
/// (two for gmail, where the fold gives one), the raw status, the account
/// label precedence, and the `(toolkit, id)` order — against the same
/// allowlist filter the fold applies.
#[tokio::test]
async fn list_connections_detailed_projects_each_account_with_its_identity() {
    let url = spawn_backend().await;
    let rows = list_connections_detailed(&config(&url, vec!["gmail".into(), "slack".into()]))
        .await
        .expect("list connections");

    // Compared as whole rows rather than as a tuple projection, so a field
    // added to `ComposioConnectionRow` later cannot slip past this
    // assertion unexamined.
    let expect = |id: &str,
                  toolkit: &str,
                  status: &str,
                  connected: bool,
                  created_at: Option<&str>,
                  account: Option<&str>| ComposioConnectionRow {
        id: id.to_string(),
        toolkit: toolkit.to_string(),
        status: status.to_string(),
        connected,
        created_at: created_at.map(str::to_string),
        account: account.map(str::to_string),
    };
    assert_eq!(
        rows,
        vec![
            // Email wins over the username the same row carries, and is
            // trimmed.
            expect(
                "c1",
                "gmail",
                "ACTIVE",
                true,
                Some("2026-08-01T10:00:00Z"),
                Some("ops@acme.test"),
            ),
            // A blank email is not an email: falls through to the workspace.
            // Kept as its own row rather than folded into c1 — this is the
            // "two Gmail accounts" case a disconnect has to tell apart.
            expect(
                "c2",
                "gmail",
                "INITIATED",
                false,
                None,
                Some("Acme Workspace"),
            ),
            // Username is the last resort.
            expect("c3", "slack", "INITIATED", false, None, Some("acme-bot")),
        ],
        "one row per connection, sorted by (toolkit, id); notion filtered out \
         by the allowlist exactly as the fold filters it"
    );
}

/// The fold the tile grid and the reconciliation probe read must keep
/// meaning what it meant before #404 widened the call underneath it —
/// `connected` is still "any account active", not "the first one".
#[tokio::test]
async fn the_per_toolkit_fold_still_summarises_the_detailed_rows() {
    let url = spawn_backend().await;
    let cfg = config(&url, vec!["gmail".into(), "slack".into()]);
    let rows = list_connections_detailed(&cfg).await.expect("rows");
    let states = list_connection_states(&cfg).await.expect("states");

    let folded: std::collections::BTreeMap<String, bool> =
        rows.into_iter().fold(Default::default(), |mut acc, r| {
            let e = acc.entry(r.toolkit).or_insert(false);
            *e = *e || r.connected;
            acc
        });
    assert_eq!(
        states,
        folded.into_iter().collect::<Vec<_>>(),
        "the states route is exactly the OR-fold of the detailed rows"
    );
}

/// Issue #404 + #403: an id this company's own reads will not show must not
/// be deletable by naming it. The mock serves no DELETE route at all, so a
/// request that got as far as dialling would fail loudly rather than pass —
/// the refusal has to come from the guard, before the call.
#[tokio::test]
async fn disconnect_refuses_an_id_outside_this_companys_visible_connections() {
    let url = spawn_backend().await;
    // `c4` (notion) is a real, active connection upstream — but this
    // company's manifest does not grant notion, so no read here surfaces
    // it. That is the case the guard exists for: the bearer *could* delete
    // it, and the allowlist must be a boundary rather than a display filter.
    let err = delete_connection(&config(&url, vec!["gmail".into()]), "c4")
        .await
        .expect_err("a connection outside the grant is not deletable");
    // The *variant* is the assertion, not the message: it is what decides
    // the status code the console sees, and asserting only on the string
    // is what let a refusal ship as a `502`.
    assert!(
        matches!(err, DisconnectError::NotFound(_)),
        "a refused id must be NotFound, not an upstream failure: {err:?}"
    );

    // And an id that exists nowhere at all fails the same way.
    let err = delete_connection(&config(&url, vec!["gmail".into()]), "nope")
        .await
        .expect_err("an unknown id is not deletable");
    assert!(
        matches!(err, DisconnectError::NotFound(_)),
        "unexpected error: {err:?}"
    );
}

/// An empty / whitespace id is refused before the list is even fetched —
/// and as a client mistake, not as an unreachable backend.
#[tokio::test]
async fn disconnect_refuses_a_blank_id_before_any_network_call() {
    // Unreachable backend — the argument check must fire first. If it did
    // not, this would surface as `Upstream`, which is what the assertion
    // below rules out.
    let err = delete_connection(&config("http://127.0.0.1:1", vec!["gmail".into()]), "  ")
        .await
        .expect_err("a blank id is refused");
    assert!(
        matches!(err, DisconnectError::NotFound(_)),
        "unexpected error: {err:?}"
    );
}

/// Issue #820: an account that is not usable cannot be the one agents act
/// as. `c2` is a real gmail connection of this company's, and `INITIATED` —
/// pinning it would route every gmail send to an account that cannot send,
/// which is worse than the unpinned behaviour it replaces. So the refusal is
/// a product decision, not a validation nicety, and it is asserted with the
/// store: a refusal that still wrote would be a broken toolkit with a
/// reassuring error message.
///
/// The two blunter refusals share the test because they share the guard, and
/// the assertion that matters for all three is the same one — nothing
/// reached [`crate::company::composio::set_default`].
#[tokio::test]
async fn pinning_an_account_that_cannot_send_is_refused_and_stores_nothing() {
    use crate::company::composio::load_defaults;
    use crate::ports::types::CompanyId;
    use crate::store::FsSecretStore;

    let url = spawn_backend().await;
    let dir = tempfile::Builder::new()
        .prefix("oc-composio-pin-")
        .tempdir()
        .expect("tempdir");
    let secrets = FsSecretStore::new(dir.path());
    let company = CompanyId::new("acme");
    let cfg = config(&url, vec!["gmail".into(), "slack".into()]);

    let err = set_default_connection(&cfg, &company, &secrets, "c2")
        .await
        .expect_err("an account that is not connected cannot be pinned");
    // `NotFound` and not `Upstream`: the backend answered fine, and the
    // console must render this as the operator's mistake with the fix in it
    // ("re-authorize it"), not as a provider outage.
    assert!(
        matches!(err, DisconnectError::NotFound(_)),
        "unexpected error: {err:?}"
    );
    assert!(
        err.to_string().contains("INITIATED") && err.to_string().contains("not connected"),
        "the message names the status the operator has to fix: {err}"
    );

    // An id belonging to nobody, and an id belonging to this company under a
    // toolkit its manifest does not grant — the same boundary
    // `delete_connection` draws, so a pin cannot reach what no read shows.
    for id in ["nope", "c4", "   "] {
        match set_default_connection(&cfg, &company, &secrets, id).await {
            Err(DisconnectError::NotFound(_)) => {}
            other => panic!("`{id}` must be refused as NotFound, got {other:?}"),
        }
    }

    assert!(
        load_defaults(&company, &secrets)
            .await
            .expect("defaults read")
            .is_empty(),
        "a refused pin must not be stored — the whole point is that the next \
         agent turn is unchanged"
    );

    // The control: `c1` is the same toolkit, ACTIVE, and goes through. Without
    // it a guard that refused everything would pass every assertion above.
    let toolkit = set_default_connection(&cfg, &company, &secrets, "c1")
        .await
        .expect("an active account is pinnable");
    assert_eq!(toolkit, "gmail", "the pinned toolkit is reported back");
    assert_eq!(
        load_defaults(&company, &secrets)
            .await
            .expect("defaults read")
            .get("gmail")
            .map(String::as_str),
        Some("c1")
    );
}

/// The console's open-mode source (issue #397): the backend's real catalog,
/// normalised. Connectable entries only, trimmed + lowercased, de-duplicated,
/// sorted.
#[tokio::test]
async fn list_catalog_toolkits_returns_the_backends_connectable_catalog() {
    let url = spawn_backend().await;
    let catalog = list_catalog_toolkits(&config(&url, Vec::new()))
        .await
        .expect("catalog fetch");
    assert_eq!(
        catalog.iter().map(|e| e.slug.as_str()).collect::<Vec<_>>(),
        vec!["gmail", "hubspot"],
        "connectable entries only, normalised, de-duplicated and sorted"
    );
}

/// Issue #600: the display metadata the backend publishes reaches the
/// caller instead of being reduced to a slug.
///
/// This is the regression test for the defect itself. Every field asserted
/// here was present in the response and discarded by a single
/// `.map(|entry| entry.slug)`, which is why the console had nothing to
/// group by, nothing to brand with, and nothing to search but the slug.
#[tokio::test]
async fn list_catalog_toolkits_carries_the_display_metadata() {
    let url = spawn_backend().await;
    let catalog = list_catalog_toolkits(&config(&url, Vec::new()))
        .await
        .expect("catalog fetch");

    let hubspot = catalog
        .iter()
        .find(|e| e.slug == "hubspot")
        .expect("hubspot is connectable");
    assert_eq!(hubspot.name, "HubSpot");
    assert_eq!(hubspot.description, "CRM and marketing automation.");
    assert_eq!(
        hubspot.logo.as_deref(),
        Some("https://logos.composio.dev/api/hubspot"),
        "the logo URL is what lets a tile be branded rather than a text row"
    );
    assert_eq!(
        hubspot.categories,
        vec!["crm".to_string(), "marketing".to_string()],
        "categories are trimmed and emptied-out entries dropped, but otherwise \
         forwarded verbatim — the console buckets them, not this layer"
    );

    let gmail = catalog
        .iter()
        .find(|e| e.slug == "gmail")
        .expect("gmail is connectable");
    assert_eq!(gmail.description, "Send and read email.");
    assert_eq!(
        gmail.logo, None,
        "an unpublished logo is None, not an empty string the console would \
         render as a broken image"
    );
    assert_eq!(
        gmail.name, "Gmail",
        "the FIRST entry for a slug wins, matching the de-duplication the slug \
         set used to do — not the later `Gmail (dup)`"
    );
}

/// A backend predating the dynamic catalog sends no `catalog[]`. Its plain
/// slug allowlist is used rather than reporting an empty catalog — which the
/// console would (correctly) render as a degraded fallback.
#[tokio::test]
async fn list_catalog_toolkits_falls_back_to_the_plain_allowlist() {
    let url = spawn_backend_with(get(legacy_toolkits_handler)).await;
    let catalog = list_catalog_toolkits(&config(&url, Vec::new()))
        .await
        .expect("catalog fetch");
    assert_eq!(
        catalog,
        vec![
            CatalogEntry::from_slug("gmail"),
            CatalogEntry::from_slug("notion"),
        ],
        "slug-only entries: the backend published nothing else, and the console \
         renders these with its own typography rather than dropping them"
    );
}

/// An unreachable backend is an error, never a quietly-empty catalog — the
/// caller has to be able to tell "nothing is permitted" from "I could not
/// ask".
#[tokio::test]
async fn list_catalog_toolkits_surfaces_a_fetch_failure() {
    let out = list_catalog_toolkits(&config("http://127.0.0.1:1", Vec::new())).await;
    out.expect_err("an unreachable backend must not read as an empty catalog");
}

#[tokio::test]
async fn list_connection_states_empty_allowlist_admits_every_toolkit() {
    let url = spawn_backend().await;
    let states = list_connection_states(&config(&url, Vec::new()))
        .await
        .expect("list connections");
    assert_eq!(
        states,
        vec![
            ("gmail".to_string(), true),
            ("notion".to_string(), true),
            ("slack".to_string(), false),
        ]
    );
}
}

/// The mandatory tenant-isolation test (issue #110): two per-tenant configs (A
/// and B) over a mock backend that records the `Authorization` header of each
/// request and answers with tenant-specific data. Proves the ONLY isolation
/// lever — which token the client is constructed with — actually holds: A's
/// request carries token A (never B), and A's result carries only A's account.
#[cfg(all(test, feature = "composio"))]
mod isolation_tests {
use super::*;

#[tokio::test]
async fn each_tenant_only_ever_carries_its_own_token_and_sees_its_own_accounts() {
    let (url, log) = spawn_backend().await;

    let tool_a = list_connections_tool(&config(&url, "token-a"));
    let tool_b = list_connections_tool(&config(&url, "token-b"));

    let out_a = tool_a.execute(json!({})).await.unwrap();
    let text_a = out_a.output();
    let out_b = tool_b.execute(json!({})).await.unwrap();
    let text_b = out_b.output();

    // A saw only A's account; never B's account nor B's token.
    assert!(
        text_a.contains("a@example.com"),
        "A missing its account: {text_a}"
    );
    assert!(
        !text_a.contains("b@example.com"),
        "A leaked B's account: {text_a}"
    );
    assert!(!text_a.contains("token-b"), "A leaked B's token: {text_a}");
    // Symmetrically for B.
    assert!(
        text_b.contains("b@example.com"),
        "B missing its account: {text_b}"
    );
    assert!(
        !text_b.contains("a@example.com"),
        "B leaked A's account: {text_b}"
    );

    // A's own token is scrubbed out of its own successful output.
    assert!(
        !text_a.contains("token-a"),
        "A leaked its own token: {text_a}"
    );

    // The backend received exactly the two distinct bearers — each request
    // carried its own tenant's token, never the other's.
    let seen = log.lock().unwrap().clone();
    assert_eq!(seen.len(), 2, "expected one request per tenant: {seen:?}");
    assert!(
        seen.iter().any(|a| a == "Bearer token-a"),
        "missing A bearer: {seen:?}"
    );
    assert!(
        seen.iter().any(|a| a == "Bearer token-b"),
        "missing B bearer: {seen:?}"
    );
    assert!(
        !seen
            .iter()
            .any(|a| a.contains("token-a") && a.contains("token-b")),
        "a single request must never carry both tokens: {seen:?}"
    );
}

/// The rotation contract at the tool boundary: a projected platform token the
/// cluster rewrites in place must reach the backend on the **next** call, with
/// no roster rebuild — and the freshly-resolved value must be the one the
/// scrub vector protects, so a backend that reflects it still cannot leak it.
#[tokio::test]
async fn a_rotated_projected_token_is_presented_and_scrubbed_per_call() {
    use crate::company::credentials::TinyhumansTokenSource;

    // Reflect the bearer back inside an envelope failure, and record it.
    async fn reflect(State(log): State<AuthLog>, headers: HeaderMap) -> axum::Json<Value> {
        let auth = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        log.lock().unwrap().push(auth.clone());
        axum::Json(json!({ "success": false, "error": format!("upstream said: {auth}") }))
    }
    let log: AuthLog = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/agent-integrations/composio/connections", get(reflect))
        .with_state(log.clone());
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let dir = tempfile::Builder::new()
        .prefix("oc-composio-rot-")
        .tempdir()
        .expect("tempdir");
    let path = dir.path().join("token");
    std::fs::write(&path, "projected-secret-before").unwrap();

    // ONE config, built once — exactly what a roster holds across turns.
    let config = TenantComposio::new(
        format!("http://{addr}"),
        Credential::from_source(Arc::new(TinyhumansTokenSource::projected_file(&path))),
        Vec::new(),
    );
    let tool = list_connections_tool(&config);

    let first = tool.execute(json!({})).await.unwrap();
    assert!(
        !first.output().contains("projected-secret-before"),
        "the resolved token leaked into agent-visible output: {}",
        first.output()
    );

    // The kubelet rewrites the file in place; the SAME tool must present the
    // new token and scrub that one.
    std::fs::write(&path, "projected-secret-after").unwrap();
    let second = tool.execute(json!({})).await.unwrap();
    assert!(
        !second.output().contains("projected-secret-after"),
        "the rotated token leaked into agent-visible output: {}",
        second.output()
    );

    let seen = log.lock().unwrap().clone();
    assert_eq!(
        seen,
        vec![
            "Bearer projected-secret-before".to_string(),
            "Bearer projected-secret-after".to_string()
        ],
        "each call must carry the token the file held at that moment: {seen:?}"
    );
}

/// A mock backend that echoes the caller's bearer inside an error body; the
/// tool's scrub must strip it before the agent ever sees it.
#[tokio::test]
async fn error_body_reflecting_the_token_is_scrubbed() {
    async fn reflect(headers: HeaderMap) -> axum::Json<Value> {
        let auth = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        // A 2xx envelope failure whose message reflects the raw bearer.
        axum::Json(json!({ "success": false, "error": format!("upstream said: {auth}") }))
    }
    let app = Router::new().route("/agent-integrations/composio/connections", get(reflect));
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let url = format!("http://{addr}");

    let tool = list_connections_tool(&config(&url, "reflected-secret-token"));
    let out = tool.execute(json!({})).await.unwrap();
    let text = out.output();
    assert!(
        !text.contains("reflected-secret-token"),
        "the reflected token leaked into agent-visible output: {text}"
    );
}

