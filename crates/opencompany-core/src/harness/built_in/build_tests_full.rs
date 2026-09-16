use super::*;

#[tokio::test]
async fn memory_tools_are_wired_to_the_company_context_store() {
    // The flip of the old withholding lock. The doc comment on
    // `memory_tools` demanded that whatever un-withholds these must first
    // confirm each company's own `ContextStore` genuinely backs them — so
    // that is exactly what this asserts: a store through the tool lands in
    // THIS company's context rows, under the agent's own label prefix,
    // reachable by the same port the memory_loop and the Brain view read.
    use crate::ports::ContextStore;
    use crate::ports::types::CompanyId;
    use std::sync::Arc;

    let dir = tempfile::tempdir().expect("tempdir");
    let context: Arc<dyn ContextStore> =
        Arc::new(crate::store::FsContextStore::new(dir.path().to_path_buf()));
    let company = CompanyId::new("acme");
    let tools = super::super::memory_tools::memory_tools(
        context.clone(),
        company.clone(),
        "ceo".to_string(),
    );
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert_eq!(names, ["memory_store", "memory_recall", "memory_forget"]);

    let store = &tools[0];
    let reply = store
        .execute(serde_json::json!({"title": "Pin", "body": "the fact"}))
        .await
        .expect("execute");
    assert!(!reply.is_error, "{reply:?}");
    let rows = context.list(&company, "agent-memory/ceo/").await.unwrap();
    assert_eq!(
        rows.len(),
        1,
        "the tool write must land on the company port"
    );
    assert_eq!(rows[0].label, "agent-memory/ceo/pin");
}

/// The grant alone is not enough: a wired `shell`/`code` namespace the
/// capability tier denies must not be described in the sandbox brief,
/// because `filter_by_capabilities` is about to strip the matching tools
/// from the vector handed to the builder. This is the fix for the P1
/// codex found on PR #1670 — before it, `sandbox_brief_flags` did not
/// exist and the brief was built from the grant flags alone.
#[test]
fn sandbox_brief_flags_withhold_a_capability_denied_namespace() {
    use std::collections::HashSet;

    let deny_shell = toolbelt::CapabilityFilter::DenyNamespaces(HashSet::from(["shell"]));
    assert_eq!(
        sandbox_brief_flags(true, true, true, &deny_shell),
        (true, false, true),
        "a denied `shell` must not be reported even though it was wired"
    );

    let deny_code = toolbelt::CapabilityFilter::DenyNamespaces(HashSet::from(["code"]));
    assert_eq!(
        sandbox_brief_flags(true, true, true, &deny_code),
        (true, true, false),
        "a denied `code` must not be reported even though it was granted"
    );

    let deny_both =
        toolbelt::CapabilityFilter::DenyNamespaces(HashSet::from(["shell", "code"]));
    assert_eq!(
        sandbox_brief_flags(true, true, true, &deny_both),
        (true, false, false)
    );
}

/// The identity filter changes nothing — the flags are exactly the wired
/// grant flags, files included (files are never a gateable namespace).
#[test]
fn sandbox_brief_flags_pass_through_under_allow_all() {
    assert_eq!(
        sandbox_brief_flags(true, true, true, &toolbelt::CapabilityFilter::AllowAll),
        (true, true, true)
    );
    assert_eq!(
        sandbox_brief_flags(false, false, false, &toolbelt::CapabilityFilter::AllowAll),
        (false, false, false)
    );
}

/// An ungranted/unwired namespace stays absent regardless of the capability
/// filter — denial can only ever narrow, never widen, what the grant wired.
#[test]
fn sandbox_brief_flags_never_add_a_namespace_the_grant_did_not_wire() {
    use std::collections::HashSet;

    let allow_all = toolbelt::CapabilityFilter::AllowAll;
    assert_eq!(
        sandbox_brief_flags(false, false, false, &allow_all),
        (false, false, false)
    );

    // Denying a namespace that was never wired is a no-op on that flag.
    let deny_shell = toolbelt::CapabilityFilter::DenyNamespaces(HashSet::from(["shell"]));
    assert_eq!(
        sandbox_brief_flags(false, false, false, &deny_shell),
        (false, false, false)
    );
}

#[test]
fn grants_cover_matches_namespace_glob_and_star() {
    assert!(grants_cover(&["docs.*".into()], "docs"));
    assert!(grants_cover(&["docs".into()], "docs"));
    assert!(grants_cover(&["docs.read".into()], "docs"));
    assert!(grants_cover(&["*".into()], "docs"));
    assert!(!grants_cover(&["web.*".into()], "docs"));
    assert!(!grants_cover(&[], "docs"));
    // A prefix must end on a namespace boundary, not a substring.
    assert!(!grants_cover(&["documentation.*".into()], "docs"));
}

#[test]
fn file_tools_are_sandboxed_to_the_workspace() {
    let ws = Path::new("/tmp/agent-ws");
    let policy = workspace_security(ws);
    assert!(policy.workspace_only, "file tools must be workspace-only");
    assert_eq!(policy.workspace_dir, ws);
    assert_eq!(policy.action_dir, ws);

    let tools = file_tools(ws);
    assert_eq!(tools.len(), 6, "read/write/edit/list/grep/glob");
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(names.contains(&"file_read"), "got {names:?}");
    assert!(names.contains(&"file_write"), "got {names:?}");
}

// --- Agent-workspace provisioning (issue #409) --------------------------

#[test]
fn ensure_agent_workspace_mints_the_whole_chain_and_is_idempotent() {
    let root = tempfile::tempdir().expect("tempdir");
    let company = CompanyId::new("acme");

    // Nothing under the root exists yet — not the company segment, not the
    // agent segment. This is a company that has never run.
    let named = agent_workspace(root.path(), &company, "ceo");
    assert!(!named.exists(), "precondition: nothing minted yet");

    let made = ensure_agent_workspace(root.path(), &company, "ceo").expect("first ensure");
    assert_eq!(made, named, "creation and naming must agree exactly");
    assert!(made.is_dir());

    // Idempotent: a second call on an existing tree is a success, not an
    // `AlreadyExists` error — the dispatch path calls this on every turn.
    let again = ensure_agent_workspace(root.path(), &company, "ceo").expect("second ensure");
    assert_eq!(again, made);
    assert!(again.is_dir());
}

/// The sandbox is named under the workspace naming rule, so a snake_case
/// roster id and an underscored company id land on dashed directories —
/// the same convention the note tree beside them is kept in.
#[test]
fn the_sandbox_path_is_lowercase_and_dashed() {
    let root = tempfile::tempdir().expect("tempdir");
    let named = agent_workspace(
        root.path(),
        &CompanyId::new("Agentic_Law Firm"),
        "page_builder",
    );

    assert_eq!(
        named,
        root.path()
            .join("agentic-law-firm")
            .join("page-builder")
            .join("workspace")
    );
}

/// An agent upgraded into the new naming keeps the work it had in flight.
///
/// The sandbox is private scratch that nothing outside this process
/// addresses, so moving it is invisible — while leaving it behind would
/// strand a half-finished file on disk, present and unreachable, with
/// nothing reporting it.
#[test]
fn a_pre_rule_sandbox_is_moved_onto_the_canonical_path() {
    let root = tempfile::tempdir().expect("tempdir");
    let company = CompanyId::new("acme");

    let legacy = root
        .path()
        .join("acme")
        .join("page_builder")
        .join("workspace");
    std::fs::create_dir_all(&legacy).expect("legacy sandbox");
    std::fs::write(legacy.join("draft.md"), "half-finished").expect("in-flight work");

    let made = ensure_agent_workspace(root.path(), &company, "page_builder").expect("ensure");

    assert_eq!(made, agent_workspace(root.path(), &company, "page_builder"));
    assert_eq!(
        std::fs::read_to_string(made.join("draft.md")).expect("the work came with it"),
        "half-finished"
    );
    assert!(!legacy.exists(), "the legacy path is not left as a twin");
}

/// A sandbox that already exists at the canonical path is never overwritten
/// by a stale legacy one — the move is a one-time adoption, not a sync.
#[test]
fn a_live_sandbox_is_never_replaced_by_a_legacy_one() {
    let root = tempfile::tempdir().expect("tempdir");
    let company = CompanyId::new("acme");

    let canonical =
        ensure_agent_workspace(root.path(), &company, "page_builder").expect("ensure");
    std::fs::write(canonical.join("current.md"), "live").expect("live work");
    let legacy = root
        .path()
        .join("acme")
        .join("page_builder")
        .join("workspace");
    std::fs::create_dir_all(&legacy).expect("legacy sandbox");
    std::fs::write(legacy.join("stale.md"), "stale").expect("stale work");

    let again = ensure_agent_workspace(root.path(), &company, "page_builder").expect("ensure");

    assert_eq!(again, canonical);
    assert_eq!(
        std::fs::read_to_string(canonical.join("current.md")).expect("still there"),
        "live"
    );
    assert!(!canonical.join("stale.md").exists());
}

