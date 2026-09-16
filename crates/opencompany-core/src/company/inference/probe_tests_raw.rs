use super::*;

// ---- the four cases the branch order exists for -------------------------
//
// architecture.md names these four by hand, and they are the whole reason
// the ordering is what it is. If one of them starts failing, the ordering
// has been "simplified" back into a bug.

#[test]
fn a_407_proxy_challenge_is_unknown_not_auth() {
    // It contains the word "authentication". Check auth first and a
    // corporate proxy deletes a valid key.
    assert_eq!(
        classify("HTTP 407 Proxy Authentication Required"),
        ProbeClass::Unknown
    );
    assert!(!classify("HTTP 407 Proxy Authentication Required").destroys_credential());
}

#[test]
fn a_bare_waf_403_is_unknown_not_auth() {
    // An unidentified intermediary saying "forbidden" is not proof the key
    // is bad. Cloudflare is named explicitly because it is the common one.
    assert_eq!(classify("error from cloudflare: 403"), ProbeClass::Unknown);
    assert_eq!(classify("502 Bad Gateway"), ProbeClass::Unknown);
}

#[test]
fn a_403_that_names_no_credential_refusal_keeps_the_key() {
    // Every one of these is a documented 403 from a provider we ship, and in
    // every one the credential is **valid**. Before the positive list they
    // all classified as `Auth` and deleted it — the string the classifier
    // read carried our own `Forbidden` reason phrase, which was one of the
    // four words the rule accepted as credential wording.
    for body in [
        // Together, for a prompt that ran past the context window. One long
        // message and the key was gone.
        "403: Input token count + max_tokens must be less than the context \
         length of the model being queried",
        // OpenRouter, whose 403 covers moderation as well as permissions.
        "403: Forbidden (insufficient permissions, guardrail block, or \
         moderation flag)",
        // OpenAI, for where the request came from.
        "403: Country, region, or territory not supported",
        // Anthropic's permission_error. It names the API key in its own text,
        // which is why matching the bare word `key` was never safe.
        "403: Your API key does not have permission to use the specified \
         resource.",
        // Google, Groq, xAI and Cerebras, in their own words.
        "403: PERMISSION_DENIED",
        "403: not allowed due to permission restrictions",
        "403: Ask your team admin for permission.",
        "403: PermissionDeniedError",
        // Fireworks' non-credential 403s.
        "403: FireRouter is not available for Fireworks accounts with data \
         residency enabled",
    ] {
        assert!(
            !classify(body).destroys_credential(),
            "this 403 must not delete the key: {body:?}"
        );
    }
}

#[test]
fn a_403_that_does_name_a_credential_refusal_is_still_auth() {
    // Fireworks is the reason the fix could not be "403 is never auth": it
    // maps a genuinely bad credential to 403 as well as 401, and these are
    // the only two bad-key messages it documents. Neither matches
    // `invalid api key`, so both are listed by hand.
    assert_eq!(
        classify("403: The API key you provided is invalid"),
        ProbeClass::Auth
    );
    assert_eq!(
        classify("403: You must provide an API key"),
        ProbeClass::Auth
    );
    assert!(classify("403: invalid credential").destroys_credential());
}

#[test]
fn the_reason_phrase_is_not_part_of_what_is_classified() {
    // The bug, stated as the one-line property that prevents its return: the
    // text handed to `classify` carries the vendor's body and the status
    // code, and nothing this module wrote. `Forbidden` appearing here would
    // put the old failure back whatever the rules say.
    let text = build_failure_text(
        reqwest::StatusCode::FORBIDDEN,
        "{\"error\":\"context length exceeded\"}",
    );
    assert!(!text.to_ascii_lowercase().contains("forbidden"), "{text}");
    assert!(text.starts_with("403: "));
    assert!(!classify(&text).destroys_credential());

    // And 401 keeps working with an empty body, which is how several
    // providers send it — the status is the whole signal there.
    let text = build_failure_text(reqwest::StatusCode::UNAUTHORIZED, "");
    assert!(
        !text.to_ascii_lowercase().contains("unauthorized"),
        "{text}"
    );
    assert_eq!(classify(&text), ProbeClass::Auth);
}

