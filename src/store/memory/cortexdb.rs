//! CortexDB HTTP adapter — a standalone memory service reached over the
//! `remote` driver seam (issue: CortexDB as a memory engine).
//!
//! CortexDB is not one of the three engines `tinymemory-remote` ships a
//! dialect for, and its adapters (`Dialect`, `RemoteMemory`, `HttpClient`,
//! `StoredEntry`, …) are `pub(crate)` to that crate — unreachable from here.
//! So this module implements [`Memory`] directly over `reqwest`, the same
//! contract the vendored adapters implement, rather than extending a type this
//! crate cannot see.
//!
//! # Wire shape
//!
//! CortexDB has no notion of a `(namespace, key)` record. It is an
//! event-sourced store: writes are `POST /v1/experience` envelopes filed under
//! a hierarchical `scope` string, and reads are `POST /v1/recall`, which runs
//! retrieval (BM25 + vector + optional graph) and returns hits grouped by
//! layer (`events`, `episodes`, `facts`, `beliefs`, `understanding`).
//!
//! The mapping this driver uses:
//!
//! - **Scope.** A tinymemory `namespace` string already carries this host's
//!   own tenant isolation (see `super::namespace` — every namespace is rooted
//!   at a per-company hash). This driver hashes that namespace again into a
//!   CortexDB scope segment (`org:opencompany/ns:<hex>`), so two companies —
//!   or two namespaces within one company — never share a CortexDB scope and
//!   therefore never share recall.
//! - **Identity.** `(namespace, key)` and the tinymemory bookkeeping
//!   (`category`, `session_id`, `taint`) travel as a JSON envelope
//!   (`Content::Json`), not as prose, so nothing here asks CortexDB's
//!   extraction pipeline to preserve them faithfully. `view=raw` recall reads
//!   events verbatim, which is what this driver always asks for.
//! - **Idempotency.** The write key is a hash of `(namespace, key, content)`.
//!   Storing the same content under the same key twice is therefore a no-op
//!   replay (matching [`Memory::store`]'s own idempotence), while a changed
//!   `content` mints a new event. CortexDB is bi-temporal and keeps history by
//!   design; this driver's read side (`get`/`list`/`recall`) treats the
//!   *most recently observed* event for a key as the one upsert semantics
//!   promise, which is the honest way to present an event log through a
//!   key-value contract.
//! - **Forget.** `POST /v1/forget` retracts derived layers reliably; whether
//!   it can also retract raw events is a CortexDB-side policy this driver does
//!   not control. `forget` asks for every layer and reports what the server
//!   says it removed — see [`CortexdbMemory::forget`] for the caveat.
//!
//! # Auth
//!
//! Every request carries `Authorization: Bearer <token>` **and**
//! `X-Cortex-Actor: <actor>`. CortexDB treats a mismatch between the token's
//! subject and the actor header as a hard `401`, so both travel on every call,
//! always.

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tinymemory_api::traits::Memory;
use tinymemory_api::types::{MemoryCategory, MemoryEntry, MemoryTaint, NamespaceSummary};

/// Stable driver id used by configuration, status output, and the console
/// catalog.
pub const CORTEXDB_DRIVER_ID: &str = "cortexdb";

/// Default local port a self-hosted CortexDB container binds
/// (`cortexdb/cortexdb:latest`).
pub const CORTEXDB_DEFAULT_ENDPOINT: &str = "http://127.0.0.1:3141";

/// The header CortexDB checks against the bearer token's subject.
const ACTOR_HEADER: &str = "X-Cortex-Actor";

/// Root every scope this driver writes sits under, so a CortexDB instance
/// shared with another product's data cannot collide with this host's scopes.
const SCOPE_ROOT: &str = "org:opencompany";

const EXPERIENCE_PATH: &str = "/v1/experience";
const RECALL_PATH: &str = "/v1/recall";
const FORGET_PATH: &str = "/v1/forget";
const READY_PATH: &str = "/v1/admin/ready";
const EVENTS_PATH: &str = "/v1/events";
const SCOPES_LIST_PATH: &str = "/v1/scopes/list";

