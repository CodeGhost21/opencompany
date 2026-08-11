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
//! GET    …/workspace/blob/{nodeId}    one binary node's payload, streamed
//! GET    …/workspace/search?q=…       which notes mention a phrase
//! POST   …/workspace                  create a folder/file (JSON body)
//! POST   …/workspace/upload           upload a file of any kind (multipart)
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
//!
//! ## Text and bytes are different resources (issue #553)
//!
//! A node holds prose or it holds bytes, never both, and the two are read
//! through different routes: `…/file/{id}` answers with text and backlinks,
//! `…/blob/{id}` streams a payload. Asking the wrong one is an error that names
//! the right one rather than an empty body — the port's honest answer to a
//! prose-shaped read of a payload is `""`, which as an HTTP response would
//! render as a blank editor over a file that is not blank.

use std::num::NonZeroUsize;

use axum::body::Body;
use axum::extract::multipart::MultipartError;
use axum::extract::{DefaultBodyLimit, Multipart, Path, Query};
use axum::http::{StatusCode, header};
use axum::response::Response;
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::company::artifact_mirror::{MirrorOutcome, mirror_node_edit};
use crate::company::workspace_links::file_with_backlinks;
use crate::company::workspace_search::{
    DEFAULT_SEARCH_LIMIT, MAX_SEARCH_RESULTS, search_workspace,
};
use crate::error::OpenCompanyError;
use crate::ports::artifacts::ArtifactAuthor;
use crate::ports::generate_id;
use crate::ports::workspace::{NodeKind, WorkspaceNode, WorkspaceOrigin};
use crate::runtime::UPLOAD_BODY_LIMIT_BYTES;
use crate::runtime::workspace_quota::human;
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
        // A static sibling of `/workspace/{node_id}` below, which is exactly
        // what `/workspace/upload` already is — axum's router prefers a literal
        // segment over a parameter, so `search` is never captured as a node id.
        .merge(scoped("/workspace/search", get(search)))
        .merge(scoped("/workspace/blob/{node_id}", get(read_blob)))
        .merge(
            scoped("/workspace/upload", post(upload))
                // On this route only. The default limit is a couple of
                // megabytes, which every upload this route exists for would
                // exceed; raising it globally would lift it for every JSON
                // handler in the process, which is the opposite of what an
                // upload endpoint should cost its neighbours.
                //
                // Two limits, in this order (issue #647):
                //
                // 1. The **store's** per-file cap decides policy. It sees the
                //    whole payload, so it refuses by name and size —
                //    "`hero.mov` is 91.4 MiB, over the 64.0 MiB limit for a
                //    single file" — and it honours the company's configured
                //    `[workspace] max_blob_mb`, which this layer cannot: routers
                //    are built once, before any company exists.
                // 2. This layer is the **backstop**, four times the default cap.
                //    It exists so an unbounded body cannot be buffered, not to
                //    express policy, and the headroom is what lets the store
                //    speak first for every realistic over-cap upload.
                //
                // Both answer 413. Setting this *at* the cap — what it used to
                // be — made (1) unreachable: the body limit fired mid-parse, and
                // a body that stops mid-part is indistinguishable from a
                // malformed one, so the honest 413 came out as `400 malformed
                // multipart`. `upload` below still has to classify, because this
                // layer's own failure arrives through the same parse error.
                //
                // The trade: this is also how much one in-flight upload may hold
                // in memory, because the write path buffers by design.
                .layer(DefaultBodyLimit::max(
                    crate::runtime::UPLOAD_BODY_LIMIT_BYTES as usize,
                )),
        )
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
    /// Set only on a **binary** node (issue #553), and omitted entirely
    /// otherwise — so `mime` being present is the console's test for "render or
    /// download this instead of editing it", with no present-but-null case to
    /// disambiguate. The tree read happens on every mount, so three nulls per
    /// prose note is a cost worth not paying.
    #[serde(skip_serializing_if = "Option::is_none")]
    mime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
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
            mime: node.mime,
            size: node.size,
            sha256: node.sha256,
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

