use super::*;
use crate::analytics::config::{
    CLIENT_ID_ENV, CLIENT_SECRET_ENV, ENABLE_ENV, ENDPOINT_ENV, resolve,
};
use crate::analytics::types::OpaqueId;
use crate::analytics::{Event, Outcome, Trigger};
use crate::app::config::MapEnv;
use crate::app::deployment::{DEPLOYMENT_ENV, Deployment};
use crate::ports::brain::Cognition;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Obviously-fake credentials. Never a real one, in a file or anywhere else.
const TEST_CLIENT_ID: &str = "not-a-real-client-id";
const TEST_CLIENT_SECRET: &str = "not-a-real-client-secret";

/// The headers each request arrived with, in order, name and value.
type SeenHeaders = Arc<std::sync::Mutex<Vec<Vec<(String, String)>>>>;

/// A local collector that counts what it is sent and keeps the bodies and
/// the headers.
struct Collector {
    hits: Arc<AtomicUsize>,
    bodies: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    headers: SeenHeaders,
    url: String,
    shutdown: tokio::sync::oneshot::Sender<()>,
    handle: tokio::task::JoinHandle<()>,
}

async fn spawn_collector() -> Collector {
    spawn_collector_with(Duration::ZERO, 0, axum::http::StatusCode::BAD_REQUEST).await
}

/// A collector that takes `delay` to answer and refuses the first
/// `refuse_first` requests with `refusal`, so a test can observe what
/// happens while a request is in flight, what happens after one event is
/// rejected, and what happens when the refusal is about the credential
/// rather than about the event.
async fn spawn_collector_with(
    delay: Duration,
    refuse_first: usize,
    refusal: axum::http::StatusCode,
) -> Collector {
    let hits = Arc::new(AtomicUsize::new(0));
    let bodies = Arc::new(std::sync::Mutex::new(Vec::new()));
    let headers = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_hits = hits.clone();
    let seen_bodies = bodies.clone();
    let seen_headers = headers.clone();

    let app = axum::Router::new().route(
        "/track",
        axum::routing::post(
            move |received: axum::http::HeaderMap,
                  axum::Json(body): axum::Json<serde_json::Value>| {
                let hits = seen_hits.clone();
                let bodies = seen_bodies.clone();
                let headers = seen_headers.clone();
                async move {
                    if !delay.is_zero() {
                        tokio::time::sleep(delay).await;
                    }
                    let seen = hits.fetch_add(1, Ordering::SeqCst);
                    bodies.lock().unwrap().push(body);
                    headers.lock().unwrap().push(
                        received
                            .iter()
                            .map(|(name, value)| {
                                (
                                    name.as_str().to_string(),
                                    value.to_str().unwrap_or_default().to_string(),
                                )
                            })
                            .collect(),
                    );
                    if seen < refuse_first {
                        refusal
                    } else {
                        axum::http::StatusCode::OK
                    }
                }
            },
        ),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/track", listener.local_addr().unwrap());
    let (shutdown, rx) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await;
    });

    Collector {
        hits,
        bodies,
        headers,
        url,
        shutdown,
        handle,
    }
}

impl Collector {
    async fn stop(self) {
        let _ = self.shutdown.send(());
        let _ = self.handle.await;
    }

    fn header(&self, request: usize, name: &str) -> Option<String> {
        self.headers.lock().unwrap()[request]
            .iter()
            .find(|(seen, _)| seen == name)
            .map(|(_, value)| value.clone())
    }
}

fn envelope() -> Envelope {
    Envelope::new(
        OpaqueId::instance("0123456789abcdef0123456789abcdef"),
        Deployment::HostedTenant,
        Cognition::default(),
    )
}

/// A reporting environment pointed at `endpoint`, which `pairs` overrides.
fn env(endpoint: &str, pairs: &[(&str, &str)]) -> MapEnv {
    let mut all = vec![
        (CLIENT_ID_ENV, TEST_CLIENT_ID),
        (CLIENT_SECRET_ENV, TEST_CLIENT_SECRET),
        (ENDPOINT_ENV, endpoint),
    ];
    all.extend_from_slice(pairs);
    MapEnv::new(all)
}

fn events() -> Vec<Event> {
    vec![
        Event::InstanceStarted {
            companies: 1,
            storage: "fs",
            setup_complete: true,
        },
        Event::TurnFinished {
            trigger: Trigger::OperatorMessage,
            outcome: Outcome::Ok,
            failure: None,
            duration_ms: 12,
            effects_executed: 0,
            approvals_parked: 0,
        },
    ]
}

/// **Issue #1739's first acceptance criterion.** A build that *has* the
/// transport compiled in, pointed at a live collector, with a credential in
/// the environment, and not declared hosted: it must send nothing.
///
/// Note what is deliberately stacked against the assertion — the feature is
/// on, the client exists, the endpoint resolves, both halves of the
/// credential are present. The only thing that is not is consent. That is
/// the configuration a self-hoster who copied a hosted deployment's env file
/// would have.
#[tokio::test]
async fn a_self_hosted_build_makes_no_request() {
    let collector = spawn_collector().await;
    let env = env(&collector.url, &[]);

    let decision = resolve(Deployment::from_env(&env), &env);
    let tracker = build(&decision, envelope());
    for event in events() {
        tracker.track(event);
    }
    tracker.flush().await;

    assert_eq!(
        collector.hits.load(Ordering::SeqCst),
        0,
        "a self-hosted build must not dial out"
    );
    collector.stop().await;
}

