//! The company-wide `#general` channel: its identity and its stored
//! membership.

use serde::{Deserialize, Serialize};

use crate::ports::types::CompanyRecord;

/// The id of the company-wide channel, stamped on every message written to it.
pub const GENERAL_CHANNEL_ID: &str = "general";

/// The display name of the company-wide channel.
pub const GENERAL_CHANNEL_NAME: &str = "General";

/// The company-wide channel as a stored entity.
///
/// Created with the company and kept on its record. Its membership is always
/// the non-retired roster, maintained by the record's roster writers through
/// [`CompanyRecord::sync_general_members`]; nothing edits it by hand.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneralChannel {
    /// Always [`GENERAL_CHANNEL_ID`].
    pub id: String,
    /// Always [`GENERAL_CHANNEL_NAME`].
    pub name: String,
    /// Every non-retired roster teammate, manifest order then overlay order.
    #[serde(default)]
    pub members: Vec<String>,
}

impl Default for GeneralChannel {
    fn default() -> Self {
        Self {
            id: GENERAL_CHANNEL_ID.to_string(),
            name: GENERAL_CHANNEL_NAME.to_string(),
            members: Vec::new(),
        }
    }
}

/// How a roster change moved `#general`'s membership.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GeneralMembershipDelta {
    /// Ids that joined.
    pub added: Vec<String>,
    /// Ids that left.
    pub removed: Vec<String>,
}

impl GeneralMembershipDelta {
    /// Whether nothing moved.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

impl CompanyRecord {
    /// Recomputes `#general`'s members from the roster and restores its
    /// identity, returning who joined and who left.
    pub fn sync_general_members(&mut self) -> GeneralMembershipDelta {
        let mut roster: Vec<String> = Vec::new();
        for id in self
            .manifest
            .agents
            .iter()
            .map(|agent| agent.id.as_str())
            .chain(self.overlay_agents.iter().map(|agent| agent.id.as_str()))
        {
            if !self.is_retired(id) && !roster.iter().any(|seen| seen == id) {
                roster.push(id.to_string());
            }
        }
        let previous = std::mem::take(&mut self.general_channel.members);
        let delta = GeneralMembershipDelta {
            added: roster
                .iter()
                .filter(|id| !previous.contains(id))
                .cloned()
                .collect(),
            removed: previous
                .iter()
                .filter(|id| !roster.contains(id))
                .cloned()
                .collect(),
        };
        self.general_channel.id = GENERAL_CHANNEL_ID.to_string();
        self.general_channel.name = GENERAL_CHANNEL_NAME.to_string();
        self.general_channel.members = roster;
        delta
    }
}

#[cfg(test)]
#[path = "general_channel_tests.rs"]
mod tests;