/// The bug, pinned. With the workspace absent, `validate_parent_path` walks
/// up past it to an ancestor that really *is* outside the sandbox and
/// refuses a plainly-inside relative path — the refusal an agent granted
/// `files` but not `shell` used to hit on every write.
#[tokio::test]
async fn a_missing_workspace_makes_a_plain_relative_write_look_like_an_escape() {
    let root = tempfile::tempdir().expect("tempdir");
    let workspace = agent_workspace(root.path(), &CompanyId::new("acme"), "ceo");
    assert!(!workspace.exists(), "precondition: never provisioned");

    let policy = workspace_security(&workspace);
    let err = policy
        .validate_parent_path("notes.md")
        .await
        .expect_err("a missing workspace refuses the write");
    assert!(
        err.contains("escapes workspace"),
        "the guard blames traversal for a missing directory: {err}"
    );
}

/// The fix. The same policy over an *ensured* workspace resolves the same
/// relative path, inside the sandbox.
#[tokio::test]
async fn an_ensured_workspace_resolves_a_relative_write_inside_the_sandbox() {
    let root = tempfile::tempdir().expect("tempdir");
    let workspace =
        ensure_agent_workspace(root.path(), &CompanyId::new("acme"), "ceo").expect("ensure");

    let policy = workspace_security(&workspace);
    let resolved = policy
        .validate_parent_path("notes.md")
        .await
        .expect("an existing workspace accepts a relative write");

    let canonical = workspace.canonicalize().expect("canonicalize");
    assert!(
        resolved.starts_with(&canonical),
        "{} is not inside {}",
        resolved.display(),
        canonical.display()
    );
    assert_eq!(
        resolved.file_name().and_then(|n| n.to_str()),
        Some("notes.md")
    );

    // A nested path whose parent does not exist yet still resolves — the
    // guard only ever needed *some* existing ancestor inside the sandbox.
    let nested = policy
        .validate_parent_path("reports/q3/summary.md")
        .await
        .expect("a not-yet-created subdirectory still resolves");
    assert!(nested.starts_with(&canonical));
}

/// Provisioning does not loosen the guard: a genuine escape is still
/// refused — and it comes back **word for word** the same as the
/// missing-workspace refusal above. That is why #409 was filed rather than
/// closed by the one-line create.
///
/// Reaching the resolved-parent arm with a real escape takes some care,
/// which is itself part of the finding. A symlink whose *immediate* parent
/// resolves (`escape/loot.txt`) is caught earlier, by the string-level
/// symlink check, with a different and perfectly clear message. The arm
/// under test is reached only when no existing ancestor can be canonicalized
/// up front: a symlink out of the sandbox plus a not-yet-created
/// subdirectory under it. So in a `workspace_only` agent sandbox this
/// wording fires for exactly two conditions — a hostile symlink and a
/// workspace that was never created — and says "escapes workspace" for both.
///
/// Pinned here so a wording change upstream surfaces in this repo instead of
/// drifting silently.
#[cfg(unix)]
#[tokio::test]
async fn a_real_escape_and_a_missing_workspace_are_refused_in_identical_words() {
    let root = tempfile::tempdir().expect("tempdir");
    let outside = tempfile::tempdir().expect("outside tempdir");
    let workspace =
        ensure_agent_workspace(root.path(), &CompanyId::new("acme"), "ceo").expect("ensure");
    std::os::unix::fs::symlink(outside.path(), workspace.join("escape")).expect("symlink");

    let policy = workspace_security(&workspace);

    // The easy half: the immediate parent resolves, so the string-level
    // symlink check refuses it first — clearly, and distinguishably.
    let shallow = policy
        .validate_parent_path("escape/loot.txt")
        .await
        .expect_err("a symlink out of the sandbox is refused");
    assert!(
        shallow.contains("Path not allowed by security policy"),
        "expected the string-level refusal: {shallow}"
    );

    // The arm this issue is about: nothing up front can be canonicalized,
    // so the ancestor walk runs and lands outside the sandbox.
    let deep = policy
        .validate_parent_path("escape/nested/loot.txt")
        .await
        .expect_err("a real escape must still be refused");
    assert!(
        deep.contains("Resolved parent path escapes workspace"),
        "expected the resolved-parent refusal: {deep}"
    );

    // The same arm, reached instead by a workspace nobody ever created.
    let absent = agent_workspace(root.path(), &CompanyId::new("acme"), "nobody");
    let missing = workspace_security(&absent)
        .validate_parent_path("notes.md")
        .await
        .expect_err("a missing workspace refuses too");
    assert!(
        missing.contains("Resolved parent path escapes workspace"),
        "{missing}"
    );

    // Verbatim identical up to the path each names. A reader given either
    // one goes looking for a traversal attempt; only one of them is.
    let strip = |m: &str| {
        m.split_once("escapes workspace: ")
            .map(|(head, _)| head.to_string())
            .unwrap_or_default()
    };
    assert_eq!(
        strip(&deep),
        strip(&missing),
        "an attack and an unprovisioned directory should not read alike"
    );
}

/// A `..` traversal is refused earlier, by the string-level check, and
/// *does* read differently — so the ambiguity above is specifically about
/// the resolved-parent arm, not about every refusal.
#[tokio::test]
async fn a_dot_dot_traversal_is_refused_with_a_distinguishable_message() {
    let root = tempfile::tempdir().expect("tempdir");
    let workspace =
        ensure_agent_workspace(root.path(), &CompanyId::new("acme"), "ceo").expect("ensure");

    let err = workspace_security(&workspace)
        .validate_parent_path("../../loot.txt")
        .await
        .expect_err("a traversal is refused");
    assert!(
        err.contains("Path not allowed by security policy"),
        "expected the string-level refusal: {err}"
    );
    assert!(
        !err.contains("escapes workspace"),
        "this arm is already distinguishable: {err}"
    );
}

#[test]
fn model_for_tier_maps_hints_and_defaults() {
    assert_eq!(model_for_tier(Some("reasoning")), "reasoning-v1");
    assert_eq!(model_for_tier(Some("AGENTIC")), "agentic-v1");
    assert_eq!(model_for_tier(Some("frontend")), "agentic-v1");
    assert_eq!(model_for_tier(None), "chat-v1");
    assert_eq!(model_for_tier(Some("mystery")), "chat-v1");
}

fn manifest_agent(role: &str, description: Option<&str>) -> ManifestAgent {
    ManifestAgent {
        provider: None,
        global: false,
        id: "ceo".to_string(),
        role: role.to_string(),
        name: None,
        description: description.map(str::to_string),
        tier: None,
        harness: None,
        tools: None,
        delegates_to: Vec::new(),
        context: None,
        budget_usd_daily: None,
        prompt: None,
        prompt_files: Vec::new(),
        prompt_files_resolved: Vec::new(),
        classes: Vec::new(),
        ledgers: None,
        can_declare_ledgers: true,
        model: None,
    }
}

#[test]
fn persona_frames_role_company_and_description() {
    let agent = manifest_agent("Chief Executive", Some("Sets direction."));
    let persona = persona_prompt("Acme", &agent, None);
    assert!(persona.contains("Chief Executive"), "{persona}");
    assert!(persona.contains("Acme"), "{persona}");
    assert!(persona.contains("first person"), "{persona}");
    assert!(persona.ends_with("Sets direction."), "{persona}");
}

#[test]
fn persona_omits_absent_or_blank_description() {
    let persona = persona_prompt("Acme", &manifest_agent("Engineer", Some("   ")), None);
    assert!(persona.contains("Engineer"));
    assert!(!persona.contains("   Engineer"));
    // No trailing description clause.
    assert!(persona.trim_end().ends_with("role."), "{persona}");
}

// --- Dispatched-agent toolbelt contract (issue #188a) -------------------
//
// These tests PIN the tool surface a dispatched company agent receives by
// building a real agent via `build_agent` and reading back its live
// `tools()` list. They lock three things so a future change can neither
// silently widen nor narrow the belt:
//
//   a. the EXACT set of tool names a dispatched desk agent gets (snapshot);
//   b. delegation tools are ABSENT for a dispatched agent but PRESENT for
//      the orchestrator (the depth-cap = 1 / "no re-delegation" invariant,
//      issue #178 — the single most important thing to pin);
//   c. none of the deferred families (browser / search / node / subagent-
//      spawn / skill-exec / memory-tree / `forget`) appear in the belt.
//
// Compiled under the module's default `--features openhuman` config (the
// whole `harness` module is `openhuman`-gated), so the pinned set is the
// openhuman-only belt: the `media` (#109) and `composio` (#110) tool arms
// are inert without their features and are never wired here. Their
// namespace mapping is pinned separately by the `namespace_of` tests in
// `toolbelt.rs`.

use crate::company::Policy;
use crate::harness::mcp_probe::McpFailureQueue;
use crate::harness::orchestrator::{DelegationQueue, WorkflowRunnerHandle};
use crate::harness::policy::ApprovalRequestQueue;
use crate::harness::provider::MockProvider;
use crate::ports::CompanyStore;
use crate::ports::types::{
    ChunkAddr, ChunkHit, ChunkMeta, CompanyRecord, CompanySummary, ContextChunk, LedgerEntry,
};

