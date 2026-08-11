//! The seam between a task artifact and the shared workspace tree (issue #552,
//! folding in #327's missing push channel).
//!
//! # The problem this closes
//!
//! `publish_artifact` used to drain into the [`ArtifactStore`] and stop. An
//! artifact is reachable from exactly one place — the Artifacts tab of one
//! card — so a deliverable an agent explicitly published was invisible to the
//! operator browsing the workspace and to every *other* agent, whose only view
//! of shared company state is the note tree. "The CMO wrote the launch brief"
//! had no answer anyone could navigate to.
//!
//! # Two surfaces, one truth
//!
//! A published deliverable now lives twice, and the split is deliberate:
//!
//! * The **artifact chain is authoritative**. It holds the full version
//!   history, the authorship of each revision, and therefore
//!   [`ArtifactRecord::human_edit_diff`] — the one quality datum the artifact
//!   port exists to produce.
//! * The **workspace node is a projection** holding the *current* body only.
//!   It is what makes the deliverable browsable and readable by teammates.
//!
//! The rejected alternative was to make the node the storage and have the
//! artifact reference it. That would push versioning down into
//! [`WorkspaceStore`] across all three backends, turn every artifact read into
//! a two-store join, and re-open the `(task_id, source)` identity contract that
//! #244 settled. A projection costs one extra write; the inversion costs the
//! port.
//!
//! **The invariant**: `node.body == chain.latest().body` after any successful
//! write on either surface.
//!
//! # Ordering: chain first, wherever there is a choice
//!
//! Every path here writes the chain before the node when it can. The two
//! failure modes are not symmetric:
//!
//! * Chain ahead of node — a stale node. Visible, harmless, and self-healing:
//!   the next write on either surface reconciles it.
//! * Node ahead of chain — an edit to a published deliverable that the version
//!   history never recorded. That is silent, permanent, and corrupts
//!   `human_edit_diff`, which is the exact rot the artifact port was built to
//!   prevent.
//!
//! So a failed mirror is logged and tolerated in the first direction and
//! avoided in the second. One path cannot have it: the agent's
//! `workspace_write` tool must complete its compare-and-swap before it knows
//! the write landed at all, so there the node necessarily moves first. It is
//! the narrowest window available rather than a different policy.
//!
//! # The guarantee is owed to deliverables, not to every note
//!
//! "Avoided in the second direction" is a promise about *published* nodes, and
//! it costs something to keep: the reverse lookup runs on every save, so a
//! strict reading would make an unreachable artifact store refuse edits to
//! ordinary notes too — notes with no chain to corrupt, on a save that
//! otherwise never touches that store. That trades the whole tree's
//! availability for a guarantee none of it is owed.
//!
//! [`mirror_node_edit`] therefore separates *cannot record* from *cannot tell*
//! (see [`MirrorOutcome`]). Its callers keep failing closed once a node is
//! known to be a deliverable, and choose for themselves what an unanswerable
//! store means. The console `PUT` takes the availability side and says so in
//! its own doc, including what that costs when the store is down.
//!
//! # Why this module is in the default build
//!
//! [`mirror_node_edit`] has three callers across two layers — the console's
//! workspace `PUT` and artifact-append routes (`src/server/ops/`, always
//! compiled) and the agent's `workspace_write` tool (`src/harness/`, compiled
//! only under the `openhuman` feature). The shared half therefore cannot live
//! in the harness, or the default build could not reach it.

use crate::Result;
use crate::error::OpenCompanyError;
use crate::ports::artifacts::{ArtifactAuthor, ArtifactRecord, ArtifactStore};
use crate::ports::now_millis;
use crate::ports::types::CompanyId;
use crate::ports::workspace::{NodeKind, WorkspaceNode, WorkspaceOrigin, WorkspaceStore};

use super::workspace_scaffold::{create_folder, ensure_agent_folder};

/// One publish, as [`materialize`] needs it.
///
/// A struct rather than seven positional parameters: five of the fields are
/// `&str`, so a call site that transposed `task_id` and `source` would compile
/// perfectly and file every deliverable in the wrong folder.
#[derive(Debug, Clone, Copy)]
pub struct PublishTarget<'a> {
    /// The agent that published this file — the owner of the `Agents/<id>/`
    /// folder it lands under, and the authorship stamped on every node created
    /// or written along the way.
    pub agent_id: &'a str,
    /// The card the publish belongs to. Names the folder beneath the agent's,
    /// so two tasks by one agent cannot collide on a common filename.
    pub task_id: &'a str,
    /// The normalized workspace-relative path the agent published, e.g.
    /// `specs/launch.md`. Interior segments become folders.
    pub source: &'a str,
    /// What to store — the file's text, or its bytes (issue #553). Text lands
    /// as an ordinary note; bytes land as a binary node, which is what stopped
    /// a paid image generation from becoming a dangling digest.
    pub payload: MirrorPayload<'a>,
    /// The node the previous version of this artifact was mirrored into, when
    /// there was one. Reused if it still resolves; see [`materialize`].
    pub existing_node_id: Option<&'a str>,
}

