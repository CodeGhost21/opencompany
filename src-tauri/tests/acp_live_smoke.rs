//! Live smoke test against a real, installed `claude-agent-acp` — not the
//! scripted fixture `acp_client.rs` drives.
//!
//! Requires `claude-agent-acp` on `PATH`
//! (`npm install -g @agentclientprotocol/claude-agent-acp`) and an
//! authenticated `claude` CLI (`claude auth status`). Costs real API /
//! subscription usage on every run, so this is `#[ignore]`d and never
//! selected by CI — run explicitly:
//!
//! ```text
//! cargo test -p opencompany-desktop --test acp_live_smoke -- --ignored --nocapture
//! ```
//!
//! Exists to validate, against the real adapter rather than the fixture, the
//! two assumptions issue #1245's harness-level `model` field depends on:
//! that `session/new` actually advertises a model-category config option or
//! the unstable `models` block, and that an env var set on the spawned
//! process actually steers which model that option reports as current.

use std::path::Path;
use std::sync::{Arc, Mutex};

use opencompany_desktop_lib::acp::client::{AcpClient, ClientHandler, ConfinedFiles};
use opencompany_desktop_lib::acp::confine::Confinement;
use serde_json::Value;

fn handler(root: &Path) -> Arc<dyn ClientHandler> {
    Arc::new(ConfinedFiles::new(
        Confinement::new(root).unwrap(),
        Some("yes".to_string()),
    ))
}

#[derive(Clone, Default)]
struct Updates(Arc<Mutex<Vec<Value>>>);

impl Updates {
    fn sink(&self) -> Arc<dyn Fn(Value) + Send + Sync> {
        let inner = Arc::clone(&self.0);
        Arc::new(move |value| inner.lock().unwrap().push(value))
    }
    fn said(&self) -> String {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|u| u["update"]["sessionUpdate"] == "agent_message_chunk")
            .filter_map(|u| u["update"]["content"]["text"].as_str())
            .collect::<Vec<_>>()
            .join("")
    }
}

#[tokio::test]
#[ignore = "spawns a real, authenticated claude-agent-acp and costs real usage"]
async fn a_real_claude_agent_acp_answers_a_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let updates = Updates::default();

    let client = AcpClient::spawn(
        "claude-agent-acp",
        &[],
        &root,
        &[],
        handler(&root),
        updates.sink(),
    )
    .await
    .expect(
        "claude-agent-acp must be on PATH: npm install -g @agentclientprotocol/claude-agent-acp",
    );
    client.initialize().await.expect("initialize");
    let session = client.new_session(&root).await.expect("session/new");

    let stop_reason = client
        .prompt(
            &session,
            "Reply with exactly the single word PONG and nothing else.",
        )
        .await
        .expect("prompt");
    assert_eq!(stop_reason, "end_turn", "updates were: {:?}", updates.0);

    let said = updates.said();
    assert!(said.contains("PONG"), "got: {said:?}");
}

/// Bypasses `new_session`'s narrow `sessionId`-only parsing to see the full
/// raw `session/new` response, so this can inspect `configOptions`/`models`
/// without a helper this crate doesn't have yet (that helper is #1245's job;
/// this test is what justifies building it at all).
#[tokio::test]
#[ignore = "spawns a real, authenticated claude-agent-acp and costs real usage"]
async fn session_new_advertises_a_model_config_option() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let updates = Updates::default();

    let client = AcpClient::spawn(
        "claude-agent-acp",
        &[],
        &root,
        &[],
        handler(&root),
        updates.sink(),
    )
    .await
    .expect("claude-agent-acp must be on PATH");
    client.initialize().await.expect("initialize");

    let raw = client
        .call(
            "session/new",
            serde_json::json!({ "cwd": root.display().to_string(), "mcpServers": [] }),
        )
        .await
        .expect("session/new");

    let model_option = raw["configOptions"].as_array().and_then(|opts| {
        opts.iter()
            .find(|o| o.get("category").and_then(|c| c.as_str()) == Some("model"))
    });

    assert!(
        model_option.is_some() || raw.get("models").is_some(),
        "expected a `configOptions` entry with category \"model\" or an unstable \
         `models` block in session/new's response, got: {raw:#}"
    );
}

