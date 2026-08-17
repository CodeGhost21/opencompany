//! The ledger surface: `GET/POST /ledgers`, `GET/DELETE /ledgers/{slug}`,
//! `GET/POST /ledgers/{slug}/entries`, `DELETE /ledgers/{slug}/entries/{id}`,
//! and `GET /ledgers/{slug}/rendered`, under both scope forms.
//!
//! # Where the deletion rule is actually enforced
//!
//! [`ScopedCompany::actor`] is `Some` for a signed-in person and `None` for the
//! machine principal — the same distinction the operator-message journal draws.
//! That is what [`author`] turns into a [`LedgerAuthor`], and
//! [`crate::company::ledgers`] refuses a delete from anything but a person.
//!
//! The check therefore lives **once**, in the service, and this module's job is
//! only to name the caller honestly. A route that decided for itself would be a
//! second answer to the question, and the agent tools would need a third.

use axum::extract::{Path, Query};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::company::ledgers;
use crate::ledger::{Entry, LedgerAuthor, LedgerSource, LedgerSpec};
use crate::ports::types::ActorKind;
use crate::server::error::ApiError;
use crate::server::ops::{ScopedCompany, scoped};

/// Builds the ledger route fragment.
pub fn router() -> Router<AppState> {
    scoped("/ledgers", get(list_ledgers).post(define_ledger))
        .merge(scoped(
            "/ledgers/{slug}",
            get(read_ledger).delete(retire_ledger),
        ))
        // The rendered Markdown the `derived/` folder holds, served from the
        // same derivation rather than by reading the file back — so the console
        // cannot be shown a stale copy if a workspace write ever failed.
        .merge(scoped("/ledgers/{slug}/rendered", get(rendered)))
        .merge(scoped(
            "/ledgers/{slug}/entries",
            post(record_entry).get(read_ledger),
        ))
        .merge(scoped(
            "/ledgers/{slug}/entries/{entry_id}",
            delete(delete_entry),
        ))
}

/// One ledger as the console lists it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LedgerSummary {
    slug: String,
    title: String,
    purpose: String,
    /// `events` or `native`. A native ledger's rows are written by the surface
    /// that owns them, which is why the console renders it read-only here.
    source: LedgerSource,
    /// Where its rendered file lives in the workspace.
    derived: String,
    /// How it is actually written, in a sentence — what the console shows in
    /// place of a compose box on a native ledger.
    written_by: String,
    /// Whether the runtime ships it. A built-in cannot be retired.
    builtin: bool,
    fields: Vec<crate::ledger::Field>,
    statuses: Vec<crate::ledger::StatusSpec>,
    sections: Vec<crate::ledger::Section>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    writers: Vec<String>,
    /// Rows not in a closed status.
    open: usize,
    /// Rows in one.
    closed: usize,
}

/// What `GET /ledgers` answers.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LedgerList {
    ledgers: Vec<LedgerSummary>,
    /// Declarations that could not be loaded, and why.
    ///
    /// Surfaced rather than swallowed: a company whose ledger silently stopped
    /// appearing has no way to find out why, and the answer is always in a
    /// declaration it wrote.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    faults: Vec<String>,
    /// How many more this company may declare.
    remaining: usize,
}

/// One row, as the console renders it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EntryRow {
    id: String,
    fields: std::collections::BTreeMap<String, String>,
    status: String,
    title: String,
    /// Whether the row's status is one the ledger calls closed.
    closed: bool,
    opened_by: LedgerAuthor,
    updated_by: LedgerAuthor,
    opened_at: u64,
    updated_at: u64,
    events: u32,
}

impl EntryRow {
    fn of(entry: &Entry, spec: &LedgerSpec) -> Self {
        let status = entry.status(spec);
        Self {
            id: entry.id.clone(),
            fields: entry.fields.clone(),
            closed: spec.is_closed(&status),
            status,
            title: entry.title(spec).to_string(),
            opened_by: entry.opened_by.clone(),
            updated_by: entry.updated_by.clone(),
            opened_at: entry.opened_at_millis,
            updated_at: entry.updated_at_millis,
            events: entry.events,
        }
    }
}

/// What `GET /ledgers/{slug}` answers.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LedgerRead {
    ledger: LedgerSummary,
    entries: Vec<EntryRow>,
    /// How many matched before the bound was applied — so a short list is
    /// distinguishable from all of them.
    matched: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    faults: Vec<String>,
}

/// The narrowing `GET /ledgers/{slug}` accepts.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadQuery {
    entry: Option<String>,
    status: Option<String>,
    q: Option<String>,
    sort: Option<String>,
    limit: Option<usize>,
}

/// The body `POST /ledgers/{slug}/entries` accepts.
///
/// One shape for every write, because there is one write: opening, amending and
/// closing a row all merge fields into it. `status` and `reason` are named
/// separately only because a console form has fields for them; both are folded
/// into `fields` before the service sees them, so nothing downstream has two
/// ways of being told the same thing.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecordBody {
    id: String,
    #[serde(default)]
    fields: std::collections::BTreeMap<String, Option<String>>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

/// The query `DELETE /ledgers/{slug}` accepts.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RetireQuery {
    /// Delete the rows too. Off by default: retiring a ledger nobody reads is
    /// worth doing, and deleting what it recorded is a separate decision.
    #[serde(default)]
    purge: bool,
}

/// The caller, named honestly.
///
/// A signed-in person becomes a [`LedgerAuthor::human`]; the machine principal
/// becomes a system author, which the service refuses every deletion from. That
/// is deliberate and not an oversight: the platform credential is a tenant, not
/// a person, and *only a person deletes* has to mean a person.
fn ctx(scope: &ScopedCompany) -> ledgers::Ledgers {
    scope.runtime.as_ref().into()
}