/// What [`materialize`] is being asked to put in the tree.
///
/// Borrowed rather than owned: the drain already holds the bytes it read, and a
/// copy of a 200 MiB video to cross one function boundary would be the single
/// largest allocation on the publish path.
#[derive(Debug, Clone, Copy)]
pub enum MirrorPayload<'a> {
    /// Prose — an editable, diffable, backlinkable note.
    Text(&'a str),
    /// Opaque bytes, with the media type the publisher inferred.
    Bytes {
        /// The file's contents, written verbatim.
        bytes: &'a [u8],
        /// The media type to store the node under.
        mime: &'a str,
    },
}

/// Put `target`'s body into the shared tree and return the node id holding it.
///
/// The layout is `Agents/<agent-id>/<task-id>/<source…>`. The agent's folder is
/// minted on demand by
/// [`ensure_agent_folder`](super::workspace_scaffold::ensure_agent_folder) —
/// member folders appear the first time somebody produces something, so this
/// must **call** it rather than assume it exists.
///
/// # Interior path segments become folders
///
/// `specs/launch.md` lands as `…/<task-id>/specs/launch.md`, not as a file
/// literally named `specs/launch.md`. Flattening to the basename would make
/// `specs/a.md` and `docs/a.md` — two genuinely different deliverables of one
/// task — collide on one node and overwrite each other.
///
/// # Re-publish reuses the node, unless the operator removed it
///
/// `existing_node_id` is reused when it still resolves to a file, so a second
/// publish of the same path revises the note the operator has been reading
/// rather than opening a rival beside it. When it is absent (a pre-#552 record)
/// or no longer resolves (the operator deleted it, and deletions stick), a
/// fresh node is materialized and the *new* version carries the new id. Older
/// versions keep the id of the node that actually held them — honest history,
/// the same shape as `run_id`.
///
/// # Ambiguity is refused, never guessed
///
/// Identity here is by path and no backend enforces unique sibling names, so
/// every lookup is check-then-act. A name carried by a node of the wrong kind,
/// or by more than one node, is a [`Conflict`](OpenCompanyError::Conflict)
/// rather than a coin flip — the same fail-closed rule
/// [`workspace_scaffold`](super::workspace_scaffold) applies one level up.
pub async fn materialize(
    workspace: &dyn WorkspaceStore,
    company: &CompanyId,
    target: PublishTarget<'_>,
) -> Result<String> {
    // The cheap path, and the common one on a re-publish: the node from last
    // time still exists, so revise it in place and keep every reference to it
    // (the console's deep link, an operator's bookmark) working.
    //
    // A node whose *shape* changed — a markdown draft re-exported as a PDF, or
    // a PDF replaced by prose — cannot be revised in place, because neither
    // write path will convert one kind of node into the other (and the store
    // refuses if asked). Falling through to the path resolution below mints a
    // fresh node of the right kind, which is the same answer this function
    // already gives when the operator deleted the old one: the new version
    // carries the new id, older versions keep the id that actually held them.
    if let Some(existing) = target.existing_node_id
        && let Some((node, _)) = workspace.read(company, existing).await?
        && node.kind == NodeKind::File
        && node.is_binary() == matches!(target.payload, MirrorPayload::Bytes { .. })
    {
        write_payload(workspace, company, existing, target).await?;
        return Ok(existing.to_string());
    }

    let segments = split_source(target.source)?;
    let (dirs, filename) = segments
        .split_last()
        .map(|(last, rest)| (rest, *last))
        .expect("split_source rejects an empty path");

    let agent_folder = ensure_agent_folder(workspace, company, target.agent_id).await?;

    // One tree read, then a walk that keeps its own view current: each folder
    // this creates is pushed onto `nodes`, so a `specs/deep/note.md` resolves
    // its second segment against the first segment it just minted rather than
    // against a snapshot that predates it.
    let mut nodes = workspace.tree(company).await?;
    let mut parent = agent_folder;
    for name in std::iter::once(target.task_id).chain(dirs.iter().copied()) {
        parent = resolve_folder(
            workspace,
            company,
            &mut nodes,
            &parent,
            name,
            target.agent_id,
        )
        .await?;
    }

    match resolve_file(&nodes, &parent, filename)? {
        // A node is already there under this exact path — an earlier publish
        // whose id we lost, or a note the agent wrote by hand. Revising it is
        // the only non-destructive answer: minting a rival would leave the path
        // permanently ambiguous, which the tool layer's resolver then refuses
        // for every agent.
        Some(id) => {
            // The same shape guard as above: a node already at this path whose
            // kind disagrees with what is being published is deleted and
            // replaced, because no write can turn one into the other. Deleting
            // is safe here in a way it is not above — the path is the
            // deliverable's identity, so this node IS the thing being
            // superseded, and its history lives on the artifact chain.
            let replace = match workspace.read(company, &id).await? {
                Some((node, _)) => {
                    node.is_binary() != matches!(target.payload, MirrorPayload::Bytes { .. })
                }
                None => false,
            };
            if replace {
                workspace.delete(company, &id).await?;
                return create_payload(workspace, company, Some(parent), filename, target).await;
            }
            write_payload(workspace, company, &id, target).await?;
            Ok(id)
        }
        None => create_payload(workspace, company, Some(parent), filename, target).await,
    }
}