/// The positive control that makes the test above non-vacuous: the same
/// collector, the same events, the same code path, one variable changed.
///
/// It also pins the whole wire contract — **one request per event**, the two
/// auth headers by their exact spelling, and OpenPanel's discriminated-union
/// body with the identity as `profileId` rather than as a property.
#[tokio::test]
async fn a_hosted_tenant_reports_with_the_full_envelope() {
    let collector = spawn_collector().await;
    let env = env(&collector.url, &[(DEPLOYMENT_ENV, "hosted-tenant")]);

    let decision = resolve(Deployment::from_env(&env), &env);
    assert!(decision.reports(), "{decision:?}");
    let tracker = build(&decision, envelope());
    for event in events() {
        tracker.track(event);
    }
    tracker.flush().await;

    // Two events, two requests. OpenPanel has no batch endpoint, so this is
    // the one number that changed shape rather than value.
    assert_eq!(
        collector.hits.load(Ordering::SeqCst),
        2,
        "one request per event"
    );

    let bodies = collector.bodies.lock().unwrap().clone();
    let first = &bodies[0];
    assert_eq!(first["type"], "track");
    assert_eq!(first["payload"]["name"], "instance_started");
    assert_eq!(
        first["payload"]["profileId"],
        "i_0123456789abcdef0123456789abcdef"
    );

    let properties = &first["payload"]["properties"];
    assert_eq!(properties["deployment"], "hosted-tenant");
    assert!(properties["app_version"].is_string());
    assert!(properties["harness_in_build"].is_boolean());
    assert_eq!(bodies[1]["payload"]["name"], "turn_finished");

    for request in 0..2 {
        assert_eq!(
            collector.header(request, CLIENT_ID_HEADER).as_deref(),
            Some(TEST_CLIENT_ID),
            "request {request} carried no client id header"
        );
        assert_eq!(
            collector.header(request, CLIENT_SECRET_HEADER).as_deref(),
            Some(TEST_CLIENT_SECRET),
            "request {request} carried no client secret header"
        );
    }

    collector.stop().await;
}

/// **The credential travels in headers and nowhere else.**
///
/// The transport this replaced stamped Mixpanel's token into every event's
/// property bag, which put a credential one `dbg!` away from a test fixture
/// or a captured body. Nothing does that now, and this is the assertion that
/// keeps it true: not one byte of either half appears in any body on the
/// wire.
#[tokio::test]
async fn no_credential_reaches_the_request_body() {
    let collector = spawn_collector().await;
    let env = env(&collector.url, &[(DEPLOYMENT_ENV, "hosted-tenant")]);
    let tracker = build(&resolve(Deployment::from_env(&env), &env), envelope());
    for event in events() {
        tracker.track(event);
    }
    tracker.flush().await;

    for body in collector.bodies.lock().unwrap().iter() {
        let rendered = body.to_string().to_ascii_lowercase();
        for half in [TEST_CLIENT_ID, TEST_CLIENT_SECRET] {
            assert!(
                !rendered.contains(&half.to_ascii_lowercase()),
                "the body carried {half}: {rendered}"
            );
        }
    }
    // The self-check: the needle really is findable where it *is* supposed
    // to be, or the guard above would pass on a transport that sent no
    // credential at all.
    assert_eq!(
        collector.header(0, CLIENT_SECRET_HEADER).as_deref(),
        Some(TEST_CLIENT_SECRET)
    );

    collector.stop().await;
}

/// An operator who switched it off stays off, even on a hosted tenant.
#[tokio::test]
async fn an_opted_out_tenant_makes_no_request() {
    let collector = spawn_collector().await;
    let env = env(
        &collector.url,
        &[(DEPLOYMENT_ENV, "hosted-tenant"), (ENABLE_ENV, "off")],
    );

    let tracker = build(&resolve(Deployment::from_env(&env), &env), envelope());
    for event in events() {
        tracker.track(event);
    }
    tracker.flush().await;

    assert_eq!(collector.hits.load(Ordering::SeqCst), 0);
    collector.stop().await;
}

/// **A refused event does not stop the drain.**
///
/// A per-event HTTP status is a per-event answer — a name the collector
/// rejects, a body it will not take — and the events behind it may be
/// perfectly good. Without this, one malformed event would silence an entire
/// drain, which is the failure mode that matters most in a module where
/// every other failure is already silent.
#[tokio::test]
async fn a_refused_event_does_not_stop_the_drain() {
    let collector =
        spawn_collector_with(Duration::ZERO, 1, axum::http::StatusCode::BAD_REQUEST).await;
    let env = env(&collector.url, &[(DEPLOYMENT_ENV, "hosted-tenant")]);
    let tracker = build(&resolve(Deployment::from_env(&env), &env), envelope());

    for _ in 0..3 {
        tracker.track(Event::InstanceStarted {
            companies: 1,
            storage: "fs",
            setup_complete: true,
        });
    }
    tracker.flush().await;

    assert_eq!(
        collector.hits.load(Ordering::SeqCst),
        3,
        "the two events behind a refused one must still be attempted"
    );
    collector.stop().await;
}

