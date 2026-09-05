//! Offline tests for the CortexDB adapter, against an in-process axum mock
//! speaking the wire shapes this driver relies on — no real CortexDB
//! instance, no network.

use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};
use tinymemory_api::traits::Memory;
use tinymemory_api::types::{MemoryCategory, RecallOpts};

use super::{CORTEXDB_DRIVER_ID, CortexdbMemory};

/// One event the mock accepted, keyed the same way CortexDB's own scope
/// grammar would file it.
#[derive(Clone)]
struct StoredEvent {
    scope: String,
    id: String,
    content: Value,
    observed_at: String,
}

/// State shared by the mock's handlers.
#[derive(Default)]
struct MockState {
    events: Mutex<Vec<StoredEvent>>,
    /// The only (token, actor) pair the mock accepts; anything else is a 401,
    /// exactly like a real CortexDB instance.
    valid_token: String,
    valid_actor: String,
    next_id: Mutex<u64>,
}

const TOKEN: &str = "test-token";
const ACTOR: &str = "opencompany-test";

/// Whether the request carries the one accepted credential pair.
fn authorized(headers: &HeaderMap, state: &MockState) -> bool {
    let bearer = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let actor = headers.get("x-cortex-actor").and_then(|v| v.to_str().ok());
    bearer == Some(state.valid_token.as_str()) && actor == Some(state.valid_actor.as_str())
}

async fn ready(headers: HeaderMap, State(state): State<Arc<MockState>>) -> StatusCode {
    if authorized(&headers, &state) {
        StatusCode::OK
    } else {
        StatusCode::UNAUTHORIZED
    }
}

async fn experience(
    headers: HeaderMap,
    State(state): State<Arc<MockState>>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    if !authorized(&headers, &state) {
        return (StatusCode::UNAUTHORIZED, Json(json!({"error": "unauthorized"})));
    }
    let scope = body
        .get("scope")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let content = body.get("content").cloned().unwrap_or(Value::Null);
    let observed_at = body
        .pointer("/context/observed_at")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut id_guard = state.next_id.lock().unwrap();
    *id_guard += 1;
    let id = format!("evt_{}", *id_guard);
    drop(id_guard);
    state.events.lock().unwrap().push(StoredEvent {
        scope,
        id: id.clone(),
        content,
        observed_at,
    });
    (
        StatusCode::OK,
        Json(json!({ "event_id": id, "duplicate": false })),
    )
}

async fn recall(
    headers: HeaderMap,
    State(state): State<Arc<MockState>>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    if !authorized(&headers, &state) {
        return (StatusCode::UNAUTHORIZED, Json(json!({"error": "unauthorized"})));
    }
    let scope = body
        .get("scope")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let events = state.events.lock().unwrap();
    let items: Vec<Value> = events
        .iter()
        .filter(|event| event.scope == scope)
        .map(|event| {
            json!({
                "id": event.id,
                "content": event.content,
                "observed_at": event.observed_at,
            })
        })
        .collect();
    (
        StatusCode::OK,
        Json(json!({ "layers": { "events": items } })),
    )
}

async fn forget(
    headers: HeaderMap,
    State(state): State<Arc<MockState>>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    if !authorized(&headers, &state) {
        return (StatusCode::UNAUTHORIZED, Json(json!({"error": "unauthorized"})));
    }
    let Some(id) = body.pointer("/selector/id").and_then(Value::as_str) else {
        return (StatusCode::OK, Json(json!({ "removed": 0 })));
    };
    let mut events = state.events.lock().unwrap();
    let before = events.len();
    events.retain(|event| event.id != id);
    let removed = before - events.len();
    (StatusCode::OK, Json(json!({ "removed": removed })))
}

/// Serves the mock on loopback and returns its base URL plus the shared state.
async fn spawn_mock(valid_actor: &str) -> (String, Arc<MockState>) {
    let state = Arc::new(MockState {
        events: Mutex::new(Vec::new()),
        valid_token: TOKEN.to_string(),
        valid_actor: valid_actor.to_string(),
        next_id: Mutex::new(0),
    });
    let app = Router::new()
        .route("/v1/admin/ready", get(ready))
        .route("/v1/experience", post(experience))
        .route("/v1/recall", post(recall))
        .route("/v1/forget", post(forget))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}"), state)
}