/// A no-op context store — the belt tests never exercise memory, they only
/// assert the wired tool surface.
struct PinContext;
#[async_trait::async_trait]
impl crate::ports::ContextStore for PinContext {
    async fn put(&self, _: &CompanyId, _: ContextChunk) -> crate::Result<ChunkAddr> {
        Ok(ChunkAddr::new("x"))
    }
    async fn list(&self, _: &CompanyId, _: &str) -> crate::Result<Vec<ChunkMeta>> {
        Ok(Vec::new())
    }
    async fn peek(
        &self,
        _: &CompanyId,
        _: &ChunkAddr,
        _: Option<std::ops::Range<usize>>,
    ) -> crate::Result<String> {
        Ok(String::new())
    }
    async fn search(&self, _: &CompanyId, _: &str, _: usize) -> crate::Result<Vec<ChunkHit>> {
        Ok(Vec::new())
    }
    async fn delete(
        &self,
        _: &CompanyId,
        _: &crate::ports::types::ChunkAddr,
    ) -> crate::Result<bool> {
        Ok(false)
    }
    async fn delete_label(
        &self,
        _: &CompanyId,
        _: &crate::ports::types::ChunkAddr,
        _: &str,
    ) -> crate::Result<bool> {
        Ok(false)
    }
}

/// A no-op company store — `build_agent` only needs a handle; nothing here
/// loads or persists.
struct PinStore;
#[async_trait::async_trait]
impl CompanyStore for PinStore {
    async fn load(&self, _: &CompanyId) -> crate::Result<Option<CompanyRecord>> {
        Ok(None)
    }
    async fn save(&self, _: &CompanyRecord) -> crate::Result<()> {
        Ok(())
    }
    async fn list(&self) -> crate::Result<Vec<CompanySummary>> {
        Ok(Vec::new())
    }
    async fn append_ledger(&self, _: &CompanyId, _: LedgerEntry) -> crate::Result<()> {
        Ok(())
    }
}

/// Minimal `HarnessDeps` for building a single agent: offline mock provider,
/// no-op stores, no meter/skills/mcp/media/composio, `AllowAll` capability
/// filter (identity). Workspace lands under a caller-owned tempdir.
fn pin_deps(root: std::path::PathBuf) -> HarnessDeps {
    // Two DISTINCT roots under one caller-owned tempdir, mirroring
    // production (`<home>/harness` beside `<home>/companies`). Reusing one
    // root here would let a test pass while the audit sink sat inside the
    // workspace tree — the exact defect issue #775 fixed.
    let workspace_root = root.join("harness");
    // Production sets this to `<home>/mcp` (see `runtime::builder`); mirror
    // it so the pinned belt reflects a real company rather than the
    // degraded no-MCP-home shape.
    let mcp_home = Some(root.join("mcp"));
    let audit_root = root;
    HarnessDeps {
        emergency_gate: None,
        notifications: None,
        ledgers: None,
        ledger_registry: Default::default(),
        provider: Arc::new(MockProvider::new("mock: ")),
        provider_slug: "mock".to_string(),
        serves: None,
        context: Arc::new(PinContext),
        store: Arc::new(PinStore),
        meter: None,
        workspace_root,
        mcp_home,
        workspace_git_enabled: false,
        audit_root,
        model_override: None,
        tasks: None,
        artifacts: None,
        skills: None,
        skills_source_dir: None,
        skills_registry: std::sync::Arc::from([]),
        default_mcp_servers: Vec::new(),
        mcp_servers: Vec::new(),
        facts: None,
        events: None,
        delegations: DelegationQueue::default(),
        workflow_runner: WorkflowRunnerHandle::default(),
        mcp_failures: McpFailureQueue::default(),
        pending_publishes: crate::harness::publish::PendingPublishQueue::default(),
        workflow_refs: crate::harness::workflow_refs::WorkflowRefQueue::default(),
        run_outputs: crate::harness::orchestrator::RunOutputCache::default(),
        run_output_store: None,
        workflow_revisions: None,
        approval_requests: ApprovalRequestQueue::default(),
        secrets: None,
        web_allowed_domains: Vec::new(),
        capabilities: toolbelt::CapabilityFilter::AllowAll,
        workflow_source_dir: None,
        plan: None,
        media: None,
        composio: None,
        #[cfg(feature = "chargebee")]
        chargebee: None,
        #[cfg(feature = "paypal")]
        paypal: None,
        hosting: None,
        steer: crate::company::steer::InflightRegistry::default(),
        run_supervisor: crate::runtime::RunSupervisor::default(),
        delivery: None,
        // Fail-closed default: with no managed search backend wired, the
        // #238 tool is never built and the pinned belt below is the
        // pre-#238 belt exactly.
        search: None,
        tenant_search: None,
        // Fail-closed default: with no workspace store wired, the #237
        // tools are never built and the pinned belt below is the
        // pre-#237 belt exactly.
        workspace: None,
        workflow_runs: None,
        deep_trace: None,
    }
}

/// Build one agent with `[speech]` on or off and a journal wired, and
/// return its live tool names.
///
/// The journal is the half `built_tool_names` leaves out (`events: None`),
/// and it is not optional here: the speech tools **are** the append, so a
/// host with no `EventLog` registers none of them by design.
fn built_tool_names_with_speech(speech_enabled: bool) -> Vec<String> {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut deps = pin_deps(dir.path().to_path_buf());
    deps.events = Some(Arc::new(crate::store::FsEventLog::new(dir.path())));
    let manifest_agent = ManifestAgent {
        provider: None,
        global: false,
        id: "designer".to_string(),
        role: "Designer".to_string(),
        name: None,
        description: None,
        tier: None,
        harness: None,
        tools: None,
        delegates_to: Vec::new(),
        context: None,
        budget_usd_daily: None,
        prompt: None,
        prompt_files: Vec::new(),
        prompt_files_resolved: Vec::new(),
        classes: Vec::new(),
        ledgers: None,
        can_declare_ledgers: true,
        model: None,
    };
    let agent = build_agent(
        &CompanyId::new("acme"),
        "Acme",
        &manifest_agent,
        ApprovalPolicy::new(&Policy::default(), None),
        &deps,
        &["*".to_string()],
        &[],
        &[],
        None,
        false,
        speech_enabled,
    )
    .expect("agent builds");
    let mut names: Vec<String> = agent.tools().iter().map(|t| t.name().to_string()).collect();
    names.sort();
    names
}

/// `[speech] enabled` is what puts a voice on the belt, and nothing else is.
///
/// Pinned by name because the whole knob is "does this company talk by
/// calling a tool", and a company that did not ask for it must keep the
/// belt it had — an agent that suddenly grows four tools it was never told
/// about is a behaviour change nobody opted into.
#[test]
fn speech_tools_are_registered_only_when_the_manifest_asks() {
    let off = built_tool_names_with_speech(false);
    for tool in crate::harness::speech_tools::SPEECH_TOOLS {
        assert!(
            !off.contains(&tool.to_string()),
            "{tool} must not be on the belt of a company that did not ask for it: {off:?}"
        );
    }

    let on = built_tool_names_with_speech(true);
    for tool in crate::harness::speech_tools::SPEECH_TOOLS {
        assert!(
            on.contains(&tool.to_string()),
            "{tool} must be on the belt when `[speech] enabled`: {on:?}"
        );
    }
}

/// CodeRabbit: `speech_enabled` is the manifest's opt-in, but the tools
/// ARE the append (module doc, above) — with no `EventLog` wired there is
/// nothing to append to, so `speech_wired` (not the bare flag) must gate
/// both the belt and the persona brief. Before this, `[speech] enabled =
/// true` on a host with no journal still told the agent to call tools
/// that were never registered.
#[test]
fn speech_tools_stay_off_the_belt_with_no_journal_even_when_the_manifest_asks() {
    let dir = tempfile::tempdir().expect("tempdir");
    let deps = pin_deps(dir.path().to_path_buf());
    assert!(
        deps.events.is_none(),
        "this test exercises the no-journal case; pin_deps must still default to it"
    );
    let manifest_agent = ManifestAgent {
        provider: None,
        global: false,
        id: "designer".to_string(),
        role: "Designer".to_string(),
        name: None,
        description: None,
        tier: None,
        harness: None,
        tools: None,
        delegates_to: Vec::new(),
        context: None,
        budget_usd_daily: None,
        prompt: None,
        prompt_files: Vec::new(),
        prompt_files_resolved: Vec::new(),
        classes: Vec::new(),
        ledgers: None,
        can_declare_ledgers: true,
        model: None,
    };
    let agent = build_agent(
        &CompanyId::new("acme"),
        "Acme",
        &manifest_agent,
        ApprovalPolicy::new(&Policy::default(), None),
        &deps,
        &["*".to_string()],
        &[],
        &[],
        None,
        false,
        /* speech_enabled */ true,
    )
    .expect("agent builds");
    let names: Vec<String> = agent.tools().iter().map(|t| t.name().to_string()).collect();
    for tool in crate::harness::speech_tools::SPEECH_TOOLS {
        assert!(
            !names.contains(&tool.to_string()),
            "{tool} must stay off the belt with no journal, even with `[speech] enabled`: \
             {names:?}"
        );
    }
}

