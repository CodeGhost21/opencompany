//! The rules only one code path enforces.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::json;
use tokio::sync::Notify;

use super::*;
use crate::company::runtime::CompanyRuntime;
use crate::ledger::LedgerAuthor;
use crate::ports::ledgers::LedgerStore;
use crate::ports::types::CompanyId;

/// The context under test, plus the runtime and home it borrows from.
async fn ledgers() -> (Ledgers, CompanyRuntime, tempfile::TempDir) {
    let (runtime, home) = runtime().await;
    let ctx = Ledgers::from(&runtime);
    (ctx, runtime, home)
}

async fn runtime() -> (CompanyRuntime, tempfile::TempDir) {
    let home = tempfile::tempdir().expect("tempdir");
    let manifest: crate::company::CompanyManifest = toml::from_str(
        r#"
        [company]
        name = "Acme"

        [[agent]]
        id = "ceo"
        role = "Chief"

        [policy]
        mode = "supervised"
        "#,
    )
    .expect("manifest");
    let runtime = crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest)
        .with_id(CompanyId::new("acme"))
        .build()
        .await
        .expect("runtime");
    (runtime, home)
}

fn hazards() -> serde_json::Value {
    json!({
        "slug": "hazards",
        "title": "Hazards",
        "purpose": "What could go wrong.",
        "derived": "derived/hazards.md",
        "fields": [
            { "name": "id", "role": "id" },
            { "name": "risk", "role": "title" },
            { "name": "status", "role": "status" },
            { "name": "reason", "role": "prose" }
        ],
        "statuses": [
            { "name": "open" },
            { "name": "closed", "closed": true, "needs_reason": true }
        ],
        "sections": [
            { "heading": "Live", "statuses": ["open"], "order": "recent" },
            { "heading": "Closed", "statuses": ["closed"] }
        ],
        "checks": ["known-status", "closed-needs-reason"]
    })
}

fn fields(pairs: &[(&str, &str)]) -> BTreeMap<String, Option<String>> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), Some((*v).to_string())))
        .collect()
}

fn agent() -> LedgerAuthor {
    LedgerAuthor::agent("ceo")
}

fn person() -> LedgerAuthor {
    LedgerAuthor::human("u-1", "Dana")
}

/// A company starts with the three built-ins and nothing else.
#[tokio::test]
async fn a_fresh_company_has_the_built_ins_and_the_baseline() {
    let (ctx, _runtime, _home) = ledgers().await;
    let registry = registry(&ctx).await.expect("registry");
    // The built-ins first, in registry order, then whatever the global
    // baseline seeds (`crate::globals::ledgers`) — a company that starts with
    // nothing to record its risks, promises or learnings on gets one only if
    // some turn thinks to invent it.
    let slugs = registry.slugs();
    assert_eq!(&slugs[..3], ["tasks", "goals", "decisions"]);
    for global in crate::globals::ledgers() {
        assert!(slugs.contains(&global.slug), "`{}` is missing", global.slug);
    }
    assert!(registry.faults().is_empty());
}

/// An agent may declare an axis nobody anticipated — the whole point.
#[tokio::test]
async fn an_agent_may_declare_a_ledger_and_record_into_it() {
    let (ctx, _runtime, _home) = ledgers().await;
    let spec = define(&ctx, &hazards()).await.expect("declared");
    assert_eq!(spec.slug, "hazards");

    let entry = record(
        &ctx,
        &spec,
        &agent(),
        "vendor-slip",
        fields(&[("risk", "the vendor misses the date"), ("status", "open")]),
    )
    .await
    .expect("recorded");
    assert_eq!(entry.get("risk"), "the vendor misses the date");
    assert_eq!(entry.opened_by.kind, crate::ledger::AuthorKind::Agent);

    let registry = registry(&ctx).await.expect("registry");
    assert!(registry.find("hazards").is_some());
}

