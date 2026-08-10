//! The [`WorkspaceStore`] port: the company's durable file tree.
//!
//! The workspace is an Obsidian-style tree of folders and Markdown notes the
//! operator organizes, edits, and links with `[[wiki links]]`. Node ids are
//! stable ULIDs, **not** paths, so a rename or move never breaks a reference.
//! The tree is seeded once from `companies/<name>/workspace/**` (WS1 walker)
//! and thereafter written by both the operator and the company's agents.
//!
//! # Authorship (issue #326)
//!
//! Every node records two origins: [`WorkspaceNode::created_by`], fixed at
//! creation, and [`WorkspaceNode::updated_by`], restamped by each content
//! write. Both are a [`WorkspaceOrigin`]. Without them a note an agent wrote is
//! indistinguishable from one the operator typed, which is untenable now that
//! agents can create notes as well as overwrite them (issue #551).
//!
//! The split is deliberate: `rename_move` does **not** touch `updated_by`, so
//! an operator tidying an agent's note into a different folder cannot make the
//! body look operator-authored.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::Result;
use crate::ports::types::CompanyId;

/// Whether a workspace node is a folder or a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    /// A directory that may contain other nodes.
    Folder,
    /// A file with Markdown content.
    File,
}

/// Who authored a workspace node.
///
/// Internally tagged, so one serde shape serves all three surfaces this value
/// crosses: the opaque `node_json` blob every backend persists, the REST wire
/// body the console reads, and the TypeScript type mirroring it. An agent
/// origin is `{"kind":"agent","id":"ceo"}`; the other two are just
/// `{"kind":"seed"}` / `{"kind":"operator"}`.
///
/// # Why this is not [`ActorKind`](crate::ports::types::ActorKind)
///
/// `ActorKind` is the crate's established "who did this" enum and is
/// deliberately fieldless and `Copy`, with the id carried alongside it in
/// [`Actor::id`](crate::ports::types::Actor). This type diverges from that
/// convention on purpose, for two reasons:
///
/// * **`Seed` is not an actor.** A node walked out of
///   `companies/<name>/workspace/**` at first boot was authored by nobody — not
///   the operator, not an agent. Folding it into `ActorKind::System` would make
///   "the runtime wrote this" and "this shipped with the company" the same
///   badge in the console, erasing the distinction issue #326 exists to draw.
/// * **The flat alternative is worse across seven surfaces.** Four independent
///   fields (`created_by_kind` + `created_by_id` + `updated_by_kind` +
///   `updated_by_id`) would make the store JSON, the REST body, the GraphQL
///   type, the TypeScript type, the console, the seeder and the agent tools
///   each re-derive the invariant "an id is present iff the kind is agent". One
///   enum states it once and serde enforces it at every boundary.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum WorkspaceOrigin {
    /// Walked out of the company bundle at first boot by the WS1 seeder.
    Seed,
    /// A human operator, through the console or the REST routes.
    ///
    /// The default, and therefore what every node written before authorship
    /// existed deserializes to. That is a one-time, direction-honest
    /// misattribution: a legacy seeded note reads as operator-authored, which
    /// is the conservative answer (it never credits an agent for something it
    /// did not write).
    #[default]
    Operator,
    /// An agent inside this company, named by its roster id.
    Agent {
        /// The agent's roster id, e.g. `ceo`.
        id: String,
    },
}

/// One node in the workspace tree. `id` is a stable ULID; `parent_id` is `None`
/// at the workspace root.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceNode {
    /// Stable ULID id.
    pub id: String,
    /// Display name (including any extension).
    pub name: String,
    /// Whether this node is a folder or a file.
    pub kind: NodeKind,
    /// The parent folder's id, or `None` at the root.
    #[serde(default)]
    pub parent_id: Option<String>,
    /// Epoch-millis timestamp of the last update.
    pub updated_at_millis: u64,
    /// Who created this node. Never restamped — the creator of a note is a
    /// fact about its history, not about its current body.
    #[serde(default)]
    pub created_by: WorkspaceOrigin,
    /// Who last wrote this node's **content**.
    ///
    /// Restamped by [`WorkspaceStore::write`] and by nothing else — in
    /// particular not by [`WorkspaceStore::rename_move`], so an operator
    /// reorganising the tree cannot mask agent authorship of the body that is
    /// actually stored.
    #[serde(default)]
    pub updated_by: WorkspaceOrigin,
}