/// One search hit as the console renders it (issue #607).
///
/// Carries the whole node — so the console can badge origin and mark a binary
/// exactly as it does in the tree — plus the two things only a search knows:
/// where the node sits (`path`, which the tree view derives from `parentId` but
/// a flat hit list cannot) and why it matched.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchHitBody {
    #[serde(flatten)]
    node: FsNode,
    /// The node's logical path, e.g. `Standards/Engineering.md`.
    path: String,
    /// `name` or `content`.
    matched: &'static str,
    /// Text around the first body match. Absent for a name match, a folder, and
    /// a binary node — a payload is never excerpted.
    #[serde(skip_serializing_if = "Option::is_none")]
    excerpt: Option<String>,
}

/// The search response: the page, and how many matched in total.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchResults {
    hits: Vec<SearchHitBody>,
    /// Matches before the limit was applied — so the console can say "20 of 137"
    /// rather than implying it is showing everything.
    total: usize,
}

/// The search query string.
#[derive(Debug, Deserialize)]
struct SearchQuery {
    /// The text to look for. Required; an empty one is a 400.
    #[serde(default)]
    q: Option<String>,
    /// Optional subtree scope, by logical path.
    #[serde(default)]
    prefix: Option<String>,
    /// Optional page size. Absent means the default; `0` is a 400 rather than a
    /// silent "everything".
    #[serde(default)]
    limit: Option<usize>,
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
    // A binary node would otherwise answer here with an empty body — the port's
    // honest answer to a prose-shaped read, and a confusing one to receive: the
    // console would render a blank editor over a file that is not blank. Naming
    // the route that does serve it turns a silent wrong answer into a directed
    // one (issue #553).
    if let Some(mime) = &node.mime {
        return Err(ApiError(OpenCompanyError::InvalidRequest(format!(
            "`{}` holds {mime} data, not text; fetch it from \
             `…/workspace/blob/{node_id}` instead",
            node.name
        ))));
    }
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

/// `GET …/workspace/search?q=…` — which notes mention a phrase (issue #607).
///
/// A thin call into
/// [`search_workspace`](crate::company::workspace_search::search_workspace),
/// the same helper behind the GraphQL `workspaceSearch` resolver and the agent
/// `workspace_search` tool. That is the point rather than a convenience: three
/// surfaces answering "which notes mention X" with three scans would drift, and
/// the drift would be invisible from whichever one the reader was not looking
/// at — the same argument that put the backlink scan in
/// [`workspace_links`](crate::company::workspace_links).
///
/// `q` is required and an empty one is a 400: an empty query is not "everything"
/// (that is the tree read on this same prefix), and answering it as such would
/// turn a cleared search box into a full-tree fetch on every keystroke.
async fn search(
    company: ScopedCompany,
    Query(query): Query<SearchQuery>,
) -> Result<Json<SearchResults>, ApiError> {
    let q = query.q.unwrap_or_default();
    // Stated, never silently unlimited. `limit=0` is a caller meaning something
    // specific, and both available guesses — "the default" and "no limit" — are
    // wrong.
    let limit = match query.limit {
        None => NonZeroUsize::new(DEFAULT_SEARCH_LIMIT).expect("the default limit is non-zero"),
        Some(n) => NonZeroUsize::new(n).ok_or_else(|| {
            ApiError(OpenCompanyError::InvalidRequest(format!(
                "`limit` is 0, which would return no matches; omit it for the default of \
                 {DEFAULT_SEARCH_LIMIT}, or pass a value between 1 and {MAX_SEARCH_RESULTS}"
            )))
        })?,
    };

    let outcome = search_workspace(
        company.runtime.workspace().as_ref(),
        company.id(),
        &q,
        query.prefix.as_deref(),
        limit,
    )
    .await?;

    Ok(Json(SearchResults {
        total: outcome.total,
        hits: outcome
            .hits
            .into_iter()
            .map(|hit| SearchHitBody {
                node: FsNode::from_node(hit.node, None),
                path: hit.path,
                matched: hit.matched.as_str(),
                excerpt: hit.excerpt,
            })
            .collect(),
    }))
}

/// `GET …/workspace/blob/{node_id}` — stream a binary node's payload.
///
/// The counterpart of [`read_file`], and the only way bytes leave the tree. The
/// body is streamed rather than buffered, so serving a 200 MiB video costs the
/// process a chunk at a time.
///
/// A folder, a prose note and an id that names nothing all 404 identically: the
/// port answers `None` for each, and telling them apart would leak which node
/// ids exist to a caller that cannot read them anyway.
///
/// `ETag` is the payload's sha256 — the digest the store computed from the
/// bytes it holds, so a conditional request is answered by the thing itself
/// rather than by a timestamp that a rename would move. `Content-Disposition`
/// is `inline`: the console renders images in place, and a browser opening the
/// URL directly should show the file rather than be forced to download it.
async fn read_blob(
    company: ScopedCompany,
    Path(NodePath { node_id }): Path<NodePath>,
) -> Result<Response, ApiError> {
    let Some((node, stream)) = company
        .runtime
        .workspace()
        .read_bytes(company.id(), &node_id)
        .await?
    else {
        return Err(ApiError(OpenCompanyError::CompanyNotFound(format!(
            "workspace blob {node_id}"
        ))));
    };
    let mime = node
        .mime
        .unwrap_or_else(|| "application/octet-stream".to_string());
    let mut response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        // Quoted per RFC 9110, and escaped defensively: a node name never
        // reaches this header, but a digest that somehow did would break the
        // response rather than the parse.
        .header(
            header::CONTENT_DISPOSITION,
            format!(
                "inline; filename=\"{}\"",
                node.name.replace('"', "").replace(['\r', '\n'], "")
            ),
        );
    if let Some(sha) = node.sha256 {
        response = response.header(header::ETAG, format!("\"{sha}\""));
    }
    if let Some(size) = node.size {
        response = response.header(header::CONTENT_LENGTH, size);
    }
    response.body(Body::from_stream(stream)).map_err(|e| {
        ApiError(OpenCompanyError::Store(format!(
            "blob response failed: {e}"
        )))
    })
}

/// Names a multipart failure for what it actually was (issue #647).
///
/// One error type carries two unrelated causes. A body that ran past the
/// route's `DefaultBodyLimit` stops mid-part, and to the multipart reader that
/// looks exactly like a body that was malformed to begin with — so reporting
/// every failure here as `InvalidRequest` told an operator whose only mistake
/// was picking a large file that their request was broken. It is not: it is
/// oversized, which is a different sentence and a different fix.
///
/// axum keeps the two apart. `MultipartError::status()` answers 413 for the
/// size-limit variants — multer's own `FieldSizeExceeded` / `StreamSizeExceeded`
/// and the `StreamReadFailed` whose source is `http_body_util`'s
/// `LengthLimitError`, which is the `DefaultBodyLimit` case — and 400 for the
/// genuinely malformed ones. Reading that rather than matching on the message
/// keeps this from breaking when axum rewords an error.
///
/// The 413 is raised as [`OpenCompanyError::WorkspaceQuota`] on purpose: that
/// is already the store's over-cap refusal, so the two causes share a status
/// (413) and a stable code (`workspace_quota_exceeded`) rather than inventing a
/// second vocabulary for "too big". A caller keying on the code cannot tell —
/// and should not have to — which of the two limits noticed.
///
/// Only the limit is named, never a size. The body was cut off, so the true
/// total is not knowable here; guessing at it would be worse than omitting it.
fn multipart_error(error: MultipartError, context: &str) -> ApiError {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        return ApiError(OpenCompanyError::WorkspaceQuota(format!(
            "this upload is larger than the {} this endpoint will read in one request, \
             so it was cut off before its size could be measured. Nothing was stored.",
            human(UPLOAD_BODY_LIMIT_BYTES),
        )));
    }
    ApiError(OpenCompanyError::InvalidRequest(format!(
        "{context}: {error}"
    )))
}