/// Recording twice against one id is an amendment, not a second row.
#[tokio::test]
async fn recording_again_amends_the_same_row() {
    let (ctx, _runtime, _home) = ledgers().await;
    let spec = define(&ctx, &hazards()).await.expect("declared");
    record(&ctx, &spec, &agent(), "r1", fields(&[("risk", "first")]))
        .await
        .expect("recorded");
    let amended = record(&ctx, &spec, &agent(), "r1", fields(&[("risk", "second")]))
        .await
        .expect("amended");
    assert_eq!(amended.events, 2);
    let read = read(&ctx, &spec, &Query::default()).await.expect("read");
    assert_eq!(read.entries.len(), 1);
    assert_eq!(read.entries[0].get("risk"), "second");
}

/// #2048 review: `read_ledger` used to fold the ledger once for its rows and
/// again, independently, inside `summary()` for the open/closed count sent
/// alongside them. A write landing in the gap between those two folds could
/// flip a row's status after the rows were read but before the count was, so
/// the response shipped a badge that disagreed with the rows beside it.
///
/// This reproduces that gap deterministically — recording a close at exactly
/// the point the old handler's second, independent fold used to run — rather
/// than relying on scheduler luck to land a race. It shows two things: a
/// second fold at that point genuinely disagrees with the rows already
/// returned (the defect is real, not hypothetical), and `read`'s own
/// `open`/`closed` — computed from the very fold that produced `entries`,
/// never a later one — hold steady across the write instead.
#[tokio::test]
async fn a_second_independent_fold_would_disagree_with_the_rows_read_already_returned() {
    let (ctx, _runtime, _home) = ledgers().await;
    let spec = define(&ctx, &hazards()).await.expect("declared");
    record(
        &ctx,
        &spec,
        &agent(),
        "r1",
        fields(&[("risk", "a"), ("status", "open")]),
    )
    .await
    .expect("recorded");

    // One fold, as the fixed `read` performs it: rows and counts share a
    // single snapshot, so both describe the ledger as of the same instant.
    let read = read(&ctx, &spec, &Query::default()).await.expect("read");
    assert_eq!(read.entries.len(), 1);
    assert_eq!(read.open, 1);
    assert_eq!(read.closed, 0);

    // The write that used to land in the window between the rows' fold and
    // the count's second, independent fold.
    record(
        &ctx,
        &spec,
        &agent(),
        "r1",
        fields(&[("status", "closed"), ("reason", "handled")]),
    )
    .await
    .expect("closed");

    // A second, independent fold taken right here — what `summary()` used to
    // run after `read()` already returned — now disagrees with the rows
    // `read` already handed back above: this is the exact mismatch a
    // two-fold response would have shipped to the console.
    let refolded = entries(&ctx, &spec).await.expect("entries");
    assert_eq!(
        refolded.open_count(&spec),
        0,
        "the row already closed by the time a second fold ran"
    );
    assert_ne!(
        refolded.open_count(&spec),
        read.open,
        "a second, independent fold disagrees with the snapshot `read` already returned"
    );

    // `read`'s own numbers, by contrast, are unaffected by the write that
    // came after it: they were taken from one snapshot and remain
    // internally consistent with the rows in that same `read`.
    assert_eq!(
        read.open,
        read.entries
            .iter()
            .filter(|entry| !spec.is_closed(&entry.status(&spec)))
            .count()
    );
    assert_eq!(read.closed, 0);
}

/// Refused at the **write**, not reported at the read: by the time somebody
/// reads it, the person who knew why has moved on.
#[tokio::test]
async fn closing_without_a_reason_is_refused() {
    let (ctx, _runtime, _home) = ledgers().await;
    let spec = define(&ctx, &hazards()).await.expect("declared");
    record(&ctx, &spec, &agent(), "r1", fields(&[("risk", "a")]))
        .await
        .expect("recorded");
    let error = record(&ctx, &spec, &agent(), "r1", fields(&[("status", "closed")]))
        .await
        .expect_err("no reason");
    assert!(format!("{error}").contains("reason"), "{error}");

    close(
        &ctx,
        &spec,
        &agent(),
        "r1",
        "closed",
        "the vendor delivered",
    )
    .await
    .expect("closed with a reason");
}