/// Build one agent under `grants` and return its live tool names, sorted, so
/// a snapshot compares byte-stably against a literal.
fn built_tool_names(grants: &[&str], is_orchestrator: bool) -> Vec<String> {
    built_tool_names_delegating(grants, is_orchestrator, &[])
}

/// [`built_tool_names`] with a `delegates_to` allowlist on the agent (issue
/// #176) — the only difference between a member that may re-delegate and one
/// that may not.
fn built_tool_names_delegating(
    grants: &[&str],
    is_orchestrator: bool,
    delegates_to: &[&str],
) -> Vec<String> {
    let dir = tempfile::tempdir().expect("tempdir");
    let deps = pin_deps(dir.path().to_path_buf());
    let manifest_agent = ManifestAgent {
        provider: None,
        global: false,
        id: "desk".to_string(),
        role: "Desk Lead".to_string(),
        name: None,
        description: None,
        tier: None,
        harness: None,
        tools: None,
        delegates_to: delegates_to.iter().map(|d| d.to_string()).collect(),
        context: None,
        budget_usd_daily: None,
        prompt: None,
        prompt_files: Vec::new(),
        prompt_files_resolved: Vec::new(),
        classes: Vec::new(),
        ledgers: None,
        can_declare_ledgers: true,
        model: None,
    };
    let policy = ApprovalPolicy::new(&Policy::default(), None);
    let grants: Vec<String> = grants.iter().map(|g| g.to_string()).collect();
    let agent = build_agent(
        &CompanyId::new("acme"),
        "Acme",
        &manifest_agent,
        policy,
        &deps,
        &grants,
        &[],
        &[],
        None,
        is_orchestrator,
        /* speech_enabled */ false,
    )
    .expect("agent builds");
    let mut names: Vec<String> = agent.tools().iter().map(|t| t.name().to_string()).collect();
    names.sort();
    names
}

/// Build one agent under `grants` with a MANAGED search backend wired, and
/// return its live tool names. Mirrors [`built_tool_names`], differing only
/// in `deps.search` — so the difference between the two is exactly "a
/// credential exists", which is one of the three gate states pinned below.
fn built_tool_names_with_search(grants: &[&str]) -> Vec<String> {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut deps = pin_deps(dir.path().to_path_buf());
    deps.search = Some(crate::harness::search::SearchBackend::new(
        "https://api.example.test".to_string(),
        crate::company::credentials::Credential::from_value("managed-platform-token"),
        crate::company::DEFAULT_SEARCH_DAILY_CALLS,
    ));
    let manifest_agent = ManifestAgent {
        provider: None,
        global: false,
        id: "desk".to_string(),
        role: "Desk Lead".to_string(),
        name: None,
        description: None,
        tier: None,
        harness: None,
        tools: None,
        delegates_to: Vec::new(),
        context: None,
        budget_usd_daily: None,
        prompt: None,
        prompt_files: Vec::new(),
        prompt_files_resolved: Vec::new(),
        classes: Vec::new(),
        ledgers: None,
        can_declare_ledgers: true,
        model: None,
    };
    let policy = ApprovalPolicy::new(&Policy::default(), None);
    let grants: Vec<String> = grants.iter().map(|g| g.to_string()).collect();
    let agent = build_agent(
        &CompanyId::new("acme"),
        "Acme",
        &manifest_agent,
        policy,
        &deps,
        &grants,
        &[],
        &[],
        None,
        false,
        /* speech_enabled */ false,
    )
    .expect("agent builds");
    let mut names: Vec<String> = agent.tools().iter().map(|t| t.name().to_string()).collect();
    names.sort();
    names
}

/// The native capabilities `native_capabilities_on_belt` reads off the SAME
/// agent [`built_tool_names_with_search`] builds — proving the brief's native
/// set is derived from tools that were actually wired, not from the grants.
fn built_native_caps_with_search(grants: &[&str]) -> Vec<String> {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut deps = pin_deps(dir.path().to_path_buf());
    deps.search = Some(crate::harness::search::SearchBackend::new(
        "https://api.example.test".to_string(),
        crate::company::credentials::Credential::from_value("managed-platform-token"),
        crate::company::DEFAULT_SEARCH_DAILY_CALLS,
    ));
    let manifest_agent = ManifestAgent {
        provider: None,
        global: false,
        id: "desk".to_string(),
        role: "Desk Lead".to_string(),
        name: None,
        description: None,
        tier: None,
        harness: None,
        tools: None,
        delegates_to: Vec::new(),
        context: None,
        budget_usd_daily: None,
        prompt: None,
        prompt_files: Vec::new(),
        prompt_files_resolved: Vec::new(),
        classes: Vec::new(),
        ledgers: None,
        can_declare_ledgers: true,
        model: None,
    };
    let policy = ApprovalPolicy::new(&Policy::default(), None);
    let grants: Vec<String> = grants.iter().map(|g| g.to_string()).collect();
    let agent = build_agent(
        &CompanyId::new("acme"),
        "Acme",
        &manifest_agent,
        policy,
        &deps,
        &grants,
        &[],
        &[],
        None,
        false,
        /* speech_enabled */ false,
    )
    .expect("agent builds");
    toolbelt::native_capabilities_on_belt(agent.tools())
        .into_iter()
        .map(str::to_string)
        .collect()
}

/// The brief's native set is read off the wired belt: an explicit `search`
/// grant with a credential wires `web_search`, so `search` shows up in the
/// belt's native capabilities — and a bare `*` (which never wires the metered
/// tool) does not.
#[test]
fn native_capabilities_on_belt_track_the_wired_search_tool() {
    let granted = built_native_caps_with_search(&["search"]);
    assert!(
        granted.contains(&"search".to_string()),
        "an explicit search grant wires web_search, so `search` is native on the belt: {granted:?}"
    );
    let wildcard = built_native_caps_with_search(&["*"]);
    assert!(
        !wildcard.contains(&"search".to_string()),
        "a bare `*` wires no metered search tool, so `search` is not native on the belt: {wildcard:?}"
    );
}

/// Build one agent under `grants` with BOTH a managed search backend and a
/// company's own `provider` connection wired, and return its live tool
/// names. The two together is the interesting case: it is what a company
/// that pasted a key into the console actually has, and what decides which
/// of the two surfaces the model is offered.
fn built_tool_names_with_byo_search(grants: &[&str], provider: &str) -> Vec<String> {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut deps = pin_deps(dir.path().to_path_buf());
    deps.search = Some(crate::harness::search::SearchBackend::new(
        "https://api.example.test".to_string(),
        crate::company::credentials::Credential::from_value("managed-platform-token"),
        crate::company::DEFAULT_SEARCH_DAILY_CALLS,
    ));
    deps.tenant_search = Some(crate::harness::search_byo::TenantSearch::for_test(
        provider,
        Some("tenant-key"),
        Some("https://searx.example"),
    ));
    let manifest_agent = ManifestAgent {
        provider: None,
        global: false,
        id: "desk".to_string(),
        role: "Desk Lead".to_string(),
        name: None,
        description: None,
        tier: None,
        harness: None,
        model: None,
        tools: None,
        delegates_to: Vec::new(),
        context: None,
        budget_usd_daily: None,
        prompt: None,
        prompt_files: Vec::new(),
        prompt_files_resolved: Vec::new(),
        classes: Vec::new(),
        ledgers: None,
        can_declare_ledgers: true,
    };
    let policy = ApprovalPolicy::new(&Policy::default(), None);
    let grants: Vec<String> = grants.iter().map(|g| g.to_string()).collect();
    let agent = build_agent(
        &CompanyId::new("acme"),
        "Acme",
        &manifest_agent,
        policy,
        &deps,
        &grants,
        &[],
        &[],
        None,
        false,
        /* speech_enabled */ false,
    )
    .expect("agent builds");
    let mut names: Vec<String> = agent.tools().iter().map(|t| t.name().to_string()).collect();
    names.sort();
    names
}

/// A company's own provider REPLACES the managed surface rather than
/// joining it, and still answers to the one name the skills know.
///
/// Both halves matter. Two "search the web" tools on one belt would let the
/// model spend the platform's metered budget for a company that pasted its
/// own key — the exact bill-swap the BYO surface exists to prevent. And a
/// belt where the canonical name changed with the provider would break the
/// shipped research skills, which name `web_search` in their instructions.
#[test]
fn a_company_provider_replaces_the_managed_search_tool_under_the_same_name() {
    let byo = built_tool_names_with_byo_search(&["search"], "brave");

    assert!(
        byo.contains(&"web_search".to_string()),
        "the canonical name must survive the provider switch: {byo:?}"
    );
    assert!(
        byo.contains(&"brave_news_search".to_string()),
        "the provider's own extras must be wired too: {byo:?}"
    );
    // Exactly one tool answers to the canonical name.
    assert_eq!(
        byo.iter().filter(|name| *name == "web_search").count(),
        1,
        "two search tools under one name: {byo:?}"
    );
    // And the managed family's siblings are absent — nothing on this belt
    // reaches the platform's metered backend.
    assert!(
        !byo.contains(&"exa_search".to_string()),
        "a Brave company must not carry Exa tools: {byo:?}"
    );
}

