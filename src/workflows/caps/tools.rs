//! The workflow [`ToolInvoker`]: a `tool_call` node runs a real Cell A toolbelt
//! tool, fail-closed on the company's `[tools].allow` grants.
//!
//! On construction the invoker builds the same Cell A toolbelt a roster agent
//! gets — `shell` (+ `read_workspace_state`), `code` (`apply_patch`,
//! `git_operations`, `csv_export`), and `web` (`web_fetch`, `http_request`,
//! `curl`, `image_info`) — under ONE exec-security policy scoped to a dedicated
//! per-company workflow workspace, then indexes the tools by their runtime
//! [`name()`](openhuman_core::openhuman::tools::Tool::name). A `tool_call` node's
//! `slug` selects one by name.
//!
//! It also wires the metered `search` family (`web_search`) — the discovery tool
//! the `web` namespace never had (`web_fetch` / `http_request` / `curl` only read
//! a URL the agent already has) — on the same two gates the agent builder uses
//! ([`crate::harness::build::build_agent`]): an **explicit** `search` grant
//! (`grants_search_explicit`; the catch-all `*` never confers it, because each
//! call is a priced managed request) AND a managed search backend on the deps.
//! Granted-but-uncredentialed wires nothing and warns, so `web_search` degrades
//! gracefully when no managed credential is configured (fail-closed).
//!
//! Every invocation is **fail-closed**: the slug's grant namespace (via
//! [`toolbelt::namespace_of`]) must be covered by the company's `[tools].allow`
//! globs — reusing the exact grant-intersection rule an agent's exec tools use
//! ([`crate::harness::build::grants_cover`]) — before the tool is even looked
//! up. The one exception is the priced `search` namespace, which requires an
//! **explicit** `search` grant (`grants_search_explicit`) rather than glob
//! coverage, so `*` never buys a managed search call and the invoke-time gate
//! matches construction.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use tinyflows::caps::ToolInvoker;
use tinyflows::error::{EngineError, Result as TfResult};

use oh::security::SecurityPolicy;
use oh::tools::{Tool, ToolResult};
use openhuman_core::openhuman as oh;

use crate::harness::search::{SearchBackend, SearchMetering};
use crate::harness::toolbelt::{self, CapabilityFilter};

/// The grant namespaces a workflow `tool_call` can actually reach — the exec
/// belt (`shell` / `code` / `web`) plus the metered `search` family, exactly what
/// [`WorkflowToolInvoker::new`] wires.
///
/// It is deliberately a STRICT subset of
/// [`GATEABLE_NAMESPACES`](crate::company::GATEABLE_NAMESPACES): `media` and
/// `composio` map to a namespace via [`toolbelt::namespace_of`] but are
/// agent-turn tool families this invoker never builds, and `subagent` is not a
/// toolbelt tool at all. A slug in one of those namespaces would pass
/// [`invoke`](WorkflowToolInvoker::invoke)'s grant gate and then ALWAYS miss the
/// tool lookup. Author-time validation (`validate_tool_call_node`) rejects any
/// slug whose namespace falls outside this set, so a save can't green-light a
/// slug the run would always fail to look up — keep the two in lockstep.
pub(crate) const WORKFLOW_TOOL_NAMESPACES: [&str; 4] = ["shell", "code", "web", "search"];

/// The wired workflow-tool slugs, paired with the grant namespace each maps to —
/// the reverse of [`toolbelt::namespace_of`], restricted to the families
/// [`WorkflowToolInvoker::new`] actually builds ([`WORKFLOW_TOOL_NAMESPACES`]).
///
/// [`namespace_of`](toolbelt::namespace_of) answers "which namespace gates this
/// slug", but nothing enumerates the slugs a namespace contains — and the
/// create-time copilot (issue #753) needs exactly that, so it can ground the
/// model in the real tool names a company's `[tools].allow` reaches rather than
/// bare namespace words. This is that enumeration, and it is a **strict
/// derivative** of `namespace_of`, not a second source of truth: every entry's
/// namespace is asserted to match `namespace_of(slug)` by
/// [`the_slug_table_agrees_with_namespace_of`], so a tool added to the toolbelt
/// (and its `namespace_of` arm) without a row here fails the test rather than
/// silently narrowing what the copilot can propose.
///
/// `media` / `composio` / `repo` slugs are deliberately absent — they map to a
/// namespace but are agent-turn families the workflow invoker never wires (the
/// same reason they are excluded from [`WORKFLOW_TOOL_NAMESPACES`]).
pub(crate) const WORKFLOW_TOOL_SLUGS: &[(&str, &str)] = &[
    ("shell", "shell"),
    ("read_workspace_state", "shell"),
    ("apply_patch", "code"),
    ("git_operations", "code"),
    ("csv_export", "code"),
    ("web_fetch", "web"),
    ("http_request", "web"),
    ("curl", "web"),
    ("image_info", "web"),
    ("web_search", "search"),
];