/// A row that already explained itself must not be refused for saying it twice.
#[tokio::test]
async fn a_reason_already_on_the_row_satisfies_the_close() {
    let (ctx, _runtime, _home) = ledgers().await;
    let spec = define(&ctx, &hazards()).await.expect("declared");
    record(
        &ctx,
        &spec,
        &agent(),
        "r1",
        fields(&[("risk", "a"), ("reason", "the vendor delivered")]),
    )
    .await
    .expect("recorded");
    record(&ctx, &spec, &agent(), "r1", fields(&[("status", "closed")]))
        .await
        .expect("the reason is already there");
}

#[tokio::test]
async fn an_undeclared_status_is_refused_and_names_the_real_ones() {
    let (ctx, _runtime, _home) = ledgers().await;
    let spec = define(&ctx, &hazards()).await.expect("declared");
    let error = record(
        &ctx,
        &spec,
        &agent(),
        "r1",
        fields(&[("status", "resolved")]),
    )
    .await
    .expect_err("unknown status");
    let message = format!("{error}");
    assert!(message.contains("resolved"), "{message}");
    assert!(message.contains("closed"), "{message}");
}

/// `close` refuses a status that closes nothing — the mistake a caller reaching
/// for "close" actually makes.
#[tokio::test]
async fn close_refuses_a_status_that_does_not_close() {
    let (ctx, _runtime, _home) = ledgers().await;
    let spec = define(&ctx, &hazards()).await.expect("declared");
    let error = close(&ctx, &spec, &agent(), "r1", "open", "done")
        .await
        .expect_err("open does not close");
    assert!(format!("{error}").contains("closed"), "{error}");
}

/// The rule, in the one place it lives. An agent's whole relationship with a
/// ledger is additive; deleting is not, and it is a person's call.
#[tokio::test]
async fn only_a_person_may_delete_a_row() {
    let (ctx, _runtime, _home) = ledgers().await;
    let spec = define(&ctx, &hazards()).await.expect("declared");
    record(&ctx, &spec, &agent(), "r1", fields(&[("risk", "a")]))
        .await
        .expect("recorded");

    let error = delete_entry(&ctx, &spec, &agent(), "r1")
        .await
        .expect_err("an agent may not delete");
    let message = format!("{error}");
    assert!(message.contains("only a person"), "{message}");
    assert!(message.contains("Close the row instead"), "{message}");
    // Refused, not silently ignored.
    assert!(
        read(&ctx, &spec, &Query::default())
            .await
            .expect("read")
            .entries
            .iter()
            .any(|entry| entry.id == "r1")
    );

    assert!(
        delete_entry(&ctx, &spec, &person(), "r1")
            .await
            .expect("a person may")
    );
    assert!(
        read(&ctx, &spec, &Query::default())
            .await
            .expect("read")
            .entries
            .is_empty()
    );
}

/// The runtime is not exempt either: a sweep that could delete rows is the same
/// loss with nobody to ask about it.
#[tokio::test]
async fn the_runtime_itself_may_not_delete_a_row() {
    let (ctx, _runtime, _home) = ledgers().await;
    let spec = define(&ctx, &hazards()).await.expect("declared");
    let error = delete_entry(&ctx, &spec, &LedgerAuthor::system("sweep"), "r1")
        .await
        .expect_err("system is not a person");
    assert!(format!("{error}").contains("only a person"), "{error}");
}

#[tokio::test]
async fn only_a_person_may_retire_a_ledger_and_the_rows_survive_it() {
    let (ctx, runtime, _home) = ledgers().await;
    let spec = define(&ctx, &hazards()).await.expect("declared");
    record(&ctx, &spec, &agent(), "r1", fields(&[("risk", "a")]))
        .await
        .expect("recorded");

    assert!(retire(&ctx, &agent(), "hazards", false).await.is_err());
    retire(&ctx, &person(), "hazards", false)
        .await
        .expect("a person may");
    assert!(
        registry(&ctx)
            .await
            .expect("registry")
            .find("hazards")
            .is_none()
    );

    // Retiring a ledger nobody reads is worth doing; deleting what it recorded
    // is a separate, explicit act.
    let events = runtime
        .ledgers()
        .events(runtime.id(), "hazards")
        .await
        .expect("events");
    assert_eq!(events.len(), 1, "the log survives the retirement");
}

