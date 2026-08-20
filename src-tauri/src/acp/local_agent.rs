//! `LocalAcpAgent`: the `transport = "local"` implementation of the host
//! crate's [`AcpAgent`] port (issue #1245) — a real coding CLI, spawned once
//! per declared local-acp harness and driven over stdio through the existing
//! [`AcpClient`].
//!
//! ## One process, many sessions
//!
//! A harness can serve more than one teammate, but [`AcpClient::spawn`] opens
//! one subprocess with one global update sink — ACP's `session/update`
//! notifications are not routed per caller, only tagged with the `sessionId`
//! they belong to. So this buffers every notification by `sessionId` as it
//! arrives, and a `prompt` call drains only its own session's buffer after
//! `session/prompt` returns rather than reading whatever the sink last saw.
//!
//! ## Permission requests: fails closed (deliberate, and a known gap)
//!
//! `docs/spec/runtime/harnesses.md` says an ACP agent "is still subject to
//! the company's approval policy" — this does not yet route ACP permission
//! requests through that policy gate; it refuses every one it did not
//! explicitly configure to allow, via the same [`ConfinedFiles`] the fixture
//! tests already use. That is the safe direction to be wrong in: a refused
//! edit is a visible failure the operator can act on, where a silently
//! auto-approved one would not be. Wiring ACP's `session/request_permission`
//! into `ApprovalRequestQueue` is real follow-up work, not done here.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use opencompany::Result;
use opencompany::error::OpenCompanyError;
use opencompany::ports::acp::{AcpAgent, AcpAgentFactory, AcpTurn, AcpUpdate};
use opencompany::ports::types::CompanyId;
use serde_json::Value;
use tokio::sync::Mutex as AsyncMutex;

use crate::acp::client::{AcpClient, ClientHandler, ConfinedFiles};
use crate::acp::confine::Confinement;
use crate::acp::discovery::HARNESSES;

/// Per-CLI startup model env var, confirmed live against the real adapter
/// (issue #1245's live smoke test) — not guessed. `None` means this build has
/// no known lever for that CLI: `model` is still accepted on the manifest,
/// but nothing is injected, rather than silently spawning a process that
/// ignores the setting.
fn model_env_var(agent: &str) -> Option<&'static str> {
    match agent {
        "claude" => Some("ANTHROPIC_MODEL"),
        "goose" => Some("GOOSE_MODEL"),
        // codex: no confirmed startup-model env var. Buzz (block/buzz), the
        // one other project this design is modeled on, has none either.
        _ => None,
    }
}

/// One spawned local-transport ACP harness, serving every teammate bound to
/// it.
pub struct LocalAcpAgent {
    command: &'static str,
    args: Vec<String>,
    env: Vec<(String, String)>,
    /// The company's agent-workspace root (`HarnessDeps::workspace_root`).
    /// Each session roots at `workspace_root/<company>/<agent>/workspace`,
    /// mirroring `harness::built_in::build::agent_workspace` exactly, so an
    /// ACP-run teammate's files land in the same conventional place a
    /// `built_in`-run one's would.
    workspace_root: PathBuf,
    client: AsyncMutex<Option<Arc<AcpClient>>>,
    /// `session_key` (`"{company}::{agent_id}"`) → ACP `sessionId`.
    sessions: AsyncMutex<HashMap<String, String>>,
    /// `session/update` notifications, demultiplexed by ACP `sessionId` —
    /// see the module docs for why this exists at all.
    pending_updates: Arc<StdMutex<HashMap<String, Vec<Value>>>>,
}