fn client(base_url: &str, actor: &str) -> CortexdbMemory {
    CortexdbMemory::new(base_url, TOKEN, actor).expect("valid config")
}

#[tokio::test]
async fn name_is_the_driver_id() {
    let (base_url, _state) = spawn_mock(ACTOR).await;
    assert_eq!(client(&base_url, ACTOR).name(), CORTEXDB_DRIVER_ID);
}

#[tokio::test]
async fn store_then_recall_round_trips() {
    let (base_url, _state) = spawn_mock(ACTOR).await;
    let memory = client(&base_url, ACTOR);

    memory
        .store("company-a", "greeting", "hello there", MemoryCategory::Core, None)
        .await
        .expect("store succeeds");

    let hits = memory
        .recall(
            "hello",
            10,
            RecallOpts {
                namespace: Some("company-a"),
                ..Default::default()
            },
        )
        .await
        .expect("recall succeeds");

    assert_eq!(hits.len(), 1, "expected exactly one recalled entry: {hits:?}");
    assert_eq!(hits[0].key, "greeting");
    assert_eq!(hits[0].content, "hello there");
    assert_eq!(hits[0].category, MemoryCategory::Core);

    let fetched = memory
        .get("company-a", "greeting")
        .await
        .expect("get succeeds")
        .expect("entry exists");
    assert_eq!(fetched.content, "hello there");
}

#[tokio::test]
async fn a_second_store_under_the_same_key_replaces_the_first_on_read() {
    let (base_url, _state) = spawn_mock(ACTOR).await;
    let memory = client(&base_url, ACTOR);

    memory
        .store("company-a", "note", "first", MemoryCategory::Core, None)
        .await
        .expect("first store succeeds");
    // Ensure a distinct observed_at so "most recent" is unambiguous even at
    // millisecond resolution.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    memory
        .store("company-a", "note", "second", MemoryCategory::Core, None)
        .await
        .expect("second store succeeds");

    let fetched = memory
        .get("company-a", "note")
        .await
        .expect("get succeeds")
        .expect("entry exists");
    assert_eq!(fetched.content, "second");
}

#[tokio::test]
async fn two_companies_never_share_recall() {
    let (base_url, _state) = spawn_mock(ACTOR).await;
    let memory = client(&base_url, ACTOR);

    memory
        .store("company-a", "secret", "company A's secret", MemoryCategory::Core, None)
        .await
        .expect("store for company-a succeeds");
    memory
        .store("company-b", "secret", "company B's secret", MemoryCategory::Core, None)
        .await
        .expect("store for company-b succeeds");

    let a_hits = memory
        .recall(
            "secret",
            10,
            RecallOpts {
                namespace: Some("company-a"),
                ..Default::default()
            },
        )
        .await
        .expect("recall succeeds");
    assert_eq!(a_hits.len(), 1);
    assert_eq!(a_hits[0].content, "company A's secret");

    let b_hits = memory
        .recall(
            "secret",
            10,
            RecallOpts {
                namespace: Some("company-b"),
                ..Default::default()
            },
        )
        .await
        .expect("recall succeeds");
    assert_eq!(b_hits.len(), 1);
    assert_eq!(b_hits[0].content, "company B's secret");
}

#[tokio::test]
async fn an_actor_mismatch_is_a_surfaced_error_not_a_silent_empty_result() {
    let (base_url, _state) = spawn_mock(ACTOR).await;
    // The mock only accepts `ACTOR`; this client claims a different one, so
    // every request is a 401.
    let memory = client(&base_url, "someone-else");

    let error = memory
        .store("company-a", "k", "v", MemoryCategory::Core, None)
        .await
        .expect_err("a 401 must surface as Err, not as a quiet no-op");
    let message = error.to_string();
    assert!(
        message.contains("actor") || message.to_ascii_lowercase().contains("unauthorized"),
        "error should name the auth failure: {message}"
    );

    // Read paths must not swallow it into "nothing found" either.
    let error = memory
        .get("company-a", "k")
        .await
        .expect_err("a 401 on recall must also surface as Err");
    assert!(!error.to_string().is_empty());
}