/// How long a single request may take before it is abandoned.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// How many hits `recall` asks CortexDB for per layer through `/v1/recall`.
///
/// CortexDB refuses a bare `limit` on `/v1/recall` ("ranked retrieval is not
/// paginated"); the ceiling travels as `budgets.per_layer_limits` instead, and
/// only the `events` layer is asked for since this driver only ever ingests
/// under [`Modality::ToolResult`](Modality) via `Content::Json`, which never
/// lands in `episodes`/`facts`/`beliefs`/`understanding`. Because ranked
/// recall cannot page, any read that must be *exhaustive* (`get`, `list`,
/// `forget`, and `recall`'s own stale-hit correction) does not use this path
/// at all — it walks `GET /v1/events` instead, which does page; see
/// [`CortexdbMemory::scope_events`].
const EVENTS_LAYER_LIMIT: u32 = 500;

/// How many events one `GET /v1/events` listing page asks for.
const EVENTS_PAGE_SIZE: u32 = 200;

/// Longest walk [`CortexdbMemory::scope_events`] will take before refusing to
/// answer from a possibly-truncated log, rather than silently reporting a
/// partial scope as complete.
const MAX_EVENT_PAGES: usize = 100;

/// How many scopes one `GET /v1/scopes/list` listing page asks for.
const SCOPES_PAGE_SIZE: u32 = 200;

/// Longest walk [`CortexdbMemory::list_scopes`] will take before refusing to
/// answer from a possibly-truncated listing.
const MAX_SCOPE_PAGES: usize = 100;

/// Largest response body this driver reads before giving up on decoding it.
const MAX_ERROR_BODY_CHARS: usize = 512;

/// A CortexDB service (self-hosted or managed) exposed through TinyMemory's
/// [`Memory`] contract.
///
/// `Debug` is hand-written: it must never render the bearer token.
pub struct CortexdbMemory {
    http: reqwest::Client,
    base_url: String,
    token: String,
    actor: String,
}

