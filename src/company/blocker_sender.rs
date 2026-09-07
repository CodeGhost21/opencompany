//! Who a parked blocker is attributed to — the teammate whose direct message
//! it surfaces in (issue #1862).
//!
//! A blocker is a question, and a question in a company is asked *by* someone.
//! [`resolve_sender`] answers "by whom", so the card lands in the operator's DM
//! with the responsible teammate rather than in an undifferentiated queue.
//!
//! It is deliberately **dumb**: it reads whatever trigger-time attribution the
//! park already carries and picks the first rung that names a real roster
//! agent. It never inspects *why* the work stopped — the gap class is #1866's
//! job, decided once in [`blockers`](crate::harness::built_in::blockers) — and
//! never promises the stop will resume, which is #1863's boundary. All it
//! decides is whose name goes on the question.

use crate::ports::types::{CompanyRecord, StartedBy};

/// The host's own last-resort identity, used when nothing else named a sender.
///
/// A reserved channel slug ([`crate::runtime::channel`]) no company can give a
/// teammate, so a blocker with no attribution still resolves to a real,
/// collision-free DM channel instead of vanishing.
pub const HOST_SENDER: &str = "workflow";

/// The trigger-time attribution a park carries into sender resolution.
///
/// Every field is optional because each park site knows a different subset: a
/// workflow run knows its [`StartedBy`], a planning pass knows the card's
/// assignee, and neither knows the other's. An all-`None` bag is honest — it
/// means "nothing was named" and resolves to the orchestrator or the host.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockerSenderSignals {
    /// The [`StartedBy`] of the run this stop came from, when it came from one.
    /// Only [`StartedBy::Agent`] attributes a teammate; an operator- or
    /// schedule-started run names nobody to route to.
    pub started_by: Option<StartedBy>,
    /// The desk that owns the stopped work, resolved through its lead.
    pub owner_desk: Option<String>,
    /// The teammate the stopped card is assigned to — the planning pass's one
    /// piece of attribution.
    pub assignee: Option<String>,
}

/// Resolves the teammate a blocker is attributed to, as a roster agent id.
///
/// The rungs, first match wins:
/// 1. **The triggering agent.** A run an agent started owns its own stops.
/// 2. **The owning desk's lead.** A leadless [`Auto`] desk (issue #1835)
///    resolves `None` here — its per-message selector needs the message text a
///    park does not have — and falls through rather than guessing a member.
/// 3. **The card's assignee.** The planning pass's responsible teammate.
/// 4. **The orchestrator**, which answers anything otherwise unaddressed.
/// 5. **The host** ([`HOST_SENDER`]), so a blocker always lands somewhere real.
///
/// [`Auto`]: crate::ports::types::ResponderMode::Auto
pub fn resolve_sender(record: &CompanyRecord, signals: &BlockerSenderSignals) -> String {
    if let Some(StartedBy::Agent(id)) = &signals.started_by
        && record.is_roster_agent(id)
    {
        return id.clone();
    }
    if let Some(desk) = &signals.owner_desk
        && let Some(lead) = crate::runtime::delegation_tools::desk_lead(record, desk)
    {
        return lead;
    }
    if let Some(assignee) = &signals.assignee
        && record.is_roster_agent(assignee)
    {
        return assignee.clone();
    }
    if let Some(id) = crate::company::types::orchestrator_id(&record.effective_agents()) {
        return id.to_string();
    }
    HOST_SENDER.to_string()
}

