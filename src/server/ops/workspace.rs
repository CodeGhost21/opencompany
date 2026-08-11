//! Workspace reads + writes: list the tree, read one file with its backlinks,
//! create a node, overwrite a file, rename/move a node, and delete (folders
//! recursive) — under both scope forms.
//!
//! Bodies mirror the console's `FsNode` (`frontend/src/api/workspace.ts`).
//! Writes land in the [`WorkspaceStore`](crate::ports::WorkspaceStore); node
//! ids are stable ULIDs so a rename/move never breaks a reference.
//!
//! ```text
//! GET    …/workspace                  the whole tree (metadata; no bodies)
//! GET    …/workspace/file/{nodeId}    one file: content + inbound backlinks
//! POST   …/workspace                  create a folder/file (or upload)
//! PUT    …/workspace/file/{nodeId}    overwrite file content
//! PATCH  …/workspace/{nodeId}         rename / move
//! DELETE …/workspace/{nodeId}         delete a node (folders recursive)
//! ```
//!
//! ## Why the two `GET`s are REST and not GraphQL
//!
//! Every other console read goes through GraphQL, and `Company.workspaceTree` /
//! `workspaceFile` have existed since the read plane landed — but the operator
//! console ships **no GraphQL client**. Reaching them from the Workspace tab
//! would mean a second wire protocol, a second auth path, a second error
//! envelope and ISO-8601 string timestamps in a view whose siblings all use
//! epoch millis. These twins keep the console on one client; the backlink scan
//! itself is shared with the resolver
//! ([`file_with_backlinks`](crate::company::workspace_links::file_with_backlinks))
//! so the two surfaces cannot answer differently.
//!
//! ## Known limits (issue #177 — documented, not worked around)
//!
//! * **No live push.** A write that lands while the tab is open is only visible
//!   on a refetch (refresh button / window focus). Tracked by issue #327.
//! * **No CAS on the console write path.** Agent writes require an
//!   `expected_updated_at` compare-and-swap token; the console `PUT` does not,
//!   so a concurrent agent write can be overwritten by the operator's save. That
//!   is the store's stated design — the operator is the dominant editor, and the
//!   agent's *next* CAS write fails and re-reads, so the agent side self-heals.

use axum::extract::Path;
use axum::http::StatusCode;
use axum::routing::{patch, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::company::artifact_mirror::{MirrorOutcome, mirror_node_edit};
use crate::company::workspace_links::file_with_backlinks;
use crate::error::OpenCompanyError;
use crate::ports::artifacts::ArtifactAuthor;
use crate::ports::generate_id;
use crate::ports::workspace::{NodeKind, WorkspaceNode, WorkspaceOrigin};
use crate::server::error::ApiError;
use crate::server::ops::artifacts::OPERATOR_EDIT_NOTE;
use crate::server::ops::{ScopedCompany, scoped};

/// Builds the workspace route fragment.
pub fn router() -> Router<AppState> {
    scoped("/workspace", post(create_node).get(list_tree))
        .merge(scoped(
            "/workspace/file/{node_id}",
            put(write_file).get(read_file),
        ))
        .merge(scoped(
            "/workspace/{node_id}",
            patch(rename_move).delete(delete_node),
        ))
}

/// A workspace node as the console renders it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FsNode {
    id: String,
    name: String,
    kind: NodeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    updated_at: u64,
    /// Who created the node, and who last wrote its body (issue #326).
    ///
    /// Always serialized, unlike `parentId` / `content` above: the console
    /// renders an authorship badge off these, and an absent field there would
    /// be indistinguishable from "unknown" when the honest answer is
    /// `operator`. The port already defaults a legacy node to `operator`, so
    /// this is never null.
    created_by: WorkspaceOrigin,
    updated_by: WorkspaceOrigin,
}

impl FsNode {
    fn from_node(node: WorkspaceNode, content: Option<String>) -> Self {
        Self {
            id: node.id,
            name: node.name,
            kind: node.kind,
            parent_id: node.parent_id,
            content,
            updated_at: node.updated_at_millis,
            created_by: node.created_by,
            updated_by: node.updated_by,
        }
    }
}