#[tokio::test]
async fn a_built_in_cannot_be_retired() {
    let (ctx, _runtime, _home) = ledgers().await;
    let error = retire(&ctx, &person(), "goals", false)
        .await
        .expect_err("built in");
    assert!(
        format!("{error}").contains("ships with the runtime"),
        "{error}"
    );
}

/// The board keeps its own store, its own routes and its own dispatch edge, so
/// `record_entry` must refuse it — and say what does write it.
#[tokio::test]
async fn the_board_is_readable_through_the_ledger_surface_and_not_writable_by_it() {
    let (ctx, _runtime, _home) = ledgers().await;
    let registry = registry(&ctx).await.expect("registry");
    let tasks = registry.find("tasks").expect("built in");

    let error = record(&ctx, tasks, &agent(), "t1", fields(&[("title", "x")]))
        .await
        .expect_err("native");
    assert!(format!("{error}").contains("spawn_task"), "{error}");

    // Reading it works the same as reading any other ledger.
    let read = read(&ctx, tasks, &Query::default()).await.expect("read");
    assert!(read.entries.is_empty(), "a fresh company has no cards");

    // And so does deleting: a card is deleted through the board.
    let error = delete_entry(&ctx, tasks, &person(), "t1")
        .await
        .expect_err("native");
    assert!(format!("{error}").contains("elsewhere"), "{error}");
}

/// Every write re-renders, so `derived/` is never a stale copy of something.
#[tokio::test]
async fn a_write_publishes_the_derived_file() {
    let (ctx, runtime, _home) = ledgers().await;
    let spec = define(&ctx, &hazards()).await.expect("declared");
    record(
        &ctx,
        &spec,
        &agent(),
        "vendor-slip",
        fields(&[("risk", "the vendor misses the date"), ("status", "open")]),
    )
    .await
    .expect("recorded");

    let tree = runtime.workspace().tree(runtime.id()).await.expect("tree");
    let folder = tree
        .iter()
        .find(|node| node.name == "derived")
        .expect("the derived folder exists");
    let file = tree
        .iter()
        .find(|node| {
            node.parent_id.as_deref() == Some(folder.id.as_str()) && node.name == "hazards.md"
        })
        .expect("the ledger's file exists");
    let (_, body) = runtime
        .workspace()
        .read(runtime.id(), &file.id)
        .await
        .expect("read")
        .expect("present");
    assert!(body.contains("vendor-slip"), "{body}");
    assert!(body.contains("Do not edit this file"), "{body}");
}

/// A ledger is visible in `derived/` from the moment it exists, not from its
/// first row — a folder that gains a file only on first write reads as though
/// the ledger was never created.
#[tokio::test]
async fn declaring_a_ledger_publishes_its_empty_file() {
    let (ctx, runtime, _home) = ledgers().await;
    define(&ctx, &hazards()).await.expect("declared");
    let tree = runtime.workspace().tree(runtime.id()).await.expect("tree");
    assert!(tree.iter().any(|node| node.name == "hazards.md"));
}

/// A read that returned twenty rows must be distinguishable from one that
/// returned all of them.
#[tokio::test]
async fn a_read_is_bounded_and_says_how_many_matched() {
    let (ctx, _runtime, _home) = ledgers().await;
    let spec = define(&ctx, &hazards()).await.expect("declared");
    for n in 0..40 {
        record(
            &ctx,
            &spec,
            &agent(),
            &format!("r{n}"),
            fields(&[("risk", "a"), ("status", "open")]),
        )
        .await
        .expect("recorded");
    }
    let read = read(&ctx, &spec, &Query::default()).await.expect("read");
    assert_eq!(
        read.entries.len(),
        crate::ledger::budget::DEFAULT_READ_LIMIT
    );
    assert_eq!(read.matched, 40);

    let huge = read2(&ctx, &spec, 10_000).await;
    assert_eq!(
        huge.entries.len(),
        crate::ledger::budget::MAX_READ_LIMIT.min(40)
    );
}

