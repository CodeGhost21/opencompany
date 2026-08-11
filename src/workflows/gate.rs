//! Issue #460: the company's [`ApprovalPolicy`] decides which `tool_call` nodes
//! stop for an operator, before the run reaches them.
//!
//! # The hole this closes
//!
//! [`WorkflowToolInvoker::invoke`](super::caps) resolves a slug's grant
//! namespace, checks it fail-closed against `[tools].allow`, looks the tool up
//! and executes it. [`ApprovalPolicy`] is never consulted, so a `tool_call`
//! node running `shell` or `http_request` on a `supervised` company produced no
//! approval card, no recorded decision and no grant accounting — while the
//! *same call* from an agent node in the *same graph* did, ever since #395.
//!
//! What was NOT missing is exec-security: `[policy].mode` has always fed
//! [`toolbelt::exec_security`](crate::harness::toolbelt), which sets the
//! autonomy tier, blocks high-risk commands and confines execution to the
//! workflow workspace. That layer is untouched here. The gap was the
//! operator-facing half, and this closes only that.
//!
//! # Why the gate is not in the invoker
//!
//! The obvious shape — consult the policy inside `invoke` and refuse — is
//! wrong twice over, and both reasons are load-bearing.
//!
//! **It would be a regression, not a fix.** An agent node parks *after* a turn
//! that already happened: openhuman resolves the refusal inline, the model is
//! told no and narrates it, and the node still succeeds. A `tool_call` node has
//! no turn — the call **is** the node — so a refusal fails the node, and
//! `on_error` defaults to `stop`. Under the default `supervised` mode
//! [`consequence_of`](crate::policy::consequence_of) puts `shell`,
//! `http_request`, `curl`, `web_fetch`, `apply_patch`, `csv_export` and
//! `git_operations` at [`Reach::Consequence`](crate::policy::Reach), so
//! refusing in the invoker would break essentially every effectful `tool_call`
//! node on every default-mode company.
//!
//! **The card could not be honoured.** Approving a harness-projected tool call
//! mints a single-use [`GrantedCall`](crate::runtime::grants::GrantedCall) that
//! is redeemed by re-dispatching *the agent that asked*. There is no agent on
//! this path, so nothing would ever redeem it: the grant would expire on its
//! TTL and the operator would be told the agent did not act, about a call no
//! agent ever made.
//!
//! And the seam says so itself. [`ToolInvoker::invoke`](tinyflows::caps::ToolInvoker)
//! receives `(slug, args, conn)` — no node id, no run state. It cannot name the
//! node on a card, and on a continuation it could not recognise a call the
//! operator had already approved. A fix that has to invent both does not belong
//! there.
//!
//! # Where it goes instead
//!
//! One level up, where the host holds both facts. [`run_workflow_inner`] already
//! translates the graph **per run**, and holds the [`CompanyRecord`] (the mode
//! and the grants) — so the policy verdict is taken there and written onto the
//! node as the engine's own `requires_approval` flag.
//!
//! That flag is a **generic per-node** gate in tinyflows, not a node kind: the
//! engine checks it before the node's work and, once the node's id is listed in
//! the run input's `approvals` array, falls through and executes the node
//! normally. So a marked `tool_call` node inherits every piece #395 already
//! built, unchanged:
//!
//! * the engine pauses before the tool runs, and reports the node on
//!   `pending_approvals`;
//! * [`park_pending_gates`](super::runner) turns that into a decidable card;
//! * [`resume_from_effect`](crate::runtime::workflow_resume) re-runs the graph
//!   with the approval in the trigger input, and the call executes;
//! * the #438 delivery ledger stops the continuation re-sending reports.
//!
//! It also means the gate is visible **on the canvas**, which is the other half
//! of what #460 complains about: two node kinds in one graph, governed
//! differently, with nothing saying which is which.
//!
//! # This is not #561
//!
//! #460's acceptance criteria ask for suspend/resume, and the issue's Related
//! section points at #561 as the expensive way to get it. That reading is
//! wrong, and the correction matters for anyone scoping the neighbouring
//! issues: #561 is expensive because the fail-closed block lives in
//! **openhuman's turn loop**, and approving therefore costs a re-dispatch. The
//! workflow path never enters that loop. A paused tinyflows run is *settled*,
//! and #395 already shipped resume-by-re-run for exactly this shape.
//!
//! # Deliberate deviations, stated rather than quietly satisfied
//!
//! * **Standing grants cannot apply "identically on both paths"** (a criterion
//!   in #460's body). A standing grant is scoped to a teammate
//!   ([`admits_scope`](crate::runtime::grants::StandingGrant::admits_scope)) and
//!   a `tool_call` node has no teammate — it acts as the company. This module
//!   therefore matches grants against a synthetic `workflow:{id}` principal,
//!   which nothing mints for today, so the answer is always **fail-closed**: a
//!   teammate's standing grant does not open a workflow node's gate. Widening
//!   that is a consent decision for the maintainer, not one to make by
//!   defaulting.
//! * **A `Deny` verdict is not enforced here.** Under `readonly`,
//!   [`ApprovalPolicy::check`] denies external effects outright. Honouring that
//!   would be a *new refusal* on a path that runs today, i.e. exactly the
//!   regression argued against above, and #460 is explicit that exec-security
//!   (which already applies the `readonly` autonomy tier) is not the gap. Only
//!   `RequireApproval` gates.
//! * **An authored `requires_approval = false` does not win.** The policy adds
//!   a gate the author did not ask for; an author cannot opt their node out of
//!   the company's approval policy.
//! * **`sub_workflow` children are not gated.** tinyflows cannot resume a child
//!   across the boundary (`nodes/integration/sub_workflow.rs`), and a paused
//!   child *halts the parent* with an unresumable error. Gating a child would
//!   convert a working run into a dead one with no card to decide, so children
//!   keep today's behaviour. This is a real residual hole in #460 and it needs
//!   engine work to close.
//! * **Dry runs are not gated.** Every effect is stubbed
//!   ([`dry_run`](super::caps)), so there is nothing to approve, and pausing
//!   would stop a dry run from walking the rest of the graph — which is the one
//!   thing it exists to do.