/// **A refused *credential* does stop the drain, unlike a refused event.**
///
/// A `401` is not the collector's verdict on one event; it is its verdict on
/// this process, so every event behind it in the queue gets the same answer.
/// Carrying on would fire up to 500 requests every thirty seconds for the
/// life of a misconfigured tenant — a thousand a minute at the operator's
/// own collector — to learn something already known.
///
/// The contrast with `a_refused_event_does_not_stop_the_drain` is the point:
/// same collector, same three events, one status code changed, opposite
/// behaviour. Neither test means much without the other.
#[tokio::test]
async fn a_refused_credential_stops_the_drain() {
    let collector = spawn_collector_with(
        Duration::ZERO,
        usize::MAX,
        axum::http::StatusCode::UNAUTHORIZED,
    )
    .await;
    let env = env(&collector.url, &[(DEPLOYMENT_ENV, "hosted-tenant")]);
    let tracker = build(&resolve(Deployment::from_env(&env), &env), envelope());

    for _ in 0..3 {
        tracker.track(Event::InstanceStarted {
            companies: 1,
            storage: "fs",
            setup_complete: true,
        });
    }
    tracker.flush().await;

    assert_eq!(
        collector.hits.load(Ordering::SeqCst),
        1,
        "a 401 is about the credential, not about the event, so the two behind it \
         must not be attempted"
    );
    collector.stop().await;
}