/// One workspace file with its body and the notes that link to it.
///
/// The REST twin of the GraphQL `WorkspaceFile`, differing only in timestamp
/// shape (epoch millis, like every other console read) — the backlinks come
/// from the same shared scan.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceFileBody {
    id: String,
    name: String,
    content: String,
    updated_at: u64,
    /// Who created this note, and who last wrote the body above (issue #326).
    created_by: WorkspaceOrigin,
    updated_by: WorkspaceOrigin,
    /// Other files whose content links to this one via `[[name]]`.
    backlinks: Vec<FsNode>,
}

/// The create-node body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateNode {
    name: String,
    kind: NodeKind,
    #[serde(default)]
    parent_id: Option<String>,
    #[serde(default)]
    content: Option<String>,
}

/// The overwrite-file body.
#[derive(Debug, Deserialize)]
struct WriteFile {
    content: String,
}

/// The rename/move body.
///
/// `parent_id` uses a double option so an omitted `parentId` (leave the parent
/// unchanged) is distinguished from an explicit `"parentId": null` (move the
/// node back to the workspace root).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RenameMove {
    #[serde(default)]
    name: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    parent_id: Option<Option<String>>,
}

/// Deserializes into `Some(inner)` when the field is present (so an explicit
/// `null` becomes `Some(None)`); the `#[serde(default)]` leaves an omitted field
/// as `None`.
fn double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(deserializer).map(Some)
}

/// The overwrite-file response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WriteAck {
    updated_at: u64,
}

/// The sub-resource path (`node_id`).
#[derive(Debug, Deserialize)]
struct NodePath {
    node_id: String,
}

/// `GET …/workspace` — every node in the tree, metadata only.
///
/// Bodies are deliberately omitted: a tree read is the console's *navigation*
/// call and happens on every mount, focus and refresh, so shipping every note's
/// content would make it grow without bound with the workspace. The console
/// fetches a body when a note is opened ([`read_file`]) — the same
/// index-then-fetch split the agent-facing `workspace_list` / `workspace_read`
/// tools make.
async fn list_tree(company: ScopedCompany) -> Result<Json<Vec<FsNode>>, ApiError> {
    let nodes = company.runtime.workspace().tree(company.id()).await?;
    Ok(Json(
        nodes
            .into_iter()
            .map(|node| FsNode::from_node(node, None))
            .collect(),
    ))
}

/// `GET …/workspace/file/{node_id}` — one file's content plus the notes that
/// link to it.
///
/// A folder id 404s rather than answering with an empty body: the console only
/// ever opens files, so a folder id here is a caller bug, and reporting it as an
/// empty note would hide it.
async fn read_file(
    company: ScopedCompany,
    Path(NodePath { node_id }): Path<NodePath>,
) -> Result<Json<WorkspaceFileBody>, ApiError> {
    let found =
        file_with_backlinks(company.runtime.workspace().as_ref(), company.id(), &node_id).await?;
    let Some((node, content, backlinks)) = found.filter(|(node, _, _)| node.kind == NodeKind::File)
    else {
        return Err(ApiError(OpenCompanyError::CompanyNotFound(format!(
            "workspace file {node_id}"
        ))));
    };
    Ok(Json(WorkspaceFileBody {
        id: node.id,
        name: node.name,
        content,
        updated_at: node.updated_at_millis,
        created_by: node.created_by,
        updated_by: node.updated_by,
        backlinks: backlinks
            .into_iter()
            .map(|node| FsNode::from_node(node, None))
            .collect(),
    }))
}

async fn create_node(
    company: ScopedCompany,
    Json(body): Json<CreateNode>,
) -> Result<Json<FsNode>, ApiError> {
    let node = WorkspaceNode {
        id: generate_id(),
        name: body.name,
        kind: body.kind,
        parent_id: body.parent_id,
        updated_at_millis: crate::ports::now_millis(),
        // These routes are the console's, and the console is the operator.
        created_by: WorkspaceOrigin::Operator,
        updated_by: WorkspaceOrigin::Operator,
        mime: None,
        size: None,
        sha256: None,
    };
    company
        .runtime
        .workspace()
        .create(company.id(), &node, body.content.as_deref())
        .await?;
    let content = match node.kind {
        NodeKind::File => Some(body.content.unwrap_or_default()),
        NodeKind::Folder => None,
    };
    Ok(Json(FsNode::from_node(node, content)))
}

