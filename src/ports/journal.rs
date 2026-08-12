//! The [`JournalStore`] port: the durable sink under the runtime journal
//! (issue #726).
//!
//! [`RuntimeJournal`](crate::runtime::journal::RuntimeJournal) holds the
//! at-most-once effect set, the parked-approval queue, single-use and standing
//! grants, and the cycle brackets. Until this port existed it wrote all of that
//! to a `journal.jsonl` inside the company bundle **unconditionally** — outside
//! the port surface [`StorageHandles`](crate::store::StorageHandles) swaps for
//! every other durable store. On a hosted tenant with
//! `OPENCOMPANY_STORAGE=mongodb` the container's `/data` is documented ephemeral
//! scratch (see `docs/spec/runtime/storage.md`), so container replacement — a
//! deploy, a reschedule, a node drain, an OOM kill — discarded the file: every
//! previously executed effect became eligible to fire again, and every parked
//! approval and grant silently vanished.
//!
//! ## Two byte-level operations, and no semantics
//!
//! This trait is deliberately *not* a mirror of `RuntimeJournal`'s ~25 methods.
//! The journal's entire persistence contract is: append one opaque line, and
//! read every line back in the order they were appended. Everything semantic —
//! the record enum with its `#[serde(default)]` archaeology, the corrupt-line
//! skip, the merged-line recovery, the in-memory replay state — stays in
//! `RuntimeJournal` and is therefore backend-agnostic by construction. A
//! backend stores strings; it never learns what a `JournalRecord` is, and a new
//! record variant needs no backend change.
//!
//! ## Why not ride [`EventLog`](crate::ports::EventLog)
//!
//! [`CompanyEvent`](crate::ports::CompanyEvent) is a closed, binding enum with
//! no marker variants, which is the reason the journal exists as a separate log
//! in the first place. And the event log is *pruned* under a retention policy —
//! rotating away an `EffectExecuted` key would silently un-commit it and let an
//! at-most-once effect fire a second time. The journal is append-only with no
//! retention, and those two contracts cannot share one log.
//!
//! ## Migration is a one-time, receipt-gated, verbatim import
//!
//! The fs implementation *is* today's file at today's path, so the default
//! backend migrates nothing. For sqlite and mongodb the builder copies an
//! existing `journal.jsonl` in **file order, verbatim** (raw strings — a corrupt
//! or merged line migrates byte-for-byte, so `RuntimeJournal`'s own recovery
//! still applies to it), then writes a receipt.
//!
//! The receipt is what makes a crash mid-import safe:
//! [`complete_import`](JournalStore::complete_import) clears whatever a previous
//! attempt wrote before re-copying, and only then records the receipt — so an
//! interrupted import re-runs the whole wipe-and-copy instead of replaying a
//! truncated prefix. A truncated prefix is precisely the bug this port exists to
//! fix: it would drop at-most-once keys.
//!
//! The source file is left in place. The receipt makes a second import
//! impossible, and a rollback to an older binary still finds the history it
//! knows how to read — where renaming the file would hand that binary an
//! *empty* at-most-once set.

use async_trait::async_trait;

use crate::Result;
use crate::ports::types::CompanyId;

/// The durable byte sink under one company's runtime journal.
#[async_trait]
pub trait JournalStore: Send + Sync {
    /// Appends one opaque record line, which MUST be durable before this
    /// returns.
    ///
    /// The at-most-once guarantee is that an effect's key reaches durable
    /// storage *before* the side effect runs, so a backend that acknowledges a
    /// buffered or unjournaled write breaks the contract this port carries. The
    /// fs backend performs a single `O_APPEND` `write_all` and waits on the
    /// syscall; the mongodb backend inserts with `j:true` write concern.
    ///
    /// `line` never contains a newline — the caller serialises one JSON record
    /// per call — and a backend must preserve it byte-for-byte.
    ///
    /// Errors are fail-closed: an `Err` reaches the caller *before* the side
    /// effect, so the effect does not run. The residual ambiguity (a timeout on
    /// a write the server did commit) leaves a committed key with no effect,
    /// which is the at-most-once contract's documented safe direction.
    async fn append_journal(&self, id: &CompanyId, line: &str) -> Result<()>;

    /// Every line ever appended for `id`, in append order.
    ///
    /// Order is load-bearing rather than cosmetic: replay folds records in
    /// sequence, so a park read back after the resolution that drains it would
    /// resurrect a resolved approval.
    ///
    /// Damaged lines are returned as they are stored, not filtered — deciding
    /// what a line means is the journal's job, and a backend that silently
    /// dropped one would be un-committing an effect key on the caller's behalf.
    async fn read_journal(&self, id: &CompanyId) -> Result<Vec<String>>;

    /// Whether this backend has already taken (or does not need) a one-time
    /// import of a pre-existing filesystem journal.
    ///
    /// `true` closes the gate forever: the builder never imports again, so a
    /// `journal.jsonl` reappearing later — a rollback, a stray copy into the
    /// data dir — cannot wipe and replace the backend's own history.
    ///
    /// The fs backend answers `true` unconditionally: its store *is* the file,
    /// so there is nothing to import and [`complete_import`](Self::complete_import)
    /// is unreachable for it.
    async fn journal_imported(&self, id: &CompanyId) -> Result<bool>;

    /// Replaces this company's journal with `lines` and records the import
    /// receipt.
    ///
    /// Clear-then-copy-then-receipt, in that order, and the order is the whole
    /// safety argument — see the module docs. Implementations do it atomically
    /// where the backend allows (sqlite: one transaction); where it does not
    /// (mongodb), the receipt landing last still makes an interrupted attempt
    /// re-run from the top rather than leave a truncated journal behind a
    /// closed gate.
    ///
    /// An empty `lines` is a legitimate call, not a no-op to optimise away: it
    /// is how a company with no prior filesystem journal closes its gate.
    async fn complete_import(&self, id: &CompanyId, lines: Vec<String>) -> Result<()>;
}