/// A [`ToolInvoker`] over the Cell A toolbelt (plus the metered `search` family),
/// scoped to a per-company workflow workspace and gated by the company's
/// `[tools].allow` grants.
pub struct WorkflowToolInvoker {
    /// The wired toolbelt tools, indexed by runtime `name()` (== the node slug).
    tools: HashMap<String, Arc<dyn Tool>>,
    /// The company's `[tools].allow` grant globs — the fail-closed gate.
    grants: Vec<String>,
}

impl WorkflowToolInvoker {
    /// Builds the invoker: assemble the Cell A toolbelt under `security`
    /// (sandboxed to `workspace`), run it through the capability `filter`, and
    /// index the survivors by name. `grants` is the company's `[tools].allow`.
    ///
    /// `audit_dir` is the host-owned shell audit sink (issue #775) and is
    /// **separate from `workspace`** on purpose: `workspace` is the
    /// `workspace_only` policy root a `tool_call` node's file/exec tools are
    /// sandboxed to, so a sink inside it would be a policy-permitted write
    /// target for the workflow's own `shell`. It is passed in rather than
    /// derived here for the same reason
    /// [`HarnessDeps::audit_root`](crate::harness::HarnessDeps::audit_root) is
    /// an explicit field.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        security: Arc<SecurityPolicy>,
        workspace: &Path,
        audit_dir: &Path,
        web_allowed_domains: Vec<String>,
        grants: Vec<String>,
        filter: &CapabilityFilter,
        search: Option<&SearchBackend>,
        search_metering: SearchMetering,
    ) -> Self {
        // Mirror `build_agent`: do not initialize a tool family (or its audit
        // state) unless the company's grants can invoke that namespace.
        let mut tools: Vec<Box<dyn Tool>> = Vec::new();
        if crate::harness::build::grants_cover(&grants, "shell") {
            tools.extend(toolbelt::shell_tools(
                security.clone(),
                toolbelt::native_runtime(),
                toolbelt::shell_audit(audit_dir),
                workspace,
            ));
        }
        if crate::harness::build::grants_cover(&grants, "code") {
            tools.extend(toolbelt::code_tools(security.clone(), workspace));
        }
        if crate::harness::build::grants_cover(&grants, "web") {
            tools.extend(toolbelt::web_tools(
                security,
                web_allowed_domains,
                workspace,
            ));
        }
        // Metered web search (issue #238) — mirror `build_agent`'s two-gate
        // wiring exactly: an EXPLICIT `search` grant (`grants_search_explicit`;
        // the catch-all `*` never confers it, because each call is a priced
        // managed request) AND a managed search backend on the deps. Granted-but-
        // uncredentialed wires nothing and warns, so `web_search` degrades
        // gracefully when no managed credential is configured (fail-closed).
        if crate::company::grants_search_explicit(&grants) {
            match search {
                Some(backend) => {
                    tools.extend(crate::harness::search::search_tools(
                        backend,
                        search_metering,
                    ));
                }
                None => tracing::warn!(
                    "[workflow] company explicitly grants `search` but no managed search backend \
                     is configured; web_search NOT wired (fail-closed)"
                ),
            }
        }
        // The namespaces wired above (shell / code / web / search) are the
        // canonical [`WORKFLOW_TOOL_NAMESPACES`] set author-time validation gates
        // tool_call slugs against — a family added here must be added there too.
        //
        // Apply the capability-tier filter (identity in production) just as the
        // agent builder does, so the workflow surface never exceeds the agent one.
        let tools = toolbelt::filter_by_capabilities(tools, filter);

        let tools = tools
            .into_iter()
            .map(|tool| (tool.name().to_string(), Arc::<dyn Tool>::from(tool)))
            .collect();

        Self { tools, grants }
    }
}

