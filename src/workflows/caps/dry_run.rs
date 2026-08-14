//! Stub capabilities for a **dry run** (issue #542).
//!
//! A dry run walks the *real* graph — real compile, real branch selection, real
//! item flow — but over these stubs in place of the effectful capabilities, so
//! it proves a workflow's routing and output shape without any real effect:
//! zero agent inference, zero tool/http execution.
//!
//! # Fail-closed by construction, not by remembering
//!
//! Every effectful slot is stubbed, so there is no path by which a *future*
//! node kind could reach a real effect in a dry run: the engine only ever calls
//! what is on the bundle, and on a dry bundle every effectful entry is one of
//! these. A stub that forgot to exist would be a compile error at
//! [`build_capabilities`](super::build_capabilities), not a silent live effect.
//!
//! # What is NOT stubbed, and why
//!
//! The read-only, effect-free capabilities stay real:
//!
//! * the **resolver** ([`StoreWorkflowResolver`](super::resolver)) — resolving a
//!   `sub_workflow` child is a read, and the child runs under this same dry
//!   bundle, so a dry run propagates into sub-workflows rather than stopping at
//!   the boundary;
//! * **state** is the inert [`NoopState`](super::state::NoopState) — never the
//!   durable [`CompanyStateStore`](super::state::CompanyStateStore), so a dry
//!   run cannot persist run state either;
//! * `llm` / `code` / `memory` are unchanged (already unwired stubs / `None`).
//!
//! # The grant check is kept, deliberately
//!
//! [`DryRunTools`] still runs the fail-closed `[tools].allow` grant check before
//! returning its canned echo. The check is pure — it reads the company's grants
//! and touches nothing outside the process — so keeping it means a `tool_call`
//! the company does not grant is refused in a dry run *exactly* as it is live.
//! A test run is meant to prove routing, and "this node would have been denied"
//! is part of the routing.

use async_trait::async_trait;
use serde_json::{Value, json};
use tinyflows::caps::{AgentRunner, HttpClient, ToolInvoker};
use tinyflows::error::{EngineError, Result as TfResult};

use super::tools::{WorkflowToolWiring, refusal_for};

/// A marker key set on every dry-stub output, so a downstream node (or a test)
/// can tell a stubbed item from a real one.
pub(super) const DRY_RUN_MARKER: &str = "dry_run";

/// The [`AgentRunner`] a dry run wires in place of
/// [`HarnessAgentRunner`](super::HarnessAgentRunner): it returns a structured
/// echo of what the node *would* have asked, with **zero** pool routing and zero
/// inference.
///
/// The reply mirrors the real runner's `{ text, agent_ref }` envelope shape so a
/// downstream `=item.text` binding still resolves — a dry run must exercise the
/// same routing the real one would — plus the [`DRY_RUN_MARKER`] so nothing
/// mistakes the fixture for a real turn.
pub(super) struct DryRunAgent;

#[async_trait]
impl AgentRunner for DryRunAgent {
    async fn run_agent(
        &self,
        agent_ref: &str,
        request: Value,
        _conn: Option<&str>,
    ) -> TfResult<Value> {
        // Same extraction the real runner uses, so the fixture echoes exactly
        // the instruction the live turn would have received.
        let instruction = super::message_from_request(&request);
        tracing::debug!(
            agent = agent_ref,
            "workflow dry run: stubbing agent node (no inference)"
        );
        Ok(json!({
            "text": format!("[dry run] agent `{agent_ref}` would run: {instruction}"),
            "agent_ref": agent_ref,
            "instruction": instruction,
            DRY_RUN_MARKER: true,
        }))
    }
}

/// The [`ToolInvoker`] a dry run wires in place of
/// [`WorkflowToolInvoker`](super::tools::WorkflowToolInvoker): it keeps the
/// exact same fail-closed grant gate — so an ungranted `tool_call` refuses
/// identically in a dry run — but returns a canned echo instead of executing the
/// tool, so nothing touches the workspace, the network, or a priced backend.
pub(super) struct DryRunTools {
    /// The company's `[tools].allow` grant globs — the same gate the live
    /// invoker applies, reused verbatim.
    grants: Vec<String>,
    wiring: WorkflowToolWiring,
}

impl DryRunTools {
    /// Builds a dry invoker gated by the company's `[tools].allow`.
    pub(super) fn new(grants: Vec<String>, wiring: WorkflowToolWiring) -> Self {
        Self { grants, wiring }
    }
}