impl fmt::Debug for CortexdbMemory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CortexdbMemory")
            .field("base_url", &self.base_url)
            .field("actor", &self.actor)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl CortexdbMemory {
    /// Connects to a CortexDB instance with a bearer token and the actor that
    /// token was issued to.
    ///
    /// # Errors
    ///
    /// Returns an error when `endpoint` is not an absolute `http(s)` URL, or
    /// `api_key` is blank.
    pub fn new(endpoint: &str, api_key: &str, actor: impl Into<String>) -> anyhow::Result<Self> {
        anyhow::ensure!(
            endpoint.starts_with("http://") || endpoint.starts_with("https://"),
            "cortexdb endpoint {endpoint:?} must be an absolute http(s) url"
        );
        anyhow::ensure!(
            !api_key.trim().is_empty(),
            "cortexdb API key must not be empty"
        );
        let http = reqwest::Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()?;
        Ok(Self {
            http,
            base_url: endpoint.trim_end_matches('/').to_string(),
            token: api_key.to_string(),
            actor: actor.into(),
        })
    }

    /// Connect using the [`CORTEXDB_DRIVER_ID`] name for `api()`-style symmetry
    /// with the other remote adapters.
    ///
    /// # Errors
    ///
    /// As [`CortexdbMemory::new`].
    pub fn api(endpoint: &str, api_key: &str, actor: impl Into<String>) -> anyhow::Result<Self> {
        Self::new(endpoint, api_key, actor)
    }

    fn request(&self, method: Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}{path}", self.base_url))
            .bearer_auth(&self.token)
            .header(ACTOR_HEADER, &self.actor)
    }

    /// Maps a tinymemory namespace onto a CortexDB scope string.
    ///
    /// The namespace already carries this host's own tenant isolation (see
    /// `super::namespace`), so hashing it again is not what keeps two
    /// companies apart — it is what keeps two companies' data in two distinct
    /// CortexDB scopes, so a CortexDB-side bug or a broad `view` cannot read
    /// across a scope boundary the namespace already promised.
    fn scope_for(namespace: &str) -> String {
        format!("{SCOPE_ROOT}/ns:{}", digest_hex(namespace.as_bytes()))
    }

    /// Deterministic idempotency key: replaying the same content under the
    /// same key is a no-op; changed content mints a new event.
    fn idempotency_key(namespace: &str, key: &str, content: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(namespace.as_bytes());
        hasher.update([0u8]);
        hasher.update(key.as_bytes());
        hasher.update([0u8]);
        hasher.update(content.as_bytes());
        hex::encode(hasher.finalize())
    }

    async fn ingest(&self, namespace: &str, key: &str, envelope: &Value) -> anyhow::Result<()> {
        let body = json!({
            "scope": Self::scope_for(namespace),
            "modality": "tool_result",
            "content": { "kind": "json", "data": envelope },
            "context": { "observed_at": now_rfc3339() },
            "idempotency_key": Self::idempotency_key(
                namespace,
                key,
                envelope.get("content").and_then(Value::as_str).unwrap_or_default(),
            ),
        });
        let response = self
            .request(Method::POST, EXPERIENCE_PATH)
            .query(&[("wait", "captured")])
            .json(&body)
            .send()
            .await
            .map_err(|source| {
                anyhow::anyhow!("cortexdb request to {EXPERIENCE_PATH} failed: {source}")
            })?;
        Self::check_status(response, EXPERIENCE_PATH)
            .await
            .map(|_| ())
    }

    /// Recalls the raw events filed in `namespace`'s scope.
    ///
    /// `query` narrows retrieval; an empty query still returns the scope's raw
    /// events (CortexDB treats an empty query as "everything", ranked by
    /// recency) up to [`EVENTS_LAYER_LIMIT`].
    async fn recall_raw(&self, namespace: &str, query: &str) -> anyhow::Result<Vec<DecodedRecord>> {
        let body = json!({
            "scope": Self::scope_for(namespace),
            "query": query,
            "view": "raw",
            "budgets": { "per_layer_limits": { "events": EVENTS_LAYER_LIMIT } },
            "citation_mode": "structured_only",
        });
        let response = self
            .request(Method::POST, RECALL_PATH)
            .json(&body)
            .send()
            .await
            .map_err(|source| {
                anyhow::anyhow!("cortexdb request to {RECALL_PATH} failed: {source}")
            })?;
        let value = Self::check_status(response, RECALL_PATH).await?;
        Ok(decode_events(&value))
    }

    async fn check_status(response: reqwest::Response, path: &str) -> anyhow::Result<Value> {
        let status = response.status();
        let text = response.text().await.map_err(|source| {
            anyhow::anyhow!("reading cortexdb's response to {path} failed: {source}")
        })?;
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            anyhow::bail!(
                "cortexdb rejected the token or the X-Cortex-Actor header for {path} \
                 (status {status}) — the actor must match the token's subject"
            );
        }
        if !status.is_success() {
            let body: String = text.chars().take(MAX_ERROR_BODY_CHARS).collect();
            anyhow::bail!("cortexdb answered {status} for {path}: {body}");
        }
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).map_err(|source| {
            anyhow::anyhow!("cortexdb answered {path} with an unreadable body: {source}")
        })
    }

    /// Fetches every event filed under `scope`, following `next_cursor` to
    /// the end.
    ///
    /// `/v1/recall` cannot page a raw listing (see [`EVENTS_LAYER_LIMIT`]),
    /// so every exhaustive read this driver performs — `get`/`list`'s fold,
    /// `forget`'s every-version selector, `namespace_summaries`'s scope
    /// decode, and `recall`'s stale-hit correction — walks `GET /v1/events`
    /// instead, which does. A page short of `has_more: false` is refused
    /// rather than treated as the whole scope, because a short read here
    /// would report a superseded value as current.
    async fn scope_events(&self, scope: &str) -> anyhow::Result<Vec<DecodedRecord>> {
        let mut all = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_EVENT_PAGES {
            let mut query = vec![
                ("scope".to_string(), scope.to_string()),
                ("limit".to_string(), EVENTS_PAGE_SIZE.to_string()),
            ];
            if let Some(cursor) = &cursor {
                query.push(("cursor".to_string(), cursor.clone()));
            }
            let response = self
                .request(Method::GET, EVENTS_PATH)
                .query(&query)
                .send()
                .await
                .map_err(|source| {
                    anyhow::anyhow!("cortexdb request to {EVENTS_PATH} failed: {source}")
                })?;
            let page = Self::check_status(response, EVENTS_PATH).await?;
            let items = page
                .get("items")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for item in &items {
                // CortexDB has been observed to emit a record more than once
                // across pages; the cursor still reaches every record, so
                // de-duplicating by id here is enough — see the sibling
                // vendored adapter's `events()` for the same caveat measured
                // against a live instance.
                if let Some(id) = item.get("id").and_then(Value::as_str)
                    && !seen.insert(id.to_string())
                {
                    continue;
                }
                if let Some(record) = decode_event(item) {
                    all.push(record);
                }
            }
            let next_cursor = page
                .get("next_cursor")
                .and_then(Value::as_str)
                .map(str::to_string);
            match (page.get("has_more").and_then(Value::as_bool), next_cursor) {
                (Some(true), Some(next)) => cursor = Some(next),
                _ => return Ok(all),
            }
        }
        anyhow::bail!(
            "listing cortexdb scope `{scope}` exceeded {MAX_EVENT_PAGES} pages of \
             {EVENTS_PAGE_SIZE}; refusing to answer from a possibly-truncated log"
        )
    }

    /// Every scope path `GET /v1/scopes/list` reports, paginated the same way
    /// as [`Self::scope_events`].
    async fn list_scopes(&self) -> anyhow::Result<Vec<String>> {
        let mut all = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_SCOPE_PAGES {
            let mut query = vec![("limit".to_string(), SCOPES_PAGE_SIZE.to_string())];
            if let Some(cursor) = &cursor {
                query.push(("cursor".to_string(), cursor.clone()));
            }
            let response = self
                .request(Method::GET, SCOPES_LIST_PATH)
                .query(&query)
                .send()
                .await
                .map_err(|source| {
                    anyhow::anyhow!("cortexdb request to {SCOPES_LIST_PATH} failed: {source}")
                })?;
            let page = Self::check_status(response, SCOPES_LIST_PATH).await?;
            let items = page
                .get("items")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for item in &items {
                if let Some(path) = item.get("path").and_then(Value::as_str)
                    && seen.insert(path.to_string())
                {
                    all.push(path.to_string());
                }
            }
            let next_cursor = page
                .get("next_cursor")
                .and_then(Value::as_str)
                .map(str::to_string);
            match (page.get("has_more").and_then(Value::as_bool), next_cursor) {
                (Some(true), Some(next)) => cursor = Some(next),
                _ => return Ok(all),
            }
        }
        anyhow::bail!(
            "listing cortexdb scopes exceeded {MAX_SCOPE_PAGES} pages of {SCOPES_PAGE_SIZE}; \
             refusing to answer from a possibly-truncated listing"
        )
    }

    /// Folds every event filed under `namespace`'s scope down to one
    /// [`DecodedRecord`] per key — the most recently observed event for a key
    /// wins. This is the shared "upsert semantics over an event log" read
    /// this module's docs promise, and it backs `get`, `list`, `forget`, and
    /// `recall`'s stale-hit correction alike, so all four agree on what
    /// "current" means for a key.
    async fn latest_by_key(
        &self,
        namespace: &str,
    ) -> anyhow::Result<std::collections::HashMap<String, DecodedRecord>> {
        let events = self.scope_events(&Self::scope_for(namespace)).await?;
        let mut latest: std::collections::HashMap<String, DecodedRecord> =
            std::collections::HashMap::new();
        for record in events {
            if record.namespace != namespace {
                continue;
            }
            match latest.get(&record.key) {
                Some(existing) if existing.observed_at >= record.observed_at => {}
                _ => {
                    latest.insert(record.key.clone(), record);
                }
            }
        }
        Ok(latest)
    }

    /// The single record most-recently observed for `(namespace, key)`, if
    /// any — CortexDB's answer to "upsert" read through an event log.
    async fn latest(&self, namespace: &str, key: &str) -> anyhow::Result<Option<DecodedRecord>> {
        Ok(self.latest_by_key(namespace).await?.remove(key))
    }
}