/// Durable per-company workspace tree. Company A's files MUST be invisible to
/// company B.
#[async_trait]
pub trait WorkspaceStore: Send + Sync {
    /// Returns every node in the tree (order unspecified; callers build the
    /// tree from `parent_id`).
    async fn tree(&self, company: &CompanyId) -> Result<Vec<WorkspaceNode>>;
    /// Reads one node and, for files, its content. Folders yield an empty body.
    async fn read(&self, company: &CompanyId, id: &str) -> Result<Option<(WorkspaceNode, String)>>;
    /// Overwrites a file's content, returning the updated node. A folder id is
    /// an [`OpenCompanyError::InvalidRequest`](crate::error::OpenCompanyError).
    ///
    /// `author` is stamped onto [`WorkspaceNode::updated_by`]; it is the
    /// caller's identity, never anything derived from `content`.
    async fn write(
        &self,
        company: &CompanyId,
        id: &str,
        content: &str,
        author: WorkspaceOrigin,
    ) -> Result<WorkspaceNode>;
    /// Creates a node (folder or file). The node's `id` must be fresh; the
    /// `parent_id`, when set, must name an existing folder. `content` seeds a
    /// file body.
    ///
    /// No `author` argument: the node arrives fully formed, so the caller sets
    /// [`WorkspaceNode::created_by`] and [`WorkspaceNode::updated_by`] on it
    /// directly.
    async fn create(
        &self,
        company: &CompanyId,
        node: &WorkspaceNode,
        content: Option<&str>,
    ) -> Result<()>;
    /// Renames and/or reparents a node, returning the updated node. Moving a
    /// folder under its own descendant (a cycle) is rejected.
    ///
    /// Leaves both origin fields alone — see [`WorkspaceNode::updated_by`].
    ///
    /// `parent` distinguishes three intents: `None` leaves the parent
    /// unchanged, `Some(None)` moves the node to the workspace root, and
    /// `Some(Some(id))` reparents it under folder `id`.
    async fn rename_move(
        &self,
        company: &CompanyId,
        id: &str,
        name: Option<&str>,
        parent: Option<Option<&str>>,
    ) -> Result<WorkspaceNode>;
    /// Deletes a node; folders are removed recursively. Returns whether a node
    /// was removed.
    async fn delete(&self, company: &CompanyId, id: &str) -> Result<bool>;
    /// Whether the workspace has no nodes — the gate the seeder checks so a
    /// seeded-then-emptied workspace is never re-seeded.
    async fn is_empty(&self, company: &CompanyId) -> Result<bool>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every backend persists a node as opaque JSON, so a node written before
    /// authorship existed has neither field. It must still load — and must load
    /// as `Operator`, the conservative answer that never credits an agent for
    /// something it did not write.
    ///
    /// This is the whole of the migration story: no `ALTER TABLE`, no
    /// `add_column_if_missing`, no backfill.
    #[test]
    fn a_legacy_node_without_origins_loads_as_operator() {
        let legacy = r#"{
            "id": "n-1",
            "name": "voice.md",
            "kind": "file",
            "parentId": null,
            "updatedAtMillis": 1700000000000
        }"#;
        let node: WorkspaceNode = serde_json::from_str(legacy).expect("legacy node must load");
        assert_eq!(node.created_by, WorkspaceOrigin::Operator);
        assert_eq!(node.updated_by, WorkspaceOrigin::Operator);
    }

    /// The internally-tagged wire shape, pinned.
    ///
    /// The same bytes are read by three independent consumers — the stores'
    /// `node_json`, the REST body the console parses, and the GraphQL
    /// projection — so a stray `rename_all` or a switch to an adjacently-tagged
    /// representation would break the console at runtime with nothing in Rust
    /// CI noticing. This test is what turns that into a compile-suite failure.
    #[test]
    fn the_agent_origin_wire_shape_is_tagged_kind_plus_id() {
        let agent = WorkspaceOrigin::Agent {
            id: "ceo".to_string(),
        };
        assert_eq!(
            serde_json::to_value(&agent).unwrap(),
            serde_json::json!({ "kind": "agent", "id": "ceo" })
        );
        assert_eq!(
            serde_json::to_value(WorkspaceOrigin::Seed).unwrap(),
            serde_json::json!({ "kind": "seed" })
        );
        assert_eq!(
            serde_json::to_value(WorkspaceOrigin::Operator).unwrap(),
            serde_json::json!({ "kind": "operator" })
        );

        // …and back, so the shape is a round trip rather than a one-way render.
        let parsed: WorkspaceOrigin =
            serde_json::from_value(serde_json::json!({ "kind": "agent", "id": "ceo" })).unwrap();
        assert_eq!(parsed, agent);
    }

    /// A node carrying both origins round-trips through the exact `node_json`
    /// path the backends use.
    #[test]
    fn a_node_round_trips_both_origins() {
        let node = WorkspaceNode {
            id: "n-1".to_string(),
            name: "brief.md".to_string(),
            kind: NodeKind::File,
            parent_id: Some("f-1".to_string()),
            updated_at_millis: 42,
            created_by: WorkspaceOrigin::Agent {
                id: "cmo".to_string(),
            },
            updated_by: WorkspaceOrigin::Operator,
        };
        let json = serde_json::to_string(&node).unwrap();
        assert_eq!(serde_json::from_str::<WorkspaceNode>(&json).unwrap(), node);
        // camelCase on the node, matching every other field on it.
        assert!(json.contains("\"createdBy\""), "{json}");
        assert!(json.contains("\"updatedBy\""), "{json}");
    }
}