/// The BYO surface rides the SAME explicit grant as the metered one. A
/// company key does not turn `search` into a wildcard-conferred namespace:
/// the queries still leave the building, and which index reads them is a
/// decision the manifest makes by name.
#[test]
fn a_wildcard_grant_confers_no_search_tools_even_with_a_company_provider() {
    let wildcard = built_tool_names_with_byo_search(&["*"], "exa");
    assert!(
        !wildcard.contains(&"web_search".to_string()),
        "{wildcard:?}"
    );
    assert!(
        !wildcard.contains(&"exa_get_contents".to_string()),
        "{wildcard:?}"
    );
}

// --- Company-workspace wiring gates (issue #237) -----------------------

/// Build one agent with a workspace store wired and return its tool names.
/// Mirrors [`built_tool_names`], differing only in `deps.workspace`.
fn built_tool_names_with_workspace(grants: &[&str]) -> Vec<String> {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut deps = pin_deps(dir.path().to_path_buf());
    deps.workspace = Some(Arc::new(crate::store::FsOps::new(dir.path())));
    let manifest_agent = ManifestAgent {
        provider: None,
        global: false,
        id: "desk".to_string(),
        role: "Desk Lead".to_string(),
        name: None,
        description: None,
        tier: None,
        harness: None,
        tools: None,
        delegates_to: Vec::new(),
        context: None,
        budget_usd_daily: None,
        prompt: None,
        prompt_files: Vec::new(),
        prompt_files_resolved: Vec::new(),
        classes: Vec::new(),
        ledgers: None,
        can_declare_ledgers: true,
        model: None,
    };
    let policy = ApprovalPolicy::new(&Policy::default(), None);
    let grants: Vec<String> = grants.iter().map(|g| g.to_string()).collect();
    let agent = build_agent(
        &CompanyId::new("acme"),
        "Acme",
        &manifest_agent,
        policy,
        &deps,
        &grants,
        &[],
        &[],
        None,
        false,
        /* speech_enabled */ false,
    )
    .expect("agent builds");
    let mut names: Vec<String> = agent.tools().iter().map(|t| t.name().to_string()).collect();
    names.sort();
    names
}

/// Build one agent with an artifact store wired, so the #244 publish gate
/// can be exercised in both its states.
fn built_tool_names_with_artifacts(grants: &[&str]) -> Vec<String> {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut deps = pin_deps(dir.path().to_path_buf());
    deps.artifacts = Some(Arc::new(crate::store::FsOps::new(dir.path())));
    let manifest_agent = ManifestAgent {
        provider: None,
        global: false,
        id: "desk".to_string(),
        role: "Desk Lead".to_string(),
        name: None,
        description: None,
        tier: None,
        harness: None,
        tools: None,
        delegates_to: Vec::new(),
        context: None,
        budget_usd_daily: None,
        prompt: None,
        prompt_files: Vec::new(),
        prompt_files_resolved: Vec::new(),
        classes: Vec::new(),
        ledgers: None,
        can_declare_ledgers: true,
        model: None,
    };
    let policy = ApprovalPolicy::new(&Policy::default(), None);
    let grants: Vec<String> = grants.iter().map(|g| g.to_string()).collect();
    let agent = build_agent(
        &CompanyId::new("acme"),
        "Acme",
        &manifest_agent,
        policy,
        &deps,
        &grants,
        &[],
        &[],
        None,
        false,
        /* speech_enabled */ false,
    )
    .expect("agent builds");
    let mut names: Vec<String> = agent.tools().iter().map(|t| t.name().to_string()).collect();
    names.sort();
    names
}

/// Issue #244's two gates, in one table.
///
/// The fail-closed row is the load-bearing one: an agent granted file tools
/// with **no artifact store** must not be offered `publish_artifact`. The
/// tool stages into a queue; with nothing to drain it, a call would report
/// success, tell the agent its deliverable was safe, and drop it. Not
/// offering it is the only honest option.
#[test]
fn publish_artifact_needs_both_a_file_grant_and_a_store() {
    let tool = crate::harness::publish::PUBLISH_ARTIFACT_TOOL.to_string();

    // `files` (and its aliases and the wildcard) + a store → present.
    // Publishing spends nothing and reaches nothing outside the company, so
    // unlike `media`/`search` it rides the ordinary namespace rule.
    for grant in ["files", "docs", "files.write", "*"] {
        let names = built_tool_names_with_artifacts(&[grant]);
        assert!(
            names.contains(&tool),
            "`{grant}` + a store must wire publish_artifact: {names:?}"
        );
    }

    // No file grant → absent. An agent that cannot write a file has nothing
    // to publish.
    let unfiled = built_tool_names_with_artifacts(&["web"]);
    assert!(
        !unfiled.contains(&tool),
        "an agent with no file tools must not be offered publish_artifact: {unfiled:?}"
    );

    // File grant, NO store → absent, fail-closed.
    let storeless = built_tool_names(&["files"], false);
    assert!(
        !storeless.contains(&tool),
        "without an artifact store the tool would stage into a void: {storeless:?}"
    );
    // …and the rest of the file belt is untouched, so the gate withholds one
    // tool rather than breaking the agent.
    assert!(
        storeless.contains(&"file_write".to_string()),
        "{storeless:?}"
    );
}

/// **Issue #1192, the standard issue #886 stated.** The verdict the console
/// renders must equal what the toolbelt actually wires — asserted by running
/// both over the same grant matrix, not by reading the two implementations
/// and agreeing they look alike.
///
/// The console panel calls
/// [`grants_files_or_docs`](crate::company::grants_files_or_docs); this gate
/// calls it too, so today the equality is true by construction. That is the
/// point of pinning it: the day somebody re-inlines a `starts_with` on
/// either side — or "tidies" the predicate into the `_explicit` family,
/// where `*` confers nothing — this fails instead of a panel quietly
/// reporting a capability no agent has, which is the failure #886 was filed
/// about and the failure #886 said a test like this one prevents.
///
/// An artifact store is wired throughout, so the store gate is held constant
/// and the grant is the only variable — which is exactly the axis the
/// console field answers on. (The store half is not a console field at all:
/// production always configures one, so a `artifactStoreConfigured` flag
/// would serialize a hardcoded `true`.)
#[test]
fn the_capability_verdict_matches_what_the_toolbelt_wires() {
    let tool = crate::harness::publish::PUBLISH_ARTIFACT_TOOL.to_string();
    for grant in [
        "*",
        "files",
        "docs",
        "files.write",
        "docs.read",
        "web",
        "shell",
        "documentation",
        "docsy",
        "filesystem",
        "composio",
        "repo",
    ] {
        let verdict = crate::company::grants_files_or_docs(&[grant.to_string()]);
        let wired = built_tool_names_with_artifacts(&[grant]).contains(&tool);
        assert_eq!(
            verdict, wired,
            "`{grant}`: the console would report publishing={verdict} while the toolbelt \
             wires={wired}"
        );
    }
}

/// The three gate states of the metered `web_search` surface (issue #238),
/// in one table.
///
/// The load-bearing row is the first: a broad `*` grant does **not** wire
/// `web_search` even with a credential present. Every call is a priced
/// request on the managed platform, so — like `media` and `composio` — it
/// must be opted into by name and can never ride in on the wildcard a
/// company set for its file and shell tools.
#[test]
fn web_search_is_wired_only_by_explicit_grant_and_credential() {
    // `*` + credential → absent. The wildcard never confers spend.
    let wildcard = built_tool_names_with_search(&["*"]);
    assert!(
        !wildcard.contains(&"web_search".to_string()),
        "a bare `*` must NOT confer the metered search family: {wildcard:?}"
    );

    // explicit `search` + credential → present.
    let granted = built_tool_names_with_search(&["search"]);
    assert!(
        granted.contains(&"web_search".to_string()),
        "an explicit `search` grant with a credential must wire web_search: {granted:?}"
    );
    // The sub-grant form works the same way `media.*` / `composio.*` do.
    let sub_granted = built_tool_names_with_search(&["search.web"]);
    assert!(
        sub_granted.contains(&"web_search".to_string()),
        "{sub_granted:?}"
    );

    // explicit `search`, NO credential → absent, fail-closed.
    let uncredentialed = built_tool_names(&["search"], false);
    assert!(
        !uncredentialed.contains(&"web_search".to_string()),
        "a search grant with no managed credential must wire nothing: {uncredentialed:?}"
    );

    // An unrelated grant confers nothing even with a credential wired.
    let unrelated = built_tool_names_with_search(&["web.*"]);
    assert!(
        !unrelated.contains(&"web_search".to_string()),
        "an unrelated grant must not confer web_search: {unrelated:?}"
    );
}

/// Granting `search` must not quietly hand over anything *else*: the
/// credentialed `["search"]` belt is the ungranted belt plus exactly one
/// tool. A namespace that widens the belt beyond its own family is how a
/// grant stops meaning what the operator read.
#[test]
fn the_search_grant_adds_exactly_one_tool() {
    let mut baseline = built_tool_names(&[], false);
    let granted = built_tool_names_with_search(&["search"]);
    baseline.push("web_search".to_string());
    baseline.sort();
    assert_eq!(granted, baseline, "the `search` grant widened the belt");
}