#[test]
fn a_400_about_our_request_shape_does_not_delete_the_key() {
    // `authentication` used to match as a bare word. An endpoint telling us
    // we used the wrong auth header is talking about our request, not about
    // the operator's key, and the key it is refusing to look at is fine.
    assert!(
        !classify("400: Bearer authentication is not supported, use x-api-key")
            .destroys_credential()
    );
    // Groq's 424 for a failed downstream dependency, which it documents as
    // "(e.g., Remote MCP authentication)".
    assert!(
        !classify("424: dependent request failed (Remote MCP authentication)")
            .destroys_credential()
    );
    // But a typed authentication error from Anthropic or DeepSeek still is
    // one.
    assert_eq!(classify("401: authentication_error"), ProbeClass::Auth);
    assert_eq!(
        classify("400: Authentication Fails (no such user)"),
        ProbeClass::Auth
    );
}

#[test]
fn a_status_code_inside_an_id_does_not_match() {
    // Word boundaries. Without them a request id or a model name carrying
    // these digits reads as a status code and deletes the operator's key.
    assert_eq!(classify("request id req_1403 failed"), ProbeClass::Unknown);
    assert_eq!(classify("trace 4032 aborted"), ProbeClass::Unknown);
    assert_eq!(classify("model gpt-4010 is odd"), ProbeClass::Unknown);
}

// ---- all six classes ----------------------------------------------------

#[test]
fn every_class_has_a_real_error_string_that_reaches_it() {
    let cases: &[(&str, ProbeClass)] = &[
        ("401 Unauthorized", ProbeClass::Auth),
        ("Incorrect API key provided", ProbeClass::Auth),
        ("invalid_api_key", ProbeClass::Auth),
        (
            "The model `gpt-5.6-sol-pro` does not exist",
            ProbeClass::Model,
        ),
        ("model_not_found", ProbeClass::Model),
        (
            "Model 'anthropic/claude-sonnet-5' is not available",
            ProbeClass::Model,
        ),
        ("You exceeded your current quota", ProbeClass::Quota),
        ("429 Too Many Requests", ProbeClass::Quota),
        ("insufficient credits", ProbeClass::Quota),
        ("404 Not Found", ProbeClass::Endpoint),
        ("dns error: not found", ProbeClass::Endpoint),
        ("operation timed out", ProbeClass::Timeout),
        ("request timeout after 10s", ProbeClass::Timeout),
        ("something nobody has seen before", ProbeClass::Unknown),
        ("", ProbeClass::Unknown),
    ];
    for (raw, expected) in cases {
        assert_eq!(classify(raw), *expected, "classifying {raw:?}");
    }
}

#[test]
fn a_missing_model_is_not_read_as_a_missing_endpoint() {
    // The endpoint branch matches a bare "not found". Checked after `model`,
    // so this sends the operator to their model id rather than their URL.
    assert_eq!(
        classify("The model `acme-1` was not found"),
        ProbeClass::Model
    );
    // And a genuine endpoint miss still reaches `endpoint`.
    assert_eq!(classify("404 page not found"), ProbeClass::Endpoint);
}

#[test]
fn exactly_one_class_deletes_the_credential() {
    let destructive: Vec<&str> = [
        ProbeClass::Auth,
        ProbeClass::Model,
        ProbeClass::Quota,
        ProbeClass::Endpoint,
        ProbeClass::Timeout,
        ProbeClass::Unknown,
    ]
    .into_iter()
    .filter(|c| c.destroys_credential())
    .map(|c| c.as_str())
    .collect();
    assert_eq!(destructive, vec!["auth"]);
}

#[test]
fn classification_is_case_insensitive_and_ignores_surrounding_noise() {
    assert_eq!(classify("  \n401 UNAUTHORIZED\n "), ProbeClass::Auth);
}

// ---- the copy -----------------------------------------------------------

#[test]
fn the_copy_never_echoes_the_upstream_string() {
    // That text can carry headers or key fragments, and this sentence lands
    // in a screenshot-able banner.
    let raw = "401 Unauthorized: Bearer sk-not-a-real-key rejected";
    let sentence = describe(classify(raw), "Acme");
    assert!(!sentence.contains("sk-not-a-real-key"));
    assert!(!sentence.contains(raw));
}

#[test]
fn only_the_destructive_class_fails_to_say_saved() {
    for class in [
        ProbeClass::Model,
        ProbeClass::Quota,
        ProbeClass::Endpoint,
        ProbeClass::Timeout,
        ProbeClass::Unknown,
    ] {
        assert!(
            describe(class, "Acme").starts_with("Saved"),
            "{class:?} kept the record, so its copy must say so"
        );
    }
    assert!(describe(ProbeClass::Auth, "Acme").starts_with("Could not reach Acme"));
}

// ---- what a failure may write down ---------------------------------------

