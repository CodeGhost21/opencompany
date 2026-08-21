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