/// One record decoded off a CortexDB `events` hit.
#[derive(Debug, Clone)]
struct DecodedRecord {
    id: String,
    namespace: String,
    key: String,
    content: String,
    category: MemoryCategory,
    session_id: Option<String>,
    taint: MemoryTaint,
    observed_at: String,
    score: Option<f64>,
}

impl DecodedRecord {
    fn into_entry(self) -> MemoryEntry {
        MemoryEntry {
            id: self.id,
            key: self.key,
            content: self.content,
            namespace: Some(self.namespace),
            category: self.category,
            timestamp: self.observed_at,
            session_id: self.session_id,
            score: self.score,
            taint: self.taint,
        }
    }
}

/// Decodes every `events` hit CortexDB's `/v1/recall` response carries.
///
/// Reads both response shapes CortexDB has been observed to use: a `layers`
/// map keyed by layer name, and a flat `results`/`items` array (a store
/// version that has not yet grouped by layer). Anything that does not decode
/// as this driver's own JSON envelope is skipped rather than raised — one
/// unreadable hit must not discard the whole recall.
fn decode_events(response: &Value) -> Vec<DecodedRecord> {
    let mut items: Vec<&Value> = Vec::new();
    if let Some(layers) = response.get("layers").and_then(Value::as_object)
        && let Some(events) = layers.get("events").and_then(Value::as_array)
    {
        items.extend(events.iter());
    }
    if items.is_empty() {
        for key in ["results", "items"] {
            if let Some(array) = response.get(key).and_then(Value::as_array) {
                items.extend(array.iter());
                break;
            }
        }
    }
    items.iter().filter_map(|item| decode_event(item)).collect()
}