#[test]
fn a_basic_token_is_encoded_the_way_reqwest_sends_it() {
    assert_eq!(base64_standard(b"alice:hunter2"), "YWxpY2U6aHVudGVyMg==");
    assert_eq!(base64_standard(b"a"), "YQ==");
    assert_eq!(base64_standard(b"ab"), "YWI=");
    assert_eq!(base64_standard(b""), "");
    assert_eq!(percent_decode("p%40ss%zz"), "p@ss%zz");
}

#[test]
fn every_form_of_the_endpoint_credential_is_scrubbed_from_text() {
    let endpoint = "http://alice:p%40ss@127.0.0.1:9/v1/models";
    let token = base64_standard(b"alice:p@ss");
    let echoed = format!(
        "rejected Basic {token} / {} for alice:p@ss (raw p%40ss)",
        token.trim_end_matches('=')
    );
    let scrubbed = scrub_endpoint_credential(endpoint, &echoed);
    for secret in [
        "p@ss",
        "p%40ss",
        token.as_str(),
        token.trim_end_matches('='),
    ] {
        assert!(
            !scrubbed.contains(secret),
            "{secret:?} survived: {scrubbed}"
        );
    }
    assert!(
        scrubbed.contains("alice"),
        "the account name still reads: {scrubbed}"
    );

    // Username only: that username is the token, in every form it can echo.
    let token_only = "http://sk-not%2Ba-real-key@127.0.0.1:9/v1/models";
    let basic = base64_standard(b"sk-not+a-real-key:");
    let echoed = format!(
        "bad key sk-not+a-real-key (sent sk-not%2Ba-real-key) in Basic {basic} / {}",
        basic.trim_end_matches('=')
    );
    let scrubbed = scrub_endpoint_credential(token_only, &echoed);
    for secret in [
        "sk-not+a-real-key",
        "sk-not%2Ba-real-key",
        basic.as_str(),
        basic.trim_end_matches('='),
    ] {
        assert!(
            !scrubbed.contains(secret),
            "{secret:?} survived: {scrubbed}"
        );
    }
    // No userinfo: the text is untouched.
    assert_eq!(
        scrub_endpoint_credential("http://127.0.0.1:9/v1", "Basic abc"),
        "Basic abc"
    );
}

/// An upstream that echoes the request's `Authorization` header back in its
/// 401 body. Served on loopback by hand, so the test needs nothing but tokio.
#[tokio::test]
async fn a_probe_failure_never_logs_the_basic_credential_an_endpoint_carried() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 8192];
        let n = stream.read(&mut buf).await.unwrap();
        let request = String::from_utf8_lossy(&buf[..n]).to_string();
        let authorization = request
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("authorization")
                    .then(|| value.trim().to_string())
            })
            .unwrap_or_default();
        let body = format!("{{\"error\":\"rejected [{authorization}] for alice:hunter2\"}}");
        let response = format!(
            "HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\n\
             content-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
        stream.shutdown().await.ok();
        authorization
    });

    let failure = probe_models(
        &format!("http://alice:hunter2@{address}/v1"),
        None,
        catalogue::AuthStyle::None,
        LOCAL_OFFERED,
        catalogue::CatalogShape::OpenAi,
    )
    .await
    .expect_err("a 401 is a failure");
    let sent = server.await.unwrap();

    assert_eq!(
        sent, "Basic YWxpY2U6aHVudGVyMg==",
        "the premise: reqwest sends the endpoint's userinfo as Basic auth"
    );
    assert!(
        failure.raw.contains("[Basic ***]"),
        "the echo reached the log text, scrubbed: {}",
        failure.raw
    );
    for secret in ["hunter2", "YWxpY2U6aHVudGVyMg"] {
        assert!(
            !failure.raw.contains(secret),
            "{secret} reached the log text: {}",
            failure.raw
        );
    }
}

// ---- the paged catalog (keys rework, issue #2306, slice 2a) ------------