/// `PUT …/workspace/file/{node_id}` — overwrite a note's body.
///
/// # A published deliverable is edited on both surfaces (issue #552)
///
/// Since #552 a note in this tree may be the projection of a task artifact, and
/// an operator's save of one is *the human edit* — the single datum the artifact
/// port exists to capture. Overwriting only the node would leave the version
/// history saying the agent's draft was shipped unchanged, and
/// `human_edit_diff` answering `None` for an artifact a human rewrote.
///
/// So the chain is written **first**, then the node. The two failure modes are
/// not symmetric: a version recorded whose node write then fails leaves a stale
/// node, which is visible and heals on the next write; a node written whose
/// version was never recorded is silent, permanent, and corrupts the diff. Of
/// the two, only the first is survivable, so it is the one this ordering
/// chooses. See [`artifact_mirror`](crate::company::artifact_mirror).
///
/// An ordinary note — which is nearly all of them — matches no artifact, so the
/// lookup answers `Ordinary` and this behaves exactly as it did before. The
/// lookup is a scan of the company's artifacts per save; it is bounded by what
/// artifacts are (a task's drafts, not a repository) and is named as the place
/// to add an index if it ever hurts.
///
/// # When the artifact store cannot answer at all
///
/// The lookup is the only reason this route reads the artifact store, and it
/// runs for every note. Propagating its failure would mean an artifact-store
/// fault rejects the save of a plain note — losing an operator's edit to a
/// store that note does not depend on, to protect a chain it does not have.
/// So a lookup that *fails* is warned about and the node write proceeds.
///
/// This is a deliberate narrowing of the ordering guarantee, not an exception
/// to it. Fail-closed still holds wherever it can be applied: once the store
/// answers and names this node a deliverable, a version that cannot be appended
/// still refuses the save. What is given up is only the case where the store
/// cannot be read at all — there, a published deliverable edited during the
/// outage lands node-ahead-of-chain, the silent direction. The window is an
/// unreachable artifact store, the alternative is refusing every note in the
/// company for the same duration, and the divergence heals on the next
/// successful save of that note.
async fn write_file(
    company: ScopedCompany,
    Path(NodePath { node_id }): Path<NodePath>,
    Json(body): Json<WriteFile>,
) -> Result<Json<WriteAck>, ApiError> {
    // No kind check first: a folder id can never match an artifact (nothing
    // stamps one), so the lookup answers `None` for it and the `write` below
    // still rejects it — an extra read per save to pre-empt an error case would
    // cost every ordinary save to save nothing.
    if let MirrorOutcome::Undetermined(err) = mirror_node_edit(
        company.runtime.artifacts().as_ref(),
        company.id(),
        &node_id,
        &body.content,
        ArtifactAuthor::Operator,
        "operator",
        Some(OPERATOR_EDIT_NOTE.to_string()),
    )
    .await?
    {
        tracing::warn!(
            company = %company.id(),
            node = %node_id,
            error = %err,
            "[workspace] could not read the artifact store, so whether this note is a \
             published deliverable is unknown; saving it anyway. If it was published, its \
             chain is one version behind until the next successful save"
        );
    }

    let node = company
        .runtime
        .workspace()
        .write(
            company.id(),
            &node_id,
            &body.content,
            WorkspaceOrigin::Operator,
        )
        .await?;
    Ok(Json(WriteAck {
        updated_at: node.updated_at_millis,
    }))
}

async fn rename_move(
    company: ScopedCompany,
    Path(NodePath { node_id }): Path<NodePath>,
    Json(body): Json<RenameMove>,
) -> Result<Json<FsNode>, ApiError> {
    let node = company
        .runtime
        .workspace()
        .rename_move(
            company.id(),
            &node_id,
            body.name.as_deref(),
            body.parent_id.as_ref().map(Option::as_deref),
        )
        .await?;
    let content = match node.kind {
        NodeKind::File => company
            .runtime
            .workspace()
            .read(company.id(), &node_id)
            .await?
            .map(|(_, body)| body),
        NodeKind::Folder => None,
    };
    Ok(Json(FsNode::from_node(node, content)))
}

async fn delete_node(
    company: ScopedCompany,
    Path(NodePath { node_id }): Path<NodePath>,
) -> Result<StatusCode, ApiError> {
    if company
        .runtime
        .workspace()
        .delete(company.id(), &node_id)
        .await?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError(OpenCompanyError::CompanyNotFound(format!(
            "workspace node {node_id}"
        ))))
    }
}