fn author(scope: &ScopedCompany) -> LedgerAuthor {
    match &scope.actor {
        Some(actor) if matches!(actor.kind, ActorKind::Operator | ActorKind::User) => {
            LedgerAuthor::human(actor.id.clone(), actor.id.clone())
        }
        Some(actor) => LedgerAuthor::agent(actor.id.clone()),
        None => LedgerAuthor::system("platform"),
    }
}

async fn summary(
    ctx: &ledgers::Ledgers,
    spec: &LedgerSpec,
) -> Result<LedgerSummary, ApiError> {
    let entries = ledgers::entries(ctx, spec).await?;
    Ok(LedgerSummary {
        slug: spec.slug.clone(),
        title: spec.title.clone(),
        purpose: spec.purpose.clone(),
        source: spec.source,
        derived: spec.derived.clone(),
        written_by: spec.written_by.clone(),
        builtin: spec.builtin,
        fields: spec.fields.clone(),
        statuses: spec.statuses.clone(),
        sections: spec.sections.clone(),
        writers: spec.writers.clone(),
        open: entries.open_count(spec),
        closed: entries.closed_count(spec),
    })
}

async fn list_ledgers(scope: ScopedCompany) -> Result<Json<LedgerList>, ApiError> {
    let registry = ledgers::registry(&ctx(&scope)).await?;
    let mut out = Vec::new();
    for spec in registry.specs() {
        out.push(summary(&ctx(&scope), spec).await?);
    }
    let declared = registry.specs().iter().filter(|spec| !spec.builtin).count();
    Ok(Json(LedgerList {
        ledgers: out,
        faults: registry.faults().to_vec(),
        remaining: crate::ledger::MAX_DECLARED.saturating_sub(declared),
    }))
}

async fn define_ledger(
    scope: ScopedCompany,
    Json(document): Json<serde_json::Value>,
) -> Result<(StatusCode, Json<LedgerSummary>), ApiError> {
    let spec = ledgers::define(&ctx(&scope), &document).await?;
    let summary = summary(&ctx(&scope), &spec).await?;
    Ok((StatusCode::CREATED, Json(summary)))
}

async fn read_ledger(
    scope: ScopedCompany,
    Path(path): Path<LedgerPath>,
    Query(query): Query<ReadQuery>,
) -> Result<Json<LedgerRead>, ApiError> {
    let registry = ledgers::registry(&ctx(&scope)).await?;
    let spec = registry.require(&path.slug)?;
    let read = ledgers::read(
        &ctx(&scope),
        spec,
        &ledgers::Query {
            entry: query.entry,
            status: query.status,
            text: query.q,
            sort: query.sort,
            limit: query.limit,
        },
    )
    .await?;
    Ok(Json(LedgerRead {
        ledger: summary(&ctx(&scope), spec).await?,
        entries: read.entries.iter().map(|e| EntryRow::of(e, spec)).collect(),
        matched: read.matched,
        faults: read.faults,
    }))
}

/// The exact Markdown `derived/<NAME>.md` holds.
async fn rendered(
    scope: ScopedCompany,
    Path(path): Path<LedgerPath>,
) -> Result<String, ApiError> {
    let registry = ledgers::registry(&ctx(&scope)).await?;
    let spec = registry.require(&path.slug)?;
    Ok(ledgers::render(&ctx(&scope), spec).await?)
}

async fn record_entry(
    scope: ScopedCompany,
    Path(path): Path<LedgerPath>,
    Json(body): Json<RecordBody>,
) -> Result<Json<EntryRow>, ApiError> {
    let registry = ledgers::registry(&ctx(&scope)).await?;
    let spec = registry.require(&path.slug)?;
    let mut fields = body.fields;
    if let Some(status) = body.status
        && let Some(field) = spec.status_field()
    {
        fields.insert(field.name.clone(), Some(status));
    }
    if let Some(reason) = body.reason {
        fields.insert(crate::ledger::REASON_FIELD.to_string(), Some(reason));
    }
    let entry = ledgers::record(&ctx(&scope), spec, &author(&scope), &body.id, fields).await?;
    Ok(Json(EntryRow::of(&entry, spec)))
}

async fn delete_entry(
    scope: ScopedCompany,
    Path(path): Path<EntryPath>,
) -> Result<StatusCode, ApiError> {
    let registry = ledgers::registry(&ctx(&scope)).await?;
    let spec = registry.require(&path.slug)?;
    let removed =
        ledgers::delete_entry(&ctx(&scope), spec, &author(&scope), &path.entry_id).await?;
    Ok(if removed {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    })
}

async fn retire_ledger(
    scope: ScopedCompany,
    Path(path): Path<LedgerPath>,
    Query(query): Query<RetireQuery>,
) -> Result<StatusCode, ApiError> {
    ledgers::retire(&ctx(&scope), &author(&scope), &path.slug, query.purge).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct LedgerPath {
    // The scoped forms differ in whether they carry `{id}`, so the path struct
    // has to tolerate both. `slug` is the only segment either form guarantees.
    #[serde(default)]
    #[allow(dead_code)]
    id: Option<String>,
    slug: String,
}

#[derive(Debug, Deserialize)]
struct EntryPath {
    #[serde(default)]
    #[allow(dead_code)]
    id: Option<String>,
    slug: String,
    entry_id: String,
}

#[cfg(test)]
#[path = "ledgers_test.rs"]
mod test;