/// Overwrites `node_id` with whatever `target` carries, on the matching path.
async fn write_payload(
    workspace: &dyn WorkspaceStore,
    company: &CompanyId,
    node_id: &str,
    target: PublishTarget<'_>,
) -> Result<()> {
    match target.payload {
        MirrorPayload::Text(body) => {
            workspace
                .write(company, node_id, body, origin(target.agent_id))
                .await?;
        }
        MirrorPayload::Bytes { bytes, mime } => {
            workspace
                .write_binary(company, node_id, bytes, Some(mime), origin(target.agent_id))
                .await?;
        }
    }
    Ok(())
}

/// Creates a fresh node of the right kind holding `target`'s payload.
async fn create_payload(
    workspace: &dyn WorkspaceStore,
    company: &CompanyId,
    parent: Option<String>,
    filename: &str,
    target: PublishTarget<'_>,
) -> Result<String> {
    let mut node = WorkspaceNode {
        id: crate::ports::generate_id(),
        name: filename.to_string(),
        kind: NodeKind::File,
        parent_id: parent,
        updated_at_millis: now_millis(),
        created_by: origin(target.agent_id),
        updated_by: origin(target.agent_id),
        mime: None,
        size: None,
        sha256: None,
    };
    match target.payload {
        MirrorPayload::Text(body) => {
            workspace.create(company, &node, Some(body)).await?;
        }
        MirrorPayload::Bytes { bytes, mime } => {
            node.mime = Some(mime.to_string());
            workspace.create_binary(company, &node, bytes).await?;
        }
    }
    Ok(node.id)
}

/// Record an edit to `node_id` on the artifact chain that owns it, when one
/// does.
///
/// The reverse lookup that keeps the two surfaces from diverging: a workspace
/// node the operator (or an agent) rewrites may be a *published deliverable*,
/// and an edit to one that never reached the version history is exactly the
/// silent corruption the artifact port exists to prevent.
///
/// Answers [`MirrorOutcome::Ordinary`] — and touches nothing — when `node_id`
/// names an ordinary note. Most of the tree is ordinary notes, so this is the
/// common answer and deliberately not an error.
///
/// # Two failures, told apart on purpose
///
/// The lookup and the append fail for different reasons and are not returned
/// alike. A failed **append** is an `Err`: the store answered, so this node is
/// known to be a published deliverable, and the caller must not write the node
/// behind a version that was never recorded. A failed **lookup** is
/// [`MirrorOutcome::Undetermined`] inside `Ok`, because it establishes nothing
/// — the node may be a deliverable or may be one of the ordinary notes that
/// are nearly the whole tree, and only the caller knows whether its own work
/// can proceed without that answer.
///
/// Collapsing the second into [`MirrorOutcome::Ordinary`] would read as "no
/// chain here, carry on" on every store fault, which is precisely how the
/// fail-closed guarantee for deliverables would stop applying without anything
/// appearing to change.
///
/// # The scan, named rather than hidden
///
/// This lists the company's artifacts and looks for one whose *latest* version
/// carries `node_id`. That is a linear scan per save. It is bounded by what
/// artifacts are — a task's drafts and posts, not a repository — and buying an
/// index before there is a workload to size it against would be guessing. The
/// latest version rather than any version is the point: an operator's deletion
/// of a node sticks, so an old version's id names a node that is gone, and
/// matching on it would mirror today's edit into yesterday's history.
pub async fn mirror_node_edit(
    artifacts: &dyn ArtifactStore,
    company: &CompanyId,
    node_id: &str,
    body: &str,
    author: ArtifactAuthor,
    author_id: &str,
    note: Option<String>,
) -> Result<MirrorOutcome> {
    let mut record = match published_record_for_node(artifacts, company, node_id).await {
        Ok(Some(record)) => record,
        Ok(None) => return Ok(MirrorOutcome::Ordinary),
        Err(err) => return Ok(MirrorOutcome::Undetermined(err)),
    };
    let version = record.push_version(body, author, author_id, now_millis(), note);
    // The appended version lives in the same node as the one before it. Without
    // this the *next* edit's reverse lookup — which reads the latest version —
    // would find nothing and silently stop mirroring.
    record.stamp_workspace_node(node_id);
    // Fail-closed, and the one place in this function that is: the lookup
    // succeeded, so this node *is* a deliverable, and a caller that wrote it
    // anyway would leave the history claiming the agent's draft shipped
    // unchanged.
    artifacts.upsert(company, &record).await?;
    Ok(MirrorOutcome::Recorded(MirroredEdit {
        artifact_id: record.id,
        version,
    }))
}