/// The mechanism issue #1245's `LocalAcpAgent` will actually use: an env var
/// set on the spawned process, not a live ACP config-option switch. Runs the
/// adapter twice, under two different `ANTHROPIC_MODEL` values, and confirms
/// the reported "current" model differs — proof the env var is actually
/// consulted at startup, not silently ignored.
#[tokio::test]
#[ignore = "spawns a real, authenticated claude-agent-acp twice and costs real usage"]
async fn anthropic_model_env_var_steers_the_startup_model() {
    async fn current_model_id(model_env: &str) -> Option<String> {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let updates = Updates::default();
        let client = AcpClient::spawn(
            "claude-agent-acp",
            &[],
            &root,
            &[("ANTHROPIC_MODEL", model_env)],
            handler(&root),
            updates.sink(),
        )
        .await
        .expect("claude-agent-acp must be on PATH");
        client.initialize().await.expect("initialize");
        let raw = client
            .call(
                "session/new",
                serde_json::json!({ "cwd": root.display().to_string(), "mcpServers": [] }),
            )
            .await
            .expect("session/new");

        // Stable path: the configOptions entry whose category is "model" names
        // its current value's `currentValue` (not `configId`/`id`'s own spec
        // name of "value" — claude-agent-acp's real wire shape, confirmed
        // live). Unstable path: `models.currentModelId`.
        raw["configOptions"]
            .as_array()
            .and_then(|opts| {
                opts.iter()
                    .find(|o| o.get("category").and_then(|c| c.as_str()) == Some("model"))
            })
            .and_then(|opt| opt.get("currentValue").and_then(|v| v.as_str()))
            .map(str::to_string)
            .or_else(|| raw["models"]["currentModelId"].as_str().map(str::to_string))
    }

    let haiku = current_model_id("claude-haiku-4-5").await;
    let sonnet = current_model_id("claude-sonnet-4-5").await;

    assert!(
        haiku.is_some() && sonnet.is_some(),
        "session/new must report a current model id under ANTHROPIC_MODEL: \
         haiku={haiku:?} sonnet={sonnet:?}"
    );
    assert_ne!(
        haiku, sonnet,
        "ANTHROPIC_MODEL must actually steer the reported current model, not be ignored"
    );
}

/// The full `LocalAcpAgent` path, through the `AcpAgent` trait rather than
/// the raw `AcpClient` the tests above drive directly — the same seam
/// `harness::lanes::build` calls in production. Proves the whole chain: model
/// env-var injection, lazy session creation, and raw-JSON-to-`AcpUpdate`
/// parsing all work against the real adapter, not just each piece in
/// isolation.
#[tokio::test]
#[ignore = "spawns a real, authenticated claude-agent-acp and costs real usage"]
async fn local_acp_agent_answers_a_prompt_through_the_acp_agent_trait() {
    use opencompany::ports::acp::AcpAgentFactory;
    use opencompany::ports::types::CompanyId;
    use opencompany_desktop_lib::acp::LocalAcpAgentFactory;

    let dir = tempfile::tempdir().unwrap();
    let workspace_root = dir.path().canonicalize().unwrap();

    let agent = LocalAcpAgentFactory
        .build("claude", None, &workspace_root)
        .expect("claude-agent-acp must be on PATH");

    let company = CompanyId::new("acme-live-smoke");
    let turn = agent
        .prompt(
            &company,
            &format!("{}::researcher", company.as_ref()),
            "Reply with exactly the single word PONG and nothing else.",
        )
        .await
        .expect("prompt");

    assert_eq!(
        turn.stop_reason, "end_turn",
        "updates were: {:?}",
        turn.updates
    );
    let said: String = turn
        .updates
        .iter()
        .filter_map(|u| match u {
            opencompany::ports::acp::AcpUpdate::MessageChunk(text) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(said.contains("PONG"), "got: {said:?}");

    // The per-agent workspace directory was created, mirroring
    // `harness::built_in::build::agent_workspace`'s layout.
    assert!(
        workspace_root
            .join("acme-live-smoke")
            .join("researcher")
            .join("workspace")
            .is_dir()
    );
}