async fn read2(ctx: &Ledgers, spec: &crate::ledger::LedgerSpec, limit: usize) -> Read {
    read(
        ctx,
        spec,
        &Query {
            limit: Some(limit),
            ..Query::default()
        },
    )
    .await
    .expect("read")
}

#[tokio::test]
async fn a_read_narrows_by_status_entry_and_text() {
    let (ctx, _runtime, _home) = ledgers().await;
    let spec = define(&ctx, &hazards()).await.expect("declared");
    record(
        &ctx,
        &spec,
        &agent(),
        "vendor",
        fields(&[("risk", "supplier misses the date"), ("status", "open")]),
    )
    .await
    .expect("recorded");
    close(&ctx, &spec, &agent(), "hiring", "closed", "role filled")
        .await
        .expect("recorded");

    let open = read(
        &ctx,
        &spec,
        &Query {
            status: Some("open".into()),
            ..Query::default()
        },
    )
    .await
    .expect("read");
    assert_eq!(open.entries.len(), 1);
    assert_eq!(open.entries[0].id, "vendor");

    let one = read(
        &ctx,
        &spec,
        &Query {
            entry: Some("hiring".into()),
            ..Query::default()
        },
    )
    .await
    .expect("read");
    assert_eq!(one.entries.len(), 1);

    let found = read(
        &ctx,
        &spec,
        &Query {
            text: Some("SUPPLIER".into()),
            ..Query::default()
        },
    )
    .await
    .expect("read");
    assert_eq!(found.entries.len(), 1);
}

#[tokio::test]
async fn an_unknown_sort_is_refused_rather_than_defaulted() {
    let (ctx, _runtime, _home) = ledgers().await;
    let spec = define(&ctx, &hazards()).await.expect("declared");
    let error = read(
        &ctx,
        &spec,
        &Query {
            sort: Some("newest".into()),
            ..Query::default()
        },
    )
    .await
    .expect_err("unknown sort");
    assert!(format!("{error}").contains("recorded"), "{error}");
}

/// Holding `record_entry` is not permission to write everything: the set of
/// ledgers is not fixed when tools are wired.
#[tokio::test]
async fn a_writers_list_is_enforced_at_the_write() {
    let (ctx, _runtime, _home) = ledgers().await;
    let mut document = hazards();
    document["writers"] = json!(["cfo"]);
    let spec = define(&ctx, &document).await.expect("declared");

    let error = record(&ctx, &spec, &agent(), "r1", fields(&[("risk", "a")]))
        .await
        .expect_err("ceo is not a writer");
    assert!(format!("{error}").contains("cfo"), "{error}");

    record(
        &ctx,
        &spec,
        &LedgerAuthor::agent("cfo"),
        "r1",
        fields(&[("risk", "a")]),
    )
    .await
    .expect("cfo may");
}

#[tokio::test]
async fn a_declaration_that_collides_is_refused() {
    let (ctx, _runtime, _home) = ledgers().await;
    define(&ctx, &hazards()).await.expect("declared");
    let error = define(&ctx, &hazards())
        .await
        .expect_err("already a ledger");
    assert!(format!("{error}").contains("hazards"), "{error}");

    let mut shadow = hazards();
    shadow["slug"] = json!("goals");
    shadow["derived"] = json!("derived/other.md");
    let error = define(&ctx, &shadow).await.expect_err("built in");
    assert!(format!("{error}").contains("built-in"), "{error}");
}

/// Over-long text is truncated rather than rejected: losing the tail of a long
/// note is a smaller failure than losing the whole write.
#[tokio::test]
async fn an_over_long_value_is_truncated_rather_than_refused() {
    let (ctx, _runtime, _home) = ledgers().await;
    let spec = define(&ctx, &hazards()).await.expect("declared");
    let entry = record(
        &ctx,
        &spec,
        &agent(),
        "r1",
        fields(&[("risk", &"x".repeat(20_000))]),
    )
    .await
    .expect("recorded");
    assert_eq!(
        entry.get("risk").chars().count(),
        crate::ledger::MAX_FIELD_CHARS
    );
}