impl LocalAcpAgent {
    /// `agent` is one of `ACP_AGENTS` (the manifest already validated this).
    /// `model`, when set, is forwarded via that agent's own startup lever
    /// when this build knows one.
    pub fn new(agent: &str, model: Option<&str>, workspace_root: PathBuf) -> Result<Self> {
        let def = HARNESSES.iter().find(|h| h.id == agent).ok_or_else(|| {
            OpenCompanyError::Config(format!("no local ACP harness definition for `{agent}`"))
        })?;

        let mut env = Vec::new();
        if let (Some(model), Some(var)) = (model, model_env_var(agent)) {
            env.push((var.to_string(), model.to_string()));
        }

        Ok(Self {
            command: def.command,
            args: def.args.iter().map(|a| a.to_string()).collect(),
            env,
            workspace_root,
            client: AsyncMutex::new(None),
            sessions: AsyncMutex::new(HashMap::new()),
            pending_updates: Arc::new(StdMutex::new(HashMap::new())),
        })
    }

    /// The spawned client, spawning it on first call.
    async fn client(&self) -> Result<Arc<AcpClient>> {
        let mut guard = self.client.lock().await;
        if let Some(client) = guard.as_ref() {
            return Ok(client.clone());
        }

        std::fs::create_dir_all(&self.workspace_root).map_err(|error| {
            OpenCompanyError::Config(format!(
                "could not create ACP workspace root {}: {error}",
                self.workspace_root.display()
            ))
        })?;
        let confinement = Confinement::new(&self.workspace_root)
            .map_err(|error| OpenCompanyError::Config(format!("acp workspace: {error}")))?;
        // V1 fails closed — see the module docs.
        let handler: Arc<dyn ClientHandler> = Arc::new(ConfinedFiles::new(confinement, None));

        let pending = Arc::clone(&self.pending_updates);
        let sink: crate::acp::client::UpdateSink = Arc::new(move |update: Value| {
            let session_id = update["sessionId"].as_str().unwrap_or_default().to_string();
            pending
                .lock()
                .unwrap()
                .entry(session_id)
                .or_default()
                .push(update);
        });

        let args: Vec<&str> = self.args.iter().map(String::as_str).collect();
        let env: Vec<(&str, &str)> = self
            .env
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let client = AcpClient::spawn(
            self.command,
            &args,
            &self.workspace_root,
            &env,
            handler,
            sink,
        )
        .await
        .map_err(|error| {
            OpenCompanyError::Config(format!("could not start `{}`: {error}", self.command))
        })?;
        client
            .initialize()
            .await
            .map_err(|error| OpenCompanyError::Config(format!("acp initialize: {error}")))?;

        let client = Arc::new(client);
        *guard = Some(client.clone());
        Ok(client)
    }

    /// The per-(company, agent) session directory, created if it does not
    /// exist yet — mirrors `harness::built_in::build::agent_workspace`.
    fn session_root(&self, company: &CompanyId, agent_id: &str) -> Result<PathBuf> {
        let dir = self
            .workspace_root
            .join(company.as_ref())
            .join(agent_id)
            .join("workspace");
        std::fs::create_dir_all(&dir).map_err(|error| {
            OpenCompanyError::Config(format!(
                "could not create ACP session workspace {}: {error}",
                dir.display()
            ))
        })?;
        Ok(dir)
    }

    /// This session's cached ACP `sessionId`, opening one if none exists yet.
    async fn session_for(
        &self,
        client: &AcpClient,
        session_key: &str,
        root: &Path,
    ) -> Result<String> {
        let mut sessions = self.sessions.lock().await;
        if let Some(id) = sessions.get(session_key) {
            return Ok(id.clone());
        }
        let id = client
            .new_session(root)
            .await
            .map_err(|error| OpenCompanyError::Config(format!("acp session/new: {error}")))?;
        sessions.insert(session_key.to_string(), id.clone());
        Ok(id)
    }

    /// `session_key` is `"{company}::{agent_id}"` — recovers `agent_id` by
    /// stripping the company prefix, since `AcpAgent::prompt` does not carry
    /// it separately. Agent ids are `snake_case` (manifest-validated) and
    /// cannot themselves contain `::`, so this split is unambiguous.
    fn agent_id_of<'a>(company: &CompanyId, session_key: &'a str) -> &'a str {
        session_key
            .strip_prefix(company.as_ref())
            .and_then(|rest| rest.strip_prefix("::"))
            .unwrap_or(session_key)
    }
}