use serde_json::{Value, json};
use tinyflows::model::{NodeKind, WorkflowGraph};

use oh::agent::tool_policy::{ToolCallContext, ToolPolicy, ToolPolicyDecision, ToolPolicyRequest};
use openhuman_core::openhuman as oh;

use crate::harness::policy::ApprovalPolicy;
use crate::ports::types::CompanyRecord;

/// One `tool_call` node the company's policy stopped, and why.
///
/// Carried from the gate pass to [`park_pending_gates`](super::runner) so the
/// operator's card can name the tool and the reason rather than a bare node id
/// — the complaint #468 makes about the Approvals tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GatedToolCall {
    /// The node the engine will pause on.
    pub node_id: String,
    /// The toolbelt slug that node would have run.
    pub slug: String,
    /// The policy's own words for why it stopped — the same string the agent
    /// path puts on its card.
    pub reason: String,
}

/// Marks every `tool_call` node whose call the company's [`ApprovalPolicy`]
/// would park, so the engine stops the run there.
///
/// Returns what was gated, in graph order. An empty result leaves the graph
/// byte-identical to what [`translate`](super::translate) produced, so a `full`
/// company — and every existing test — runs exactly as it did before.
///
/// # The policy instance is deliberately unshared
///
/// [`ApprovalPolicy::check`] has a side effect: a `RequireApproval` verdict
/// **pushes the projected effect onto its request queue** for the brain to park.
/// That is right on the agent path and wrong here — the shared queue is drained
/// by [`park_gated_calls`](super::caps) and the chat cycle, both of which would
/// park a *second*, tool-call-shaped card for the same decision, and that one
/// carries the un-redeemable grant described in this module's docs. So this
/// builds its own policy over the default private queue nobody drains
/// ([`ApprovalPolicy::new`]'s documented behaviour), takes the verdict, and
/// drops it. The gate card is the only card.
///
/// Leaving the agent unbound is what makes the grant arms fail closed: both
/// [`consume_grant`] and `standing_grant_allows` short-circuit on a policy with
/// no agent, which is precisely the synthetic-principal decision this module's
/// docs record. The context below names that principal so a log line says which
/// workflow asked.
pub(crate) async fn apply_tool_call_gates(
    graph: &mut WorkflowGraph,
    record: &CompanyRecord,
    workflow_id: &str,
    run_id: &str,
) -> Vec<GatedToolCall> {
    // The manifest's `[policy]` verbatim — same mode, same `always_approve`,
    // same `auto_approve_under_usd` the roster runs under. No per-agent budget:
    // a workflow node is not a teammate, and the company-wide ceiling is
    // enforced elsewhere.
    let policy = ApprovalPolicy::new(&record.manifest.policy, None);
    let mut gated = Vec::new();

    for node in &mut graph.nodes {
        if node.kind != NodeKind::ToolCall {
            continue;
        }
        // A `tool_call` without a slug fails in the engine's own node with a
        // clear error; there is no call to classify, so leave it be.
        let Some(slug) = node.config.get("slug").and_then(Value::as_str) else {
            continue;
        };
        let args = node
            .config
            .get("args")
            .cloned()
            .unwrap_or_else(|| json!({}));

        let request = ToolPolicyRequest::new(
            slug,
            args,
            ToolCallContext::session(
                run_id,
                "workflow",
                workflow_principal(workflow_id),
                &node.id,
                0,
            ),
        );
        // Only `RequireApproval` gates — see the module docs on `Deny`.
        let ToolPolicyDecision::RequireApproval { reason } = policy.check(&request).await else {
            continue;
        };

        let slug = slug.to_string();
        tracing::debug!(
            company = %record.id,
            workflow = workflow_id,
            node = %node.id,
            tool = %slug,
            "workflow tool_call: the company's policy stops this call for an operator"
        );
        // Written LAST and unconditionally: the policy's gate outranks an
        // authored `requires_approval`, in both directions. An author may add a
        // gate the policy does not require; they may not remove one it does.
        if let Value::Object(config) = &mut node.config {
            config.insert("requires_approval".to_string(), json!(true));
        }
        gated.push(GatedToolCall {
            node_id: node.id.clone(),
            slug,
            reason,
        });
    }

    gated
}

