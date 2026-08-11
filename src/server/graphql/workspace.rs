//! The workspace tree read: `Company.workspaceTree` / `workspaceFile` over the
//! [`WorkspaceStore`] port, with `[[wikilink]]` backlinks computed at read.

use std::sync::Arc;

use async_graphql::{ID, SimpleObject};

use super::iso8601;
use crate::company::runtime::CompanyRuntime;
use crate::company::workspace_links::file_with_backlinks;
use crate::ports::workspace::{NodeKind, WorkspaceNode, WorkspaceOrigin};

/// Who authored a workspace node. Mirrors [`WorkspaceOrigin`] (issue #326).
///
/// Flattened into `kind` + optional `agentId` rather than exposed as a GraphQL
/// union: `agentId` is non-null exactly when `kind` is `agent`, and a union of
/// two empty types plus one single-field type buys a client nothing but three
/// inline fragments. The Rust type keeps the invariant; this is its projection.
#[derive(SimpleObject, Clone)]
#[graphql(name = "WorkspaceOrigin")]
pub struct WorkspaceOriginGql {
    /// `seed`, `operator` or `agent`.
    pub kind: String,
    /// The agent's roster id — set only when `kind` is `agent`.
    pub agent_id: Option<String>,
}

impl From<WorkspaceOrigin> for WorkspaceOriginGql {
    fn from(origin: WorkspaceOrigin) -> Self {
        match origin {
            WorkspaceOrigin::Seed => Self {
                kind: "seed".to_string(),
                agent_id: None,
            },
            WorkspaceOrigin::Operator => Self {
                kind: "operator".to_string(),
                agent_id: None,
            },
            WorkspaceOrigin::Agent { id } => Self {
                kind: "agent".to_string(),
                agent_id: Some(id),
            },
        }
    }
}

/// One node (folder or file) in the workspace tree. Mirrors [`WorkspaceNode`].
#[derive(SimpleObject, Clone)]
#[graphql(name = "FsNode")]
pub struct FsNodeGql {
    /// The node id (stable ULID).
    pub id: ID,
    /// The node name.
    pub name: String,
    /// `folder` or `file`.
    pub kind: String,
    /// The parent node id, or null at the root.
    pub parent_id: Option<ID>,
    /// When it was last updated, ISO-8601 UTC.
    pub updated_at: String,
    /// Who created the node.
    pub created_by: WorkspaceOriginGql,
    /// Who last wrote the node's content (a rename or move does not change it).
    pub updated_by: WorkspaceOriginGql,
}

impl From<WorkspaceNode> for FsNodeGql {
    fn from(node: WorkspaceNode) -> Self {
        let kind = match node.kind {
            NodeKind::Folder => "folder",
            NodeKind::File => "file",
        };
        Self {
            id: ID(node.id),
            name: node.name,
            kind: kind.to_string(),
            parent_id: node.parent_id.map(ID),
            updated_at: iso8601(node.updated_at_millis),
            created_by: node.created_by.into(),
            updated_by: node.updated_by.into(),
        }
    }
}

/// A single workspace file with its content and inbound `[[wikilink]]` backlinks.
#[derive(SimpleObject)]
#[graphql(name = "WorkspaceFile")]
pub struct WorkspaceFileGql {
    /// The file id.
    pub id: ID,
    /// The file name.
    pub name: String,
    /// The file content.
    pub content: String,
    /// When it was last updated, ISO-8601 UTC.
    pub updated_at: String,
    /// Who created this file.
    pub created_by: WorkspaceOriginGql,
    /// Who last wrote the content above.
    pub updated_by: WorkspaceOriginGql,
    /// Other files whose content links to this one via `[[name]]`.
    pub backlinks: Vec<FsNodeGql>,
}

/// Resolves `Company.workspaceTree`.
pub(crate) async fn resolve_tree(
    runtime: &Arc<CompanyRuntime>,
) -> async_graphql::Result<Vec<FsNodeGql>> {
    let nodes = runtime.workspace().tree(runtime.id()).await?;
    Ok(nodes.into_iter().map(FsNodeGql::from).collect())
}

/// Resolves `Company.workspaceFile(id)`, returning null when absent.
///
/// The node + content + backlink scan is
/// [`file_with_backlinks`](crate::company::workspace_links::file_with_backlinks),
/// shared with the REST `GET …/workspace/file/{id}` route the console reads, so
/// the two surfaces can never report different backlinks for the same note.
pub(crate) async fn resolve_file(
    runtime: &Arc<CompanyRuntime>,
    id: &str,
) -> async_graphql::Result<Option<WorkspaceFileGql>> {
    let Some((node, content, backlinks)) =
        file_with_backlinks(runtime.workspace().as_ref(), runtime.id(), id).await?
    else {
        return Ok(None);
    };

    Ok(Some(WorkspaceFileGql {
        id: ID(node.id),
        name: node.name,
        content,
        updated_at: iso8601(node.updated_at_millis),
        created_by: node.created_by.into(),
        updated_by: node.updated_by.into(),
        backlinks: backlinks.into_iter().map(FsNodeGql::from).collect(),
    }))
}