/// A `PagedEnvelope` probe reads every page, following `total`, and sends
/// the bearer on every request — not just the first.
#[tokio::test]
async fn a_paged_probe_reads_every_page_with_the_bearer() {
    use axum::extract::Query;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    // Each page's `(query string, bearer)` as the fake server saw it.
    type Seen = Arc<Mutex<Vec<(String, Option<String>)>>>;
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let seen_for_route = seen.clone();
    let app = axum::Router::new().route(
        "/agent-integrations/openrouter/models",
        axum::routing::get(
            move |Query(params): Query<HashMap<String, String>>,
                  headers: axum::http::HeaderMap| {
                let seen = seen_for_route.clone();
                async move {
                    let query = params
                        .iter()
                        .map(|(k, v)| format!("{k}={v}"))
                        .collect::<Vec<_>>()
                        .join("&");
                    let auth = headers
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string);
                    seen.lock().unwrap().push((query, auth));
                    let offset: usize = params.get("offset").and_then(|o| o.parse().ok()).unwrap_or(0);
                    let data = if offset == 0 {
                        serde_json::json!([{"id": "acme/test-model"}, {"id": "acme/other-model"}])
                    } else {
                        serde_json::json!([{"id": "acme/third-model"}])
                    };
                    axum::Json(serde_json::json!({
                        "success": true,
                        "data": {"data": data, "total": 3, "limit": 500, "offset": offset},
                    }))
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let base = format!("http://{address}/agent-integrations/openrouter");

    let ids = probe_models(
        &base,
        Some("th-not-a-real-key"),
        catalogue::AuthStyle::Bearer,
        LOCAL_OFFERED,
        catalogue::CatalogShape::PagedEnvelope,
    )
    .await
    .expect("the paged catalog reads");
    server.abort();

    assert_eq!(
        ids,
        vec!["acme/test-model", "acme/other-model", "acme/third-model"]
    );
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2, "one request per page, got: {seen:?}");
    assert!(seen[0].0.contains("limit=500") && seen[0].0.contains("offset=0"));
    assert!(seen[1].0.contains("limit=500") && seen[1].0.contains("offset=2"));
    for (_, auth) in seen.iter() {
        assert_eq!(auth.as_deref(), Some("Bearer th-not-a-real-key"));
    }
}

/// A page that answers the OpenAI shape (not the envelope) classifies
/// `Unknown`, never `Auth` — a body that fails to parse says nothing about
/// the credential.
#[tokio::test]
async fn a_paged_probe_that_gets_an_openai_body_is_unknown_not_auth() {
    let app = axum::Router::new().route(
        "/agent-integrations/openrouter/models",
        axum::routing::get(|| async {
            axum::Json(serde_json::json!({"data": [{"id": "acme/test-model"}]}))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let base = format!("http://{address}/agent-integrations/openrouter");

    let failure = probe_models(
        &base,
        Some("th-not-a-real-key"),
        catalogue::AuthStyle::Bearer,
        LOCAL_OFFERED,
        catalogue::CatalogShape::PagedEnvelope,
    )
    .await
    .expect_err("an OpenAI-shaped body is not the envelope");
    server.abort();

    assert_eq!(failure.class, ProbeClass::Unknown);
}

// ---- the catalog body cap (bug KR-L1-01, keys rework issue #2306) ------

/// The regression test for the bug itself: a realistic OpenRouter-sized
/// catalog (600 ids, comfortably past the old 64 KiB failure-body cap
/// this success read used to share) is read to the last id, not silently
/// truncated into an empty list.
#[tokio::test]
async fn a_realistic_sized_catalog_past_the_old_64kib_cap_is_read_in_full() {
    const COUNT: usize = 600;
    // Padding per entry so the whole body lands comfortably past 900 KB —
    // the measured size of OpenRouter's real `/models` response — while
    // staying far under `CATALOG_BODY_CAP` (16 MiB).
    let filler = "x".repeat(1600);
    let entries: Vec<_> = (0..COUNT)
        .map(|i| {
            serde_json::json!({"id": format!("acme/test-model-{i}"), "description": filler})
        })
        .collect();
    let body = serde_json::json!({"data": entries}).to_string();
    assert!(
        body.len() > 900_000,
        "test fixture must exceed 900 KB to reproduce the bug: {} bytes",
        body.len()
    );

    let app = axum::Router::new().route(
        "/models",
        axum::routing::get(move || {
            let body = body.clone();
            async move {
                (
                    [(axum::http::header::CONTENT_TYPE, "application/json")],
                    body,
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let ids = probe_models(
        &format!("http://{address}"),
        Some("sk-not-a-real-key"),
        catalogue::AuthStyle::Bearer,
        LOCAL_OFFERED,
        catalogue::CatalogShape::OpenAi,
    )
    .await
    .expect("a large but valid catalog must be read in full, not truncated");
    server.abort();

    assert_eq!(ids.len(), COUNT, "every id must survive the read");
    assert_eq!(ids[0], "acme/test-model-0");
    assert_eq!(ids[COUNT - 1], format!("acme/test-model-{}", COUNT - 1));
}

/// A body that runs past `CATALOG_BODY_CAP` is refused outright, as a
/// classified `Unknown`/`truncated` failure — never silently parsed as an
/// empty catalog. This is the exact failure mode the bug report measured
/// against real OpenRouter: the body is cut mid-string, which as raw text
/// would be garbage JSON, and the fix is to never hand that text to the
/// parser at all.
#[tokio::test]
async fn a_catalog_body_over_the_cap_is_an_explicit_error_not_an_empty_list() {
    // One entry whose `description` alone is bigger than the whole cap —
    // the cheapest way to produce a real, over-the-wire body that exceeds
    // `CATALOG_BODY_CAP` without generating and comparing 16 MiB of
    // meaningful content.
    let oversized = "x".repeat(CATALOG_BODY_CAP + 1024);
    let body = format!(r#"{{"data":[{{"id":"acme/test-model","description":"{oversized}"#);

    let app = axum::Router::new().route(
        "/models",
        axum::routing::get(move || {
            let body = body.clone();
            async move {
                (
                    [(axum::http::header::CONTENT_TYPE, "application/json")],
                    body,
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let failure = probe_models(
        &format!("http://{address}"),
        Some("sk-not-a-real-key"),
        catalogue::AuthStyle::Bearer,
        LOCAL_OFFERED,
        catalogue::CatalogShape::OpenAi,
    )
    .await
    .expect_err("a body over the cap must be an explicit failure, not Ok(vec![])");
    server.abort();

    assert!(
        failure.truncated,
        "the failure must be flagged as a truncated catalog, not a generic Unknown"
    );
    assert_eq!(
        failure.class,
        ProbeClass::Unknown,
        "still non-destructive — a huge catalog says nothing about the credential"
    );
}

// ---- the SSRF guard -----------------------------------------------------

const LOCAL_OFFERED: ProbePolicy = ProbePolicy {
    allow_loopback: true,
};
const SERVER_SIDE: ProbePolicy = ProbePolicy {
    allow_loopback: false,
};

#[test]
fn an_ordinary_endpoint_is_allowed() {
    assert_eq!(
        check_endpoint("https://api.openai.com/v1", SERVER_SIDE),
        Ok(())
    );
    assert_eq!(check_endpoint("https://8.8.8.8/v1", SERVER_SIDE), Ok(()));
}

#[test]
fn only_http_and_https_are_probeable() {
    assert_eq!(
        check_endpoint("file:///etc/passwd", SERVER_SIDE),
        Err(EndpointRefusal::Scheme)
    );
    assert_eq!(
        check_endpoint("gopher://acme.test/v1", SERVER_SIDE),
        Err(EndpointRefusal::Scheme)
    );
    assert_eq!(
        check_endpoint("api.openai.com/v1", SERVER_SIDE),
        Err(EndpointRefusal::Unparseable)
    );
}

#[test]
fn the_cloud_metadata_address_is_refused_wherever_it_is_offered() {
    // 169.254.169.254 is where a container's credentials live. There is no
    // deployment on which a company's model endpoint is there.
    for policy in [LOCAL_OFFERED, SERVER_SIDE] {
        assert_eq!(
            check_endpoint("http://169.254.169.254/latest/meta-data/", policy),
            Err(EndpointRefusal::LinkLocal)
        );
    }
}

#[test]
fn link_local_is_refused_in_both_address_families() {
    assert_eq!(
        check_endpoint("http://169.254.1.1/v1", LOCAL_OFFERED),
        Err(EndpointRefusal::LinkLocal)
    );
    assert_eq!(
        check_endpoint("http://[fe80::1]/v1", LOCAL_OFFERED),
        Err(EndpointRefusal::LinkLocal)
    );
}

#[test]
fn an_ipv4_mapped_ipv6_address_gets_the_ipv4_answer() {
    // Checking only the v6 shape is how `::ffff:169.254.169.254` reaches a
    // metadata service through a guard that looks like it works.
    assert_eq!(
        check_endpoint("http://[::ffff:169.254.169.254]/v1", LOCAL_OFFERED),
        Err(EndpointRefusal::LinkLocal)
    );
    assert_eq!(
        check_endpoint("http://[::ffff:127.0.0.1]:11434/v1", SERVER_SIDE),
        Err(EndpointRefusal::Loopback)
    );
}

#[test]
fn loopback_is_an_explicit_allowance_not_a_hole() {
    // Allowed only where the local-runtime category is offered, because
    // that is exactly what Ollama needs.
    assert_eq!(
        check_endpoint("http://127.0.0.1:11434/v1", LOCAL_OFFERED),
        Ok(())
    );
    assert_eq!(
        check_endpoint("http://[::1]:11434/v1", LOCAL_OFFERED),
        Ok(())
    );
    assert_eq!(
        check_endpoint("http://127.0.0.1:11434/v1", SERVER_SIDE),
        Err(EndpointRefusal::Loopback)
    );
}

#[test]
fn private_and_carrier_grade_ranges_are_refused() {
    for addr in [
        "http://10.0.0.5/v1",
        "http://192.168.1.10/v1",
        "http://172.16.0.1/v1",
        "http://100.64.0.1/v1",
        "http://0.0.0.0/v1",
    ] {
        assert_eq!(
            check_endpoint(addr, LOCAL_OFFERED),
            Err(EndpointRefusal::PrivateNetwork),
            "{addr}"
        );
    }
    assert_eq!(
        check_endpoint("http://[fc00::1]/v1", LOCAL_OFFERED),
        Err(EndpointRefusal::PrivateNetwork)
    );
}

#[test]
fn a_key_is_never_sent_to_an_http_endpoint_off_this_host() {
    // `http` stays in the allowed set because the local-runtime category
    // needs it and there is no certificate to have at `localhost`. What is
    // refused is a **credential** leaving this host in the clear.
    assert_eq!(
        check_endpoint_with_credential("http://gateway.acme.test/v1", SERVER_SIDE, true),
        Err(EndpointRefusal::Cleartext)
    );
    // Without one there is nothing to leak, and this is a real shape: a
    // keyless gateway on an intranet.
    assert_eq!(
        check_endpoint_with_credential("http://gateway.acme.test/v1", SERVER_SIDE, false),
        Ok(())
    );
    // https is the point of the rule, not a coincidence of it.
    assert_eq!(
        check_endpoint_with_credential("https://gateway.acme.test/v1", SERVER_SIDE, true),
        Ok(())
    );
    // Loopback never leaves the host — by name, which is what Ollama's own
    // documentation prints, as well as by literal.
    for local in [
        "http://localhost:11434/v1",
        "http://ollama.localhost:11434/v1",
        "http://127.0.0.1:11434/v1",
        "http://[::1]:11434/v1",
        "http://[::ffff:127.0.0.1]:11434/v1",
    ] {
        assert_eq!(
            check_endpoint_with_credential(local, LOCAL_OFFERED, true),
            Ok(()),
            "{local} is this host"
        );
    }
    // And the address rules still run first: a credentialed https probe at
    // the metadata address is refused as link-local, not waved through.
    assert_eq!(
        check_endpoint_with_credential("https://169.254.169.254/v1", SERVER_SIDE, true),
        Err(EndpointRefusal::LinkLocal)
    );
}

#[test]
fn a_credentialed_request_does_not_follow_a_redirect_off_its_origin() {
    // `reqwest` strips `Authorization` when the host changes and keeps a
    // custom header, and the catalogue's one non-bearer entry sends the key
    // as `x-api-key` — so a provider that can answer `302` could name any
    // host to hand it to.
    let origin = "https://api.acme.test/v1/models";
    assert!(same_origin(origin, "https://api.acme.test/v2/models"));
    assert!(same_origin(
        origin,
        "https://API.ACME.TEST/v1/models?page=2"
    ));
    assert!(!same_origin(origin, "https://elsewhere.test/v1/models"));
    // Scheme and port are part of an origin, both ways.
    assert!(!same_origin(origin, "http://api.acme.test/v1/models"));
    assert!(!same_origin(origin, "https://api.acme.test:8443/v1/models"));
    // Unparseable is not a match: refusing costs a catalogue read, and
    // following costs the key.
    assert!(!same_origin(origin, "api.acme.test/v1/models"));
}

#[test]
fn a_redirect_target_gets_the_same_answer_as_the_first_hop() {
    // A permitted host that redirects to the metadata address is the whole
    // trick, so the address half is public for the redirect check to reuse.
    assert_eq!(check_endpoint("https://acme.test/v1", SERVER_SIDE), Ok(()));
    assert_eq!(
        check_address("169.254.169.254".parse().unwrap(), SERVER_SIDE),
        Err(EndpointRefusal::LinkLocal)
    );
}

#[test]
fn userinfo_and_ports_do_not_hide_the_host() {
    assert_eq!(
        check_endpoint("http://user:pw@169.254.169.254:80/v1", SERVER_SIDE),
        Err(EndpointRefusal::LinkLocal)
    );
    assert_eq!(
        check_endpoint("http://169.254.169.254@example.test/v1", SERVER_SIDE),
        Ok(()),
        "the authority after the last @ is the real host"
    );
}

#[test]
fn a_hostname_is_allowed_because_resolving_it_here_would_prove_nothing() {
    // A name resolved in a pure check is a DNS lookup in a pure function,
    // and the resolve can change underneath it anyway. The address check is
    // applied where the connection is actually made.
    assert_eq!(
        check_endpoint("https://localhost.acme.test/v1", SERVER_SIDE),
        Ok(())
    );
}

// ---- the IO half's pure helpers ----------------------------------------

#[test]
fn the_loopback_allowance_is_tied_to_the_local_runtime_category() {
    // Not a free-standing `true`. If the catalogue ever stops offering a
    // local runtime, the reason for the allowance is gone and so is the
    // allowance — one place to change rather than five call sites.
    assert_eq!(
        default_policy().allow_loopback,
        !catalogue::LOCAL_RUNTIMES.is_empty()
    );
}

#[test]
fn a_guard_refusal_keeps_the_credential() {
    // The SSRF guard answers a question about the address. Treating it as
    // an auth failure would delete a key over a typo in a URL.
    let failure = ProbeFailure::refused(EndpointRefusal::LinkLocal);
    assert_eq!(failure.class, ProbeClass::Endpoint);
    assert!(!failure.class.destroys_credential());
}

#[test]
fn model_ids_are_read_from_the_openai_shape_and_from_a_bare_array() {
    let wrapped = r#"{"data":[{"id":"gpt-5"},{"id":"gpt-5-mini"}]}"#;
    assert_eq!(parse_model_ids(wrapped), vec!["gpt-5", "gpt-5-mini"]);
    let bare = r#"[{"id":"llama3"}]"#;
    assert_eq!(parse_model_ids(bare), vec!["llama3"]);
}

#[test]
fn a_body_that_is_not_a_catalog_is_an_empty_list_rather_than_a_failure() {
    // A 200 from something that is not a model listing is still a reachable
    // endpoint. Failing here would refuse every provider that does not
    // publish an OpenAI-shaped catalog, which the connect flow explicitly
    // supports adding.
    assert!(parse_model_ids("not json at all").is_empty());
    assert!(parse_model_ids(r#"{"models":["a"]}"#).is_empty());
    assert!(parse_model_ids(r#"{"data":[{"name":"no id here"}]}"#).is_empty());
}

#[test]
fn a_transport_failure_says_which_condition_it_was() {
    // `reqwest`'s own Display buries the cause, so a DNS failure and a
    // timeout read identically and both classify as `unknown`. These fixed
    // phrases are what let `classify` tell them apart.
    assert_eq!(classify("timeout"), ProbeClass::Timeout);
    assert_eq!(classify("connection refused"), ProbeClass::Endpoint);
    assert_eq!(
        classify("redirect not followed: unreachable"),
        ProbeClass::Endpoint
    );
    assert_eq!(classify("the check did not complete"), ProbeClass::Unknown);
}

#[test]
fn the_probes_own_url_never_reaches_the_classifier() {
    // This probe's URL always ends in `/models`, so interpolating it into
    // the classifier's input makes EVERY failure contain the word "model" —
    // and a refused connection classified as a missing model id, sending the
    // operator to check a model they never typed.
    let failure = ProbeFailure::classified_as(
        "connection refused",
        "http://127.0.0.1:9/v1/models: error sending request".to_string(),
    );
    assert_eq!(failure.class, ProbeClass::Endpoint);
    assert!(
        failure.raw.contains("/models"),
        "the URL is still worth having in a log"
    );
}

#[test]
fn dns_and_refusal_are_endpoint_facts_not_unknowns() {
    for raw in [
        "connection refused",
        "no such host",
        "could not resolve host",
        "temporary failure in name resolution",
        "dns error",
        "network is unreachable",
        "connection reset by peer",
    ] {
        assert_eq!(
            classify(raw),
            ProbeClass::Endpoint,
            "`{raw}` is the clearest evidence there is that nothing is at that address"
        );
    }
}

#[test]
fn a_local_runtime_that_is_not_running_is_not_a_connection_worth_keeping() {
    // The one category-specific exception to "only `auth` rolls back". A
    // runtime that is not listening is a fact about the operator's machine
    // and their next move is to start it — not to keep a row pointing at a
    // port with nothing behind it.
    for class in [ProbeClass::Endpoint, ProbeClass::Timeout] {
        assert!(rolls_back(class, catalogue::Category::Local), "{class:?}");
    }
}

#[test]
fn the_same_class_against_a_cloud_provider_keeps_everything() {
    // And this asymmetry is the point: `endpoint` against a vendor's host
    // is a fact about the network in between — a proxy, a WAF, a slow
    // gateway — sitting between a perfectly good key and an endpoint that
    // is fine. Rolling back there is the bug the classifier exists to stop.
    for class in [ProbeClass::Endpoint, ProbeClass::Timeout, ProbeClass::Quota] {
        assert!(!rolls_back(class, catalogue::Category::Cloud), "{class:?}");
    }
}

#[test]
fn auth_rolls_back_whatever_the_category() {
    for category in [
        catalogue::Category::Cloud,
        catalogue::Category::Local,
        catalogue::Category::Cli,
    ] {
        assert!(rolls_back(ProbeClass::Auth, category), "{category:?}");
    }
}

#[test]
fn a_refusal_never_says_saved() {
    // `describe` opens every sentence but one with "Saved", which is true
    // when the row was kept. On the rollback path no row exists, and an
    // operator told it was saved while nothing appears has been lied to
    // about the one thing they can see.
    for class in [
        ProbeClass::Auth,
        ProbeClass::Endpoint,
        ProbeClass::Timeout,
        ProbeClass::Unknown,
    ] {
        let said = describe_refusal(class, "Ollama");
        assert!(!said.contains("Saved"), "{class:?}: {said}");
    }
}

#[test]
fn a_refusal_names_the_next_thing_to_do() {
    assert!(describe_refusal(ProbeClass::Endpoint, "Ollama").contains("Start it"));
    assert!(
        describe_refusal(ProbeClass::Auth, "Groq").contains("rejected the credential"),
        "the auth sentence is unchanged — it was already right"
    );
}

// ---- auth style on the wire --------------------------------------------
//
// Asserted on the **headers actually sent**, not on a return value: this bug
// was invisible to every test that only checked what a function returned,
// because the function returned fine and the request was malformed.

/// The headers one `apply_auth` call produces, as `(name, value)` pairs.
fn headers_for(auth: catalogue::AuthStyle, key: Option<&str>) -> Vec<(String, String)> {
    let client = reqwest::Client::new();
    let request = apply_auth(client.get("https://example.test/v1/models"), auth, key);
    let built = request.build().expect("a request");
    built
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_ascii_lowercase(),
                value.to_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header, _)| header == name)
        .map(|(_, value)| value.as_str())
}

#[test]
fn anthropic_gets_x_api_key_and_a_version_and_no_authorization() {
    // A bearer with no `anthropic-version` is rejected by Anthropic's native
    // API as MALFORMED — a 400, not a 401 — which is the diagnostic that
    // tells a broken request from a bad key. The reported symptom was
    // exactly that 400 on a key that was fine.
    let headers = headers_for(
        catalogue::AuthStyle::Anthropic,
        Some("sk-ant-not-a-real-key"),
    );
    assert_eq!(header(&headers, "x-api-key"), Some("sk-ant-not-a-real-key"));
    assert_eq!(
        header(&headers, "anthropic-version"),
        Some(catalogue::ANTHROPIC_VERSION)
    );
    assert_eq!(
        header(&headers, "authorization"),
        None,
        "a bearer alongside x-api-key is the shape that was failing"
    );
}

#[test]
fn a_bearer_provider_is_unchanged() {
    let headers = headers_for(catalogue::AuthStyle::Bearer, Some("sk-not-a-real-key"));
    assert_eq!(
        header(&headers, "authorization"),
        Some("Bearer sk-not-a-real-key")
    );
    assert_eq!(header(&headers, "x-api-key"), None);
    assert_eq!(header(&headers, "anthropic-version"), None);
}

#[test]
fn a_provider_with_no_key_gets_no_auth_header_at_all() {
    // The keyless local runtime. An empty header is worse than none.
    for key in [None, Some(""), Some("   ")] {
        for auth in [
            catalogue::AuthStyle::Bearer,
            catalogue::AuthStyle::Anthropic,
            catalogue::AuthStyle::None,
        ] {
            let headers = headers_for(auth, key);
            assert_eq!(header(&headers, "authorization"), None, "{auth:?} {key:?}");
            assert_eq!(header(&headers, "x-api-key"), None, "{auth:?} {key:?}");
        }
    }
}

#[test]
fn a_keyless_auth_style_sends_nothing_even_with_a_key() {
    // `AuthStyle::None` is a statement about the endpoint, not about whether
    // we happen to hold a credential.
    let headers = headers_for(catalogue::AuthStyle::None, Some("sk-not-a-real-key"));
    assert_eq!(header(&headers, "authorization"), None);
    assert_eq!(header(&headers, "x-api-key"), None);
}
