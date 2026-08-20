//! Engine-to-engine migration over the contract's Portability family.
//!
//! `export_page` → `import_records`, page by page, between two providers this
//! host constructed through [`open_driver`](super::driver::open_driver). The
//! records cross in the exporting driver's own vocabulary — kind, namespace
//! and taint round-trip untouched (`ExportRecord`'s own contract), so every
//! company's namespaces move together and provenance is never re-stamped.
//!
//! This is the data half of the engine-switch runbook in
//! `docs/spec/runtime/memory-engine.md`: migrate, then flip the
//! `OPENCOMPANY_MEMORY*` variables and restart.
//!
//! # Failure is a stop, never a guess
//!
//! The contract's error type is still one coarse variant (`MemoryError::Other`
//! — tinymemory#18 §A4), so mid-migration this code cannot tell a transient
//! 500 from a real rejection. It therefore never retries (an import retried
//! into a driver that half-applied the page could double-write) and instead
//! stops at the first failed page, reporting the cursor that *started* that
//! page. `--resume-cursor` re-enters there: `import_records` reports
//! already-present records as `skipped`, which is what makes re-running the
//! failed page safe.

use std::sync::Arc;

use tinymemory_api::provider::MemoryProvider;

use crate::Result;
use crate::error::OpenCompanyError;

/// What a finished (or stopped) migration did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MigrateSummary {
    /// Pages pulled from the source.
    pub pages: u32,
    /// Records the source exported.
    pub exported: u64,
    /// Records the target wrote.
    pub imported: u64,
    /// Records the target recognised as already present (a resumed run).
    pub skipped: u64,
}

/// A migration that stopped partway: what happened, and where to resume.
#[derive(Debug)]
pub struct MigrateStopped {
    /// What had already moved before the stop.
    pub summary: MigrateSummary,
    /// The cursor that started the failed page — pass to `--resume-cursor`.
    /// `None` means the very first page failed.
    pub resume_cursor: Option<String>,
    /// The target driver's own reasons, bounded by it.
    pub errors: Vec<String>,
}

/// The outcome: complete, or stopped with a resume point.
pub type MigrateOutcome = std::result::Result<MigrateSummary, Box<MigrateStopped>>;

/// Copies every record `from` exports into `to`, `page_size` records at a time,
/// starting at `resume` (a cursor a previous stopped run printed, or `None`
/// for the beginning). `on_page` observes progress after each imported page.
///
/// # Errors
///
/// A source `export_page` failure surfaces as a crate error (nothing was
/// half-applied). A target `import_records` failure — or a page the target
/// reports `failed > 0` for — returns [`MigrateStopped`] with the resume
/// cursor, because the *next* attempt must re-enter at the failed page, not at
/// the beginning.
pub async fn migrate(
    from: &Arc<dyn MemoryProvider>,
    to: &Arc<dyn MemoryProvider>,
    page_size: usize,
    resume: Option<String>,
    mut on_page: impl FnMut(&MigrateSummary),
) -> Result<MigrateOutcome> {
    let mut summary = MigrateSummary::default();
    let mut cursor = resume;
    loop {
        // The cursor that names THIS page, kept for the stop report.
        let page_start = cursor.clone();
        let page = from
            .export_page(page_start.as_deref(), page_size)
            .await
            .map_err(|e| {
                OpenCompanyError::Store(format!(
                    "source engine `{}` failed to export a page: {e}",
                    from.driver_id()
                ))
            })?;
        summary.pages += 1;
        summary.exported += page.records.len() as u64;

        if !page.records.is_empty() {
            let outcome = match to.import_records(page.records).await {
                Ok(outcome) => outcome,
                Err(e) => {
                    return Ok(Err(Box::new(MigrateStopped {
                        summary,
                        resume_cursor: page_start,
                        errors: vec![format!(
                            "target engine `{}` failed to import: {e}",
                            to.driver_id()
                        )],
                    })));
                }
            };
            summary.imported += u64::from(outcome.imported);
            summary.skipped += u64::from(outcome.skipped);
            if outcome.failed > 0 {
                return Ok(Err(Box::new(MigrateStopped {
                    summary,
                    resume_cursor: page_start,
                    errors: outcome.errors,
                })));
            }
        }
        on_page(&summary);

        // `next_cursor: None` is the terminator — NOT an empty page, which is
        // legal mid-export (the contract says so explicitly).
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    Ok(Ok(summary))
}

#[cfg(test)]
mod test {
    use super::*;
    use tinymemory_api::types::{MemoryCategory, MemoryTaint};
    use tinymemory_conformance::InMemoryProvider;

    fn provider() -> Arc<dyn MemoryProvider> {
        Arc::new(InMemoryProvider::default())
    }

    async fn seed(p: &Arc<dyn MemoryProvider>, n: usize) {
        for i in 0..n {
            p.store(
                &format!("oc/team-{}", i % 3),
                &format!("key-{i}"),
                &format!("record {i}"),
                MemoryCategory::Core,
                None,
                MemoryTaint::Internal,
            )
            .await
            .expect("seed");
        }
    }

    async fn count(p: &Arc<dyn MemoryProvider>) -> u64 {
        let mut total = 0;
        let mut cursor: Option<String> = None;
        loop {
            let page = p.export_page(cursor.as_deref(), 10).await.expect("page");
            total += page.records.len() as u64;
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => return total,
            }
        }
    }

    /// The whole point: every record crosses, across page boundaries, and the
    /// target's own export agrees with the source's count.
    #[tokio::test]
    async fn every_record_crosses_between_engines() {
        let (from, to) = (provider(), provider());
        seed(&from, 23).await;
        let outcome = migrate(&from, &to, 7, None, |_| {})
            .await
            .expect("no source failure")
            .expect("no stop");
        assert_eq!(outcome.exported, 23);
        assert_eq!(outcome.imported, 23);
        assert_eq!(outcome.skipped, 0);
        assert!(outcome.pages >= 4, "23 records at page size 7 is 4 pages");
        assert_eq!(
            count(&to).await,
            23,
            "the target must hold what the source held"
        );
    }

    /// A resumed run re-imports the failed page; the target reports the
    /// already-present half as skipped rather than double-writing it.
    #[tokio::test]
    async fn a_rerun_skips_what_already_crossed() {
        let (from, to) = (provider(), provider());
        seed(&from, 5).await;
        let first = migrate(&from, &to, 2, None, |_| {})
            .await
            .expect("ok")
            .expect("complete");
        assert_eq!(first.imported, 5);
        let second = migrate(&from, &to, 2, None, |_| {})
            .await
            .expect("ok")
            .expect("complete");
        assert_eq!(second.imported, 0, "nothing new to write");
        assert_eq!(second.skipped, 5, "everything recognised as present");
        assert_eq!(count(&to).await, 5, "no duplicates");
    }

    /// An empty source completes with zero of everything rather than erroring —
    /// migrating before first use is legal, if pointless.
    #[tokio::test]
    async fn an_empty_source_completes_empty() {
        let (from, to) = (provider(), provider());
        let outcome = migrate(&from, &to, 10, None, |_| {})
            .await
            .expect("ok")
            .expect("complete");
        assert_eq!(outcome.exported, 0);
        assert_eq!(count(&to).await, 0);
    }
}