#[tokio::test]
async fn forget_removes_the_stored_event() {
    let (base_url, _state) = spawn_mock(ACTOR).await;
    let memory = client(&base_url, ACTOR);

    memory
        .store("company-a", "temp", "throwaway", MemoryCategory::Daily, None)
        .await
        .expect("store succeeds");
    assert!(memory
        .get("company-a", "temp")
        .await
        .expect("get succeeds")
        .is_some());

    let removed = memory
        .forget("company-a", "temp")
        .await
        .expect("forget succeeds");
    assert!(removed, "forget should report the record was removed");

    assert!(memory
        .get("company-a", "temp")
        .await
        .expect("get succeeds")
        .is_none());
}

#[tokio::test]
async fn forget_of_an_absent_key_is_false_not_an_error() {
    let (base_url, _state) = spawn_mock(ACTOR).await;
    let memory = client(&base_url, ACTOR);
    let removed = memory
        .forget("company-a", "never-stored")
        .await
        .expect("forget of an absent key does not error");
    assert!(!removed);
}

#[tokio::test]
async fn health_probe_reports_ready() {
    let (base_url, _state) = spawn_mock(ACTOR).await;
    let memory = client(&base_url, ACTOR);
    assert!(memory.health_check().await);
}

#[tokio::test]
async fn health_probe_reports_down_on_actor_mismatch() {
    let (base_url, _state) = spawn_mock(ACTOR).await;
    let memory = client(&base_url, "wrong-actor");
    assert!(!memory.health_check().await);
    match memory.health_probe().await {
        Some(tinymemory_api::health::MemoryHealth::Down { .. }) => {}
        other => panic!("expected Down, got {other:?}"),
    }
}

/// The bind-time capability audit — the same one `open_driver` runs in
/// production — passes for this driver: `MemoryTraitProvider` derives its
/// advertised capabilities from its accessors, and this driver implements the
/// mandatory `Memory` trait in full, so the two can never disagree.
#[tokio::test]
async fn the_bind_time_capability_audit_passes() {
    use crate::store::memory::driver::{MemoryDriverConfig, MemoryMode, RemoteDeployment, open_driver};

    let (base_url, _state) = spawn_mock("opencompany").await;
    // SAFETY (test-only): OPENCOMPANY_MEMORY_ACTOR is read once inside
    // `open_driver`, and this test does not run concurrently with another
    // that reads the same variable within this crate's cortexdb driver path.
    // Not set here: the default actor is "opencompany", matching the mock.
    let config = MemoryDriverConfig {
        mode: MemoryMode::Remote,
        driver_id: Some(CORTEXDB_DRIVER_ID.to_string()),
        url: Some(base_url),
        api_key: Some(TOKEN.to_string()),
        data_dir: None,
        deployment: RemoteDeployment::SelfHosted,
    };
    let (provider, class) = open_driver(&config)
        .expect("cortexdb must bind")
        .expect("a driver_id was named, so this is not the store-mode None");
    assert_eq!(provider.driver_id(), CORTEXDB_DRIVER_ID);
    assert_eq!(class, tinymemory::registry::DriverClass::External);
    // `open_driver` itself runs `audit_provider` before returning; getting a
    // provider back at all is the audit already having passed. Assert it a
    // second time explicitly, since that is exactly what this test is for.
    tinymemory_api::provider::audit_provider(provider.as_ref())
        .expect("advertised capabilities must match the implemented surface");
}

/// Empty helper kept next to the map import so the compiler does not flag it
/// as unused when this file's test set changes shape.
#[allow(dead_code)]
fn _unused_map_marker() -> HashMap<(), ()> {
    HashMap::new()
}