/// What [`mirror_node_edit`] was able to do — and, when it could not act,
/// whether the caller may carry on without it.
///
/// [`Ordinary`](MirrorOutcome::Ordinary) and
/// [`Undetermined`](MirrorOutcome::Undetermined) both mean "nothing was
/// recorded", and that is the whole reason they are separate variants rather
/// than one absent value: the first is a complete answer from a healthy store
/// and the second is no answer at all.
#[derive(Debug)]
pub enum MirrorOutcome {
    /// `node_id` is a published deliverable, and this edit is now a version on
    /// its chain.
    Recorded(MirroredEdit),
    /// The store answered, and `node_id` names no artifact — an ordinary note.
    /// There is no chain here for a node write to get ahead of.
    Ordinary,
    /// The store could not be read, so whether `node_id` is published is
    /// **unknown** rather than "no". Carries the fault so a caller that
    /// tolerates it can still say why in a log.
    Undetermined(OpenCompanyError),
}

/// What [`mirror_node_edit`] appended, for a caller that wants to log or return
/// it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirroredEdit {
    /// The artifact the edit was recorded on.
    pub artifact_id: String,
    /// The version number the edit became.
    pub version: u32,
}

/// The artifact whose current body lives in `node_id`, if any.
///
/// Shared by [`mirror_node_edit`] and the console's artifact-append route,
/// which needs the same "is this a published deliverable?" answer from the
/// other direction.
pub async fn published_record_for_node(
    artifacts: &dyn ArtifactStore,
    company: &CompanyId,
    node_id: &str,
) -> Result<Option<ArtifactRecord>> {
    Ok(artifacts
        .list(company, None)
        .await?
        .into_iter()
        .find(|record| record.workspace_node_id() == Some(node_id)))
}

/// This agent's authorship stamp. A published deliverable is the agent's work,
/// so every node created or written along its path is attributed to it.
fn origin(agent_id: &str) -> WorkspaceOrigin {
    WorkspaceOrigin::Agent {
        id: agent_id.to_string(),
    }
}

/// Split a normalized publish path into its segments, rejecting anything that
/// cannot name a chain of workspace nodes.
///
/// The publish tool normalizes before it gets here, so this is a guard against
/// a hand-built `PendingPublish` rather than the ordinary path — but a `..`
/// reaching [`WorkspaceStore::create`] as a node *name* would render a
/// traversal-shaped path in the console, and the sqlite and mongodb backends do
/// not reject one.
fn split_source(source: &str) -> Result<Vec<&str>> {
    let segments: Vec<&str> = source
        .split('/')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if segments.is_empty() {
        return Err(OpenCompanyError::InvalidRequest(format!(
            "`{source}` names no workspace path segments, so it cannot be published into the tree"
        )));
    }
    for segment in &segments {
        if *segment == "." || *segment == ".." || segment.contains('\\') || segment.contains('\0') {
            return Err(OpenCompanyError::InvalidRequest(format!(
                "`{source}` contains a segment that cannot name a workspace node"
            )));
        }
    }
    Ok(segments)
}

/// Adopt-or-create the folder `name` under `parent`, keeping `nodes` current.
async fn resolve_folder(
    workspace: &dyn WorkspaceStore,
    company: &CompanyId,
    nodes: &mut Vec<WorkspaceNode>,
    parent: &str,
    name: &str,
    agent_id: &str,
) -> Result<String> {
    let matches: Vec<&WorkspaceNode> = children_named(nodes, parent, name);
    match matches.as_slice() {
        [one] if one.kind == NodeKind::Folder => Ok(one.id.clone()),
        [_] => Err(OpenCompanyError::Conflict(format!(
            "`{name}` already exists as a note, not a folder, so a deliverable cannot be published \
             beneath it"
        ))),
        [] => {
            let id = create_folder(
                workspace,
                company,
                name,
                Some(parent.to_string()),
                origin(agent_id),
            )
            .await?;
            nodes.push(WorkspaceNode {
                id: id.clone(),
                name: name.to_string(),
                kind: NodeKind::Folder,
                parent_id: Some(parent.to_string()),
                updated_at_millis: now_millis(),
                created_by: origin(agent_id),
                updated_by: origin(agent_id),
                mime: None,
                size: None,
                sha256: None,
            });
            Ok(id)
        }
        many => Err(OpenCompanyError::Conflict(format!(
            "{count} nodes under this folder are named `{name}`, so the path is ambiguous",
            count = many.len()
        ))),
    }
}

/// The existing file `name` under `parent`, or `None` when the name is free.
fn resolve_file(nodes: &[WorkspaceNode], parent: &str, name: &str) -> Result<Option<String>> {
    let matches = children_named(nodes, parent, name);
    match matches.as_slice() {
        [one] if one.kind == NodeKind::File => Ok(Some(one.id.clone())),
        [_] => Err(OpenCompanyError::Conflict(format!(
            "`{name}` already exists as a folder, not a note, so a deliverable cannot be published \
             over it"
        ))),
        [] => Ok(None),
        many => Err(OpenCompanyError::Conflict(format!(
            "{count} nodes under this folder are named `{name}`, so the path is ambiguous",
            count = many.len()
        ))),
    }
}

