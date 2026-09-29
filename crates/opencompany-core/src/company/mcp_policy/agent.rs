//! The per-agent layer of a server's tool policy: one teammate's own decisions
//! about the tools on a server the company already reaches.
//!
//! The company document ([`McpToolPolicies`]) says what a tool does for
//! everyone. This layer says what it does for *one* teammate, and it may only
//! **narrow**: a stored per-agent mode moves the resolved mode along
//! `AlwaysAllow < NeedsApproval < Blocked` in the restricting direction, never
//! back. Three reasons, in ascending order of how badly the alternative fails:
//!
//! 1. Every other per-agent layer in this crate is an intersection
//!    ([`agent_scoped_grants`](crate::runtime::builder::agent_scoped_grants)), so
//!    a widening layer would be the one exception to the operator's model.
//! 2. The enforcement seam can only express restriction. The attached server
//!    carries a deny list, and the transport's filter consults deny before
//!    allow, so a widening rule would be silently ignored — worse than refusing
//!    to express it.
//! 3. [`resolve_policy`] already refuses to let a mere suggestion reach
//!    [`ApprovalMode::AlwaysAllow`]. A per-agent widening would be a second
//!    route to it, invisible on the server's own page.
//!
//! Narrow-only is therefore a property of the type:
//! [`ApprovalMode::max_restrictive`] is the only way a per-agent mode reaches
//! the result, and a stored setting the clamp discards is reported as
//! [`PolicySource::AgentClamped`] rather than dropped — a console that rendered
//! a control whose value the host throws away is the failure this crate has
//! already shipped once.
//!
//! No per-agent tier defaults, deliberately: a tier classifies the *tool*, not
//! the teammate, and a per-agent tier would give the tier a second source as
//! well as the mode.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{
    ApprovalMode, McpToolInventory, McpToolPolicies, ResolvedPolicy, ToolPolicy, policy_tool_names,
    resolve_policy,
};
use crate::company::mcp::McpServerDecl;
use crate::runtime::tools::grants_cover_server;

/// One teammate's own tool decisions on one server.
///
/// A wrapper struct rather than a bare map so a later per-agent field (who set
/// it, and when) is not a wire break.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentToolPolicies {
    /// This teammate's per-tool decisions. Ordered so the document is
    /// byte-stable across writes and the fingerprint over it is canonical.
    #[serde(default)]
    pub overrides: BTreeMap<String, ToolPolicy>,
}

impl AgentToolPolicies {
    /// Whether this entry carries any decision. An empty entry is pruned on
    /// write, so a reset leaves no residue — residue resolves identically but
    /// hashes differently, which would move the fingerprint on a write that
    /// changed nothing and rebuild every roster.
    pub fn is_empty(&self) -> bool {
        self.overrides.is_empty()
    }

    /// Drops per-tool rows that decide nothing.
    pub fn prune(&mut self) {
        self.overrides.retain(|_, policy| !policy.is_empty());
    }
}

/// Which rule decided a tool's mode for one teammate.
///
/// [`Self::AgentClamped`] exists to name a *discarded* setting: the teammate has
/// a stored mode less restrictive than the server's, so the server's mode stands
/// and the console must say so instead of rendering the stored value as live.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicySource {
    /// Nothing is pinned anywhere: the tier default or the fallback decided.
    ServerInherited,
    /// The company document pins this tool, and the teammate adds nothing.
    ServerPinned,
    /// The teammate's own mode decided.
    AgentPinned,
    /// The teammate's stored mode would widen, so it was discarded.
    AgentClamped,
}

impl PolicySource {
    /// The source of a mode no per-agent rule touched.
    pub fn for_server(is_override: bool) -> Self {
        if is_override {
            PolicySource::ServerPinned
        } else {
            PolicySource::ServerInherited
        }
    }
}

/// One tool's policy as it stands for one teammate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedPolicyForAgent {
    /// What the server's own page says, byte-for-byte what [`resolve_policy`]
    /// answers. Carried so the two views cannot drift.
    pub server: ResolvedPolicy,
    /// The mode enforced for this teammate.
    pub mode: ApprovalMode,
    /// The teammate's stored mode, present even when the clamp discarded it —
    /// naming it is what lets a console explain the refusal instead of showing
    /// a value nothing honours.
    pub asked: Option<ApprovalMode>,
    /// Which rule decided [`Self::mode`].
    pub source: PolicySource,
}

/// Resolves one tool's mode for one teammate.
///
/// The server's answer is computed by [`resolve_policy`] unchanged, then the
/// teammate's own mode — when it has one — is folded in with
/// [`ApprovalMode::max_restrictive`]. A document with no `agents` key gives
/// `asked = None`, so the result is the server's answer with no clamp: upgrade
/// neutrality holds by serde and one `if`, not by care.
///
/// An agent id nobody has written a rule for resolves the same way, which is
/// what makes an unknown or retired teammate harmless rather than a special
/// case.
pub fn resolve_policy_for_agent(
    policies: &McpToolPolicies,
    agent: &str,
    tool: &str,
    suggested: Option<crate::company::mcp_policy::ToolTier>,
) -> ResolvedPolicyForAgent {
    let server = resolve_policy(policies, tool, suggested);
    let asked = policies
        .agents
        .get(agent)
        .and_then(|entry| entry.overrides.get(tool))
        .and_then(|policy| policy.mode);
    let mode = asked.map_or(server.mode, |a| server.mode.max_restrictive(a));
    let source = match asked {
        None => PolicySource::for_server(server.is_override),
        Some(a) if a == mode => PolicySource::AgentPinned,
        Some(_) => PolicySource::AgentClamped,
    };
    ResolvedPolicyForAgent {
        server,
        mode,
        asked,
        source,
    }
}