#[async_trait]
impl ToolInvoker for DryRunTools {
    async fn invoke(&self, slug: &str, args: Value, _conn: Option<&str>) -> TfResult<Value> {
        // Issue #846: the replay arm, mirrored from the live invoker.
        //
        // A dry run cannot reach here through the host's own path — dry runs are
        // never continuations, park no gate and stub every effect — so this is
        // not load-bearing today. It is here because the alternative is worse
        // than redundant: without it, a graph carrying a replay slug would fall
        // through to `namespace_of` and fail the node with "not a wired workflow
        // tool", which is a dry run reporting a routing failure that the real run
        // does not have. The two invokers agreeing about every slug is the
        // property a test run's answer is worth anything for.
        if let Some(result) = super::super::replay::replayed_result(slug, &args) {
            return Ok(result);
        }
        // FAIL-CLOSED grant check FIRST, identical to the live invoker
        // (`WorkflowToolInvoker::invoke`): a dry run must refuse an ungranted
        // tool exactly as a real one does, because that refusal is part of the
        // routing a test run exists to prove. The check is pure — no effect.
        if let Some(message) = refusal_for(slug, &self.grants, &self.wiring) {
            return Err(EngineError::Capability(message));
        }
        tracing::debug!(slug, "workflow dry run: stubbing tool_call (not executed)");
        Ok(json!({
            "text": format!("[dry run] tool_call `{slug}` was not executed"),
            "slug": slug,
            "args": args,
            DRY_RUN_MARKER: true,
        }))
    }
}

/// The [`HttpClient`] a dry run wires in place of
/// [`GuardedHttpClient`](super::http::GuardedHttpClient): it echoes the request
/// descriptor back without issuing anything, so no outbound request (guarded or
/// not) leaves the process.
pub(super) struct DryRunHttp;

#[async_trait]
impl HttpClient for DryRunHttp {
    async fn request(&self, request: Value, _conn: Option<&str>) -> TfResult<Value> {
        tracing::debug!("workflow dry run: stubbing http_request (not sent)");
        Ok(json!({
            "status": Value::Null,
            "body": "[dry run] http_request was not sent",
            "request": request,
            DRY_RUN_MARKER: true,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dry_agent_echoes_without_inference() {
        let out = DryRunAgent
            .run_agent("ceo", json!({ "prompt": "ship it" }), None)
            .await
            .expect("dry agent never fails");
        assert_eq!(out[DRY_RUN_MARKER], json!(true));
        assert_eq!(out["agent_ref"], "ceo");
        // The `text` envelope is preserved so a downstream `=item.text` resolves.
        assert!(out["text"].as_str().unwrap().contains("ship it"), "{out}");
    }

    #[tokio::test]
    async fn dry_tools_keep_the_grant_gate_but_do_not_execute() {
        // No `code` grant → a `code`-namespace slug is refused, exactly as live.
        let ungranted = DryRunTools::new(
            vec!["web.*".to_string()],
            WorkflowToolWiring {
                wired_namespaces: ["web"].into_iter().collect(),
                ..WorkflowToolWiring::default()
            },
        );
        let denied = ungranted.invoke("csv_export", json!({}), None).await;
        assert!(
            matches!(denied, Err(EngineError::Capability(ref m)) if m.contains("not granted")),
            "{denied:?}"
        );
        // An unknown slug is refused as unwired, exactly as live.
        let unwired = ungranted.invoke("email.send", json!({}), None).await;
        assert!(
            matches!(unwired, Err(EngineError::Capability(ref m)) if m.contains("not a wired")),
            "{unwired:?}"
        );
        // A granted slug returns the canned echo — no execution.
        let granted = DryRunTools::new(
            vec!["code.*".to_string()],
            WorkflowToolWiring {
                wired_namespaces: ["code"].into_iter().collect(),
                ..WorkflowToolWiring::default()
            },
        );
        let echoed = granted
            .invoke("csv_export", json!({ "filename": "x.csv" }), None)
            .await
            .expect("granted dry tool echoes");
        assert_eq!(echoed[DRY_RUN_MARKER], json!(true));
        assert_eq!(echoed["slug"], "csv_export");
    }

    #[tokio::test]
    async fn dry_tools_refuse_granted_search_without_a_backend() {
        let dry = DryRunTools::new(
            vec!["search".to_string()],
            WorkflowToolWiring {
                missing: [(
                    "search",
                    super::super::tools::MissingReason::SearchBackendNotConfigured,
                )]
                .into_iter()
                .collect(),
                ..WorkflowToolWiring::default()
            },
        );
        let refused = dry.invoke("web_search", json!({}), None).await;
        assert!(
            matches!(refused, Err(EngineError::Capability(ref message))
                if message.contains("no managed search backend")
                    && message.contains("ask the platform operator")),
            "{refused:?}"
        );
    }

    #[tokio::test]
    async fn dry_http_echoes_without_sending() {
        let out = DryRunHttp
            .request(json!({ "url": "http://127.0.0.1:9/" }), None)
            .await
            .expect("dry http never sends");
        assert_eq!(out[DRY_RUN_MARKER], json!(true));
        assert_eq!(out["status"], Value::Null);
    }
}
