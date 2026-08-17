//! Issue #782 — end-to-end proof that an upstream node's output reaches a
//! downstream **agent** node's turn.
//!
//! # Why a unit test could not have caught this
//!
//! Every part worked in isolation: `translate` lowered an agent node, the engine
//! resolved config `=`-expressions, and the runner composed a turn message. What
//! was missing was the *join* — an agent node lowered to only a static `prompt`,
//! so an upstream step's output had **no channel** into the next agent's turn and
//! was silently dropped. A `trigger -> agent(analyst) -> agent(copywriter)`
//! pipeline ran the copywriter with its authored instruction and nothing else, so
//! it truthfully reported that "nothing has been pasted into our conversation".
//!
//! So this drives the **real** path — real graph, real `translate`, real
//! `run_workflow`, real `HarnessAgentRunner`, real `HarnessPool` — and stubs only
//! the model, via a content-aware OpenAI-compatible endpoint on loopback (the
//! shape [`gated_tool_turn_test`](super::gated_tool_turn_test) established). The
//! endpoint answers by *which node is calling* (each node's prompt carries a
//! marker), so the assertions do not depend on the order sibling branches run in.

use std::sync::{Arc, Mutex};

use axum::Json;
use axum::routing::post;
use serde_json::{Value, json};

use crate::company::{CompanyManifest, parse_workflow};
use crate::harness::HarnessPool;
use crate::ports::WorkflowRunContext;
use crate::ports::types::{CompanyId, CompanyRecord};

use super::gated_tool_turn_test::deps;

/// A company whose roster carries **distinct** teammates for every node, so no
/// two agent nodes share a session.
///
/// This is load-bearing for what these tests prove. The harness keeps one live
/// session per roster agent, and history survives across a session's turns — so
/// if two nodes ran the *same* teammate, the downstream turn would see the
/// upstream reply through **conversation history** whether or not #782's binding
/// exists, and the test would pass even with the bug present. Distinct teammates
/// (which is also the real bug's shape: an `analyst -> copywriter` hand-off is
/// two different people) means the only path from one node's output to the next
/// is the `input = "=items"` fold under test.
fn record_with_agents(agent_ids: &[&str]) -> CompanyRecord {
    let mut src = String::from("[company]\nname = \"Acme\"\n\n[policy]\nmode = \"full\"\n");
    for id in agent_ids {
        src.push_str(&format!(
            "\n[[agent]]\nid = \"{id}\"\nrole = \"Worker\"\ntier = \"orchestrator\"\n"
        ));
    }
    let manifest: CompanyManifest = toml::from_str(&src).expect("manifest parses");
    CompanyRecord {
        id: CompanyId::new("acme"),
        manifest,
        ledger: Vec::new(),
        lifecycle: "running".to_string(),
        overlay_agents: Vec::new(),
        overlay_desk_members: Vec::new(),
        overlay_desk_order: Vec::new(),
        overlay_desks: Vec::new(),
        overlay_workflows: Vec::new(),
        overlay_budgets: Vec::new(),
        overlay_policy: None,
        overlay_desk_tools: Default::default(),
        disabled_workflows: Vec::new(),
        template_provenance: None,
    }
}

/// A model endpoint that answers by which node is calling. Each agent node's
/// prompt carries a distinctive marker (`NODE_A_PROMPT` / `NODE_A2_PROMPT` /
/// `NODE_B_PROMPT`); the reply for A/A2 is a marker a downstream turn must carry,
/// and B's reply is inert. Order-independent by construction: the answer is a
/// function of the request, not a position in a script.
///
/// The recorded request bodies (`seen`) are where a test reads what a given node
/// was actually sent — the composed turn message rides in the `messages` array,
/// so a node that received its predecessor's output shows that output here.
struct Endpoint {
    seen: Mutex<Vec<Value>>,
}