/// The console channel a resolved sender's DM lives under — the id a park
/// stamps as its thread and a badge lands on.
pub fn dm_thread(sender: &str) -> String {
    format!("dm:{sender}")
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::ports::types::{CompanyId, CompanyRecord};

    /// A record from a bare manifest, with every overlay empty. Agents and desks
    /// come from the TOML; nothing else matters to sender resolution.
    fn record(manifest_toml: &str) -> CompanyRecord {
        CompanyRecord {
            id: CompanyId::new("acme"),
            manifest: toml::from_str(manifest_toml).expect("parse manifest"),
            ledger: Vec::new(),
            lifecycle: "running".to_string(),
            overlay_agents: Vec::new(),
            overlay_desk_members: Vec::new(),
            overlay_desk_order: Vec::new(),
            overlay_desks: Vec::new(),
            overlay_workflows: Vec::new(),
            overlay_agent_edits: Vec::new(),
            overlay_retired_agents: Vec::new(),
            overlay_policy: None,
            overlay_tool_grants: None,
            overlay_desk_tools: Default::default(),
            overlay_budgets: Vec::new(),
            disabled_workflows: Vec::new(),
            template_provenance: None,
            setup: None,
            name_confirmed: false,
            activation_completed_at: None,
            created_at_millis: None,
        }
    }

    const MANIFEST: &str = "[company]\nname = \"Acme\"\n\
         [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
         [[agent]]\nid = \"eng\"\nrole = \"Engineer\"\n\
         [[agent]]\nid = \"des\"\nrole = \"Designer\"\n\
         [[group_chat]]\nid = \"studio\"\nname = \"Studio\"\nmembers = [\"des\", \"eng\"]\n";

    #[test]
    fn the_triggering_agent_wins() {
        let record = record(MANIFEST);
        let sender = resolve_sender(
            &record,
            &BlockerSenderSignals {
                started_by: Some(StartedBy::Agent("eng".to_string())),
                owner_desk: Some("studio".to_string()),
                assignee: Some("des".to_string()),
            },
        );
        assert_eq!(
            sender, "eng",
            "the agent that started the run owns its stop"
        );
    }

    #[test]
    fn an_unknown_triggering_agent_falls_through() {
        let record = record(MANIFEST);
        let sender = resolve_sender(
            &record,
            &BlockerSenderSignals {
                started_by: Some(StartedBy::Agent("ghost".to_string())),
                owner_desk: Some("studio".to_string()),
                assignee: None,
            },
        );
        assert_eq!(
            sender, "des",
            "a started_by naming nobody on the roster does not win; the desk lead does"
        );
    }

    #[test]
    fn an_operator_started_run_does_not_attribute_an_agent() {
        let record = record(MANIFEST);
        let sender = resolve_sender(
            &record,
            &BlockerSenderSignals {
                started_by: Some(StartedBy::Operator),
                owner_desk: None,
                assignee: Some("eng".to_string()),
            },
        );
        assert_eq!(
            sender, "eng",
            "an operator-started run names no agent, so the assignee answers"
        );
    }

    #[test]
    fn the_owner_desk_lead_answers_when_no_agent_triggered() {
        let record = record(MANIFEST);
        let sender = resolve_sender(
            &record,
            &BlockerSenderSignals {
                started_by: None,
                owner_desk: Some("studio".to_string()),
                assignee: Some("ceo".to_string()),
            },
        );
        assert_eq!(
            sender, "des",
            "the desk's first member leads and outranks the assignee"
        );
    }

    #[test]
    fn the_assignee_answers_when_no_run_and_no_desk() {
        let record = record(MANIFEST);
        let sender = resolve_sender(
            &record,
            &BlockerSenderSignals {
                started_by: None,
                owner_desk: None,
                assignee: Some("eng".to_string()),
            },
        );
        assert_eq!(sender, "eng");
    }

    #[test]
    fn the_orchestrator_answers_an_unattributed_stop() {
        let record = record(MANIFEST);
        let sender = resolve_sender(&record, &BlockerSenderSignals::default());
        assert_eq!(
            sender, "ceo",
            "with nothing named, the first (orchestrator) agent answers"
        );
    }

    #[test]
    fn the_host_answers_when_the_company_has_no_roster() {
        let record = record("[company]\nname = \"Empty\"\n");
        let sender = resolve_sender(
            &record,
            &BlockerSenderSignals {
                started_by: Some(StartedBy::Agent("nobody".to_string())),
                owner_desk: Some("nowhere".to_string()),
                assignee: Some("nobody".to_string()),
            },
        );
        assert_eq!(
            sender, HOST_SENDER,
            "no roster names anyone; the host takes it"
        );
    }

    #[test]
    fn the_dm_thread_is_the_console_channel_form() {
        assert_eq!(dm_thread("eng"), "dm:eng");
        assert_eq!(dm_thread(HOST_SENDER), "dm:workflow");
    }
}