/// Decodes one hit, if it carries this driver's `Content::Json` envelope.
fn decode_event(item: &Value) -> Option<DecodedRecord> {
    let content = item.get("content")?;
    let data = match content.get("kind").and_then(Value::as_str) {
        Some("json") => content.get("data")?,
        // Some CortexDB responses report content inline rather than nested
        // under `kind`/`data` — accept an object carrying our fields directly.
        _ if content.is_object() => content,
        _ => return None,
    };
    let object = data.as_object()?;
    let namespace = object.get("namespace")?.as_str()?.to_string();
    let key = object.get("key")?.as_str()?.to_string();
    let content_text = object.get("content")?.as_str()?.to_string();
    let category = object
        .get("category")
        .and_then(Value::as_str)
        .and_then(|raw| raw.parse::<MemoryCategory>().ok())
        .unwrap_or(MemoryCategory::Custom(String::new()));
    let session_id = object
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let taint = object
        .get("taint")
        .and_then(Value::as_str)
        .map(MemoryTaint::from_db_str)
        .unwrap_or_default();
    let observed_at = item
        .get("observed_at")
        .and_then(Value::as_str)
        .or_else(|| item.pointer("/context/observed_at").and_then(Value::as_str))
        .unwrap_or_default()
        .to_string();
    let id = item
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let score = item
        .get("confidence")
        .and_then(Value::as_f64)
        .or_else(|| item.get("score").and_then(Value::as_f64));
    Some(DecodedRecord {
        id,
        namespace,
        key,
        content: content_text,
        category,
        session_id,
        taint,
        observed_at,
        score,
    })
}

/// Lowercase hex SHA-256 digest.
fn digest_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Current wall-clock time as an RFC 3339 / ISO-8601 UTC instant.
///
/// No `chrono`/`time` dependency in this crate; reuses the same
/// dependency-free formatter the console's GraphQL layer already carries.
fn now_rfc3339() -> String {
    crate::server::graphql::iso8601(crate::ports::now_millis())
}

/// A minimal, dependency-free lowercase-hex encoder.
///
/// `sha2` is already a direct dependency of this crate; a hex crate is not, so
/// this module carries the handful of lines it would otherwise pull in.
mod hex {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";

    pub(super) fn encode(bytes: impl AsRef<[u8]>) -> String {
        let bytes = bytes.as_ref();
        let mut out = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            out.push(DIGITS[(byte >> 4) as usize] as char);
            out.push(DIGITS[(byte & 0x0f) as usize] as char);
        }
        out
    }
}

