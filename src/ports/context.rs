//! The [`ContextStore`] port: addressable chunks the brain queries lazily.

use std::ops::Range;

use async_trait::async_trait;

use crate::Result;
use crate::ports::types::{ChunkAddr, ChunkHit, ChunkMeta, CompanyId, ContextChunk};

/// The RLM environment: addressable context chunks. Mirrors Medulla's
/// `ContextStore` port.
#[async_trait]
pub trait ContextStore: Send + Sync {
    /// Stores a chunk, returning its content address.
    async fn put(&self, id: &CompanyId, chunk: ContextChunk) -> Result<ChunkAddr>;
    /// Lists chunk metadata under `prefix`.
    async fn list(&self, id: &CompanyId, prefix: &str) -> Result<Vec<ChunkMeta>>;
    /// Reads a chunk (optionally a byte range) as text.
    async fn peek(
        &self,
        id: &CompanyId,
        addr: &ChunkAddr,
        range: Option<Range<usize>>,
    ) -> Result<String>;
    /// Reads many chunk bodies at once: one entry per requested addr, in
    /// request order, `None` where nothing is stored (or a single read fails).
    ///
    /// `Err` is reserved for batch-level failures (connection, statement,
    /// enumeration); a chunk that is missing, or whose single read or decode
    /// fails, answers `None` so one bad row cannot fail a bulk read the
    /// caller degrades per-row anyway. The default loops [`Self::peek`] —
    /// exactly the call-site loop it replaces — and backends override it to
    /// shed the per-chunk overhead that loop pays. How much they shed is
    /// backend-specific, and only mongodb collapses to literally one read:
    /// it answers the batch with a single `$in` query, sqlite acquires one
    /// connection and prepares one statement it then executes per addr, and
    /// the provider facade reads its one enumerated partition once.
    async fn peek_many(&self, id: &CompanyId, addrs: &[ChunkAddr]) -> Result<Vec<Option<String>>> {
        let mut bodies = Vec::with_capacity(addrs.len());
        for addr in addrs {
            bodies.push(self.peek(id, addr, None).await.ok());
        }
        Ok(bodies)
    }
    /// Searches chunks for `query`, returning up to `limit` hits.
    async fn search(&self, id: &CompanyId, query: &str, limit: usize) -> Result<Vec<ChunkHit>>;
    /// Permanently removes the chunk at `addr`, returning whether it existed.
    ///
    /// Address-level: chunks are content-addressed, so on backends where one
    /// address can carry several index entries (fs), the whole address goes —
    /// every label that pointed at that body. `false` means nothing was there,
    /// which callers surface honestly rather than treating as an error: a
    /// forget of something already gone is a no-op, not a fault.
    ///
    /// Required, no default — a defaulted `Ok(false)` would let a backend
    /// silently serve a forget that forgets nothing, the exact dishonesty the
    /// null-engine warnings exist to prevent.
    async fn delete(&self, id: &CompanyId, addr: &ChunkAddr) -> Result<bool>;
}