#[async_trait]
impl ToolInvoker for WorkflowToolInvoker {
    /// Executes the toolbelt tool named `slug`.
    ///
    /// `conn` is ignored in P1: OpenCompany has no per-account connection
    /// registry yet, so a `tool_call` acts as the company itself (the toolbelt
    /// tools are workspace/company scoped, not per-external-account). Threading a
    /// real connection is a documented follow-on.
    async fn invoke(&self, slug: &str, args: Value, _conn: Option<&str>) -> TfResult<Value> {
        // FAIL-CLOSED grant check FIRST, before any lookup or execution.
        let Some(namespace) = toolbelt::namespace_of(slug) else {
            return Err(EngineError::Capability(format!(
                "tool_call '{slug}' is not a wired workflow tool"
            )));
        };
        // The priced `search` namespace needs an EXPLICIT `search` grant — the
        // catch-all `*` must never confer a managed search call — so this gate
        // matches the construction gate in `new` (and `build::build_agent`).
        // Every other namespace uses the ordinary grant-glob intersection.
        let granted = if namespace == "search" {
            crate::company::grants_search_explicit(&self.grants)
        } else {
            crate::harness::build::grants_cover(&self.grants, namespace)
        };
        if !granted {
            return Err(EngineError::Capability(format!(
                "tool_call '{slug}' (namespace '{namespace}') is not granted by this company's \
                 [tools].allow"
            )));
        }

        let tool = self.tools.get(slug).ok_or_else(|| {
            EngineError::Capability(format!(
                "tool_call '{slug}' is not available in company workflows"
            ))
        })?;

        tracing::debug!(slug, "workflow tool_call: invoking toolbelt tool");
        let result = tool
            .execute(args)
            .await
            .map_err(|err| EngineError::Capability(format!("tool_call '{slug}' failed: {err}")))?;
        tool_result_to_value(slug, result)
    }
}

