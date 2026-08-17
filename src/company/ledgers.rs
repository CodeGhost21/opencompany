//! The one place a ledger is read, written, declared or retired.
//!
//! Every surface — the REST routes, the agent tools, the console — comes
//! through here rather than reaching the [`LedgerStore`] directly, and that is
//! load-bearing rather than tidy. Four rules only hold if exactly one code path
//! enforces them:
//!
//! * **Only a person deletes.** [`delete_entry`] and [`retire`] check
//!   [`AuthorKind::may_delete`] on the caller, so an agent cannot reach a
//!   deletion by finding a route that forgot to.
//! * **A writer must be allowed to write.** [`LedgerSpec::writers`] is checked
//!   at call time, because the set of ledgers is not fixed when tools are
//!   wired — a company may declare one afterwards, so holding `record_entry` is
//!   not permission to write everything.
//! * **A close says why.** A closing status that declares `needs_reason` and is
//!   written with no reason is refused at the write rather than reported at the
//!   read, because a row that closed silently is worth nothing to whoever picks
//!   it up next and by then the person who knew has moved on.
//! * **The derived file follows the write.** Every mutation re-renders and
//!   republishes, so `derived/` is never a stale copy of something.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::Result;
use crate::company::runtime::CompanyRuntime;
use crate::error::OpenCompanyError;
use crate::ledger::{
    AuthorKind, Entries, LedgerAuthor, LedgerEvent, LedgerSource, LedgerSpec, MAX_FIELD_CHARS,
    Order, REASON_FIELD, Registry, budget, derived, engine, native, parse_order,
};
use crate::ports::now_millis;

/// Builds the company's registry: the built-ins plus whatever it declared.
///
/// # Errors
///
/// Returns whatever the store returns. A declaration that will not load is a
/// [`Registry::faults`] entry rather than an error — a company that wrote one
/// bad ledger must still reach its board.
pub async fn registry(runtime: &CompanyRuntime) -> Result<Registry> {
    let declared = runtime.ledgers().list_specs(runtime.id()).await?;
    Ok(Registry::build(declared))
}

/// Every entry on one ledger, folded and checked.
///
/// A [`LedgerSource::Native`] ledger is projected from the store that actually
/// owns it, so a caller reads the board and a declared ledger through the same
/// shape and never has to know which it got.
///
/// # Errors
///
/// Returns whatever the store returns.
pub async fn entries(runtime: &CompanyRuntime, spec: &LedgerSpec) -> Result<Entries> {
    match spec.source {
        LedgerSource::Native => {
            let tasks = runtime.tasks().list(runtime.id()).await?;
            Ok(native::entries_from_tasks(&tasks))
        }
        LedgerSource::Events => {
            let events = runtime.ledgers().events(runtime.id(), &spec.slug).await?;
            Ok(engine::fold(spec, &events))
        }
    }
}

/// One ledger's rendered Markdown, as the `derived/` file holds it.
///
/// # Errors
///
/// Returns whatever the store returns.
pub async fn render(runtime: &CompanyRuntime, spec: &LedgerSpec) -> Result<String> {
    Ok(engine::render(spec, &entries(runtime, spec).await?))
}

/// Every ledger's index, for a turn that needs to know what the company records.
///
/// Bounded by construction: an index carries identities and drops reasoning,
/// and each one ends with the call that fetches the rest. See
/// [`engine::index`] for why that closing line is not optional.
///
/// # Errors
///
/// Returns whatever the store returns.
pub async fn briefing(runtime: &CompanyRuntime, registry: &Registry) -> Result<String> {
    let mut out = String::from(
        "## The company's ledgers\n\nEach of these is a durable record with its own rows. Read one \
         with `read_ledger`; record into one with `record_entry`.\n\n",
    );
    for spec in registry.specs() {
        let entries = entries(runtime, spec).await?;
        out.push_str(&engine::index(spec, &entries));
        out.push('\n');
    }
    Ok(out)
}

/// What one read returned.
#[derive(Clone, Debug)]
pub struct Read {
    /// The rows, already ordered and bounded.
    pub entries: Vec<engine::Entry>,
    /// How many matched before the bound was applied.
    pub matched: usize,
    /// What the declared checks found on the whole ledger.
    pub faults: Vec<String>,
}

/// What a read is narrowed by.
#[derive(Clone, Debug, Default)]
pub struct Query {
    /// One entry, by id.
    pub entry: Option<String>,
    /// Only entries in this status.
    pub status: Option<String>,
    /// Only entries whose id or any field contains this.
    pub text: Option<String>,
    /// `recorded` or `recent`; the ledger's own default when absent.
    pub sort: Option<String>,
    /// Rows to return. Bounded whatever is asked for.
    pub limit: Option<usize>,
}