/// The four gate states of the workspace surface, in one table.
///
/// The load-bearing row is the second: a broad `*` grant yields the READ
/// tools but NOT `workspace_write`. Writes mutate operator-owned guidance
/// every other agent then trusts, so — like `media` and `composio` — they
/// must be opted into by name and can never ride in on a wildcard.
#[test]
fn workspace_tools_are_wired_by_grant_and_store_presence() {
    // No store wired → fail closed, nothing built, whatever the grant.
    let unwired = built_tool_names(&["workspace"], false);
    for tool in [
        "workspace_list",
        "workspace_read",
        "workspace_search",
        "workspace_write",
        "workspace_rename",
        "workspace_delete",
    ] {
        assert!(
            !unwired.contains(&tool.to_string()),
            "no store must mean no `{tool}`: {unwired:?}"
        );
    }

    // `*` → reads only. This is the whole asymmetry.
    let wildcard = built_tool_names_with_workspace(&["*"]);
    assert!(
        wildcard.contains(&"workspace_list".to_string()),
        "{wildcard:?}"
    );
    assert!(
        wildcard.contains(&"workspace_read".to_string()),
        "{wildcard:?}"
    );
    // Issue #607: search is a read and rides the read side of the gate. It
    // reads exactly what `workspace_read` already grants, and it is the
    // cheap path — behind the write grant it would be missing from every
    // default agent, leaving them on the list-then-read crawl.
    assert!(
        wildcard.contains(&"workspace_search".to_string()),
        "a bare `*` must confer workspace search: {wildcard:?}"
    );
    for tool in ["workspace_write", "workspace_rename", "workspace_delete"] {
        assert!(
            !wildcard.contains(&tool.to_string()),
            "a bare `*` must NOT confer `{tool}`: {wildcard:?}"
        );
    }

    // Explicit `workspace` → reads + all four mutations. The lifecycle pair
    // (issue #671) rides this grant rather than a new one: it reaches only
    // the agent's own folder, which is narrower than the unconfined
    // overwrite the same grant already confers.
    let explicit = built_tool_names_with_workspace(&["workspace"]);
    for tool in [
        "workspace_create",
        "workspace_write",
        "workspace_rename",
        "workspace_delete",
    ] {
        assert!(explicit.contains(&tool.to_string()), "{tool}: {explicit:?}");
    }

    // No workspace grant at all → nothing, even with a store wired.
    let ungranted = built_tool_names_with_workspace(&["web.*"]);
    for tool in [
        "workspace_list",
        "workspace_read",
        "workspace_search",
        "workspace_write",
        "workspace_rename",
        "workspace_delete",
    ] {
        assert!(
            !ungranted.contains(&tool.to_string()),
            "an unrelated grant must not confer `{tool}`: {ungranted:?}"
        );
    }
}

/// `workspace.read` is a genuinely read-only grant.
///
/// A deliberate divergence from the `media` / `composio` helpers, which
/// match any `<ns>.` prefix and would therefore let `workspace.read` confer
/// writes — a footgun on a destructive surface.
#[test]
fn a_workspace_read_grant_does_not_confer_writes() {
    let read_grant = built_tool_names_with_workspace(&["workspace.read"]);
    assert!(
        read_grant.contains(&"workspace_read".to_string()),
        "{read_grant:?}"
    );
    for tool in ["workspace_write", "workspace_rename", "workspace_delete"] {
        assert!(
            !read_grant.contains(&tool.to_string()),
            "`workspace.read` must not confer `{tool}`: {read_grant:?}"
        );
    }

    let write_grant = built_tool_names_with_workspace(&["workspace.write"]);
    for tool in ["workspace_write", "workspace_rename", "workspace_delete"] {
        assert!(
            write_grant.contains(&tool.to_string()),
            "{tool}: {write_grant:?}"
        );
    }
}

/// `workspace_search` rides the `workspace` READ grant and NEVER the
/// metered `search` grant (issue #607).
///
/// The names invite the wrong wiring, and the wrong wiring would defeat the
/// issue: `search` is the paid external-credential grant that carries
/// `web_search`, and putting workspace search behind it would mean an agent
/// needs a billed backend credential to read its own company's notes — the
/// crawl stays, and now it stays for a reason nobody would guess from the
/// tool's description. Search reads exactly what `workspace_read` already
/// grants, so it costs the operator no additional decision.
#[test]
fn workspace_search_rides_the_workspace_grant_and_not_the_metered_search_grant() {
    // The `search` grant alone confers `web_search` — and nothing from the
    // workspace family, which is not granted here at all.
    let metered = built_tool_names_with_search(&["search"]);
    assert!(metered.contains(&"web_search".to_string()), "{metered:?}");
    assert!(
        !metered.contains(&"workspace_search".to_string()),
        "the metered `search` grant must not confer workspace search: {metered:?}"
    );

    // …and the workspace read grant confers workspace search without
    // conferring the billed one.
    let workspace = built_tool_names_with_workspace(&["workspace.read"]);
    assert!(
        workspace.contains(&"workspace_search".to_string()),
        "`workspace.read` must confer workspace search: {workspace:?}"
    );
    assert!(
        !workspace.contains(&"web_search".to_string()),
        "reading company notes must not require a billed search credential: {workspace:?}"
    );
}

/// (a) The EXACT tool belt a dispatched desk agent receives with the broad
/// `*` grant. Any tool added to or removed from a dispatched agent flips
/// this snapshot and fails CI — the whole point of the pin. The set is the
/// curated exec subset (shell / code / web) plus the intrinsic memory + file
/// tools; it contains NO delegation tool and NO deferred family, and — the
/// #238 addition — no `web_search`, because a bare `*` does not confer the
/// `search` grant. Nor does it contain `mcp_registry_list_tools` /
/// `mcp_registry_tool_call`, for the identical reason: those two ride the
/// explicit `mcp_registry` grant, never the wildcard — see the
/// `mcp_registry_tools_are_wired_only_by_explicit_grant` test below for the
/// belt a company that names that grant actually receives.
#[test]
fn dispatched_desk_agent_tool_belt_is_pinned() {
    let names = built_tool_names(&["*"], false);
    let mut expected = vec![
        "apply_patch",
        "csv_export",
        "curl",
        "edit",
        "file_read",
        "file_write",
        "git_operations",
        "glob",
        "grep",
        "http_request",
        "image_info",
        "list",
        // The deliberate-memory trio (issue #1113): intrinsic, on every
        // belt — company-scoped by construction, see memory_tools.rs.
        "memory_forget",
        "memory_recall",
        "memory_store",
        "read_workspace_state",
        "request_approval",
        "shell",
        "web_fetch",
        // Issue #1861: intrinsic, on every belt and gated by nothing. The
        // ability to ask a person a question is not a capability an agent
        // can be too narrow to hold — a narrow agent is the one most likely
        // to hit something only the operator can answer, and its
        // alternatives are guessing or going quiet.
        "escalate_to_human",
    ];
    // The global baseline installs skills in every company (issue: global
    // agents/skills/workflows), so the three skill read tools are on every
    // belt now — including a company with no skills source of its own.
    expected.extend(["describe_skill", "list_skills", "read_skill_resource"]);
    expected.sort();
    assert_eq!(names, expected, "dispatched desk belt drifted: {names:?}");
}

/// The three gate states of the `mcp_registry` surface, mirroring
/// [`web_search_is_wired_only_by_explicit_grant_and_credential`].
///
/// The load-bearing row is the first: a broad `*` grant does **not** wire
/// either `mcp_registry_list_tools` or `mcp_registry_tool_call`, even with
/// a configured registry home (`pin_deps` always sets one). Before this
/// gate existed, both tools were pushed unconditionally whenever
/// `deps.mcp_home` was set — this row is the regression check for that.
#[cfg(feature = "mcp")]
#[test]
fn mcp_registry_tools_are_wired_only_by_explicit_grant() {
    const REGISTRY_TOOLS: [&str; 2] = ["mcp_registry_list_tools", "mcp_registry_tool_call"];

    // No grant at all → absent.
    let ungranted = built_tool_names(&[], false);
    for tool in REGISTRY_TOOLS {
        assert!(
            !ungranted.contains(&tool.to_string()),
            "no grant must mean no `{tool}`: {ungranted:?}"
        );
    }

    // `*` (with a configured home) → still absent. The wildcard never
    // confers a third-party-reaching, mutating surface.
    let wildcard = built_tool_names(&["*"], false);
    for tool in REGISTRY_TOOLS {
        assert!(
            !wildcard.contains(&tool.to_string()),
            "a bare `*` must NOT confer `{tool}`: {wildcard:?}"
        );
    }

    // An unrelated grant → absent.
    let unrelated = built_tool_names(&["web.*"], false);
    for tool in REGISTRY_TOOLS {
        assert!(
            !unrelated.contains(&tool.to_string()),
            "an unrelated grant must not confer `{tool}`: {unrelated:?}"
        );
    }

    // Explicit `mcp_registry` grant, with a configured home → BOTH tools,
    // together — list is never conferred without call, or vice versa.
    let granted = built_tool_names(&["mcp_registry"], false);
    for tool in REGISTRY_TOOLS {
        assert!(
            granted.contains(&tool.to_string()),
            "an explicit `mcp_registry` grant must wire `{tool}`: {granted:?}"
        );
    }

    // The sub-grant form works the same way `search.web` / `composio.gmail`
    // do.
    let sub_granted = built_tool_names(&["mcp_registry.notion"], false);
    for tool in REGISTRY_TOOLS {
        assert!(
            sub_granted.contains(&tool.to_string()),
            "`mcp_registry.notion` must wire `{tool}`: {sub_granted:?}"
        );
    }
}