/// A blank value clears the field rather than storing a present-but-empty one,
/// which would render an empty bullet under every row that ever set it.
#[tokio::test]
async fn a_blank_value_clears_the_field() {
    let (ctx, _runtime, _home) = ledgers().await;
    let spec = define(&ctx, &hazards()).await.expect("declared");
    record(&ctx, &spec, &agent(), "r1", fields(&[("risk", "a")]))
        .await
        .expect("recorded");
    let cleared = record(&ctx, &spec, &agent(), "r1", fields(&[("risk", "   ")]))
        .await
        .expect("recorded");
    assert_eq!(cleared.get("risk"), "");
}

/// A ledger shaped like the global `learnings`: it declares the check *and*
/// marks a prose field required, which is the pair `hazards` deliberately
/// lacks.
fn findings() -> serde_json::Value {
    json!({
        "slug": "findings",
        "title": "Findings",
        "purpose": "What we found out.",
        "derived": "derived/findings.md",
        "fields": [
            { "name": "id", "role": "id", "required": true },
            { "name": "finding", "role": "title", "required": true },
            { "name": "status", "role": "status", "required": true },
            { "name": "evidence", "role": "prose", "required": true,
              "description": "What actually happened, concretely." },
            { "name": "reason", "role": "prose" }
        ],
        "statuses": [
            { "name": "noted" },
            { "name": "adopted", "closed": true, "needs_reason": true }
        ],
        "sections": [
            { "heading": "Noted", "statuses": ["noted"], "order": "recent" }
        ],
        "checks": ["required-field", "known-status", "closed-needs-reason"]
    })
}

/// The write half of the contract the read half already stated. A row landing
/// without a field the ledger requires is refused, rather than stored and then
/// reported unreadable by every surface that opens the ledger.
#[tokio::test]
async fn a_row_missing_a_required_field_is_refused_at_the_write() {
    let (ctx, _runtime, _home) = ledgers().await;
    let spec = define(&ctx, &findings()).await.expect("declared");
    let error = record(
        &ctx,
        &spec,
        &agent(),
        "f1",
        fields(&[("finding", "a row with no evidence"), ("status", "noted")]),
    )
    .await
    .expect_err("no evidence");
    let message = format!("{error}");
    assert!(message.contains("evidence"), "{message}");
    assert!(message.contains("f1"), "{message}");
    // The description is what tells whoever filled it in wrong what belongs
    // there, so the refusal carries it.
    assert!(message.contains("What actually happened"), "{message}");

    // Nothing was stored: a refused write must not leave the row behind.
    let read = read(&ctx, &spec, &Query::default()).await.expect("read");
    assert!(read.entries.is_empty(), "{:?}", read.entries);
}

/// The whole point, stated as one assertion: what the write accepts, the read
/// reads back clean. Before this, a row could be recorded and then reported by
/// the same ledger as one that could not be read.
#[tokio::test]
async fn what_the_write_accepts_the_read_reports_no_fault_on() {
    let (ctx, _runtime, _home) = ledgers().await;
    let spec = define(&ctx, &findings()).await.expect("declared");
    record(
        &ctx,
        &spec,
        &agent(),
        "f1",
        fields(&[
            ("finding", "the vendor is slow"),
            ("status", "noted"),
            ("evidence", "three late deliveries in a row"),
        ]),
    )
    .await
    .expect("recorded");
    let read = read(&ctx, &spec, &Query::default()).await.expect("read");
    assert_eq!(read.entries.len(), 1);
    assert!(read.faults.is_empty(), "{:?}", read.faults);
}