/// Every node directly under `parent` carrying `name`.
fn children_named<'a>(
    nodes: &'a [WorkspaceNode],
    parent: &str,
    name: &str,
) -> Vec<&'a WorkspaceNode> {
    nodes
        .iter()
        .filter(|node| node.parent_id.as_deref() == Some(parent) && node.name == name)
        .collect()
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use super::*;
    use crate::company::workspace_scaffold::AGENTS_ROOT;
    use crate::ports::artifacts::ArtifactKind;
    use crate::store::FsOps;

    /// One `FsOps` backing both ports, so a test exercises the real stores
    /// rather than a stub that cannot tell a create from an overwrite.
    fn stores() -> (tempfile::TempDir, Arc<FsOps>, CompanyId) {
        let dir = tempfile::tempdir().expect("tempdir");
        let ops = Arc::new(FsOps::new(dir.path()));
        (dir, ops, CompanyId::new("mirror-co"))
    }

    /// A node's rendered path, so an assertion reads as a path rather than a
    /// ULID.
    async fn path_of(ws: &dyn WorkspaceStore, company: &CompanyId, id: &str) -> String {
        let nodes = ws.tree(company).await.unwrap();
        let mut parts = Vec::new();
        let mut cursor = Some(id.to_string());
        while let Some(current) = cursor {
            let Some(node) = nodes.iter().find(|n| n.id == current) else {
                break;
            };
            parts.push(node.name.clone());
            cursor = node.parent_id.clone();
        }
        parts.reverse();
        parts.join("/")
    }

    fn target<'a>(source: &'a str, body: &'a str) -> PublishTarget<'a> {
        PublishTarget {
            agent_id: "cmo",
            task_id: "t-1",
            source,
            payload: MirrorPayload::Text(body),
            existing_node_id: None,
        }
    }

    /// The headline: a published deliverable lands in the shared tree, under
    /// the publishing agent's own folder, attributed to it.
    ///
    /// The agent folder is asserted rather than assumed because it does not
    /// exist beforehand — member folders are minted on first use (#570), so
    /// this proves `materialize` calls the minter instead of expecting a
    /// folder somebody else laid down.
    #[tokio::test]
    async fn a_publish_lands_under_the_agents_own_folder_it_mints() {
        let (_dir, ops, co) = stores();
        let ws: &dyn WorkspaceStore = ops.as_ref();

        let id = materialize(ws, &co, target("launch.md", "# Launch"))
            .await
            .expect("materialize");

        assert_eq!(
            path_of(ws, &co, &id).await,
            format!("{AGENTS_ROOT}/cmo/t-1/launch.md")
        );
        let (node, body) = ws.read(&co, &id).await.unwrap().expect("the node exists");
        assert_eq!(body, "# Launch");
        assert_eq!(node.kind, NodeKind::File);
        assert_eq!(
            node.created_by,
            WorkspaceOrigin::Agent {
                id: "cmo".to_string()
            },
            "a published deliverable is the agent's work, and the tree must say so"
        );
    }

    /// A **binary** publish lands real bytes in the tree (issue #553).
    ///
    /// This is the payoff of the whole issue: before it, a generated image
    /// became a reference record naming a sandbox path, and wiping the sandbox
    /// left the digest pointing at nothing. Now the same publish produces a
    /// node the operator can open, on every backend.
    #[tokio::test]
    async fn a_binary_publish_lands_real_bytes_under_the_agents_folder() {
        let (_dir, ops, co) = stores();
        let ws: &dyn WorkspaceStore = ops.as_ref();
        let png: Vec<u8> = vec![0x89, b'P', b'N', b'G', 0xff, 0xfe, 0x00];

        let id = materialize(
            ws,
            &co,
            PublishTarget {
                payload: MirrorPayload::Bytes {
                    bytes: &png,
                    mime: "image/png",
                },
                ..target("shots/hero.png", "")
            },
        )
        .await
        .expect("materialize");

        assert_eq!(
            path_of(ws, &co, &id).await,
            format!("{AGENTS_ROOT}/cmo/t-1/shots/hero.png")
        );
        let (node, stream) = ws
            .read_bytes(&co, &id)
            .await
            .unwrap()
            .expect("the payload is retrievable");
        assert_eq!(node.mime.as_deref(), Some("image/png"));
        assert_eq!(node.size, Some(png.len() as u64));
        assert_eq!(
            node.created_by,
            WorkspaceOrigin::Agent {
                id: "cmo".to_string()
            }
        );
        let mut got = Vec::new();
        {
            use futures::StreamExt;
            let mut stream = stream;
            while let Some(chunk) = stream.next().await {
                got.extend_from_slice(&chunk.unwrap());
            }
        }
        assert_eq!(got, png, "the published bytes are the stored bytes");
    }

    /// Re-publishing the same path as a different shape replaces the node
    /// rather than failing.
    ///
    /// Neither write path converts a note into a payload or back — the store
    /// refuses both — so a markdown draft later re-exported as a PDF would
    /// otherwise error on every publish. The path is the deliverable's
    /// identity, so the node is replaced and the history stays on the artifact
    /// chain.
    #[tokio::test]
    async fn republishing_a_note_as_a_payload_replaces_the_node() {
        let (_dir, ops, co) = stores();
        let ws: &dyn WorkspaceStore = ops.as_ref();

        let first = materialize(ws, &co, target("report.md", "# Draft"))
            .await
            .unwrap();
        let second = materialize(
            ws,
            &co,
            PublishTarget {
                payload: MirrorPayload::Bytes {
                    bytes: &[0x25, 0x50, 0x44, 0x46],
                    mime: "application/pdf",
                },
                existing_node_id: Some(&first),
                ..target("report.md", "")
            },
        )
        .await
        .expect("a shape change must not fail the publish");

        let (node, _) = ws
            .read_bytes(&co, &second)
            .await
            .unwrap()
            .expect("the new node holds bytes");
        assert_eq!(node.mime.as_deref(), Some("application/pdf"));
        assert_eq!(
            path_of(ws, &co, &second).await,
            format!("{AGENTS_ROOT}/cmo/t-1/report.md"),
            "the deliverable keeps its path"
        );
        assert!(
            ws.read(&co, &first).await.unwrap().is_none(),
            "the superseded node is gone, not left beside its replacement"
        );
    }

    /// Interior segments become folders. Flattening to the basename would make
    /// two genuinely different deliverables of one task collide on one node —
    /// so the same basename in two directories must be two nodes.
    #[tokio::test]
    async fn the_same_basename_in_two_directories_is_two_nodes() {
        let (_dir, ops, co) = stores();
        let ws: &dyn WorkspaceStore = ops.as_ref();

        let spec = materialize(ws, &co, target("specs/a.md", "spec body"))
            .await
            .unwrap();
        let doc = materialize(ws, &co, target("docs/a.md", "doc body"))
            .await
            .unwrap();

        assert_ne!(spec, doc, "one node for two paths would lose a deliverable");
        assert_eq!(
            path_of(ws, &co, &spec).await,
            format!("{AGENTS_ROOT}/cmo/t-1/specs/a.md")
        );
        assert_eq!(
            path_of(ws, &co, &doc).await,
            format!("{AGENTS_ROOT}/cmo/t-1/docs/a.md")
        );
        assert_eq!(ws.read(&co, &spec).await.unwrap().unwrap().1, "spec body");
        assert_eq!(ws.read(&co, &doc).await.unwrap().unwrap().1, "doc body");
    }

    /// Re-publishing with the node from last time revises **that** node, so the
    /// operator's open tab, deep link and backlinks all keep working. Nothing
    /// new is created.
    #[tokio::test]
    async fn a_republish_revises_the_same_node() {
        let (_dir, ops, co) = stores();
        let ws: &dyn WorkspaceStore = ops.as_ref();

        let first = materialize(ws, &co, target("launch.md", "draft one"))
            .await
            .unwrap();
        let before = ws.tree(&co).await.unwrap().len();

        let again = materialize(
            ws,
            &co,
            PublishTarget {
                existing_node_id: Some(&first),
                ..target("launch.md", "draft two")
            },
        )
        .await
        .unwrap();

        assert_eq!(again, first, "a re-publish must not open a rival node");
        assert_eq!(ws.tree(&co).await.unwrap().len(), before, "nothing created");
        assert_eq!(ws.read(&co, &first).await.unwrap().unwrap().1, "draft two");
    }

    /// The operator's deletions stick. A re-publish whose remembered node is
    /// gone mints a fresh one rather than resurrecting the old id — and the
    /// path is the same, so the deliverable reappears where it belongs.
    #[tokio::test]
    async fn a_republish_after_the_operator_deleted_the_node_mints_a_fresh_one() {
        let (_dir, ops, co) = stores();
        let ws: &dyn WorkspaceStore = ops.as_ref();

        let first = materialize(ws, &co, target("launch.md", "draft one"))
            .await
            .unwrap();
        assert!(ws.delete(&co, &first).await.unwrap());

        let again = materialize(
            ws,
            &co,
            PublishTarget {
                existing_node_id: Some(&first),
                ..target("launch.md", "draft two")
            },
        )
        .await
        .unwrap();

        assert_ne!(again, first, "a deleted node must not be resurrected by id");
        assert_eq!(
            path_of(ws, &co, &again).await,
            format!("{AGENTS_ROOT}/cmo/t-1/launch.md"),
            "the replacement belongs at the same path"
        );
        assert_eq!(ws.read(&co, &again).await.unwrap().unwrap().1, "draft two");
    }

    /// Losing the id but not the node — a pre-#552 record re-published — must
    /// adopt what is already at the path rather than mint a duplicate beside
    /// it. Two nodes on one path is precisely the ambiguity the tool layer's
    /// resolver then refuses for every agent.
    #[tokio::test]
    async fn a_publish_over_an_existing_path_adopts_it_rather_than_duplicating() {
        let (_dir, ops, co) = stores();
        let ws: &dyn WorkspaceStore = ops.as_ref();

        let first = materialize(ws, &co, target("launch.md", "draft one"))
            .await
            .unwrap();
        // `existing_node_id: None` is exactly what a pre-#552 artifact carries.
        let again = materialize(ws, &co, target("launch.md", "draft two"))
            .await
            .unwrap();

        assert_eq!(again, first, "the node already at the path must be adopted");
        assert_eq!(ws.read(&co, &first).await.unwrap().unwrap().1, "draft two");
    }

    /// A folder sitting where the note should go is refused, not overwritten:
    /// the deliverable would otherwise vanish into a name that resolves to
    /// something else entirely.
    #[tokio::test]
    async fn a_folder_in_the_notes_place_is_refused() {
        let (_dir, ops, co) = stores();
        let ws: &dyn WorkspaceStore = ops.as_ref();

        // Publish once to lay down `Agents/cmo/t-1/`, then put a folder where
        // the next publish's note wants to be.
        let sibling = materialize(ws, &co, target("other.md", "x")).await.unwrap();
        let parent = ws
            .read(&co, &sibling)
            .await
            .unwrap()
            .unwrap()
            .0
            .parent_id
            .unwrap();
        create_folder(
            ws,
            &co,
            "launch.md",
            Some(parent),
            WorkspaceOrigin::Operator,
        )
        .await
        .unwrap();

        let refused = materialize(ws, &co, target("launch.md", "body"))
            .await
            .expect_err("a folder in the note's place must be refused");
        assert!(
            refused.to_string().contains("already exists as a folder"),
            "unexpected refusal: {refused}"
        );
    }

    /// A traversal segment reaching `create` as a node *name* would render a
    /// path the console cannot navigate, and the sqlite/mongodb backends do not
    /// reject one — so the guard lives here.
    #[tokio::test]
    async fn a_traversal_segment_is_refused() {
        let (_dir, ops, co) = stores();
        let ws: &dyn WorkspaceStore = ops.as_ref();

        for bad in ["../escape.md", "specs/../../escape.md", "   "] {
            assert!(
                materialize(ws, &co, target(bad, "body")).await.is_err(),
                "`{bad}` must not name a workspace path"
            );
        }
    }

    // -- mirror_node_edit ---------------------------------------------------

    /// An edit to a published node is recorded on the artifact chain, as a new
    /// version by the editing author — which is what keeps `human_edit_diff`
    /// answerable after the operator revises a deliverable in the console.
    #[tokio::test]
    async fn an_edit_to_a_published_node_appends_an_operator_version() {
        let (_dir, ops, co) = stores();
        let artifacts: &dyn ArtifactStore = ops.as_ref();

        let mut record = ArtifactRecord::new(
            "a-1",
            "t-1",
            "Launch",
            ArtifactKind::Markdown,
            "agent draft",
            "cmo",
            1,
        );
        record.stamp_workspace_node("node-1");
        artifacts.upsert(&co, &record).await.unwrap();

        let MirrorOutcome::Recorded(mirrored) = mirror_node_edit(
            artifacts,
            &co,
            "node-1",
            "operator draft",
            ArtifactAuthor::Operator,
            "operator",
            Some("operator edit before approval".to_string()),
        )
        .await
        .unwrap() else {
            panic!("a published node's edit is recorded");
        };

        assert_eq!(mirrored.artifact_id, "a-1");
        assert_eq!(mirrored.version, 2);

        let stored = artifacts.get(&co, "a-1").await.unwrap().unwrap();
        assert_eq!(stored.latest().unwrap().body, "operator draft");
        assert_eq!(stored.latest().unwrap().author, ArtifactAuthor::Operator);
        assert_eq!(
            stored.workspace_node_id(),
            Some("node-1"),
            "the appended version must carry the node too, or the NEXT edit's \
             reverse lookup finds nothing and mirroring silently stops"
        );
        assert!(
            stored.human_edit_diff().is_some(),
            "the whole reason the chain must see console edits"
        );
    }

    /// Most of the tree is ordinary notes. Editing one touches no artifact and
    /// is not an error — the common answer, and deliberately silent.
    #[tokio::test]
    async fn an_edit_to_an_unpublished_node_records_nothing() {
        let (_dir, ops, co) = stores();
        let artifacts: &dyn ArtifactStore = ops.as_ref();

        let mut published = ArtifactRecord::new(
            "a-1",
            "t-1",
            "Launch",
            ArtifactKind::Markdown,
            "body",
            "cmo",
            1,
        );
        published.stamp_workspace_node("node-1");
        artifacts.upsert(&co, &published).await.unwrap();

        let mirrored = mirror_node_edit(
            artifacts,
            &co,
            "some-other-note",
            "new body",
            ArtifactAuthor::Operator,
            "operator",
            None,
        )
        .await
        .unwrap();

        assert!(matches!(mirrored, MirrorOutcome::Ordinary), "{mirrored:?}");
        let stored = artifacts.get(&co, "a-1").await.unwrap().unwrap();
        assert_eq!(
            stored.versions.len(),
            1,
            "an unrelated note must not append"
        );
    }

    /// The lookup matches the **latest** version's node, not any version's. An
    /// artifact whose node the operator deleted and which was re-published into
    /// a new one must mirror into the new node — matching on the stale id would
    /// write today's edit into yesterday's history.
    #[tokio::test]
    async fn the_lookup_matches_the_current_node_not_a_retired_one() {
        let (_dir, ops, co) = stores();
        let artifacts: &dyn ArtifactStore = ops.as_ref();

        let mut record = ArtifactRecord::new(
            "a-1",
            "t-1",
            "Launch",
            ArtifactKind::Markdown,
            "v1",
            "cmo",
            1,
        );
        record.stamp_workspace_node("node-old");
        record.push_version("v2", ArtifactAuthor::Agent, "cmo", 2, None);
        record.stamp_workspace_node("node-new");
        artifacts.upsert(&co, &record).await.unwrap();

        assert!(
            matches!(
                mirror_node_edit(
                    artifacts,
                    &co,
                    "node-old",
                    "edit",
                    ArtifactAuthor::Operator,
                    "operator",
                    None,
                )
                .await
                .unwrap(),
                MirrorOutcome::Ordinary
            ),
            "the retired node no longer addresses this artifact"
        );
        assert!(matches!(
            mirror_node_edit(
                artifacts,
                &co,
                "node-new",
                "edit",
                ArtifactAuthor::Operator,
                "operator",
                None,
            )
            .await
            .unwrap(),
            MirrorOutcome::Recorded(_)
        ));
    }

    // -- the two store faults, told apart --------------------------------

    /// An artifact store with one chosen fault, so a test can ask for exactly
    /// the failure it means: unreadable (`list`) or unwritable (`upsert`).
    struct FaultyArtifacts {
        listed: Vec<ArtifactRecord>,
        list_fails: bool,
        upsert_fails: bool,
    }

    #[async_trait::async_trait]
    impl ArtifactStore for FaultyArtifacts {
        async fn list(&self, _: &CompanyId, _: Option<&str>) -> Result<Vec<ArtifactRecord>> {
            if self.list_fails {
                return Err(OpenCompanyError::Store("the artifact store is down".into()));
            }
            Ok(self.listed.clone())
        }
        async fn get(&self, _: &CompanyId, _: &str) -> Result<Option<ArtifactRecord>> {
            Ok(None)
        }
        async fn upsert(&self, _: &CompanyId, _: &ArtifactRecord) -> Result<()> {
            if self.upsert_fails {
                return Err(OpenCompanyError::Store("the disk is full".into()));
            }
            Ok(())
        }
        async fn delete(&self, _: &CompanyId, _: &str) -> Result<bool> {
            Ok(false)
        }
    }

    fn published_as(node_id: &str) -> ArtifactRecord {
        let mut record = ArtifactRecord::new(
            "a-1",
            "t-1",
            "Launch",
            ArtifactKind::Markdown,
            "agent draft",
            "cmo",
            1,
        );
        record.stamp_workspace_node(node_id);
        record
    }

    /// A store that cannot be listed establishes **nothing**, and must not be
    /// reported as the ordinary-note answer.
    ///
    /// This is the variant that carries the whole guarantee: `Ordinary` is what
    /// callers are entitled to write a node behind. If a read fault collapsed
    /// into it, every published deliverable would silently lose its fail-closed
    /// protection the moment the store got sick — the one moment it matters.
    #[tokio::test]
    async fn an_unreadable_store_is_undetermined_not_ordinary() {
        let co = CompanyId::new("acme");
        let artifacts = FaultyArtifacts {
            listed: Vec::new(),
            list_fails: true,
            upsert_fails: false,
        };

        let outcome = mirror_node_edit(
            &artifacts,
            &co,
            "node-1",
            "edit",
            ArtifactAuthor::Operator,
            "operator",
            None,
        )
        .await
        .expect("an unreadable store is the caller's decision, not an error");

        assert!(
            matches!(outcome, MirrorOutcome::Undetermined(_)),
            "a read fault must stay distinguishable from `Ordinary`: {outcome:?}"
        );
    }

    /// Once the store has answered and named this node a deliverable, a version
    /// that cannot be appended is an error — the caller must not go on to write
    /// the node, because that is the silent, permanent direction.
    #[tokio::test]
    async fn a_refused_append_on_a_published_node_still_fails_closed() {
        let co = CompanyId::new("acme");
        let artifacts = FaultyArtifacts {
            listed: vec![published_as("node-1")],
            list_fails: false,
            upsert_fails: true,
        };

        assert!(
            mirror_node_edit(
                &artifacts,
                &co,
                "node-1",
                "edit",
                ArtifactAuthor::Operator,
                "operator",
                None,
            )
            .await
            .is_err(),
            "a known deliverable whose version cannot be recorded must refuse the save"
        );
    }
}