/// Explicit `mcp_registry` grant, NO configured registry home → absent,
/// fail-closed — matching every other explicit namespace's
/// granted-but-uncredentialed shape (`media`, `composio`, `chargebee`,
/// `paypal`, `hosting`, `search`).
#[cfg(feature = "mcp")]
#[test]
fn mcp_registry_tools_fail_closed_with_no_registry_home() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut deps = pin_deps(dir.path().to_path_buf());
    deps.mcp_home = None;
    let manifest_agent = ManifestAgent {
        provider: None,
        global: false,
        id: "desk".to_string(),
        role: "Desk Lead".to_string(),
        name: None,
        description: None,
        tier: None,
        harness: None,
        tools: None,
        delegates_to: Vec::new(),
        context: None,
        budget_usd_daily: None,
        prompt: None,
        prompt_files: Vec::new(),
        prompt_files_resolved: Vec::new(),
        classes: Vec::new(),
        ledgers: None,
        can_declare_ledgers: true,
        model: None,
    };
    let policy = ApprovalPolicy::new(&Policy::default(), None);
    let grants: Vec<String> = vec!["mcp_registry".to_string()];
    let agent = build_agent(
        &CompanyId::new("acme"),
        "Acme",
        &manifest_agent,
        policy,
        &deps,
        &grants,
        &[],
        &[],
        None,
        false,
        false,
    )
    .expect("agent builds");
    let names: Vec<String> = agent.tools().iter().map(|t| t.name().to_string()).collect();
    assert!(
        !names.contains(&"mcp_registry_list_tools".to_string()),
        "granted-but-unconfigured must not wire `mcp_registry_list_tools`: {names:?}"
    );
    assert!(
        !names.contains(&"mcp_registry_tool_call".to_string()),
        "granted-but-unconfigured must not wire `mcp_registry_tool_call`: {names:?}"
    );
}

#[test]
fn request_approval_is_intrinsic_and_needs_no_manifest_grant() {
    let names = built_tool_names(&[], false);
    assert!(names.contains(&"request_approval".to_string()), "{names:?}");
}

/// Issue #988: the tool-iteration ceiling is **stated** on every agent this
/// crate builds, not inherited by omission.
///
/// The distinction is the whole bug. `build_agent` never called
/// `set_max_tool_iterations`, so every company agent silently ran on
/// `AgentConfig::default()`'s ten — a number no OpenCompany source
/// mentioned, and one that a teammate doing real multi-step work spends
/// before it delivers anything (#926). Deleting the call again would put the
/// build back on the vendored default and fail here.
///
/// The second assertion is what keeps this honest across a vendored bump: it
/// checks the stated number is genuinely *higher* than what omission would
/// have given, rather than comparing a constant to itself.
#[test]
fn every_built_agent_states_a_raised_tool_iteration_cap() {
    let dir = tempfile::tempdir().expect("tempdir");
    let deps = pin_deps(dir.path().to_path_buf());
    let manifest_agent = ManifestAgent {
        provider: None,
        global: false,
        id: "ceo".to_string(),
        role: "Chief Executive".to_string(),
        name: None,
        description: None,
        tier: None,
        tools: None,
        delegates_to: Vec::new(),
        context: None,
        harness: None,
        budget_usd_daily: None,
        prompt: None,
        prompt_files: Vec::new(),
        prompt_files_resolved: Vec::new(),
        classes: Vec::new(),
        ledgers: None,
        can_declare_ledgers: true,
        model: None,
    };
    let agent = build_agent(
        &CompanyId::new("acme"),
        "Acme",
        &manifest_agent,
        ApprovalPolicy::new(&Policy::default(), None),
        &deps,
        &[],
        &[],
        &[],
        None,
        false,
        /* speech_enabled */ false,
    )
    .expect("agent builds");

    assert_eq!(
        agent.agent_config().max_tool_iterations,
        MAX_TOOL_ITERATIONS,
        "the built agent is not running on the cap this crate states"
    );

    let inherited = oh::config::AgentConfig::default().max_tool_iterations;
    assert!(
        MAX_TOOL_ITERATIONS > inherited,
        "stating {MAX_TOOL_ITERATIONS} is only a fix while it exceeds the vendored \
         default of {inherited}"
    );
}

/// (b) The **default** depth cap (issues #178, #176): a dispatched desk
/// agent that named no `delegates_to` must NEVER receive a delegation tool,
/// while the orchestrator agent MUST. Building both from the same grant and
/// contrasting them is the registration check that an ordinary dispatched
/// turn cannot re-delegate.
///
/// #176 made this the default rather than the only possibility — the
/// opt-in case is pinned by
/// [`a_member_with_delegates_to_gets_exactly_the_two_hand_off_tools`], and
/// the belt above is unchanged for every agent that does not opt in.
#[test]
fn dispatched_agent_has_no_delegation_tools_but_orchestrator_does() {
    let delegation = [
        "query_company",
        "spawn_task",
        "delegate_to_desk",
        // Issue #884: the new hand-off is opt-in on exactly the same terms —
        // an ordinary dispatched agent must not silently gain the ability to
        // run somebody else's turn.
        "delegate_to_teammate",
    ];

    let dispatched = built_tool_names(&["*"], false);
    for tool in delegation {
        assert!(
            !dispatched.contains(&tool.to_string()),
            "dispatched desk agent must NOT receive delegation tool `{tool}`: {dispatched:?}"
        );
    }

    let orchestrator = built_tool_names(&["*"], true);
    for tool in delegation {
        assert!(
            orchestrator.contains(&tool.to_string()),
            "orchestrator agent MUST receive delegation tool `{tool}`: {orchestrator:?}"
        );
    }
}

/// (b2) Issue #176: a member the manifest opted in with `delegates_to` gets
/// **exactly the hand-off tools** more than it had — `spawn_task`,
/// `delegate_to_desk`, and (issue #884) `delegate_to_teammate` — and not one
/// tool of the orchestrator's authority.
///
/// Expressed as a delta against the un-opted-in belt rather than as a second
/// flat literal, so the feature-aware snapshot above stays the single place
/// the dispatched belt is written down. What this pins is the thing #176
/// could get wrong: reaching for `orchestrator_tools` and handing a desk
/// lead `add_agent`, `assign_task` and `review_task` along with the hand-off
/// it actually needs.
#[test]
fn a_member_with_delegates_to_gets_exactly_the_two_hand_off_tools() {
    let plain = built_tool_names(&["*"], false);
    let delegating = built_tool_names_delegating(&["*"], false, &["research"]);

    let added: Vec<&String> = delegating.iter().filter(|t| !plain.contains(t)).collect();
    assert_eq!(
        added,
        vec!["delegate_to_desk", "delegate_to_teammate", "spawn_task"],
        "a delegating member's belt must differ from the plain one by exactly the \
         hand-off tools: {delegating:?}"
    );
    assert!(
        plain.iter().all(|t| delegating.contains(t)),
        "opting in must ADD tools, never remove any: {delegating:?}"
    );
    for authority in [
        "query_company",
        "assign_task",
        "review_task",
        "add_agent",
        "run_workflow",
        "create_workflow",
        "read_run_output",
    ] {
        assert!(
            !delegating.contains(&authority.to_string()),
            "a desk member must NOT receive orchestrator authority `{authority}`: \
             {delegating:?}"
        );
    }
}

/// (b3) Issue #176: the wiring is inert for an agent that named no
/// allowlist, and the orchestrator's own belt is untouched by the feature.
///
/// The empty-allowlist half is what makes #176 a no-op for every manifest
/// written before it; the orchestrator half is what proves the `else if`
/// really is exclusive, since a second narrowed `delegate_to_desk` beside
/// the orchestrator's unrestricted one would put two tools of the same name
/// on one belt.
#[test]
fn an_empty_allowlist_wires_nothing_and_the_orchestrator_belt_is_unchanged() {
    assert_eq!(
        built_tool_names_delegating(&["*"], false, &[]),
        built_tool_names(&["*"], false),
        "an empty `delegates_to` must produce the pre-#176 belt byte-for-byte"
    );

    let orchestrator = built_tool_names(&["*"], true);
    assert_eq!(
        built_tool_names_delegating(&["*"], true, &["research"]),
        orchestrator,
        "an orchestrator's belt must not change when it also names `delegates_to`"
    );
    assert_eq!(
        orchestrator
            .iter()
            .filter(|t| *t == "delegate_to_desk")
            .count(),
        1,
        "exactly one `delegate_to_desk` may be wired: {orchestrator:?}"
    );
}