/// Reads one ledger, narrowed and bounded.
///
/// The bound is applied whatever the caller asks for, and [`Read::matched`]
/// reports how many there were — so a caller that got a short list can tell the
/// difference between *that is all of them* and *that is the first twenty*.
///
/// # Errors
///
/// Returns whatever the store returns, or
/// [`OpenCompanyError::InvalidRequest`] when `sort` is not an order name.
pub async fn read(runtime: &CompanyRuntime, spec: &LedgerSpec, query: &Query) -> Result<Read> {
    let folded = entries(runtime, spec).await?;
    let order = match query.sort.as_deref() {
        Some(name) => parse_order(name)?,
        None => spec.default_order(),
    };
    let mut rows: Vec<engine::Entry> = engine::ordered(&folded.entries, order)
        .into_iter()
        .filter(|entry| match &query.entry {
            Some(id) => entry.id == id.trim(),
            None => true,
        })
        .filter(|entry| match &query.status {
            Some(status) => entry.status(spec).eq_ignore_ascii_case(status.trim()),
            None => true,
        })
        .filter(|entry| match &query.text {
            Some(text) => entry.matches(text),
            None => true,
        })
        .cloned()
        .collect();
    let matched = rows.len();
    let limit = query
        .limit
        .unwrap_or(budget::DEFAULT_READ_LIMIT)
        .clamp(1, budget::MAX_READ_LIMIT);
    rows.truncate(limit);
    Ok(Read {
        entries: rows,
        matched,
        faults: folded.faults,
    })
}