/// `POST …/workspace/upload` — multipart upload of a file of any kind.
///
/// The existing create route takes a JSON body, which cannot carry bytes; this
/// is the path an image, a PDF or a zip arrives on.
///
/// # Text still goes down the text path
///
/// A file that is *typed* as text and *is* valid UTF-8 is stored as a prose
/// note, not as a payload. That is not a size optimisation: a note is
/// diffable, backlinkable, searchable and editable in the console, and a
/// Markdown file uploaded as an opaque blob would silently lose all four.
/// Anything else — including a `.txt` that turns out to be binary — becomes a
/// binary node, because the decision is made on the bytes and not only on what
/// the caller claimed.
async fn upload(
    company: ScopedCompany,
    mut multipart: Multipart,
) -> Result<Json<FsNode>, ApiError> {
    let mut file: Option<(String, Option<String>, Vec<u8>)> = None;
    let mut parent_id: Option<String> = None;

    // Every one of these three is a place the body limit can be noticed — this
    // one while draining a part the route ignores, the two below while reading
    // one it wants — so all three classify rather than the one that happened to
    // be reported first.
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| multipart_error(e, "malformed multipart upload"))?
    {
        match field.name() {
            Some("parentId") | Some("parent_id") => {
                let value = field
                    .text()
                    .await
                    .map_err(|e| multipart_error(e, "unreadable parentId"))?;
                // An empty value is the console saying "the workspace root",
                // which is what an absent field means too.
                if !value.trim().is_empty() {
                    parent_id = Some(value);
                }
            }
            Some("file") => {
                let name = field
                    .file_name()
                    .map(str::to_string)
                    .filter(|n| !n.trim().is_empty())
                    .ok_or_else(|| {
                        ApiError(OpenCompanyError::InvalidRequest(
                            "the uploaded file has no filename".to_string(),
                        ))
                    })?;
                let declared = field.content_type().map(str::to_string);
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| multipart_error(e, "unreadable file part"))?;
                file = Some((name, declared, bytes.to_vec()));
            }
            // Ignored rather than rejected: a browser's `FormData` may carry
            // fields this route has no use for, and refusing the upload over
            // one would be a puzzle to debug from the console side.
            _ => {}
        }
    }

    let Some((name, declared, bytes)) = file else {
        return Err(ApiError(OpenCompanyError::InvalidRequest(
            "the upload carried no `file` part".to_string(),
        )));
    };
    // The last path segment only: a browser may send a full path as the
    // filename, and the store's own name check would reject it — better to
    // accept the upload under the obvious name than to fail on a detail the
    // operator did not choose.
    let name = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(&name)
        .trim()
        .to_string();
    let mime = resolve_mime(&name, declared.as_deref());

    let mut node = WorkspaceNode {
        id: generate_id(),
        name,
        kind: NodeKind::File,
        parent_id,
        updated_at_millis: crate::ports::now_millis(),
        created_by: WorkspaceOrigin::Operator,
        updated_by: WorkspaceOrigin::Operator,
        mime: None,
        size: None,
        sha256: None,
    };

    match text_body(&mime, &bytes) {
        Some(text) => {
            company
                .runtime
                .workspace()
                .create(company.id(), &node, Some(&text))
                .await?;
            Ok(Json(FsNode::from_node(node, Some(text))))
        }
        None => {
            node.mime = Some(mime);
            company
                .runtime
                .workspace()
                .create_binary(company.id(), &node, &bytes)
                .await?;
            // Re-read so the response carries the size and digest the STORE
            // computed, rather than the `None`s this handler sent in. The
            // console shows both, and showing a value the store did not
            // produce is how a digest stops meaning anything.
            let stored = company
                .runtime
                .workspace()
                .tree(company.id())
                .await?
                .into_iter()
                .find(|n| n.id == node.id)
                .unwrap_or(node);
            Ok(Json(FsNode::from_node(stored, None)))
        }
    }
}