/// Every tool name a decision can be stated about for one teammate: the server's
/// own set plus the tools only this teammate has a rule for.
///
/// The second half matters for the deny list: a teammate can be blocked on a
/// tool no probe has reached and no company override names, and the block has to
/// reach the attachment anyway.
pub fn agent_policy_tool_names(
    policies: &McpToolPolicies,
    inventory: &McpToolInventory,
    agent: &str,
) -> impl Iterator<Item = String> {
    let mut names: BTreeSet<String> = policy_tool_names(policies, inventory).collect();
    if let Some(entry) = policies.agents.get(agent) {
        names.extend(entry.overrides.keys().cloned());
    }
    names.into_iter()
}

/// Whether this teammate is refused `tool` on this server outright.
pub fn blocks_tool_for_agent(
    policies: &McpToolPolicies,
    inventory: &McpToolInventory,
    agent: &str,
    tool: &str,
) -> bool {
    resolve_policy_for_agent(policies, agent, tool, inventory.suggested(tool)).mode
        == ApprovalMode::Blocked
}

/// Every tool this server refuses this teammate outright, sorted.
///
/// The per-agent face of [`blocked_tool_names`](super::blocked_tool_names), and
/// a superset of it by construction: the per-agent layer only restricts, and the
/// name set only grows. Sorted the same way, so a document with no `agents` key
/// produces the same list in the same order — which is what makes the attachment
/// byte-identical across the upgrade.
pub fn blocked_tool_names_for_agent(
    policies: &McpToolPolicies,
    inventory: &McpToolInventory,
    agent: &str,
) -> Vec<String> {
    let mut names: Vec<String> = agent_policy_tool_names(policies, inventory, agent)
        .filter(|tool| blocks_tool_for_agent(policies, inventory, agent, tool))
        .collect();
    names.sort();
    names
}

/// The `(server, tool)` pairs one teammate's approval gate lets run without
/// parking.
///
/// Two narrowings over [`mcp_allow_set`](super::mcp_allow_set), both of which
/// can only remove pairs: this teammate's own modes, and the servers its grants
/// reach. The second is not a per-agent-policy consequence — the company-wide
/// set never filtered by reach at all, so a teammate's gate treated a pair on a
/// server it cannot dial as a declared read.
///
/// Enumerates the server's own tool names rather than the widened per-agent set,
/// so this is a strict subset of the company-wide answer. A tool only a
/// per-agent rule names cannot be less restrictive than the server's mode, so
/// leaving it out can only park more.
pub fn mcp_allow_set_for_agent(
    servers: &[McpServerDecl],
    agent: &str,
    grants: &[String],
) -> crate::policy::McpReadSet {
    crate::policy::McpReadSet::from_pairs(
        servers
            .iter()
            .filter(|server| server.enabled && grants_cover_server(grants, &server.name))
            .flat_map(|server| {
                policy_tool_names(&server.tool_policies, &server.tool_inventory)
                    .filter(|tool| {
                        resolve_policy_for_agent(
                            &server.tool_policies,
                            agent,
                            tool,
                            server.tool_inventory.suggested(tool),
                        )
                        .mode
                            == ApprovalMode::AlwaysAllow
                    })
                    .map(move |tool| (server.name.clone(), tool))
            }),
    )
}

/// The teammates whose resolved mode for `tool` differs from the server's,
/// sorted by agent id.
///
/// What stops the company-wide view lying by omission. That view is true about
/// the company document and silent about its exceptions, and an auditor reading
/// it has no other way to learn they exist.
pub fn differing_agents(
    policies: &McpToolPolicies,
    inventory: &McpToolInventory,
    tool: &str,
) -> Vec<String> {
    let suggested = inventory.suggested(tool);
    let server = resolve_policy(policies, tool, suggested);
    policies
        .agents
        .keys()
        .filter(|agent| {
            resolve_policy_for_agent(policies, agent, tool, suggested).mode != server.mode
        })
        .cloned()
        .collect()
}

/// Whether `tool` cannot be called on this server by this teammate at all.
///
/// Composes the three refusals the attachment composes — off the declaration's
/// allow list, on its deny list, or blocked by the resolved policy — so a
/// statement made from this cannot disagree with what the transport will do.
pub fn refuses_tool_for_agent(decl: &McpServerDecl, agent: &str, tool: &str) -> bool {
    if !decl.allowed_tools.is_empty() && !decl.allowed_tools.iter().any(|t| t == tool) {
        return true;
    }
    if decl.disallowed_tools.iter().any(|t| t == tool) {
        return true;
    }
    blocks_tool_for_agent(&decl.tool_policies, &decl.tool_inventory, agent, tool)
}

/// Whether every tool this server is *known* to offer is refused this teammate.
///
/// A server no probe has reached offers no known tool, and answering `true` for
/// it would report "you can call nothing here" from an absence of evidence. That
/// distinction is the whole point: this drives a claim in an agent's prompt, and
/// "nothing is callable" and "nothing is known" are different facts.
pub fn every_known_tool_refused(decl: &McpServerDecl, agent: &str) -> bool {
    !decl.tool_inventory.tools.is_empty()
        && decl
            .tool_inventory
            .tools
            .keys()
            .all(|tool| refuses_tool_for_agent(decl, agent, tool))
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