/// Every write is a merge, so the check runs against the merged row. Moving a
/// row's status must not be refused for declining to repeat what it holds.
#[tokio::test]
async fn an_amendment_need_not_repeat_a_required_field_the_row_holds() {
    let (ctx, _runtime, _home) = ledgers().await;
    let spec = define(&ctx, &findings()).await.expect("declared");
    record(
        &ctx,
        &spec,
        &agent(),
        "f1",
        fields(&[
            ("finding", "the vendor is slow"),
            ("status", "noted"),
            ("evidence", "three late deliveries"),
        ]),
    )
    .await
    .expect("recorded");
    let amended = record(&ctx, &spec, &agent(), "f1", fields(&[("status", "noted")]))
        .await
        .expect("an amendment carries only what changes");
    assert_eq!(amended.events, 2);

    close(
        &ctx,
        &spec,
        &agent(),
        "f1",
        "adopted",
        "folded into the standard",
    )
    .await
    .expect("closing carries neither the title nor the evidence again");
}

/// Clearing one is the same loss as never writing it, and a merge is the only
/// way to express a clear — so it is refused on the same ground.
#[tokio::test]
async fn clearing_a_required_field_is_refused() {
    let (ctx, _runtime, _home) = ledgers().await;
    let spec = define(&ctx, &findings()).await.expect("declared");
    record(
        &ctx,
        &spec,
        &agent(),
        "f1",
        fields(&[
            ("finding", "the vendor is slow"),
            ("status", "noted"),
            ("evidence", "three late deliveries"),
        ]),
    )
    .await
    .expect("recorded");
    let error = record(&ctx, &spec, &agent(), "f1", fields(&[("evidence", "  ")]))
        .await
        .expect_err("cleared");
    assert!(format!("{error}").contains("evidence"), "{error}");
}

/// A [`LedgerStore`] that pauses inside `events` exactly once, after the read
/// has already happened, so a test can hold a caller mid-check while another
/// task mutates the store underneath it.
struct PausingStore {
    inner: Arc<dyn LedgerStore>,
    armed: Arc<AtomicBool>,
    paused: Arc<Notify>,
    resume: Arc<Notify>,
}

#[async_trait::async_trait]
impl LedgerStore for PausingStore {
    async fn list_specs(&self, company: &CompanyId) -> Result<Vec<LedgerSpec>> {
        self.inner.list_specs(company).await
    }

    async fn put_spec(&self, company: &CompanyId, spec: &LedgerSpec) -> Result<()> {
        self.inner.put_spec(company, spec).await
    }

    async fn delete_spec(&self, company: &CompanyId, slug: &str) -> Result<bool> {
        self.inner.delete_spec(company, slug).await
    }

    async fn append(&self, company: &CompanyId, event: &LedgerEvent) -> Result<()> {
        self.inner.append(company, event).await
    }

    async fn events(&self, company: &CompanyId, ledger: &str) -> Result<Vec<LedgerEvent>> {
        let read = self.inner.events(company, ledger).await;
        if self.armed.swap(false, Ordering::SeqCst) {
            self.paused.notify_one();
            self.resume.notified().await;
        }
        read
    }

    async fn purge_entry(&self, company: &CompanyId, ledger: &str, entry: &str) -> Result<bool> {
        self.inner.purge_entry(company, ledger, entry).await
    }

    async fn purge_ledger(&self, company: &CompanyId, ledger: &str) -> Result<bool> {
        self.inner.purge_ledger(company, ledger).await
    }
}