/// The media type to store a upload under.
///
/// The browser's declared type wins when it says anything specific; otherwise
/// the extension decides. `application/octet-stream` is what a browser sends
/// when it has no idea, so it is treated as no answer rather than as an answer.
fn resolve_mime(name: &str, declared: Option<&str>) -> String {
    let declared = declared
        .map(|d| d.split(';').next().unwrap_or(d).trim().to_lowercase())
        .filter(|d| !d.is_empty() && d != "application/octet-stream");
    declared.unwrap_or_else(|| {
        mime_guess::from_path(name)
            .first_raw()
            .unwrap_or("application/octet-stream")
            .to_string()
    })
}

/// The upload's text body, when it should be stored as a prose note.
///
/// Both halves must hold: the type says text **and** the bytes decode as UTF-8.
/// Trusting the type alone would store a mislabelled binary as a note and
/// mangle it; trusting the bytes alone would turn a small UTF-8-clean PDF-ish
/// blob into a "note" nobody can read.
fn text_body(mime: &str, bytes: &[u8]) -> Option<String> {
    let texty = mime.starts_with("text/")
        || matches!(
            mime,
            "application/json" | "application/xml" | "application/x-yaml" | "application/yaml"
        );
    if !texty {
        return None;
    }
    String::from_utf8(bytes.to_vec()).ok()
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