/// **The write secret never follows a redirect to another host.**
///
/// The leak this closes is not exotic. `reqwest`'s default policy follows
/// ten hops, and its cross-origin sanitization
/// (`redirect.rs::remove_sensitive_headers`, 0.12.28) removes exactly
/// `Authorization`, `Cookie`, `cookie2`, `Proxy-Authorization` and
/// `WWW-Authenticate` — and nothing else. `openpanel-client-secret` is none
/// of them, so before [`reqwest::redirect::Policy::none`] a single `307`
/// from the configured collector handed this instance's long-lived write
/// credential to whatever host the `Location` named.
///
/// `set_sensitive` is not a defence and is worth naming, because it looks
/// like one in the source: it governs `Debug` output and HPACK indexing,
/// and has no bearing on which headers survive a hop.
///
/// Two collectors on two ports, so `next.port_or_known_default() !=
/// previous.port_or_known_default()` — reqwest's own cross-host test — is
/// unambiguously true and the sanitization it does perform is in play. The
/// assertion is on the **destination**: it must be untouched. Asserting
/// only "the redirect was not followed" would pass against a client that
/// followed it and merely dropped the header, which is a different and
/// weaker property than the one being claimed.
#[tokio::test]
async fn a_redirect_never_carries_the_credential_to_another_host() {
    // Where a followed redirect would land: a real collector that records
    // every header of everything it is sent.
    let elsewhere = spawn_collector().await;
    let target = elsewhere.url.clone();

    // The configured endpoint: answers every POST with a 307 to the other
    // collector, on a different port and so a different origin.
    let redirected = Arc::new(AtomicUsize::new(0));
    let counted = redirected.clone();
    let app = axum::Router::new().route(
        "/track",
        axum::routing::post(move || {
            let hits = counted.clone();
            let target = target.clone();
            async move {
                hits.fetch_add(1, Ordering::SeqCst);
                (
                    axum::http::StatusCode::TEMPORARY_REDIRECT,
                    [(axum::http::header::LOCATION, target)],
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/track", listener.local_addr().unwrap());
    let (shutdown, rx) = tokio::sync::oneshot::channel();
    let redirector = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await;
    });

    let env = env(&url, &[(DEPLOYMENT_ENV, "hosted-tenant")]);
    let tracker = build(&resolve(Deployment::from_env(&env), &env), envelope());
    for _ in 0..3 {
        tracker.track(Event::InstanceStarted {
            companies: 1,
            storage: "fs",
            setup_complete: true,
        });
    }
    tracker.flush().await;

    assert_eq!(
        elsewhere.hits.load(Ordering::SeqCst),
        0,
        "a redirect must not carry the client credential to a host \
         OPENCOMPANY_ANALYTICS_ENDPOINT never named"
    );
    assert_eq!(
        redirected.load(Ordering::SeqCst),
        1,
        "a redirecting endpoint is a verdict on the endpoint, not on one event, so \
         the two behind it must not be attempted"
    );

    let _ = shutdown.send(());
    let _ = redirector.await;
    elsewhere.stop().await;
}

/// The control that makes the test above non-vacuous.
///
/// `elsewhere.hits == 0` would also hold if the destination collector were
/// simply broken, or if `spawn_collector` did not record what it received.
/// Same collector, same events, pointed at directly rather than through a
/// redirect: it must see all three requests, carrying the secret, so the
/// zero above is about the redirect and nothing else.
#[tokio::test]
async fn the_redirect_destination_would_have_recorded_the_credential() {
    let elsewhere = spawn_collector().await;
    let env = env(&elsewhere.url, &[(DEPLOYMENT_ENV, "hosted-tenant")]);
    let tracker = build(&resolve(Deployment::from_env(&env), &env), envelope());
    for _ in 0..3 {
        tracker.track(Event::InstanceStarted {
            companies: 1,
            storage: "fs",
            setup_complete: true,
        });
    }
    tracker.flush().await;

    assert_eq!(
        elsewhere.hits.load(Ordering::SeqCst),
        3,
        "the destination records what it is sent, so the zero above is the redirect \
         policy rather than a collector that counts nothing"
    );
    assert_eq!(
        elsewhere.header(0, CLIENT_SECRET_HEADER).as_deref(),
        Some(TEST_CLIENT_SECRET),
        "and it records the credential header, which is the thing that must not \
         have arrived across a redirect"
    );
    elsewhere.stop().await;
}

/// The queue is bounded. An unreachable collector must cost telemetry, not
/// a tenant container's memory — and `track` must not block whatever the
/// collector does.
#[tokio::test]
async fn the_queue_is_bounded() {
    // Nothing listens here; the point is that `track` never blocks and
    // never grows without bound whatever the collector does.
    let env = env(
        "http://127.0.0.1:1/track",
        &[(DEPLOYMENT_ENV, "hosted-tenant")],
    );
    let tracker = build(&resolve(Deployment::from_env(&env), &env), envelope());
    for _ in 0..2_000 {
        tracker.track(Event::InstanceStarted {
            companies: 1,
            storage: "fs",
            setup_complete: true,
        });
    }
    // No assertion on an internal count — the observable property is that
    // this returns at all, promptly, with no reachable collector.
}

/// **An unreachable collector costs one timeout, not one per queued event.**
///
/// This is the case losing the batch endpoint created. With a single POST
/// carrying everything, an unreachable collector cost exactly one
/// `SEND_TIMEOUT`. One request per event, drained sequentially, would cost
/// `queued × SEND_TIMEOUT` — up to forty minutes at a full queue — during
/// which the shutdown flush is blocked behind the same lock and a
/// container's `SIGTERM` budget is long gone.
///
/// Asserted on **connections the collector actually accepted**, not on
/// elapsed time, because a timing threshold on a black-holing socket is a
/// flaky test. The listener accepts and never answers, so each attempt is a
/// real connection that pays the full timeout: three queued events must
/// produce **one** connection, not three.
#[tokio::test]
async fn an_unreachable_collector_costs_one_timeout_for_the_whole_drain() {
    let accepted = Arc::new(AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/track", listener.local_addr().unwrap());
    let counted = accepted.clone();
    // Accepts and holds. Never reads, never answers — the shape a collector
    // behind a wedged proxy has, and the one a refused port does not
    // exercise because it fails instantly.
    let black_hole = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            counted.fetch_add(1, Ordering::SeqCst);
            held.push(socket);
        }
    });

    let env = env(&url, &[(DEPLOYMENT_ENV, "hosted-tenant")]);
    let tracker = build(&resolve(Deployment::from_env(&env), &env), envelope());
    for _ in 0..3 {
        tracker.track(Event::InstanceStarted {
            companies: 1,
            storage: "fs",
            setup_complete: true,
        });
    }

    let started = Instant::now();
    tracker.flush().await;
    let waited = started.elapsed();

    assert_eq!(
        accepted.load(Ordering::SeqCst),
        1,
        "the drain must give up after the first transport failure, not pay a \
         timeout for every queued event"
    );
    assert!(
        waited < Duration::from_secs(12),
        "a drain against a black hole took {waited:?}, which is more than one \
         send timeout and would outlive a container's shutdown budget"
    );
    // And the flush still returned, which is the property `Tracker::flush`
    // promises: analytics never prevents a shutdown.
    black_hole.abort();
}

/// **A transport failure must not carry the collector credential.**
///
/// `OPENCOMPANY_ANALYTICS_ENDPOINT` names a collector the operator runs, and
/// such a collector is routinely fronted by an authenticated proxy, which
/// carries its key in one of the two places a URL can hold one.
/// `reqwest::Error` retains the request URL and prints it, so an unreachable
/// collector — a routine event, not an exotic one — wrote that key into the
/// debug log.
///
/// Measured against reqwest 0.12.28 rather than assumed, and the two places
/// do **not** behave alike:
///
/// | in the endpoint | what `reqwest::Error`'s `Display` printed |
/// |---|---|
/// | `http://someone:KEY@127.0.0.1:1/track` | `… for url (http://127.0.0.1:1/track)` — userinfo already stripped |
/// | `http://127.0.0.1:1/track?key=KEY` | `… for url (http://127.0.0.1:1/track?key=KEY)` — **leaked verbatim** |
///
/// So the query string is the live leak; userinfo is not, today. Both are
/// covered here anyway, because "the dependency strips it" is not a
/// property this crate owns — it is one `cargo update` from being false,
/// and nothing here would fail when it changed. `without_url` removes the
/// URL outright, so neither shape can reach the line whatever reqwest
/// decides to print.
///
/// Asserted **case-insensitively**, with the self-check below: this guard
/// once shipped in a form that passed a deliberate leak, because the value
/// came back lowercased.
#[tokio::test]
async fn a_transport_failure_never_carries_the_endpoint_credential() {
    const SECRET: &str = "NotARealCollectorKey";
    let needle = SECRET.to_ascii_lowercase();

    // The self-check, on the shape that is measurably still leaking. A
    // guard that cannot find the needle in the **unstripped** error proves
    // nothing about the stripped one — the needle may never have been
    // there at all. If reqwest ever starts redacting query strings too,
    // this fails loudly and says so, rather than leaving a guard behind
    // that asserts nothing.
    let leaky = format!("http://127.0.0.1:1/track?key={SECRET}");
    let unstripped = send_failing(&leaky).await.to_string();
    assert!(
        unstripped.to_ascii_lowercase().contains(&needle),
        "the needle must be findable before stripping, or this guard is \
         vacuous: {unstripped}"
    );

    for endpoint in [
        // Port 1 refuses, so each of these is a real transport error rather
        // than a fabricated one.
        format!("http://someone:{SECRET}@127.0.0.1:1/track"),
        leaky.clone(),
        format!("http://someone:{SECRET}@127.0.0.1:1/track?key={SECRET}"),
    ] {
        let logged = super::http::loggable_send_error(send_failing(&endpoint).await);
        assert!(
            !logged.to_ascii_lowercase().contains(&needle),
            "the transport error leaked the collector credential from \
             {endpoint:?}: {logged}"
        );
        assert!(
            !logged.is_empty(),
            "stripping the URL must still leave the operator a reason: {logged}"
        );

        // And the destination is still named on the same line — through the
        // one redaction helper the boot line uses, not a second one.
        let named = crate::analytics::boot::loggable_endpoint(&endpoint);
        assert!(
            !named.to_ascii_lowercase().contains(&needle),
            "the endpoint field leaked it instead: {named}"
        );
        assert!(
            named.contains("127.0.0.1"),
            "the operator still has to be able to tell where it was going: {named}"
        );
    }
}

/// One real, refused request. Nothing listens on port 1.
async fn send_failing(endpoint: &str) -> reqwest::Error {
    reqwest::Client::new()
        .post(endpoint)
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect_err("nothing listens on port 1")
}

/// **A shutdown flush waits for a send already in flight.**
///
/// The periodic drain takes the whole queue before it awaits its POSTs, so
/// a flush that only inspected the queue would find it empty, return at
/// once, and let process exit cancel the request carrying the event —
/// losing telemetry precisely when the collector is slow, which is the one
/// case the graceful flush exists for.
///
/// Asserted by timing, against a collector that takes 600ms: the second
/// flush must not return before the first request completes. The threshold
/// is 300ms against a 600ms delay, so it neither trips on scheduling jitter
/// nor passes without the wait (the unserialized version returns in
/// microseconds).
#[tokio::test]
async fn a_flush_waits_for_a_send_already_in_flight() {
    let collector =
        spawn_collector_with(Duration::from_millis(600), 0, axum::http::StatusCode::OK).await;
    let env = env(&collector.url, &[(DEPLOYMENT_ENV, "hosted-tenant")]);
    let tracker = build(&resolve(Deployment::from_env(&env), &env), envelope());

    tracker.track(Event::InstanceStarted {
        companies: 1,
        storage: "fs",
        setup_complete: true,
    });

    // Stands in for the 30-second drain loop: it takes the queue and is
    // then parked on the POST.
    let first = {
        let tracker = tracker.clone();
        tokio::spawn(async move { tracker.flush().await })
    };
    // Long enough for the spawned task to take the queue and start its
    // request, short enough to be well inside the 600ms the collector takes.
    tokio::time::sleep(Duration::from_millis(150)).await;

    let started = Instant::now();
    tracker.flush().await;
    let waited = started.elapsed();

    assert!(
        waited >= Duration::from_millis(300),
        "the flush returned in {waited:?} while a send was still in flight; \
         on a real shutdown that event would be cancelled with the process"
    );

    first.await.expect("the in-flight send finished");
    assert_eq!(
        collector.hits.load(Ordering::SeqCst),
        1,
        "the event really was in flight and really did land"
    );
    collector.stop().await;
}

/// **A collector-wide status stops the drain, like a refused credential.**
///
/// `429`, `502`, `503` are the collector saying it cannot take traffic —
/// not a verdict on the body that happened to be in flight. Treating one as
/// a rejected *event* and carrying on is the worst available response: up
/// to `MAX_QUEUED` requests aimed at a service that has just said it is
/// overloaded, and again at the next `FLUSH_INTERVAL` for as long as it
/// stays down. An analytics client must not be the thing that keeps an
/// operator's own collector down.
///
/// The contrast with `a_refused_event_does_not_stop_the_drain` is the whole
/// point, and it is the same contrast a `401` draws: same collector, same
/// three events, one status code apart, opposite behaviour.
#[tokio::test]
async fn a_collector_that_cannot_take_traffic_stops_the_drain() {
    for refusal in [
        axum::http::StatusCode::TOO_MANY_REQUESTS,
        axum::http::StatusCode::BAD_GATEWAY,
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
    ] {
        let collector = spawn_collector_with(Duration::ZERO, usize::MAX, refusal).await;
        let env = env(&collector.url, &[(DEPLOYMENT_ENV, "hosted-tenant")]);
        let tracker = build(&resolve(Deployment::from_env(&env), &env), envelope());

        for _ in 0..3 {
            tracker.track(Event::InstanceStarted {
                companies: 1,
                storage: "fs",
                setup_complete: true,
            });
        }
        tracker.flush().await;

        assert_eq!(
            collector.hits.load(Ordering::SeqCst),
            1,
            "{refusal} is the collector's answer about itself, so the two events \
             behind it must not be attempted"
        );
        collector.stop().await;
    }
}

/// The control for the test above: a status that really *is* about one
/// event must still not stop the drain.
///
/// Without it, "stops the drain" would be satisfied by a client that gave
/// up on any refusal at all, which is the behaviour
/// `a_refused_event_does_not_stop_the_drain` exists to forbid. `400` and
/// `404` are the two an operator actually meets — a body OpenPanel will not
/// take, and a `/track` path typed wrong — and neither is a reason to
/// abandon the events queued behind it.
#[tokio::test]
async fn a_per_event_refusal_still_does_not_stop_the_drain() {
    for refusal in [
        axum::http::StatusCode::BAD_REQUEST,
        axum::http::StatusCode::NOT_FOUND,
    ] {
        let collector = spawn_collector_with(Duration::ZERO, usize::MAX, refusal).await;
        let env = env(&collector.url, &[(DEPLOYMENT_ENV, "hosted-tenant")]);
        let tracker = build(&resolve(Deployment::from_env(&env), &env), envelope());

        for _ in 0..3 {
            tracker.track(Event::InstanceStarted {
                companies: 1,
                storage: "fs",
                setup_complete: true,
            });
        }
        tracker.flush().await;

        assert_eq!(
            collector.hits.load(Ordering::SeqCst),
            3,
            "{refusal} is about one event, so the two behind it must still be tried"
        );
        collector.stop().await;
    }
}

/// **A drain cancelled mid-flight says how many events it lost.**
///
/// This is the shutdown budget, reproduced. `src/bin/opencompany.rs` wraps
/// the final flush in a `tokio::time::timeout` of at most two seconds
/// (`server::shutdown::flush_budget`), and OpenPanel has no batch endpoint,
/// so a queue of `n` costs `n` sequential round trips. When the budget runs
/// out the future is **dropped mid-drain**: the events already taken off the
/// queue are gone, and before `CancelledDrain` nothing in this module said
/// so — the only trace was a `debug!` at the call site naming no count.
///
/// The loss is not fixed, deliberately (see `Inner::drain` for why bounded
/// concurrency is the wrong trade against the black-hole guarantee). What
/// is fixed is the silence, so this asserts the **count**, which is the part
/// an operator can act on. Asserted on the counter rather than on a log
/// line, because a test that needs a subscriber to see a regression is a
/// test that stops seeing it the day the subscriber changes.
///
/// A 300 ms collector and a 120 ms budget: the first event is still in
/// flight when the timeout fires, so four of five are certain to be lost —
/// no timing race, because the assertion is a lower bound rather than an
/// exact count.
#[tokio::test]
async fn a_cancelled_drain_reports_the_tail_it_lost() {
    let collector =
        spawn_collector_with(Duration::from_millis(300), 0, axum::http::StatusCode::OK).await;
    // Built directly rather than through `build`, because the counter is on
    // the concrete type and `build` hands back an `Arc<dyn Tracker>`.
    let tracker = HttpOpenPanelTracker::new(
        &collector.url,
        &crate::analytics::config::ClientCredentials::new(TEST_CLIENT_ID, TEST_CLIENT_SECRET),
        envelope(),
    )
    .expect("the client builds");

    for _ in 0..5 {
        tracker.track(Event::InstanceStarted {
            companies: 1,
            storage: "fs",
            setup_complete: true,
        });
    }

    assert_eq!(
        tracker.lost_to_cancellation(),
        0,
        "nothing is lost before a drain is cancelled, or the assertion below is \
         measuring the wrong thing"
    );

    // Stands in for the shutdown budget, an order of magnitude smaller so
    // the test does not take two seconds to prove a two-second bound.
    let outcome = tokio::time::timeout(Duration::from_millis(120), tracker.flush()).await;
    assert!(
        outcome.is_err(),
        "the flush finished inside the budget, so nothing was cancelled and this \
         test proves nothing"
    );

    assert!(
        tracker.lost_to_cancellation() >= 4,
        "a cancelled drain lost {} events and reported {} — the tail of a shutdown \
         flush must be counted, not dropped in silence",
        5 - collector.hits.load(Ordering::SeqCst),
        tracker.lost_to_cancellation()
    );
    collector.stop().await;
}

/// **A loopback endpoint does not go through the system proxy.**
///
/// The loopback exception in `config::is_secure_endpoint` rests entirely on
/// the claim that such a request does not leave the host. A system proxy
/// makes that false: `reqwest`'s builder defaults to `auto_sys_proxy: true`,
/// which reads `HTTP_PROXY`/`ALL_PROXY` and takes exclusions **only** from
/// `NO_PROXY` — hyper-util 0.1.20's matcher has no implicit carve-out for
/// `localhost` or `127.0.0.0/8` (read, not assumed). So on a host with a
/// proxy configured, `http://127.0.0.1:…/track` went to the proxy in
/// cleartext with both credential headers on it, and the endpoint check
/// prevented nothing.
///
/// Two servers and one variable: a stand-in "proxy" that records anything
/// it is handed, and the real collector. With `HTTP_PROXY` pointing at the
/// first, the request must still arrive at the second. Asserting the proxy
/// saw **zero** is the security property; asserting the collector saw the
/// events is what stops that zero from being vacuous.
///
/// Mutates the process environment, so it holds the crate-wide
/// [`crate::test_support::EnvVarGuard`] — `reqwest` reads these variables
/// from the real environment at client-build time, which is the one thing
/// this crate's `MapEnv` seam cannot intercept.
#[tokio::test]
async fn a_loopback_endpoint_never_goes_through_a_system_proxy() {
    // Stands in for a corporate proxy: records every request and would be
    // the thing receiving the credential if the client honoured it.
    let proxy = spawn_collector().await;
    let collector = spawn_collector().await;

    let tracker = {
        let env = crate::test_support::EnvVarGuard::capture(&[
            "HTTP_PROXY",
            "http_proxy",
            "ALL_PROXY",
            "all_proxy",
            "NO_PROXY",
            "no_proxy",
        ]);
        // A proxy for everything, and no exclusions at all — the shape that
        // used to divert this traffic.
        env.remove("NO_PROXY");
        env.remove("no_proxy");
        env.remove("http_proxy");
        env.remove("all_proxy");
        env.set("ALL_PROXY", proxy.url.trim_end_matches("/track"));
        env.set("HTTP_PROXY", proxy.url.trim_end_matches("/track"));
        // Built inside the guard: `reqwest` samples the environment here,
        // not at send time.
        HttpOpenPanelTracker::new(
            &collector.url,
            &crate::analytics::config::ClientCredentials::new(
                TEST_CLIENT_ID,
                TEST_CLIENT_SECRET,
            ),
            envelope(),
        )
        .expect("the client builds")
    };

    for _ in 0..2 {
        tracker.track(Event::InstanceStarted {
            companies: 1,
            storage: "fs",
            setup_complete: true,
        });
    }
    tracker.flush().await;

    assert_eq!(
        proxy.hits.load(Ordering::SeqCst),
        0,
        "a loopback endpoint went through the system proxy, so the client secret \
         left the host in cleartext and the loopback exception protects nothing"
    );
    assert_eq!(
        collector.hits.load(Ordering::SeqCst),
        2,
        "and the events must still reach the collector directly, or the zero above \
         is a client that simply sent nothing"
    );
    proxy.stop().await;
    collector.stop().await;
}

/// **The transport refuses to be built for an endpoint the credential
/// cannot safely cross**, even from inside the crate.
///
/// `HttpOpenPanelTracker::new` is `pub(crate)` so that
/// [`build`] — which takes a `&Decision`, and a `Decision::Report` is what
/// `resolve` produces — is the only way to obtain a tracker. That closes the
/// route from outside. This closes the route from *inside*: a future caller
/// in this crate that reaches past `resolve` with
/// `http://collector.internal/track` would otherwise get a tracker that
/// posts the client secret across a network in cleartext, with
/// `is_cleartext` politely turning off the proxy on the way.
///
/// The assertion calls `config::is_secure_endpoint` rather than restating
/// the rule, so there is one implementation of it and no second reader to
/// drift.
#[tokio::test]
#[should_panic(expected = "the credential cannot safely cross")]
async fn the_transport_refuses_an_endpoint_that_never_passed_resolve() {
    let _ = HttpOpenPanelTracker::new(
        "http://collector.internal/track",
        &crate::analytics::config::ClientCredentials::new(TEST_CLIENT_ID, TEST_CLIENT_SECRET),
        envelope(),
    );
}

/// The control: the same construction with a loopback endpoint must be
/// accepted, or the test above would pass for a constructor that refused
/// everything.
#[tokio::test]
async fn the_transport_accepts_an_endpoint_resolve_would_have_allowed() {
    for allowed in [
        "http://127.0.0.1:9/track",
        "https://collector.invalid/track",
    ] {
        assert!(
            HttpOpenPanelTracker::new(
                allowed,
                &crate::analytics::config::ClientCredentials::new(
                    TEST_CLIENT_ID,
                    TEST_CLIENT_SECRET
                ),
                envelope(),
            )
            .is_ok(),
            "{allowed} is one resolve would allow and must still build"
        );
    }
}

/// The control: a drain that **finishes** must report nothing lost.
///
/// Without it, `lost_to_cancellation() >= 4` above would also pass for a
/// guard that fired on every drain, which would turn a real signal into a
/// line an operator learns to ignore — and this module's whole problem is
/// notices nobody reads.
#[tokio::test]
async fn a_drain_that_finishes_reports_nothing_lost() {
    let collector = spawn_collector().await;
    let tracker = HttpOpenPanelTracker::new(
        &collector.url,
        &crate::analytics::config::ClientCredentials::new(TEST_CLIENT_ID, TEST_CLIENT_SECRET),
        envelope(),
    )
    .expect("the client builds");

    for _ in 0..3 {
        tracker.track(Event::InstanceStarted {
            companies: 1,
            storage: "fs",
            setup_complete: true,
        });
    }
    tracker.flush().await;

    assert_eq!(
        collector.hits.load(Ordering::SeqCst),
        3,
        "the positive control: all three really did land"
    );
    assert_eq!(
        tracker.lost_to_cancellation(),
        0,
        "a drain that ran to completion must not report a lost tail"
    );
    collector.stop().await;
}

/// **The end-to-end check against a real OpenPanel instance.**
///
/// `#[ignore]` because it needs a collector, a credential and a network,
/// none of which CI has. Everything above proves this transport does what
/// this repository believes OpenPanel wants; only this proves OpenPanel
/// agrees. Every failure mode in this module is silent, so "the unit suite
/// is green" and "events are landing" are genuinely different claims.
///
/// ```text
/// OPENCOMPANY_ANALYTICS_ENDPOINT=https://<host>/api/track \
/// OPENCOMPANY_ANALYTICS_CLIENT_ID=<uuid> \
/// OPENCOMPANY_ANALYTICS_CLIENT_SECRET=<secret> \
///   cargo test --features analytics -- --ignored --nocapture \
///   analytics::openpanel::test::a_real_collector_accepts_an_event
/// ```
///
/// Credentials come from the environment and are never written anywhere:
/// not to a fixture, not to a log line, and not to this test's output,
/// which prints only the `profileId` it sent and the ids the collector
/// returned — enough to find the event in the dashboard and nothing more.
///
/// It asserts a `2xx` **and** that the body names a `deviceId`, because a
/// collector fronted by a proxy that swallows the request can answer `200`
/// with something else entirely, and a status-only assertion would call
/// that a pass.
#[tokio::test]
#[ignore = "needs a real OpenPanel instance and a credential from the environment"]
async fn a_real_collector_accepts_an_event() {
    use crate::app::config::EnvSource;

    let os_env = crate::app::config::ProcessEnv;
    let endpoint = os_env
        .get(ENDPOINT_ENV)
        .expect("set OPENCOMPANY_ANALYTICS_ENDPOINT");
    let credentials = match resolve(
        Deployment::HostedTenant,
        &MapEnv::new([
            (
                CLIENT_ID_ENV,
                os_env.get(CLIENT_ID_ENV).expect("set the client id"),
            ),
            (
                CLIENT_SECRET_ENV,
                os_env
                    .get(CLIENT_SECRET_ENV)
                    .expect("set the client secret"),
            ),
            (ENDPOINT_ENV, endpoint.clone()),
        ]),
    ) {
        crate::analytics::config::Decision::Report { credentials, .. } => credentials,
        other => panic!("the environment does not resolve to reporting: {other:?}"),
    };

    // A run-specific id, so the event is findable and no real instance's
    // numbers are disturbed.
    let id = OpaqueId::instance(&format!("{:032x}", crate::ports::now_millis()));
    println!("posting as profileId {}", id.as_str());

    let body = crate::analytics::payload(
        &Envelope::new(id, Deployment::HostedTenant, Cognition::default()),
        &Event::InstanceStarted {
            companies: 1,
            storage: "fs",
            setup_complete: true,
        },
    );

    let response = reqwest::Client::builder()
        .default_headers(super::http::request_headers(&credentials))
        .build()
        .expect("a client")
        .post(&endpoint)
        .json(&body)
        .send()
        .await
        .expect("the collector is reachable");

    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    assert!(
        status.is_success(),
        "the collector refused the event with {status}: {text}"
    );
    let parsed: serde_json::Value =
        serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
    assert!(
        parsed.get("deviceId").is_some(),
        "a 2xx with no deviceId is a proxy answering, not OpenPanel accepting: \
         {status} {text}"
    );
    println!("collector accepted it: {status} {text}");
}

/// **The header-safety check in `config` really is a subset of what a
/// header value accepts.**
///
/// `config::resolve` refuses a credential it judges unfit for a header, and
/// it makes that judgement in the un-gated build, where `reqwest` may not
/// even be in the dependency graph — so the rule is written by hand there
/// and is deliberately *stricter* than `HeaderValue`. That is only safe
/// while the subset claim holds: anything `resolve` accepts, the transport
/// must be able to put on the wire. Nothing else in the tree would notice
/// the day it stopped holding, so it is asserted here, in the one lane that
/// has a `HeaderValue` to compare against.
///
/// The reverse containment is deliberately **not** asserted: `HeaderValue`
/// takes space, tab and the whole `0xA0..=0xFF` range, and refusing those is
/// the point.
#[test]
fn the_header_safety_check_is_a_subset_of_what_a_header_accepts() {
    use crate::analytics::config::Decision;
    use reqwest::header::HeaderValue;

    // Every single byte, plus the multi-byte shapes a mangled secret
    // actually arrives in.
    let mut candidates: Vec<String> = (1u8..=255)
        .map(|byte| format!("ok{}", byte as char))
        .collect();
    candidates.extend(
        [
            "not-a-real-client-secret",
            "op_sk_9zQx-4Kd_7Yb2Lp0",
            "550e8400-e29b-41d4-a716-446655440000",
            "YWJjZGVmZ2hpamtsbW5vcHFyc3R1dnd4eXo=",
            "wrapped\nsecret",
            "tab\tseparated",
            "spaced out",
            "caf\u{e9}-latte",
            "\u{4f8b}\u{3048}",
        ]
        .map(str::to_string),
    );

    let mut accepted = 0usize;
    for candidate in &candidates {
        let decision = resolve(
            Deployment::HostedTenant,
            &env(
                "https://collector.invalid/track",
                &[
                    (CLIENT_ID_ENV, candidate.as_str()),
                    (CLIENT_SECRET_ENV, candidate.as_str()),
                ],
            ),
        );
        if matches!(decision, Decision::Report { .. }) {
            accepted += 1;
            assert!(
                HeaderValue::from_str(candidate.trim()).is_ok(),
                "`resolve` accepted {candidate:?}, which cannot go in a header — the \
                 subset claim in `config::is_header_safe` no longer holds"
            );
        }
    }

    // The control: the loop above would pass trivially if `resolve` had
    // started refusing everything.
    assert!(
        accepted > 50,
        "only {accepted} of {} candidates were accepted; the check has become so \
         strict that the subset assertion means nothing",
        candidates.len()
    );
}
