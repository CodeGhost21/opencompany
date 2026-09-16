use super::*;

/// A hard backend failure on the toolkit catalogue must surface as an error,
/// never as an empty-but-successful listing. The distinction is the whole
/// point: an agent told "no toolkits" concludes the company has connected
/// nothing and stops asking, while an agent told the catalogue could not be
/// read can say so and retry later.
#[tokio::test]
async fn list_toolkits_reports_a_backend_failure_rather_than_an_empty_catalogue() {
    let (url, _log) = spawn_failing_backend().await;
    let tool = tool_named(&config(&url, "token-a"), "composio_list_toolkits");

    let out = tool.execute(json!({})).await.unwrap();
    let text = out.output();
    assert!(
        out.is_error,
        "a 500 from the catalogue must be an error, not a listing: {text}"
    );
    assert!(
        !text.contains("token-a"),
        "the tenant token leaked into the failure text: {text}"
    );
}

/// The same contract on the action catalogue. `composio_list_tools` is what
/// an agent reads before it picks a slug, so an empty success here sends it
/// on to guess a slug that was never listed.
#[tokio::test]
async fn list_tools_reports_a_backend_failure_rather_than_an_empty_listing() {
    let (url, _log) = spawn_failing_backend().await;
    let tool = tool_named(&config(&url, "token-a"), "composio_list_tools");

    let out = tool
        .execute(json!({ "search": "send email" }))
        .await
        .unwrap();
    let text = out.output();
    assert!(
        out.is_error,
        "a 500 from the action catalogue must be an error, not a listing: {text}"
    );
    assert!(
        !text.contains("token-a"),
        "the tenant token leaked into the failure text: {text}"
    );
}

/// Cross-tenant isolation must hold on the failure path too. A backend that
/// 5xxs gives the tool nothing to render, and the one thing it must not do
/// is fall back to any other source of connections — the output carries no
/// account at all, and no other tenant's bearer was ever presented.
#[tokio::test]
async fn a_backend_failure_on_connections_yields_no_accounts_and_no_other_tenants_token() {
    let (url, log) = spawn_failing_backend().await;
    let tool = tool_named(&config(&url, "token-a"), "composio_list_connections");

    let out = tool.execute(json!({})).await.unwrap();
    let text = out.output();
    assert!(
        out.is_error,
        "a 500 on connections must be an error: {text}"
    );
    assert!(
        !text.contains("@example.com"),
        "a failed listing must render no account whatsoever: {text}"
    );
    assert!(
        !text.contains("token-a") && !text.contains("token-b"),
        "no bearer may appear in the failure text: {text}"
    );
    let seen = log.lock().unwrap().len();
    assert!(seen >= 1, "the call must actually have reached the backend");
}

/// A company that has configured no Composio credential at all must have
/// every tool refuse before the network, rather than calling the backend
/// unauthenticated and rendering whatever it returns.
#[tokio::test]
async fn an_absent_credential_refuses_every_tool_before_the_network() {
    let (url, log) = spawn_failing_backend().await;
    let config = TenantComposio::new(url, Credential::None, Vec::new());

    for name in [
        "composio_list_toolkits",
        "composio_list_connections",
        "composio_list_tools",
    ] {
        let out = tool_named(&config, name).execute(json!({})).await.unwrap();
        assert!(
            out.is_error,
            "`{name}` must refuse without a credential: {}",
            out.output()
        );
    }
    assert!(
        log.lock().unwrap().is_empty(),
        "no request may leave for a company that configured no credential"
    );
}

#[tokio::test]
async fn a_repeated_authorize_for_one_toolkit_is_deduped() {
    let log: AuthLog = Arc::new(Mutex::new(Vec::new()));
    async fn authorize(State(log): State<AuthLog>) -> axum::Json<Value> {
        log.lock().unwrap().push("authorize".to_string());
        axum::Json(json!({
            "success": true,
            "data": { "connectUrl": "https://connect.composio.dev/abc", "connectionId": "conn-1" }
        }))
    }
    let app = Router::new()
        .route("/agent-integrations/composio/authorize", post(authorize))
        .route(
            "/agent-integrations/composio/connections",
            get(async || {
                axum::Json(json!({
                    "success": true,
                    "data": { "connections": [
                        { "id": "conn-1", "toolkit": "gmail", "status": "INITIATED" }
                    ] }
                }))
            }),
        )
        .with_state(log.clone());
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let config = TenantComposio::new(
        format!("http://{addr}"),
        Credential::from_value("token-a"),
        vec!["gmail".to_string()],
    );
    let tool = tool_named(&config, "composio_authorize");

    let first = tool.execute(json!({ "toolkit": "gmail" })).await.unwrap();
    assert!(!first.is_error, "{}", first.output());
    let second = tool.execute(json!({ "toolkit": "GMAIL" })).await.unwrap();
    assert!(!second.is_error, "{}", second.output());

    assert_eq!(
        log.lock().unwrap().len(),
        1,
        "a repeat authorize for the same toolkit must not open a second handoff"
    );
}

/// Repeated managed executes carry the same backend idempotency key.
#[tokio::test]
async fn a_repeated_execute_carries_an_idempotency_key_the_backend_can_dedupe_on() {
    type BodyLog = Arc<Mutex<Vec<(Value, Option<String>)>>>;
    async fn execute(
        State(log): State<BodyLog>,
        headers: HeaderMap,
        axum::Json(body): axum::Json<Value>,
    ) -> axum::Json<Value> {
        let key = headers
            .get("idempotency-key")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        log.lock().unwrap().push((body, key));
        axum::Json(json!({
            "success": true,
            "data": { "successful": true, "data": { "id": "msg-1" } }
        }))
    }
    let log: BodyLog = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/agent-integrations/composio/execute", post(execute))
        .with_state(log.clone());
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let config = TenantComposio::new(
        format!("http://{addr}"),
        Credential::from_value("token-a"),
        vec!["gmail".to_string()],
    );
    let tool = tool_named(&config, "composio_execute");

    let args = json!({
        "tool": "GMAIL_SEND_EMAIL",
        "arguments": { "to": "ops@acme.test", "subject": "hi", "body": "hello" }
    });
    let _ = tool.execute(args.clone()).await.unwrap();
    let _ = tool.execute(args).await.unwrap();

    let seen = log.lock().unwrap().clone();
    assert_eq!(seen.len(), 2, "both calls must have reached the backend");
    let keys: Vec<Option<String>> = seen.iter().map(|(_, k)| k.clone()).collect();
    assert!(
        keys.iter().all(Option::is_some),
        "a side-effecting execute must carry an idempotency key: {keys:?}"
    );
    assert_eq!(
        keys[0], keys[1],
        "two identical executes must present the SAME key so the backend can dedupe"
    );
}