/// (c) No deferred family leaks into a dispatched belt: raw browser
/// automation, Node/NPM exec, OpenHuman sub-agent spawn tools (the
/// `subagent` namespace is reserved but empty in v1), skill *execution*,
/// the raw memory-tree tool surface, and `forget`. A negative assertion, so
/// it stays honest even as OpenHuman renames tools upstream — the pin is
/// "none of these shapes appear".
///
/// **`web_search` was removed from this list by issue #238** — and only
/// `web_search`. It was deferred for one *infrastructure* reason ("need
/// engine keys") that the managed-credential pattern dissolved; the other
/// families here were deferred for *safety* reasons that still hold. The
/// remaining `search` / `google_search` entries stay pinned, so OpenHuman's
/// broader search families cannot arrive on the back of this decision.
///
/// That `web_search` is nonetheless absent from a `*`-granted belt is not
/// this test's job any more — it is pinned deliberately by
/// [`web_search_is_wired_only_by_explicit_grant_and_credential`] and by the
/// exact snapshot in
/// [`dispatched_desk_agent_tool_belt_is_pinned`], which would both fail if
/// the wildcard ever started conferring spend.
#[test]
fn dispatched_belt_excludes_every_deferred_family() {
    let names = built_tool_names(&["*"], false);
    let forbidden = [
        // raw browser automation
        "browser",
        "browser_navigate",
        "browser_click",
        "browser_screenshot",
        // the search families still deferred (`web_search` is admitted
        // under an explicit `search` grant — issue #238)
        "search",
        "google_search",
        // Node / NPM exec
        "node",
        "npm",
        "run_node",
        "run_npm",
        // OpenHuman sub-agent spawn (subagent namespace reserved, empty v1)
        "spawn_subagent",
        "spawn_agent",
        "delegate_archivist",
        // skill execution
        "run_skill",
        "skill_run",
        "run_workflow",
        "await_workflow",
        // raw memory-tree tool surface
        "memory_tree",
        "memory_tree_search",
        "memory_tree_get",
        // destructive memory: upstream's raw `forget` stays out; the
        // scoped oc-authored `memory_forget` is a real belt tool now.
        "forget",
    ];
    for tool in forbidden {
        assert!(
            !names.contains(&tool.to_string()),
            "deferred tool `{tool}` leaked into the dispatched belt: {names:?}"
        );
    }
}

/// [`pin_deps`] with automatic Git checkpoints enabled — the one switch
/// `HarnessDeps::workspace_git_enabled` flips inside `build_agent`.
fn enabled_git_deps(root: std::path::PathBuf) -> HarnessDeps {
    let mut deps = pin_deps(root);
    deps.workspace_git_enabled = true;
    deps
}

/// `git log` inside the workspace, resolving through the checkpointer's
/// `.git` pointer, exactly as the existing checkpoint tests do.
fn git_log(workspace: &std::path::Path) -> String {
    String::from_utf8(
        std::process::Command::new("git")
            .args(["log", "--format=%s"])
            .current_dir(workspace)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
}

/// The enabled Git path, end to end through `build_agent`: the `docs.*`
/// grant wires the sandboxed `file_write`, `workspace_git_enabled: true`
/// decorates every tool with the checkpointer, and a tool call that writes
/// the workspace yields the baseline commit plus a post-call checkpoint.
#[tokio::test]
async fn workspace_git_enabled_checkpoints_a_tool_write() {
    use crate::company::Policy;
    use serde_json::json;

    let dir = tempfile::tempdir().expect("tempdir");
    let deps = enabled_git_deps(dir.path().to_path_buf());
    let company = CompanyId::new("acme");
    let manifest_agent = ManifestAgent {
        provider: None,
        global: false,
        id: "desk".to_string(),
        role: "Desk Lead".to_string(),
        name: None,
        description: None,
        tier: None,
        harness: None,
        tools: None,
        delegates_to: Vec::new(),
        context: None,
        budget_usd_daily: None,
        prompt: None,
        prompt_files: Vec::new(),
        prompt_files_resolved: Vec::new(),
        classes: Vec::new(),
        ledgers: None,
        can_declare_ledgers: true,
        model: None,
    };
    // `full` so the sandboxed write executes without a supervised prompt.
    let policy = ApprovalPolicy::new(
        &Policy {
            mode: "full".to_string(),
            ..Policy::default()
        },
        None,
    );
    let grants = vec!["docs.*".to_string()];
    let agent = build_agent(
        &company,
        "Acme",
        &manifest_agent,
        policy,
        &deps,
        &grants,
        &[],
        &[],
        None,
        false,
        /* speech_enabled */ false,
    )
    .expect("agent builds");

    let workspace = agent_workspace(&deps.workspace_root, &company, "desk");
    let write = agent
        .tools()
        .iter()
        .find(|tool| tool.name() == "file_write")
        .expect("docs.* wires file_write");
    let result = write
        .execute(json!({"path": "answer.txt", "content": "42"}))
        .await
        .expect("file_write runs");
    assert!(!result.is_error, "unexpected failure: {result:?}");
    assert_eq!(
        std::fs::read_to_string(workspace.join("answer.txt")).unwrap(),
        "42"
    );

    let history = git_log(&workspace);
    assert!(
        history.contains("checkpoint: initialize workspace"),
        "{history}"
    );
    assert!(
        history.contains("checkpoint: after file_write"),
        "{history}"
    );
}

#[cfg(feature = "mcp")]
#[test]
fn mcp_registry_list_tools_is_withheld_from_an_agent_granted_nothing() {
    let names = built_tool_names(&[], false);
    assert!(
        !names.contains(&"mcp_registry_list_tools".to_string()),
        "an agent holding no grant must not receive the MCP registry reader: {names:?}"
    );
}

#[cfg(feature = "mcp")]
#[test]
fn mcp_registry_tool_call_is_withheld_from_an_agent_granted_nothing() {
    let names = built_tool_names(&[], false);
    assert!(
        !names.contains(&"mcp_registry_tool_call".to_string()),
        "an agent holding no grant must not receive the MCP registry invoker: {names:?}"
    );
}

#[cfg(feature = "mcp")]
#[test]
fn a_docs_only_agent_receives_no_mcp_registry_tool() {
    let names = built_tool_names(&["docs.*"], false);
    for tool in ["mcp_registry_list_tools", "mcp_registry_tool_call"] {
        assert!(
            !names.contains(&tool.to_string()),
            "`docs.*` must not confer `{tool}`: {names:?}"
        );
    }
}

#[cfg(feature = "mcp")]
#[test]
fn no_configured_mcp_server_wires_no_server_backed_mcp_tool() {
    let dir = tempfile::tempdir().expect("tempdir");
    let deps = pin_deps(dir.path().to_path_buf());
    assert!(
        deps.mcp_servers.is_empty(),
        "this test's premise is a company with no configured server"
    );
    assert!(
        crate::harness::mcp::registry_for_agent(&deps.mcp_servers, &["*".to_string()])
            .is_none(),
        "no configured server must yield no registry, even under `*`"
    );

    let names = built_tool_names(&["*"], false);
    for tool in ["mcp_list_servers", "mcp_list_tools", "mcp_call"] {
        assert!(
            !names.contains(&tool.to_string()),
            "`{tool}` must not be wired without a configured server: {names:?}"
        );
    }
}

/// The tool-iteration ceiling is one crate-wide constant with no per-agent
/// lever: a tier hint, a declared daily budget and the orchestrator flag all
/// build agents that run on exactly [`MAX_TOOL_ITERATIONS`]. An agent that
/// needs a longer loop has no way to ask for one, and — the direction that
/// matters — no manifest field can raise its own ceiling.
#[test]
fn the_tool_iteration_cap_is_uniform_and_not_manifest_configurable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let deps = pin_deps(dir.path().to_path_buf());

    let build_with = |tier: Option<&str>, budget: Option<f64>, is_orchestrator: bool| {
        let manifest_agent = ManifestAgent {
            provider: None,
            global: false,
            id: "desk".to_string(),
            role: "Desk Lead".to_string(),
            name: None,
            description: None,
            tier: tier.map(str::to_string),
            harness: None,
            tools: None,
            delegates_to: Vec::new(),
            context: None,
            budget_usd_daily: budget,
            prompt: None,
            prompt_files: Vec::new(),
            prompt_files_resolved: Vec::new(),
            classes: Vec::new(),
            ledgers: None,
            can_declare_ledgers: true,
            model: None,
        };
        build_agent(
            &CompanyId::new("acme"),
            "Acme",
            &manifest_agent,
            ApprovalPolicy::new(&Policy::default(), None),
            &deps,
            &["*".to_string()],
            &[],
            &[],
            None,
            is_orchestrator,
            /* speech_enabled */ false,
        )
        .expect("agent builds")
        .agent_config()
        .max_tool_iterations
    };

    for (label, got) in [
        ("no tier", build_with(None, None, false)),
        ("deep tier", build_with(Some("deep"), None, false)),
        ("fast tier", build_with(Some("fast"), None, false)),
        ("budgeted", build_with(None, Some(500.0), false)),
        ("orchestrator", build_with(None, None, true)),
    ] {
        assert_eq!(
            got, MAX_TOOL_ITERATIONS,
            "`{label}` must run on the one stated ceiling, not its own"
        );
    }
}