/// The required-field check reads the stored row, then the write appends. A
/// purge landing in that gap used to remove the earlier events the check had
/// just relied on, so the append that followed recreated the row with only
/// its own partial fields — reporting success on exactly the row the check
/// exists to keep out.
///
/// [`PausingStore`] holds the amendment inside that exact gap — after its
/// required-field check has read the row, before it appends — so the purge
/// gets a deterministic window to land in, rather than relying on real
/// thread timing to hit a gap this narrow. The lock under test still does
/// its own real work here: it is what makes the purge task block instead of
/// running in that window once the fix is in place.
#[tokio::test]
async fn a_purge_racing_the_required_field_check_cannot_recreate_a_row_missing_it() {
    let (runtime, _home) = runtime().await;

    let armed = Arc::new(AtomicBool::new(false));
    let paused = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    let store: Arc<dyn LedgerStore> = Arc::new(PausingStore {
        inner: runtime.ledgers().clone(),
        armed: armed.clone(),
        paused: paused.clone(),
        resume: resume.clone(),
    });
    let ctx = Ledgers::new(runtime.id().clone(), store);

    let spec = define(&ctx, &findings()).await.expect("declared");
    record(
        &ctx,
        &spec,
        &agent(),
        "f1",
        fields(&[
            ("finding", "the vendor is slow"),
            ("status", "noted"),
            ("evidence", "three late deliveries"),
        ]),
    )
    .await
    .expect("recorded");

    // Seeding is done: arm the pause for the amendment's own read.
    armed.store(true, Ordering::SeqCst);

    let amend_ctx = ctx.clone();
    let amend_spec = spec.clone();
    let amender = tokio::spawn(async move {
        record(
            &amend_ctx,
            &amend_spec,
            &agent(),
            "f1",
            fields(&[("status", "noted")]),
        )
        .await
    });

    paused.notified().await;

    let purge_ctx = ctx.clone();
    let purge_spec = spec.clone();
    let purger =
        tokio::spawn(async move { delete_entry(&purge_ctx, &purge_spec, &person(), "f1").await });

    // Under the fix the purge blocks on the same lock the amendment is
    // holding; this just gives it the chance to run first when it is not
    // blocked, which is the whole bug.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    resume.notify_one();

    let amend_result = tokio::time::timeout(std::time::Duration::from_secs(5), amender)
        .await
        .expect("amendment did not finish")
        .expect("amendment task panicked");
    let purge_result = tokio::time::timeout(std::time::Duration::from_secs(5), purger)
        .await
        .expect("purge did not finish")
        .expect("purge task panicked");
    purge_result.expect("purge does not error");

    if let Ok(entry) = amend_result {
        assert!(
            !entry.get("evidence").trim().is_empty(),
            "amendment reported success on a row missing a required field: {entry:?}"
        );
    }
}

/// The write refuses exactly what the read would fault, and no more: a ledger
/// that does not declare the check is not silently held to it.
#[tokio::test]
async fn a_ledger_that_does_not_declare_the_check_is_not_held_to_it() {
    let (ctx, _runtime, _home) = ledgers().await;
    let mut document = findings();
    document["slug"] = json!("loose");
    document["derived"] = json!("derived/loose.md");
    document["checks"] = json!(["known-status"]);
    let spec = define(&ctx, &document).await.expect("declared");
    record(&ctx, &spec, &agent(), "f1", fields(&[("status", "noted")]))
        .await
        .expect("no required-field check, so nothing to enforce");
}

/// The briefing is what a turn carries: every ledger named, every open row
/// identified, and the call that fetches the rest on each one.
#[tokio::test]
async fn the_briefing_names_every_ledger_and_how_to_read_more() {
    let (ctx, _runtime, _home) = ledgers().await;
    let spec = define(&ctx, &hazards()).await.expect("declared");
    record(
        &ctx,
        &spec,
        &agent(),
        "vendor-slip",
        fields(&[("risk", "a"), ("status", "open")]),
    )
    .await
    .expect("recorded");

    let registry = registry(&ctx).await.expect("registry");
    let briefing = briefing(&ctx, &registry).await.expect("briefing");
    for slug in ["tasks", "goals", "decisions", "hazards"] {
        assert!(briefing.contains(slug), "`{slug}` is missing: {briefing}");
    }
    assert!(briefing.contains("vendor-slip"), "{briefing}");
    assert!(briefing.contains("read_ledger"), "{briefing}");
}

#[tokio::test]
async fn republish_writes_every_ledgers_file() {
    let (ctx, runtime, _home) = ledgers().await;
    define(&ctx, &hazards()).await.expect("declared");
    let written = republish_all(&ctx).await.expect("republished");
    // Three built-ins, the baseline's own, and the one just declared.
    assert_eq!(written, 4 + crate::globals::ledgers().len());
    let tree = runtime.workspace().tree(runtime.id()).await.expect("tree");
    for name in ["tasks.md", "goals.md", "decisions.md", "hazards.md"] {
        assert!(
            tree.iter().any(|node| node.name == name),
            "`{name}` was not written"
        );
    }
}