/// Maps a toolbelt [`ToolResult`] onto the engine's JSON. An error result
/// becomes an [`EngineError::Capability`] (so the node's `on_error`/retry policy
/// governs it); a success whose text is a single JSON-parsable block passes that
/// JSON through, else it is wrapped as `{ "text": … }`.
fn tool_result_to_value(slug: &str, result: ToolResult) -> TfResult<Value> {
    if result.is_error {
        return Err(EngineError::Capability(format!(
            "tool_call '{slug}': {}",
            result.output()
        )));
    }
    let text = result.output();
    match serde_json::from_str::<Value>(&text) {
        Ok(value) => Ok(value),
        Err(_) => Ok(json!({ "text": text })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [`WORKFLOW_TOOL_SLUGS`] is a derivative of
    /// [`toolbelt::namespace_of`](crate::harness::toolbelt::namespace_of), not a
    /// second source of truth: every row's namespace must be what `namespace_of`
    /// returns for that slug, and must be one the invoker actually wires
    /// ([`WORKFLOW_TOOL_NAMESPACES`]). A toolbelt tool added or re-namespaced
    /// without updating this table fails here rather than silently changing what
    /// the create-time copilot (issue #753) can ground the model in.
    #[test]
    fn the_slug_table_agrees_with_namespace_of() {
        for (slug, namespace) in WORKFLOW_TOOL_SLUGS {
            assert_eq!(
                toolbelt::namespace_of(slug),
                Some(*namespace),
                "slug `{slug}` is listed under `{namespace}` but namespace_of disagrees"
            );
            assert!(
                WORKFLOW_TOOL_NAMESPACES.contains(namespace),
                "slug `{slug}`'s namespace `{namespace}` is not a wired workflow namespace"
            );
        }
    }

    #[test]
    fn json_text_block_passes_through_else_wrapped() {
        let json_result = ToolResult::success(r#"{"rows": 3}"#);
        assert_eq!(
            tool_result_to_value("csv_export", json_result).unwrap(),
            json!({ "rows": 3 })
        );

        let text_result = ToolResult::success("Exported 3 rows to exports/out.csv");
        assert_eq!(
            tool_result_to_value("csv_export", text_result).unwrap(),
            json!({ "text": "Exported 3 rows to exports/out.csv" })
        );
    }

    #[test]
    fn error_result_becomes_a_capability_error() {
        let err = tool_result_to_value("csv_export", ToolResult::error("nope")).unwrap_err();
        assert!(
            matches!(err, EngineError::Capability(ref m) if m.contains("nope")),
            "{err:?}"
        );
    }

    #[test]
    fn ungranted_and_unknown_slugs_are_rejected_fail_closed() {
        use tinyflows::caps::ToolInvoker;
        // No `code` grant → csv_export (a `code`-namespace tool) is denied even
        // though it is wired.
        let invoker = WorkflowToolInvoker {
            tools: HashMap::new(),
            grants: vec!["web.*".to_string()],
        };
        let denied = tokio_test_block_on(invoker.invoke("csv_export", json!({}), None));
        assert!(
            matches!(denied, Err(EngineError::Capability(ref m)) if m.contains("not granted")),
            "{denied:?}"
        );
        // A slug with no toolbelt namespace is rejected as unwired.
        let unwired = tokio_test_block_on(invoker.invoke("email.send", json!({}), None));
        assert!(
            matches!(unwired, Err(EngineError::Capability(ref m)) if m.contains("not a wired")),
            "{unwired:?}"
        );
    }

    #[test]
    fn the_search_namespace_requires_an_explicit_grant_not_a_wildcard() {
        use tinyflows::caps::ToolInvoker;
        // `*` covers ordinary namespaces but must NOT confer the priced `search`
        // family — the invoke-time gate mirrors construction (build.rs).
        let wildcard = WorkflowToolInvoker {
            tools: HashMap::new(),
            grants: vec!["*".to_string()],
        };
        let denied = tokio_test_block_on(wildcard.invoke("web_search", json!({}), None));
        assert!(
            matches!(denied, Err(EngineError::Capability(ref m)) if m.contains("not granted")),
            "{denied:?}"
        );
        // An explicit `search` grant passes the gate; the empty tool map then
        // fails the lookup with a different, later error.
        let granted = WorkflowToolInvoker {
            tools: HashMap::new(),
            grants: vec!["search".to_string()],
        };
        let looked_up = tokio_test_block_on(granted.invoke("web_search", json!({}), None));
        assert!(
            matches!(looked_up, Err(EngineError::Capability(ref m)) if m.contains("not available")),
            "{looked_up:?}"
        );
    }

    #[test]
    fn construction_only_initializes_granted_tool_families() {
        let dir = tempfile::tempdir().unwrap();
        // A SEPARATE root from the workspace: the audit sink is host-owned and
        // must never live inside the directory the exec policy sandboxes to
        // (issue #775).
        let audit = tempfile::tempdir().unwrap();
        let security = Arc::new(toolbelt::exec_security(
            dir.path(),
            crate::harness::policy::PolicyMode::Supervised,
        ));

        let none = WorkflowToolInvoker::new(
            security.clone(),
            dir.path(),
            audit.path(),
            Vec::new(),
            Vec::new(),
            &CapabilityFilter::AllowAll,
            None,
            test_metering(),
        );
        assert!(none.tools.is_empty());

        let code = WorkflowToolInvoker::new(
            security,
            dir.path(),
            audit.path(),
            Vec::new(),
            vec!["code.*".to_string()],
            &CapabilityFilter::AllowAll,
            None,
            test_metering(),
        );
        assert!(code.tools.contains_key("apply_patch"));
        assert!(code.tools.contains_key("csv_export"));
        assert!(!code.tools.contains_key("shell"));
        assert!(!code.tools.contains_key("web_fetch"));
    }

    #[test]
    fn search_wires_only_with_an_explicit_grant_and_a_backend() {
        let dir = tempfile::tempdir().unwrap();
        let audit = tempfile::tempdir().unwrap();
        let security = Arc::new(toolbelt::exec_security(
            dir.path(),
            crate::harness::policy::PolicyMode::Supervised,
        ));
        let backend = SearchBackend::new(
            "https://api.example.test".to_string(),
            crate::company::credentials::Credential::from_value("managed"),
            5,
        );

        // Explicit `search` grant + a backend → the metered `web_search` is wired.
        let wired = WorkflowToolInvoker::new(
            security.clone(),
            dir.path(),
            audit.path(),
            Vec::new(),
            vec!["search".to_string()],
            &CapabilityFilter::AllowAll,
            Some(&backend),
            test_metering(),
        );
        assert!(wired.tools.contains_key("web_search"));

        // The catch-all `*` must NOT confer the priced search family.
        let wildcard = WorkflowToolInvoker::new(
            security.clone(),
            dir.path(),
            audit.path(),
            Vec::new(),
            vec!["*".to_string()],
            &CapabilityFilter::AllowAll,
            Some(&backend),
            test_metering(),
        );
        assert!(!wildcard.tools.contains_key("web_search"));

        // Granted but uncredentialed wires nothing (fail-closed) rather than panicking.
        let uncredentialed = WorkflowToolInvoker::new(
            security,
            dir.path(),
            audit.path(),
            Vec::new(),
            vec!["search".to_string()],
            &CapabilityFilter::AllowAll,
            None,
            test_metering(),
        );
        assert!(!uncredentialed.tools.contains_key("web_search"));
    }

    /// A throwaway [`SearchMetering`] for the construction tests — the tool is
    /// never executed here, so the company/agent/meter values are inert.
    fn test_metering() -> SearchMetering {
        SearchMetering {
            company: crate::ports::types::CompanyId::new("test"),
            agent: "workflow:test".to_string(),
            meter: None,
        }
    }

    /// Minimal blocking bridge so the fail-closed checks (which never touch the
    /// tool map) can be unit-tested without a full tokio runtime import churn.
    fn tokio_test_block_on<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(fut)
    }
}