/// Serves the content-aware endpoint on loopback, returning its base URL and the
/// shared handle so a test can read what each node's turn was sent.
async fn spawn_endpoint() -> (String, Arc<Endpoint>) {
    let endpoint = Arc::new(Endpoint {
        seen: Mutex::new(Vec::new()),
    });
    let handle = Arc::clone(&endpoint);
    let app = axum::Router::new().route(
        "/chat/completions",
        post(move |Json(body): Json<Value>| {
            let endpoint = Arc::clone(&endpoint);
            async move {
                let blob = serde_json::to_string(&body).unwrap_or_default();
                endpoint.seen.lock().unwrap().push(body);
                // `NODE_A2_PROMPT` is checked before `NODE_A_PROMPT` — the latter
                // is not a substring of the former, but ordering keeps the intent
                // obvious. B's request carries the A/A2 *outputs* (the markers
                // below), never their prompts, so it falls through to the inert
                // reply.
                let content = if blob.contains("NODE_A2_PROMPT") {
                    "A2_OUTPUT_MARKER"
                } else if blob.contains("NODE_A_PROMPT") {
                    "A_OUTPUT_MARKER"
                } else {
                    "B_INERT_REPLY"
                };
                Json(json!({
                    "choices": [{
                        "index": 0,
                        "message": { "role": "assistant", "content": content }
                    }],
                    "usage": { "prompt_tokens": 8, "completion_tokens": 2 }
                }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}"), handle)
}

/// The request body a node's turn produced, found by the marker in its prompt.
/// Panics with the full transcript if no such turn was recorded — a node that
/// never ran is a more useful failure than a silent `None`.
fn turn_containing(seen: &[Value], marker: &str) -> String {
    seen.iter()
        .map(|body| serde_json::to_string(body).unwrap_or_default())
        .find(|blob| blob.contains(marker))
        .unwrap_or_else(|| panic!("no model turn carried {marker}; saw: {seen:?}"))
}

/// The headline: `trigger -> agent(A) -> agent(B)`. A emits text; B's turn must
/// carry it. Before #782 B saw only its own authored prompt.
#[tokio::test]
async fn a_downstream_agent_receives_the_upstream_agents_output() {
    const GRAPH: &str = r#"
id = "pipeline"
name = "Pipeline"
[[node]]
id = "start"
kind = "trigger"
name = "Start"
[[node]]
id = "analyst"
kind = "agent"
name = "Analyst"
summary = "NODE_A_PROMPT"
agent = "analyst"
[[node]]
id = "copywriter"
kind = "agent"
name = "Copywriter"
summary = "NODE_B_PROMPT"
agent = "copywriter"
[[edge]]
from = "start"
to = "analyst"
[[edge]]
from = "analyst"
to = "copywriter"
"#;

    let dir = tempfile::tempdir().unwrap();
    let (base_url, endpoint) = spawn_endpoint().await;
    let (deps, _journal) = deps(base_url, dir.path());
    let record = record_with_agents(&["analyst", "copywriter"]);
    let pool = Arc::new(HarnessPool::new());
    pool.ensure(&record, &deps).await.expect("roster builds");

    let file = parse_workflow(GRAPH).expect("graph parses");
    super::runner::run_workflow(
        pool,
        deps,
        &record,
        &file,
        json!({ "request": "launch the product" }),
        &WorkflowRunContext::new(false),
    )
    .await
    .expect("the pipeline runs to completion");

    let seen = endpoint.seen.lock().unwrap().clone();
    // The copywriter's turn is the one carrying its own prompt marker.
    let copywriter_turn = turn_containing(&seen, "NODE_B_PROMPT");
    assert!(
        copywriter_turn.contains("A_OUTPUT_MARKER"),
        "the copywriter's turn must carry the analyst's output — the exact channel #782 restores: {copywriter_turn}"
    );
    assert!(
        copywriter_turn.contains("Input from the previous step"),
        "…delivered under the labelled heading: {copywriter_turn}"
    );
}

/// Fan-in: `A, A2 -> merge -> agent(B)`. BOTH predecessors' output must reach B —
/// the `=items` (whole set) binding is what stops a fan-in from losing all but
/// the first.
#[tokio::test]
async fn a_fan_in_agent_receives_every_predecessors_output() {
    const GRAPH: &str = r#"
id = "fanin"
name = "Fan-in"
[[node]]
id = "start"
kind = "trigger"
name = "Start"
[[node]]
id = "a"
kind = "agent"
name = "A"
summary = "NODE_A_PROMPT"
agent = "research_a"
[[node]]
id = "a2"
kind = "agent"
name = "A2"
summary = "NODE_A2_PROMPT"
agent = "research_b"
[[node]]
id = "combine"
kind = "merge"
name = "Combine"
[[node]]
id = "b"
kind = "agent"
name = "B"
summary = "NODE_B_PROMPT"
agent = "editor"
[[edge]]
from = "start"
to = "a"
[[edge]]
from = "start"
to = "a2"
[[edge]]
from = "a"
to = "combine"
[[edge]]
from = "a2"
to = "combine"
[[edge]]
from = "combine"
to = "b"
"#;

    let dir = tempfile::tempdir().unwrap();
    let (base_url, endpoint) = spawn_endpoint().await;
    let (deps, _journal) = deps(base_url, dir.path());
    let record = record_with_agents(&["research_a", "research_b", "editor"]);
    let pool = Arc::new(HarnessPool::new());
    pool.ensure(&record, &deps).await.expect("roster builds");

    let file = parse_workflow(GRAPH).expect("graph parses");
    super::runner::run_workflow(
        pool,
        deps,
        &record,
        &file,
        json!({ "request": "combine the research" }),
        &WorkflowRunContext::new(false),
    )
    .await
    .expect("the fan-in runs to completion");

    let seen = endpoint.seen.lock().unwrap().clone();
    let b_turn = turn_containing(&seen, "NODE_B_PROMPT");
    assert!(
        b_turn.contains("A_OUTPUT_MARKER"),
        "the first predecessor's output must reach the fan-in agent: {b_turn}"
    );
    assert!(
        b_turn.contains("A2_OUTPUT_MARKER"),
        "the second predecessor's output must too — a fan-in must not lose all but the first: {b_turn}"
    );
}