#[async_trait]
impl Memory for CortexdbMemory {
    fn name(&self) -> &str {
        CORTEXDB_DRIVER_ID
    }

    async fn store(
        &self,
        namespace: &str,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        self.store_with_taint(
            namespace,
            key,
            content,
            category,
            session_id,
            MemoryTaint::Internal,
        )
        .await
    }

    async fn store_with_taint(
        &self,
        namespace: &str,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
        taint: MemoryTaint,
    ) -> anyhow::Result<()> {
        let envelope = json!({
            "namespace": namespace,
            "key": key,
            "content": content,
            "category": category.to_string(),
            "session_id": session_id,
            "taint": taint.as_db_str(),
        });
        self.ingest(namespace, key, &envelope).await
    }

    async fn recall(
        &self,
        query: &str,
        limit: usize,
        opts: tinymemory_api::types::RecallOpts<'_>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        let namespace = opts
            .namespace
            .unwrap_or(tinymemory_api::types::GLOBAL_NAMESPACE);
        let mut records = self.recall_raw(namespace, query).await?;
        records.sort_by(|a, b| b.observed_at.cmp(&a.observed_at));
        // Newest write per key wins, among the hits `/v1/recall` itself
        // returned — necessary but not sufficient, see below.
        let mut seen = std::collections::HashSet::new();
        records.retain(|record| seen.insert(record.key.clone()));
        // A surviving hit can still be a *superseded* event: `/v1/recall`
        // ranks by relevance to `query`, so a stale event whose old content
        // happens to match can be the only version of a key reachable
        // through this particular query, even though a newer event for that
        // key exists. Resolve every hit against the key's actual latest
        // event before ranking further, so a corrected or
        // forgotten-and-rewritten memory can never resurface superseded
        // content through recall — the same "most recently observed event
        // wins" rule `get`/`list` apply, via the shared `latest_by_key` fold.
        // A key with nothing left to fold to (forgotten since the query ran)
        // is dropped rather than resurrected.
        if !records.is_empty() {
            let canonical = self.latest_by_key(namespace).await?;
            records = records
                .into_iter()
                .filter_map(|record| {
                    let current = canonical.get(&record.key)?;
                    let mut resolved = current.clone();
                    // The hit's score is `/v1/recall`'s relevance answer for
                    // `query`; keep it for ranking even though the content
                    // underneath may have just been swapped for the current
                    // version.
                    resolved.score = record.score;
                    Some(resolved)
                })
                .collect();
        }
        if let Some(category) = &opts.category {
            records.retain(|record| &record.category == category);
        }
        if let Some(session_id) = opts.session_id {
            records.retain(|record| record.session_id.as_deref() == Some(session_id));
        }
        if let Some(exclude) = opts.exclude_session_id {
            records.retain(|record| record.session_id.as_deref() != Some(exclude));
        }
        if let Some(min_score) = opts.min_score {
            records.retain(|record| record.score.is_none_or(|score| score >= min_score));
        }
        records.truncate(limit);
        Ok(records.into_iter().map(DecodedRecord::into_entry).collect())
    }

    async fn get(&self, namespace: &str, key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        Ok(self
            .latest(namespace, key)
            .await?
            .map(DecodedRecord::into_entry))
    }

    async fn list(
        &self,
        namespace: Option<&str>,
        category: Option<&MemoryCategory>,
        session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        let namespaces: Vec<String> = match namespace {
            Some(namespace) => vec![namespace.to_string()],
            None => self
                .namespace_summaries()
                .await?
                .into_iter()
                .map(|summary| summary.namespace)
                .collect(),
        };
        let mut out = Vec::new();
        for namespace in namespaces {
            let mut records: Vec<DecodedRecord> = self
                .latest_by_key(&namespace)
                .await?
                .into_values()
                .collect();
            records.sort_by(|a, b| b.observed_at.cmp(&a.observed_at));
            if let Some(category) = category {
                records.retain(|record| &record.category == category);
            }
            if let Some(session_id) = session_id {
                records.retain(|record| record.session_id.as_deref() == Some(session_id));
            }
            out.extend(records.into_iter().map(DecodedRecord::into_entry));
        }
        Ok(out)
    }