/// Records one event against a ledger, and republishes its derived file.
///
/// Fields are trimmed and capped on the way in ([`MAX_FIELD_CHARS`]);
/// over-long text is truncated rather than rejected, because losing the tail of
/// a long note is a smaller failure than losing the whole write.
///
/// # Errors
///
/// Returns [`OpenCompanyError::InvalidRequest`] when the ledger is native (its
/// rows are written by the surface that owns them), when the author may not
/// write it, when the event names no entry, when it sets a status the ledger
/// does not declare, or when it closes a row into a status that demands a
/// reason without giving one.
pub async fn record(
    runtime: &CompanyRuntime,
    spec: &LedgerSpec,
    author: &LedgerAuthor,
    id: &str,
    fields: BTreeMap<String, Option<String>>,
) -> Result<engine::Entry> {
    if spec.source == LedgerSource::Native {
        return Err(OpenCompanyError::InvalidRequest(format!(
            "`{}` is not written with `record_entry`. {}",
            spec.slug, spec.written_by
        )));
    }
    if !spec.writable_by(&author.id) {
        return Err(OpenCompanyError::InvalidRequest(format!(
            "`{}` is written by {} on this company, and `{}` is not one of them.",
            spec.slug,
            spec.writers.join(", "),
            author.id
        )));
    }
    let id = id.trim();
    if id.is_empty() {
        return Err(OpenCompanyError::InvalidRequest(
            "an entry needs an id. Reuse an existing one to amend that row; a new one opens a new \
             row."
                .to_string(),
        ));
    }

    let fields = normalize_fields(fields);
    if let Some(field) = spec.status_field()
        && let Some(Some(status)) = fields.get(&field.name)
    {
        if !spec.knows_status(status) {
            return Err(OpenCompanyError::InvalidRequest(format!(
                "`{status}` is not a status on `{}`; use one of: {}",
                spec.slug,
                spec.statuses
                    .iter()
                    .map(|declared| declared.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        // The reason may arrive on this event or already be on the row, so the
        // check runs against the merged result rather than against the event —
        // otherwise closing a row that already explained itself would be
        // refused for saying it twice.
        if spec
            .status(status)
            .is_some_and(|declared| declared.needs_reason)
        {
            let arriving = fields
                .get(REASON_FIELD)
                .and_then(|value| value.as_deref())
                .unwrap_or_default();
            if arriving.trim().is_empty() {
                let existing = entries(runtime, spec).await?;
                let held = existing
                    .find(id)
                    .map(|entry| entry.get(REASON_FIELD).to_string())
                    .unwrap_or_default();
                if held.trim().is_empty() {
                    return Err(OpenCompanyError::InvalidRequest(format!(
                        "closing `{id}` as `{status}` needs a `{REASON_FIELD}`. A row that does \
                         not say why it closed is worth nothing to whoever reads it next — and \
                         'did not work' costs the same to write as the reason that would have \
                         saved them."
                    )));
                }
            }
        }
    }

    let event = LedgerEvent {
        ledger: spec.slug.clone(),
        id: id.to_string(),
        author: author.clone(),
        at_millis: now_millis(),
        fields,
    };
    runtime.ledgers().append(runtime.id(), &event).await?;
    let folded = entries(runtime, spec).await?;
    publish(runtime, spec, &folded).await;
    folded.find(id).cloned().ok_or_else(|| {
        OpenCompanyError::NotFound(format!(
            "recorded `{id}` on `{}` but the fold did not return it",
            spec.slug
        ))
    })
}

/// Closes an entry: a status and a reason, which is [`record`] with two fields.
///
/// A separate entry point rather than a separate operation, because *the write
/// is still a merge* — see [`LedgerEvent`]. What it adds is the refusal to
/// close into a status that does not close anything, which is the mistake a
/// caller reaching for "close" actually makes.
///
/// # Errors
///
/// As [`record`], plus [`OpenCompanyError::InvalidRequest`] when `status` is
/// not one of the ledger's closing statuses.
pub async fn close(
    runtime: &CompanyRuntime,
    spec: &LedgerSpec,
    author: &LedgerAuthor,
    id: &str,
    status: &str,
    reason: &str,
) -> Result<engine::Entry> {
    let closing = spec.closing_statuses();
    if !closing.iter().any(|name| name.eq_ignore_ascii_case(status.trim())) {
        return Err(OpenCompanyError::InvalidRequest(if closing.is_empty() {
            format!(
                "`{}` declares no status that closes a row, so nothing on it can be closed. \
                 Record a status against it instead.",
                spec.slug
            )
        } else {
            format!(
                "`{status}` does not close a row on `{}`; use one of: {}",
                spec.slug,
                closing.join(", ")
            )
        }));
    }
    let Some(field) = spec.status_field() else {
        return Err(OpenCompanyError::InvalidRequest(format!(
            "`{}` declares no status field, so nothing on it has a status to close.",
            spec.slug
        )));
    };
    let mut fields = BTreeMap::new();
    fields.insert(field.name.clone(), Some(status.trim().to_string()));
    if !reason.trim().is_empty() {
        fields.insert(REASON_FIELD.to_string(), Some(reason.trim().to_string()));
    }
    record(runtime, spec, author, id, fields).await
}

/// Declares a new ledger.
///
/// Open to agents as well as people, deliberately: the whole point is that the
/// company discovers which axes it needs while it is running, and a declaration
/// that needed a release — or an operator — would be discovered and then not
/// made. What an agent cannot do is *undo* one; see [`retire`].
///
/// # Errors
///
/// Returns [`OpenCompanyError::InvalidRequest`] when the declaration is
/// malformed, collides with an existing ledger, or is past the cap.
pub async fn define(runtime: &CompanyRuntime, document: &serde_json::Value) -> Result<LedgerSpec> {
    let spec = crate::ledger::parse(document, false)?;
    let registry = registry(runtime).await?;
    registry.admits(&spec)?;
    runtime.ledgers().put_spec(runtime.id(), &spec).await?;
    // Rendered immediately, empty, so the ledger is visible in `derived/` from
    // the moment it exists rather than from its first row. A folder that gains
    // a file only once somebody writes to it reads as though the ledger was
    // never created.
    let folded = entries(runtime, &spec).await?;
    publish(runtime, &spec, &folded).await;
    Ok(spec)
}

/// Retires a declared ledger.
///
/// **A person's call.** The rows are left where they are — a ledger nobody
/// reads is worth retiring and the record in it is not — unless `purge` is set,
/// which is the explicit *and delete what it holds* that only a person can ask
/// for.
///
/// # Errors
///
/// Returns [`OpenCompanyError::InvalidRequest`] when the author is not a person
/// or the slug names a built-in, and [`OpenCompanyError::NotFound`] when
/// nothing carries that slug.
pub async fn retire(
    runtime: &CompanyRuntime,
    author: &LedgerAuthor,
    slug: &str,
    purge: bool,
) -> Result<()> {
    refuse_non_human(author, "retire a ledger")?;
    let registry = registry(runtime).await?;
    let spec = registry.require(slug)?;
    if spec.builtin {
        return Err(OpenCompanyError::InvalidRequest(format!(
            "`{}` ships with the runtime and cannot be retired. Every prompt and route naming it \
             is written against it existing.",
            spec.slug
        )));
    }
    runtime.ledgers().delete_spec(runtime.id(), &spec.slug).await?;
    if purge {
        runtime
            .ledgers()
            .purge_ledger(runtime.id(), &spec.slug)
            .await?;
    }
    Ok(())
}

/// Deletes one entry and everything recorded against it.
///
/// **A person's call**, and the only irreversible one on a ledger. An agent
/// that wants to be finished with a row closes it, which keeps the reason.
///
/// # Errors
///
/// Returns [`OpenCompanyError::InvalidRequest`] when the author is not a person
/// or the ledger is native (its rows are deleted through the surface that owns
/// them).
pub async fn delete_entry(
    runtime: &CompanyRuntime,
    spec: &LedgerSpec,
    author: &LedgerAuthor,
    id: &str,
) -> Result<bool> {
    refuse_non_human(author, "delete a row")?;
    if spec.source == LedgerSource::Native {
        return Err(OpenCompanyError::InvalidRequest(format!(
            "`{}` keeps its rows elsewhere, so they are deleted there. {}",
            spec.slug, spec.written_by
        )));
    }
    let removed = runtime
        .ledgers()
        .purge_entry(runtime.id(), &spec.slug, id.trim())
        .await?;
    if removed {
        let folded = entries(runtime, spec).await?;
        publish(runtime, spec, &folded).await;
    }
    Ok(removed)
}

/// Re-renders every ledger into `derived/`.
///
/// For the paths that change what a file *would* say without writing a row: a
/// declaration edited, a bound tightened, a company restored from a bundle. A
/// file rendered by a build that has since changed is a file whose reader is
/// being told last release's answer, and nothing on the write path would ever
/// notice.
///
/// # Errors
///
/// Returns whatever the store returns.
pub async fn republish_all(runtime: &CompanyRuntime) -> Result<usize> {
    let registry = registry(runtime).await?;
    let mut written = 0;
    for spec in registry.specs() {
        let folded = entries(runtime, spec).await?;
        publish(runtime, spec, &folded).await;
        written += 1;
    }
    Ok(written)
}

/// Refuses anything but a person.
fn refuse_non_human(author: &LedgerAuthor, what: &str) -> Result<()> {
    if author.kind.may_delete() {
        return Ok(());
    }
    Err(OpenCompanyError::InvalidRequest(format!(
        "only a person may {what}. Everything a teammate does to a ledger is additive and \
         therefore recoverable by reading its log; a deletion is not, and a runtime where a turn \
         can erase the record of what it did is one whose record means nothing. Close the row \
         instead — it keeps the reason."
    )))
}

/// Trims and caps every value, dropping keys that are only whitespace.
fn normalize_fields(
    fields: BTreeMap<String, Option<String>>,
) -> BTreeMap<String, Option<String>> {
    fields
        .into_iter()
        .filter_map(|(name, value)| {
            let name = name.trim().to_string();
            if name.is_empty() {
                return None;
            }
            // An empty string clears, exactly as an explicit null does. A caller
            // sending `""` means "there is nothing here", and storing it as a
            // present-but-blank field would render an empty bullet under every
            // row that ever set it.
            let value = value.and_then(|text| {
                let text = text.trim();
                if text.is_empty() {
                    None
                } else {
                    Some(budget::truncate(text, MAX_FIELD_CHARS))
                }
            });
            Some((name, value))
        })
        .collect()
}

/// Writes the ledger's rendered file into `derived/`, logging a failure rather
/// than failing the write that produced it.
///
/// The store is the record and the file is a projection of it. A workspace that
/// refused the write — a quota, a backend hiccup — must not turn a row that
/// *was* recorded into a call that reports failure, because that is the one
/// outcome the caller cannot recover from: it will retry, and the retry will
/// record the row a second time.
async fn publish(runtime: &CompanyRuntime, spec: &LedgerSpec, folded: &Entries) {
    let body = engine::render(spec, folded);
    let workspace: Arc<dyn crate::ports::workspace::WorkspaceStore> = runtime.workspace().clone();
    if let Err(error) = derived::publish(workspace.as_ref(), runtime.id(), spec, &body).await {
        tracing::warn!(
            company = %runtime.id(),
            ledger = %spec.slug,
            %error,
            "ledger recorded but its derived file could not be written"
        );
    }
}

/// The order names a caller may pass, for a schema or an error.
pub const SORTS: [&str; 2] = crate::ledger::ORDERS;

/// The order a ledger uses when a caller names none.
pub fn default_sort(spec: &LedgerSpec) -> Order {
    spec.default_order()
}

/// An author for a turn belonging to `agent`.
pub fn agent_author(agent: &str) -> LedgerAuthor {
    LedgerAuthor::agent(agent)
}

/// An author for a request made by a person.
pub fn human_author(user_id: &str, label: &str) -> LedgerAuthor {
    LedgerAuthor::human(user_id, label)
}

/// Whether `author` may delete, for a surface that wants to hide the control
/// rather than let it fail.
pub fn may_delete(author: &LedgerAuthor) -> bool {
    matches!(author.kind, AuthorKind::Human)
}

#[cfg(test)]
#[path = "ledgers_test.rs"]
mod test;