/// Translates one raw `session/update` notification into this crate's
/// [`AcpUpdate`], or `None` for a kind that is dropped rather than
/// approximated (`plan`, `available_commands_update`, …) — see
/// `harness::acp::run_turn`'s own module docs for the mapping table this
/// mirrors.
fn parse_update(raw: &Value) -> Option<AcpUpdate> {
    let update = raw.get("update")?;
    match update.get("sessionUpdate")?.as_str()? {
        "agent_message_chunk" => Some(AcpUpdate::MessageChunk(
            update["content"]["text"].as_str()?.to_string(),
        )),
        "agent_thought_chunk" => Some(AcpUpdate::ThoughtChunk),
        "tool_call" => Some(AcpUpdate::ToolCall {
            id: update["toolCallId"].as_str()?.to_string(),
            title: update["title"].as_str().unwrap_or_default().to_string(),
        }),
        "tool_call_update" => Some(AcpUpdate::ToolCallUpdate {
            id: update["toolCallId"].as_str()?.to_string(),
            status: update["status"].as_str().unwrap_or_default().to_string(),
            result: update
                .get("content")
                .and_then(|c| c.as_array())
                .map(|blocks| {
                    blocks
                        .iter()
                        .filter_map(|b| b["text"].as_str())
                        .collect::<Vec<_>>()
                        .join("")
                }),
        }),
        _ => None,
    }
}

#[async_trait]
impl AcpAgent for LocalAcpAgent {
    async fn prompt(
        &self,
        company: &CompanyId,
        session_key: &str,
        message: &str,
    ) -> Result<AcpTurn> {
        let client = self.client().await?;
        let agent_id = Self::agent_id_of(company, session_key);
        let root = self.session_root(company, agent_id)?;
        let session_id = self.session_for(&client, session_key, &root).await?;

        // Clear any stale buffer before the turn starts, so the drain below
        // sees exactly this turn's updates and nothing left over from one
        // that timed out or was cancelled without being read.
        self.pending_updates.lock().unwrap().remove(&session_id);

        let stop_reason = client
            .prompt(&session_id, message)
            .await
            .map_err(|error| OpenCompanyError::Config(format!("acp prompt: {error}")))?;

        let raw = self
            .pending_updates
            .lock()
            .unwrap()
            .remove(&session_id)
            .unwrap_or_default();
        let updates = raw.iter().filter_map(parse_update).collect();
        Ok(AcpTurn {
            updates,
            stop_reason,
        })
    }

    async fn cancel(&self, company: &CompanyId, session_key: &str) -> Result<()> {
        let session_id = {
            let sessions = self.sessions.lock().await;
            sessions.get(session_key).cloned()
        };
        let Some(session_id) = session_id else {
            // No session ever opened for this (company, agent) — nothing to
            // cancel, and asking a client that may not exist yet would spawn
            // one just to tell it to stop.
            return Ok(());
        };
        let client = { self.client.lock().await.clone() };
        let Some(client) = client else {
            return Ok(());
        };
        let _ = company; // carried for symmetry with `prompt`; not needed here
        client
            .cancel(&session_id)
            .await
            .map_err(|error| OpenCompanyError::Config(format!("acp cancel: {error}")))
    }
}

/// Builds a fresh [`LocalAcpAgent`] per call — no caching, matching
/// `harness::lanes::built_in_lane`'s own precedent of building a fresh pool
/// on every `RuntimeBuilder::build`. A rebuild is rare (a manifest or
/// inference-settings change), and the old agent's subprocess is killed on
/// drop (`AcpClient::kill_on_drop`), so nothing leaks.
pub struct LocalAcpAgentFactory;

impl AcpAgentFactory for LocalAcpAgentFactory {
    fn build(
        &self,
        agent: &str,
        model: Option<&str>,
        workspace_root: &Path,
    ) -> Result<Arc<dyn AcpAgent>> {
        Ok(Arc::new(LocalAcpAgent::new(
            agent,
            model,
            workspace_root.to_path_buf(),
        )?))
    }
}