    async fn forget(&self, namespace: &str, key: &str) -> anyhow::Result<bool> {
        let scope = Self::scope_for(namespace);
        // Every event ever filed for this key, not only the newest: a key
        // stored more than once (e.g. a caller that rewrites its envelope to
        // add metadata without changing the logical key) leaves each write
        // as its own event in the log, and retracting only the latest would
        // leave the older ones behind for `get`/`list`/`recall` to fold back
        // in as if `forget` never ran.
        let ids: Vec<String> = self
            .scope_events(&scope)
            .await?
            .into_iter()
            .filter(|record| record.namespace == namespace && record.key == key)
            .map(|record| record.id)
            .collect();
        if ids.is_empty() {
            return Ok(false);
        }
        // `selector.memory_ids` plus `cascade: "redact_events"` names the
        // records and asks for the raw events themselves, not only their
        // derived layers — verified against a live CortexDB instance, which
        // otherwise refuses `{"by": "id", ...}` (not this server's
        // `ForgetSelector` shape at all: `about_subject` / `about_entity` /
        // `predicate` / `memory_ids`) and refuses an *empty* selector without
        // `confirm_all: true`. Every named layer travels too, since a generic
        // driver cannot know which of them this envelope reached.
        let body = json!({
            "scope": scope,
            "layers": ["events", "episodes", "facts", "beliefs", "understanding"],
            "selector": { "memory_ids": ids },
            "cascade": "redact_events",
        });
        let response = self
            .request(Method::POST, FORGET_PATH)
            .json(&body)
            .send()
            .await
            .map_err(|source| {
                anyhow::anyhow!("cortexdb request to {FORGET_PATH} failed: {source}")
            })?;
        let value = Self::check_status(response, FORGET_PATH).await?;
        // `{"deleted": {"events": n, "episodes": n, ...}, "matched": n, ...}`.
        let removed = value
            .get("deleted")
            .and_then(Value::as_object)
            .map(|layers| layers.values().filter_map(Value::as_u64).sum::<u64>())
            .or_else(|| value.get("matched").and_then(Value::as_u64))
            .unwrap_or(0);
        Ok(removed > 0)
    }

    async fn namespace_summaries(&self) -> anyhow::Result<Vec<NamespaceSummary>> {
        // CortexDB has no "list every scope" endpoint: scopes auto-provision on
        // first write and nothing enumerates them. A caller that already knows
        // its namespace gets exact answers from `get`/`list`/`recall`; only the
        // "list every namespace this backend holds" case — which this host's
        // own facades never invoke, see `super::mod` — cannot be served.
        Ok(Vec::new())
    }

    async fn count(&self) -> anyhow::Result<usize> {
        Ok(0)
    }

    async fn health_check(&self) -> bool {
        matches!(
            self.health_probe().await,
            Some(tinymemory_api::health::MemoryHealth::Ready)
        )
    }

    async fn health_probe(&self) -> Option<tinymemory_api::health::MemoryHealth> {
        let response = match self.request(Method::GET, READY_PATH).send().await {
            Ok(response) => response,
            Err(source) => {
                return Some(tinymemory_api::health::MemoryHealth::down(format!(
                    "cortexdb unreachable at {READY_PATH}: {source}"
                )));
            }
        };
        if response.status() == StatusCode::UNAUTHORIZED
            || response.status() == StatusCode::FORBIDDEN
        {
            return Some(tinymemory_api::health::MemoryHealth::down(
                "cortexdb rejected the token or the X-Cortex-Actor header",
            ));
        }
        if response.status().is_success() {
            Some(tinymemory_api::health::MemoryHealth::Ready)
        } else {
            Some(tinymemory_api::health::MemoryHealth::degraded(format!(
                "cortexdb answered {} for {READY_PATH} — storage is not ready yet",
                response.status()
            )))
        }
    }
}

#[cfg(test)]
#[path = "cortexdb_test.rs"]
mod test;