/// The synthetic principal a workflow `tool_call` acts as.
///
/// Not a roster teammate, and deliberately shaped so it can never collide with
/// one: nothing mints a grant for it, so every grant arm in
/// [`ApprovalPolicy::check`] fails closed. It exists to make the policy's own
/// log lines name the workflow that asked. Mirrors the label
/// [`build_capabilities`](super::caps) already stamps on this run's search
/// metering.
fn workflow_principal(workflow_id: &str) -> String {
    format!("workflow:{workflow_id}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::types::CompanyId;
    use tinyflows::model::Node;

    /// A company record whose `[policy]` is the only thing under test.
    ///
    /// `always_approve` is written explicitly on every call — including as an
    /// empty list — because the manifest default is NOT empty
    /// ([`DEFAULT_ALWAYS_APPROVE`](crate::company::DEFAULT_ALWAYS_APPROVE) is
    /// `payment.send` / `filing.submit` / `external.publish`). Letting it
    /// default would make these tests read as if the tier decided something the
    /// always-approve list had actually decided.
    fn company(mode: &str, always_approve: &[&str]) -> CompanyRecord {
        let always = always_approve
            .iter()
            .map(|entry| format!("\"{entry}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let manifest = toml::from_str(&format!(
            r#"
[company]
name = "Acme"

[policy]
mode = "{mode}"
always_approve = [{always}]

[[agent]]
id = "ceo"
role = "Chief Executive"
description = "Runs Acme."
"#
        ))
        .expect("valid manifest");
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
            disabled_workflows: Vec::new(),
            template_provenance: None,
        }
    }

    fn tool_node(id: &str, slug: &str) -> Node {
        Node {
            id: id.to_string(),
            kind: NodeKind::ToolCall,
            type_version: 1,
            name: String::new(),
            config: json!({ "slug": slug }),
            ports: Vec::new(),
            position: None,
        }
    }

    fn graph(nodes: Vec<Node>) -> WorkflowGraph {
        WorkflowGraph {
            id: Some("wf".to_string()),
            nodes,
            ..WorkflowGraph::default()
        }
    }

    fn gate_ids(graph: &WorkflowGraph) -> Vec<&str> {
        graph
            .nodes
            .iter()
            .filter(|n| n.config.get("requires_approval") == Some(&json!(true)))
            .map(|n| n.id.as_str())
            .collect()
    }

    /// The defect itself: on the default `supervised` mode a `shell` node runs
    /// with no operator card. It must now stop.
    #[tokio::test]
    async fn a_consequential_call_gates_under_supervised() {
        let mut g = graph(vec![tool_node("run-it", "shell")]);
        let gated = apply_tool_call_gates(&mut g, &company("supervised", &[]), "wf", "run-1").await;

        assert_eq!(gate_ids(&g), ["run-it"]);
        assert_eq!(gated.len(), 1);
        assert_eq!(gated[0].slug, "shell");
        assert_eq!(gated[0].node_id, "run-it");
        assert!(
            gated[0].reason.contains("shell"),
            "the card must name the tool: {}",
            gated[0].reason
        );
    }

    /// `full` autonomy is the operator saying "don't ask me". The graph must
    /// come out byte-identical, or this change would gate companies that opted
    /// out of gating.
    #[tokio::test]
    async fn full_autonomy_leaves_the_graph_untouched() {
        let before = graph(vec![tool_node("run-it", "shell")]);
        let mut after = before.clone();
        let gated = apply_tool_call_gates(&mut after, &company("full", &[]), "wf", "run-1").await;

        assert!(gated.is_empty());
        assert_eq!(after.nodes[0].config, before.nodes[0].config);
    }

    /// A pure read of the agent's own workspace is `Reach::Nothing`, and a
    /// metered read is `Reach::Money` — neither parks under supervision. Gating
    /// either would stop runs that have nothing to decide, and for `web_search`
    /// it would be worse than useless: consent for it happens once, at grant
    /// time, via an explicit `search` grant a `*` cannot confer.
    ///
    /// `file_read` rather than `read_workspace_state`, which reads like the
    /// obvious choice and is not one: issue #459 moved it to
    /// `Reach::Consequence` because it shells out to `git` in a directory the
    /// agent can write a `.git/config` into. That is a deliberate stopgap with
    /// its own revert condition (tinyhumansai/openhuman#5494), so the tool to
    /// change here was the example, not the classification.
    #[tokio::test]
    async fn a_plain_read_and_a_metered_read_do_not_gate() {
        let mut g = graph(vec![
            tool_node("read", "file_read"),
            tool_node("search", "web_search"),
        ]);
        let gated = apply_tool_call_gates(&mut g, &company("supervised", &[]), "wf", "run-1").await;

        assert!(gated.is_empty(), "{gated:?}");
        assert!(gate_ids(&g).is_empty());
    }

    /// `always_approve` outranks the tier, exactly as it does on the agent
    /// path — so a company can gate a metered read it wants to be asked about
    /// even though `supervised` alone would let it through.
    #[tokio::test]
    async fn always_approve_gates_a_call_the_tier_would_allow() {
        let mut g = graph(vec![tool_node("search", "web_search")]);
        let gated =
            apply_tool_call_gates(&mut g, &company("full", &["web_search"]), "wf", "run-1").await;

        assert_eq!(gate_ids(&g), ["search"]);
        assert!(gated[0].reason.contains("always-approve"), "{gated:?}");
    }

    /// An author may add a gate the policy does not require; they may not
    /// remove one it does. Fail-closed in the one direction that matters.
    #[tokio::test]
    async fn an_authored_opt_out_cannot_remove_a_policy_gate() {
        let mut node = tool_node("run-it", "shell");
        node.config = json!({ "slug": "shell", "requires_approval": false });
        let mut g = graph(vec![node]);

        apply_tool_call_gates(&mut g, &company("supervised", &[]), "wf", "run-1").await;

        assert_eq!(g.nodes[0].config["requires_approval"], json!(true));
    }

    /// Only `tool_call` nodes are touched. An `agent` node already parks
    /// through #395's drain, and gating it here would park the same call twice.
    #[tokio::test]
    async fn other_node_kinds_are_never_gated() {
        let mut agent = tool_node("think", "shell");
        agent.kind = NodeKind::Agent;
        let mut http = tool_node("fetch", "http_request");
        http.kind = NodeKind::HttpRequest;
        let mut g = graph(vec![agent, http]);

        let gated = apply_tool_call_gates(&mut g, &company("supervised", &[]), "wf", "run-1").await;

        assert!(gated.is_empty(), "{gated:?}");
        assert!(gate_ids(&g).is_empty());
    }

    /// A slugless `tool_call` has no call to classify; the engine's own node
    /// reports it. Gating it would turn a clear authoring error into a card an
    /// operator cannot act on.
    #[tokio::test]
    async fn a_slugless_tool_call_is_left_alone() {
        let mut node = tool_node("broken", "shell");
        node.config = json!({});
        let mut g = graph(vec![node]);

        let gated = apply_tool_call_gates(&mut g, &company("supervised", &[]), "wf", "run-1").await;

        assert!(gated.is_empty());
        assert!(gate_ids(&g).is_empty());
    }

    /// The load-bearing assumption of gating at translate time: for every tool
    /// a `tool_call` node can actually reach, the verdict is decided by the
    /// tool NAME alone. Node `args` may still carry unresolved `=`-expressions
    /// when this pass runs, so a tool whose classification depends on its
    /// arguments (`composio_execute` is the existing one) would be classified
    /// against a template and could be gated wrongly in either direction.
    ///
    /// None are reachable today — `WORKFLOW_TOOL_NAMESPACES` is `shell` /
    /// `code` / `web` / `search`, and `composio` is not in it. This fails the
    /// moment that stops being true, rather than letting a call slip the gate
    /// silently. Same stance `web_search_is_still_a_priced_call` takes in
    /// `consequence.rs`.
    #[test]
    fn every_reachable_workflow_tool_is_classified_by_name_alone() {
        use crate::policy::consequence_of;

        // Every tool the invoker wires, across all four reachable namespaces.
        for slug in [
            "shell",
            "read_workspace_state",
            "apply_patch",
            "git_operations",
            "csv_export",
            "web_fetch",
            "http_request",
            "curl",
            "image_info",
            "web_search",
        ] {
            let bare = consequence_of(slug, &json!({}));
            for args in [
                json!({ "action": "GMAIL_SEND_EMAIL" }),
                json!({ "amount_usd": 500.0 }),
                json!({ "command": "=item.cmd" }),
            ] {
                let with_args = consequence_of(slug, &args);
                assert_eq!(
                    bare.reach, with_args.reach,
                    "`{slug}` changes reach with args {args} — it can no longer be gated at \
                     translate time, where args may still be unresolved templates"
                );
            }
        }
    }
}
