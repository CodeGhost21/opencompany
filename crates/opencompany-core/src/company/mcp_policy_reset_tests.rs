//! `reset_company_policy`: a company-scoped reset must not also erase every
//! teammate's own rule, and a document that cannot be read still has to fall
//! back to the pre-existing full wipe.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;

use super::*;

#[derive(Default)]
struct MemSecrets {
    map: Mutex<HashMap<String, String>>,
    fail_reads: bool,
}

#[async_trait]
impl SecretStore for MemSecrets {
    async fn get(&self, _c: &CompanyId, key: &str) -> Result<Option<SecretValue>> {
        if self.fail_reads {
            return Err(OpenCompanyError::Store("store is down".into()));
        }
        Ok(self
            .map
            .lock()
            .unwrap()
            .get(key)
            .map(|v| SecretValue(v.clone())))
    }
    async fn set(&self, _c: &CompanyId, key: &str, value: SecretValue) -> Result<()> {
        self.map.lock().unwrap().insert(key.to_string(), value.0);
        Ok(())
    }
}

fn company() -> CompanyId {
    CompanyId::new("acme")
}

fn with_blocked_agent(agent: &str, tool: &str) -> McpToolPolicies {
    let mut policies = McpToolPolicies::default();
    policies.agents.insert(
        agent.to_string(),
        AgentToolPolicies {
            overrides: [(
                tool.to_string(),
                ToolPolicy {
                    tier: None,
                    mode: Some(ApprovalMode::Blocked),
                },
            )]
            .into_iter()
            .collect(),
        },
    );
    policies
}

/// The bug a bare `save(McpToolPolicies::default())` reset had: replacing the
/// whole document drops every entry in `agents`, silently widening what a
/// blocked teammate can call on their very next turn.
#[tokio::test]
async fn a_reset_preserves_a_teammates_blocked_rule() {
    let secrets = MemSecrets::default();
    let key = "mcp/notion/tool_policies";
    let mut before = with_blocked_agent("writer", "delete_page");
    before
        .tier_defaults
        .insert(ToolTier::ReadOnly, ApprovalMode::AlwaysAllow);
    before.overrides.insert(
        "delete_page".into(),
        ToolPolicy {
            tier: None,
            mode: Some(ApprovalMode::Blocked),
        },
    );
    save_tool_policies(&company(), &secrets, key, &before)
        .await
        .expect("seeded");

    let replacement = reset_company_policy(&company(), &secrets, key)
        .await
        .expect("reset");

    // The company half is wiped — that is what "reset" asked for.
    assert!(replacement.tier_defaults.is_empty());
    assert!(replacement.overrides.is_empty());
    // The teammate's own rule is not.
    assert_eq!(
        replacement
            .agents
            .get("writer")
            .and_then(|rules| rules.overrides.get("delete_page"))
            .and_then(|row| row.mode),
        Some(ApprovalMode::Blocked),
        "a company-wide reset must not also drop a teammate's own rule"
    );

    // And what got saved is what was returned, not just what was computed.
    let persisted = load_tool_policies_strict(&company(), &secrets, key)
        .await
        .expect("readable")
        .expect("present");
    assert_eq!(persisted, replacement);
}

/// A company with no per-agent rules resets to the plain empty document —
/// preserving nothing does not mean inventing something.
#[tokio::test]
async fn a_reset_with_no_agent_rules_is_the_empty_document() {
    let secrets = MemSecrets::default();
    let key = "mcp/notion/tool_policies";
    let mut before = McpToolPolicies::default();
    before.overrides.insert(
        "delete_page".into(),
        ToolPolicy {
            tier: None,
            mode: Some(ApprovalMode::Blocked),
        },
    );
    save_tool_policies(&company(), &secrets, key, &before)
        .await
        .expect("seeded");

    let replacement = reset_company_policy(&company(), &secrets, key)
        .await
        .expect("reset");
    assert_eq!(replacement, McpToolPolicies::default());
}

/// An unreadable document carries no `agents` map to preserve, so the reset
/// falls back to the full wipe rather than refusing — the same repair path the
/// route's own comment documents for `reset_policy`'s unreadable case.
#[tokio::test]
async fn a_reset_of_an_unreadable_document_still_repairs_to_empty() {
    let secrets = MemSecrets {
        fail_reads: true,
        ..Default::default()
    };
    let key = "mcp/notion/tool_policies";

    let replacement = reset_company_policy(&company(), &secrets, key)
        .await
        .expect("reset still succeeds — it never reads-then-fails, it repairs");
    assert_eq!(replacement, McpToolPolicies::default());
}
