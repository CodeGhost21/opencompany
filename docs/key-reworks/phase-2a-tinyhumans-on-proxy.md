# Phase 2a — TinyHumans is a provider row on the proxy

This is the first inference slice of the keys rework (issue #2306). It does
what closed PR #2305 set out to do, without the parts that went wrong.

- **Code read at:** `upstream/main @ fcfb3e1bc` (2026-09-14). Every `file:line`
  below was reopened on that commit on 2026-09-14.
- **Continues in:** [part 2](phase-2a-tinyhumans-on-proxy-part2.md) — console
  code, ordered edits, carry-over, tests, UI, must-not-touch, done-when,
  gotchas.
- **Next slices:** [2b](phase-2b-default-shape.md) ·
  [2c](phase-2c-model-required.md) · [2d](phase-2d-no-tier-on-the-wire.md).
- **#2305 is closed.** Do not merge, cherry-pick or copy its branch
  (`feat/managed-openrouter-proxy`, head `f4c42ea48`). Everything this slice
  needs is written out here.

## 0. Read first: step 3b is gated

Step 3b moves the constants `PLATFORM_BASE_URL` and
`DEFAULT_TINYHUMANS_INFERENCE_URL` to the proxy, and with them all **legacy**
managed traffic. On `fcfb3e1bc` that traffic sends tier names as model ids. The
proxy refuses a tier name (checked 2026-09-14: `chat-v1` returns 400).

Where the tier names come from:

- `DEFAULT_HOSTED_MODEL = "chat-v1"` (`src/harness/built_in/provider.rs:58`).
  The setup probe sends it (`:2245-2252`), and so does the setup brain
  (`src/harness/roster_build.rs:201-208`).
- A turn with no override sends the bare tier (`model_for_tier`,
  `src/company/inference.rs:360-378`).

Who would be hit:

- **Hosted tenants,** which use the constant unless `OPENCOMPANY_INFERENCE_URL`
  is set (`provider.rs:188-197`). `CLAUDE.md`'s list of variables the manager
  injects has no `OPENCOMPANY_INFERENCE_URL`. That is inferred from `CLAUDE.md`;
  the manager code was not read.
- README goal 9 and decision D-legacy both keep the env default on `/openai/v1`.

**Everything else in this slice is safe without 3b.** The new `tinyhumans` row
reaches the proxy through its own catalogue endpoint, whatever the constants
say. 3b is spelled out in §5.9. It is the **last** commit, and it runs only
after the operator answers part 2 §12.1. E2E hosts are unaffected either way:
they set `OPENCOMPANY_INFERENCE_URL=http://…/v1`
(`frontend/playwright.config.ts:221`, `:226`).

## 1. Goal

TinyHumans becomes an ordinary row in `inference/providers` with slug
`tinyhumans`. The operator adds it through the same dialog as OpenRouter:

1. type the key;
2. the probe runs;
3. pick a model from whatever the endpoint lists (or type one);
4. save, with health recorded.

Every model-list and chat call **that row** makes goes to
`https://api.tinyhumans.ai/agent-integrations/openrouter`. The LLM page shows
exactly one TinyHumans row. The legacy managed chain and its routes stay as
they are.

**Dump items** (`/Volumes/T9/oc-runs/operator-dump-2026-09-14.md`):

- **13b:** "move everything to /agent integration/open router". The row does
  this now; the constants follow in gated 3b.
- **14:** "same flow as OpenRouter: add the key … select the model".
- **15:** one "is it set?" rule. A row means set (D-set). The model lives in the
  row's `models` field (Q3).

**Model ids.** Any id returned by `GET /agent-integrations/openrouter/models` is
valid. The code lists exactly what the endpoint returns and sends exactly what
was chosen. It never hardcodes, filters, prefers or rejects an id by vendor or
by name.

**Use cases.** The "Today" column is read from code, not seen in a browser.

| Company | Today on `fcfb3e1bc` | After this slice |
|---|---|---|
| Fresh company, adds TinyHumans | "Managed (TinyHumans)" option → `PUT …/inference/managed/key`; no probe, no model, no record | "TinyHumans" catalogue option → key → the endpoint's model list → one row with `models` set and health recorded |
| Only the account key `tinyhumans/key` | Legacy Managed row "Billed to this company's TinyHumans account" | Unchanged |
| Legacy key at `provider/tinyhumans/key`, no row | Legacy Managed row | Unchanged until TinyHumans is added. The add replaces the key and the legacy row disappears (part 2 §7) |
| Entry zero `inference/config = {provider: managed}` | Entry-zero row "Managed" **plus** the legacy row when the chain resolves (`ProviderList.tsx:253`, `:319`) | Entry-zero row only. Adding TinyHumans answers 400 before any write |
| Hosted tenant, nothing configured | Legacy Managed row "Billed to whoever runs this server" | Unchanged (3b is gated) |

## 2. Do not do this (lessons from #2305)

1. **No new secret-store keys.** #2305's `inference/managed/models` was
   rejected. The model goes in the `tinyhumans` record's `models`.
2. **None of these:**
   - a Managed-only model dialog;
   - a new draft-probe route;
   - a `proxied_model` rule;
   - a URL-origin derivation module.

   `OPENCOMPANY_INFERENCE_URL` is used exactly as given.
3. **No `catalog_shape` field on every `CloudProvider` row.** That would mean 27
   Rust rows plus the TS mirror that
   `the_console_mirror_lists_the_same_cloud_providers` parses. Use one function
   instead (§5.1).
4. **Never two TinyHumans rows** (§5.7, part 2 §5.10).
5. **Never skip the model step, and never leave health `unchecked`.** A
   TinyHumans add without a key or without a model is refused before any write
   (§5.6), so the probe always runs. The step must not depend on what the
   catalog happens to contain.
6. **Never a tier name from the new row.** `tier_overrides` writes the chosen id
   to all four tiers (`providers.rs:680-688`), and `model_for_tier` returns it
   (`inference.rs:365-367`). Slice 2d owns the general rule.
7. **No assumptions about which ids the catalog holds,** in code, docs or tests.
   Tests use fake ids served by a mock catalog (`acme/test-model`,
   `acme/other-model`) and the fake key `th-not-a-real-key`.
8. **Do not touch** the managed chain, the legacy managed routes, or
   `inference/managed/enabled` (items 3 and 10 are not handled).
9. **Do not flip the constants** before the §0 gate is answered.

## 3. Files (anchors on `fcfb3e1bc`)

| File | Change | Anchors |
|---|---|---|
| `src/company/inference/catalogue.rs` | `CatalogShape`, `TINYHUMANS_PROXY_PATH`, `catalog_shape_for`, the row, docs, tests | `CloudProvider` :77-92; `CLOUD_PROVIDERS` :110 (last row `modelscope`); `auth_style_for` :799-810; `INTERNAL_SLUGS` :986-1000; tests :1077 (counts), :1257-1271 (reserved), :1478 (mirror) |
| `src/company/inference/paged_catalog.rs` | **new**, pure parser | — |
| `src/company/inference.rs` | `pub mod paged_catalog;`; constant (3b) | modules ~:28-32; `PLATFORM_BASE_URL` :152; `MANAGED_SLUG` :416 |
| `src/company/inference/probe.rs` | `shape` param; paged loop; `probe_get`; `read_capped_to`; `ProbeFailure::unreadable` | `ProbeFailure` impl ~:694-725; `probe_models` :774-876; `read_capped` :1000; test call :1373 |
| `src/server/inference_models.rs` | `shape` on four fns; paged fetch; shaped cache key | `discover_models` :298-383; `fetch_catalog` :390-431; `cache_key` :451; `catalog_models` :536-617; `discovered_vocabulary` :631; `turn_vocabulary` :666; test calls :718, :766, :801, :971 |
| `src/server/ops/inference/providers.rs` | shape at 5 reads; key and model required for TinyHumans; restore replaced key | `add_provider` :309-521 (duplicate check :343-348, key write :352-362, rollbacks :389-391 / :443 / :481, health :452 / :489); `plan_add` cloud branch :717-726; `test_managed` probe :1550; `list_provider_models` :1642; `test_provider` :1805; `probe_draft` :1889 |
| `src/server/ops/inference.rs` | shape in `resolved_endpoint` / `test_config`; `ManagedDto.legacy_row` | `resolved_endpoint` :141-167; `list_models` :171/:192; `ManagedDto` :366-390; `effective_status_with` :876-878; `managed_state` :951-990; `test_config` :1342 |
| `src/harness/built_in/provider.rs` | shape at turn vocabulary; parser test; constant (3b) | `DEFAULT_TINYHUMANS_INFERENCE_URL` :55; `turn_vocabulary` call :2125-2130; `model_response_from_payload` :953 |
| `src/server/setup.rs` | shape at wizard discovery | :1296 |
| `frontend/src/inference/{catalogue,connect}.ts`, `AddProviderDialog.tsx`, `ProvidersTab.tsx`, `ProviderList.tsx`, `ProviderConnectDialog.tsx`, `frontend/src/api/inference.ts` | console (part 2 §5.10) | see part 2 |
| `docs/modules/inference/catalogue.md` | 27 rows | :19, :50, :52-56 |
| `docs/spec/runtime/config.md`, `docs/modules/openhuman/README.md` | default URL (3b only) | :122, :81 |

## 4. Current code (excerpts)

The catalog reader expects one OpenAI-shaped page (`probe.rs:853`):

```rust
let url = format!("{base}/models{}", catalogue::catalog_query(base));
```

The parser fails on the envelope, because there `data` is an object
(`inference_models.rs:222-226`, `:427`):

```rust
struct RegistryResponse { #[serde(default)] data: Vec<serde_json::Value> }
let payload = response.json::<RegistryResponse>().await.map_err(|error| { … })?;
```

The duplicate check already covers an entry zero whose slug is `tinyhumans`
(`providers.rs:343-348`):

```rust
} else if existing.iter().any(|p| p.slug == plan.slug) {
    return Err(ApiError(OpenCompanyError::InvalidRequest(format!(
        "{} is already connected. Edit the existing row rather than adding a second one.",
        plan.label))));
```

The add backstop is decided by catalog **content** (`providers.rs:442`, `:668-670`),
so it cannot guarantee a model step for TinyHumans:

```rust
if asked_model.is_none() && needs_an_explicit_model(&models) { roll_back_add(…); return Err(…) }
fn needs_an_explicit_model(models: &[String]) -> bool {
    TierVocabulary::from_catalog_ids(models.iter().map(String::as_str)) == TierVocabulary::Unknown }
```

An indexed row resolves direct and is not proxied (`inference.rs:1431-1440`).
`normalize_provider("tinyhumans")` passes the kind through (`:382-387`).

```rust
let credential = Credential::from_value(key);
let proxied = false;
Ok(InferenceDecl { provider: normalize_provider(&provider.kind).to_string(),
    base_url: provider.base_url.clone(), models: provider.models.clone(), … })
```

## 5. Target code (Rust)

### 5.1 `catalogue.rs`

Put this above `pub enum AuthStyle` (:46):

```rust
/// The shape a provider's `GET {base}/models` answers in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CatalogShape {
    /// `{"data":[{"id":…}]}`, one response.
    OpenAi,
    /// `{"success":true,"data":{"data":[…],"total":N,"limit":L,"offset":O}}`,
    /// paged. See [`super::paged_catalog`].
    PagedEnvelope,
}

/// The path the TinyHumans OpenRouter proxy is served under.
pub const TINYHUMANS_PROXY_PATH: &str = "/agent-integrations/openrouter";
```

Put this above `pub fn auth_style_for` (:799):

```rust
/// Which catalog shape to read at `base_url` for a provider of `kind`.
///
/// `tinyhumans` always pages. Any other kind pages only when its endpoint's path
/// ends in [`TINYHUMANS_PROXY_PATH`]: the legacy managed declaration has kind
/// `openrouter`, and after step 3b (or with `OPENCOMPANY_INFERENCE_URL` set to
/// the proxy) its base is the proxy. A read-only path check; no URL is built.
pub fn catalog_shape_for(kind: &str, base_url: &str) -> CatalogShape {
    if kind.trim() == super::MANAGED_SLUG
        || base_url.trim().trim_end_matches('/').ends_with(TINYHUMANS_PROXY_PATH)
    {
        CatalogShape::PagedEnvelope
    } else {
        CatalogShape::OpenAi
    }
}
```

Append this as the **last** `CLOUD_PROVIDERS` entry, after `modelscope`:

```rust
    CloudProvider {
        slug: "tinyhumans",
        label: "TinyHumans",
        endpoint: "https://api.tinyhumans.ai/agent-integrations/openrouter",
        auth: AuthStyle::Bearer,
        key_placeholder: Some("th-..."),
    },
```

Doc edits:

- `:94`: "The 26 hosted providers" becomes "The 27 hosted providers".
- Module doc `:40-44`: "TinyHumans is a row (slug `tinyhumans`): its OpenRouter
  proxy, a bearer key, a paged catalog. The legacy managed *chain* keeps its own
  auth path (`super::PLATFORM_BASE_URL`)."
- `INTERNAL_SLUGS` doc `:986-999`: add "`tinyhumans` is also a cloud row now; it
  stays listed so reservation does not depend on the table."

### 5.2 `paged_catalog.rs` (new)

Register it in `src/company/inference.rs` next to `pub mod catalogue;`:

```rust
//! The TinyHumans proxy's paged model catalog:
//! `{"success":true,"data":{"object":"list","data":[…],"total":N,"limit":L,"offset":O}}`.
//! Pure; no I/O. Readers: `probe::probe_models`, `inference_models::discover_models`.
use std::collections::HashSet;

/// Page size requested. The backend clamps `limit` to `[1, 500]`.
pub const PAGE_LIMIT: usize = 500;
/// Most pages one read follows, so a `total` never reached cannot loop.
pub const MAX_PAGES: usize = 20;
/// Largest success body read for one page.
pub const PAGE_BODY_CAP: usize = 4 * 1024 * 1024;

pub fn page_path(offset: usize) -> String { format!("/models?limit={PAGE_LIMIT}&offset={offset}") }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogEntry { pub id: String, pub name: Option<String>, pub context_length: Option<u64> }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogPage {
    pub entries: Vec<CatalogEntry>,
    /// Entries the page carried, usable or not. Paging advances by this.
    pub raw_len: usize,
    pub total: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NextPage { At(usize), Done, Truncated { read: usize, total: usize } }

/// One page. `Err` (never an empty page) on: not JSON; `success: false`; no object
/// `data`; no array `data.data` — so a plain `{"data":[…]}` body is an error.
/// Every id is kept as given. Unknown fields are ignored.
pub fn parse_page(body: &str) -> Result<CatalogPage, String> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| format!("the model catalog was not JSON: {e}"))?;
    if value.get("success").and_then(serde_json::Value::as_bool) == Some(false) {
        let reason = value.get("error").or_else(|| value.get("message"))
            .and_then(serde_json::Value::as_str).unwrap_or("no reason given");
        return Err(format!("the model catalog reported a failure: {reason}"));
    }
    let Some(data) = value.get("data").filter(|d| d.is_object()) else {
        return Err("the model catalog was not in the `{success, data}` envelope".to_string());
    };
    let Some(raw) = data.get("data").and_then(serde_json::Value::as_array) else {
        return Err("the model catalog envelope carried no `data` list".to_string());
    };
    let text = |e: &serde_json::Value, k: &str| e.get(k).and_then(serde_json::Value::as_str)
        .map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let entries = raw.iter().filter_map(|e| Some(CatalogEntry {
        id: text(e, "id")?,
        name: text(e, "display_name").or_else(|| text(e, "name")),
        context_length: e.get("context_length").and_then(serde_json::Value::as_u64),
    })).collect();
    let total = data.get("total").and_then(serde_json::Value::as_u64)
        .and_then(|t| usize::try_from(t).ok());
    Ok(CatalogPage { entries, raw_len: raw.len(), total })
}

/// Pages collected, deduplicated by id, in listing order.
#[derive(Debug, Default)]
pub struct Collector { seen: HashSet<String>, entries: Vec<CatalogEntry>, offset: usize, pages: usize }

impl Collector {
    pub fn offset(&self) -> usize { self.offset }
    /// Stops on: an empty page; reaching `total`; no `total`; [`MAX_PAGES`].
    pub fn push(&mut self, page: CatalogPage) -> NextPage {
        self.pages += 1;
        for entry in page.entries {
            if self.seen.insert(entry.id.clone()) { self.entries.push(entry); }
        }
        if page.raw_len == 0 { return NextPage::Done; }
        self.offset += page.raw_len;
        match page.total {
            Some(total) if self.offset < total && self.pages >= MAX_PAGES =>
                NextPage::Truncated { read: self.offset, total },
            Some(total) if self.offset < total => NextPage::At(self.offset),
            _ => NextPage::Done,
        }
    }
    pub fn finish(self) -> Vec<CatalogEntry> { self.entries }
}
```

Run `cargo fmt` afterwards. Its tests are in part 2 §8.1.

### 5.3 `probe.rs`

1. Import `super::catalogue::{self, CatalogShape}` and `super::paged_catalog`.
2. In `impl ProbeFailure` (~:694), add a constructor whose class can never be
   `auth`, so it can never roll a key back:
   `fn unreadable(raw: String) -> Self { Self { class: ProbeClass::Unknown, raw } }`.
3. Rename `read_capped(response)` (:1000) to `read_capped_to(response, cap)`,
   replacing `PROBE_BODY_CAP` inside it with `cap`.
4. Add the last parameter: `probe_models(base_url, credential, auth, policy, shape: CatalogShape)`.
5. Move the request / status / body block (:853-875, from
   `let request = apply_auth(…)` to the non-success `return Err`) into
   `async fn probe_get(client: &reqwest::Client, url: &str, auth: catalogue::AuthStyle,
   credential: Option<&str>, success_cap: usize) -> Result<String, ProbeFailure>`:
   - build `named` from `url` inside the helper;
   - read the body with `success_cap` on success, `PROBE_BODY_CAP` otherwise;
   - return `Ok(body)`.
6. After the client is built (:848):

```rust
if shape == CatalogShape::PagedEnvelope {
    let mut collector = paged_catalog::Collector::default();
    loop {
        let page_url = format!("{base}{}", paged_catalog::page_path(collector.offset()));
        let body = probe_get(&client, &page_url, auth, credential, paged_catalog::PAGE_BODY_CAP).await?;
        let page = paged_catalog::parse_page(&body).map_err(|e|
            ProbeFailure::unreadable(format!("{}: {e}", catalogue::redact_endpoint(&page_url))))?;
        if !matches!(collector.push(page), paged_catalog::NextPage::At(_)) { break; }
    }
    return Ok(collector.finish().into_iter().map(|e| e.id).collect());
}
let body = probe_get(&client, &url, auth, credential, PROBE_BODY_CAP).await?;
Ok(parse_model_ids(&body))
```

The redirect policy's `origin` is the first URL, and every page shares that
origin, so the same-origin guard still holds.

### 5.4 `inference_models.rs`

1. Import `CatalogShape` and `paged_catalog::{self, NextPage}`.
2. Move `fetch_catalog`'s auth / send / status block (:405-426) into
   `async fn send_classified(client, url, bearer, auth) -> Result<reqwest::Response, DiscoveryError>`.
   `fetch_catalog` calls it and keeps its `RegistryResponse` parse.
3. Add `shape: CatalogShape` as the last parameter of `discover_models`,
   `catalog_models`, `discovered_vocabulary` and `turn_vocabulary`, and pass it
   through.
4. In `discover_models`, after the client is built (:351) and before the
   `scoped_catalog_path` block:
   `if shape == CatalogShape::PagedEnvelope { return fetch_paged_catalog(&client, base, bearer, auth).await; }`
5. Add the paged fetch:

```rust
async fn fetch_paged_catalog(client: &reqwest::Client, base: &str, bearer: Option<&str>,
    auth: AuthStyle) -> Result<Vec<InferenceModel>, DiscoveryError> {
    let mut collector = paged_catalog::Collector::default();
    loop {
        let url = format!("{base}{}", paged_catalog::page_path(collector.offset()));
        let named = catalogue::redact_endpoint(&url);
        let mut response = send_classified(client, &url, bearer, auth).await?;
        let mut body: Vec<u8> = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| DiscoveryError::endpoint(
            format!("reading the model catalog from {named} failed: {e}")))? {
            if body.len() + chunk.len() > paged_catalog::PAGE_BODY_CAP {
                return Err(DiscoveryError::endpoint(format!(
                    "a model catalog page from {named} is larger than 4 MiB")));
            }
            body.extend_from_slice(&chunk);
        }
        let body = String::from_utf8(body).map_err(|e| DiscoveryError::endpoint(
            format!("model catalog from {named} was not UTF-8: {e}")))?;
        let page = paged_catalog::parse_page(&body).map_err(|e| DiscoveryError::endpoint(
            format!("model catalog from {named} was invalid: {e}")))?;
        match collector.push(page) {
            NextPage::At(_) => {}
            NextPage::Done => break,
            NextPage::Truncated { read, total } => {
                tracing::warn!(base = %catalogue::redact_endpoint(base), read, total,
                    "model catalog has more pages than one read follows");
                break;
            }
        }
    }
    Ok(collector.finish().into_iter()
        .map(|e| InferenceModel { id: e.id, name: e.name, context_length: e.context_length })
        .collect())
}
```

6. Put the shape in the cache key. Add this below `cache_key` (:451):

```rust
fn shaped_endpoint(base_url: &str, shape: CatalogShape) -> String {
    match shape {
        CatalogShape::OpenAi => cache_key(base_url),
        CatalogShape::PagedEnvelope => format!("{}\u{2}paged", cache_key(base_url)),
    }
}
```

   In `catalog_models` (:549), use
   `catalog_cache_scoped(&shaped_endpoint(base_url, shape), authenticated_scope)`.

   These stay exactly as they are:
   - the 60 s failure memo;
   - 401/403 not memoized (`FetchError::Credential`);
   - an empty catalog counts as a failure;
   - the timeout;
   - the sort;
   - `evict_company_catalogs`.

### 5.5 Callers pass the shape

| Caller | Argument added |
|---|---|
| `providers.rs:412` `add_provider` | `catalogue::catalog_shape_for(&plan.kind, &provider.base_url)` |
| `providers.rs:1550` `test_managed` | `catalogue::catalog_shape_for(inference::LEGACY_MANAGED, &base_url)` |
| `providers.rs:1642` `list_provider_models`, `:1805` `test_provider` | `catalogue::catalog_shape_for(&provider.kind, &provider.base_url)` |
| `providers.rs:1889` `probe_draft` | `catalogue::catalog_shape_for(kind, body.base_url.trim())` |
| `ops/inference.rs:141-167` `resolved_endpoint` | return a 4-tuple ending in `catalogue::catalog_shape_for(&decl.provider, &decl.base_url)`; destructure at :171, pass at :192 |
| `ops/inference.rs:1342` `test_config` | `catalogue::catalog_shape_for(&decl.provider, &decl.base_url)` |
| `provider.rs:2125` `TenantProvider::resolve` | `crate::company::inference::catalogue::catalog_shape_for(&decl.provider, &decl.base_url)` |
| `setup.rs:1296` `probe_inference` | `crate::company::inference::catalogue::catalog_shape_for(&req.provider, &decl.base_url)` |
| test calls `probe.rs:1373`, `inference_models.rs:718`, `:766`, `:801`, `:971` | `CatalogShape::OpenAi` |

`cargo build --all-targets` then names any caller this table missed.

### 5.6 `providers.rs`: key and model required; restore a replaced key

**(a) Key required.** In `plan_add`'s cloud branch (:717), before its
`return Ok(AddPlan {…})`:

```rust
if cloud.slug == crate::company::inference::MANAGED_SLUG && !has_key {
    return Err(invalid("TinyHumans needs an API key.".to_string()));
}
```

**(b) Model required.** This does not depend on catalog content. In
`add_provider`, directly after the `plan_add(…)?` call (:333) and before any
read or write:

```rust
if plan.slug == crate::company::inference::MANAGED_SLUG && asked_model.is_none() {
    return Err(ApiError(OpenCompanyError::InvalidRequest(
        "Choose a model for TinyHumans.".to_string())));
}
```

**(c) Duplicates.** No code change. `:343-348` already answers "TinyHumans is
already connected. Edit the existing row rather than adding a second one." for
an indexed `tinyhumans` row and for an entry zero on a managed config, before
any write.

**(d) Restore a replaced key.** The slot `provider/tinyhumans/key` can hold the
legacy Managed row's key with no record behind it. Every rollback clears that
slot (`roll_back_add` → `store::delete_provider` :839; `clear_orphaned_key`
:857-863). So:

1. After the duplicate check (:348), read the old value:

```rust
let previous_key = if plan.slug == crate::company::inference::MANAGED_SLUG {
    secrets.get(runtime.id(), &store::provider_key_key(&plan.slug)).await.map_err(ApiError)?
        .map(|crate::ports::types::SecretValue(raw)| raw)
        .filter(|raw| !raw.trim().is_empty())
} else { None };
```

2. Add this helper beside `clear_orphaned_key`:

```rust
/// Puts back a key that an add replaced and then rolled back. `None` does nothing.
async fn restore_previous_key(runtime: &CompanyRuntime, slug: &str, previous: Option<&str>) {
    let Some(previous) = previous else { return };
    if let Err(err) = runtime.secrets().set(runtime.id(), &store::provider_key_key(slug),
        crate::ports::types::SecretValue(previous.to_string())).await {
        tracing::error!(company = %runtime.id(), provider = %slug, error = %err,
            "could not restore the key a rolled-back add had replaced");
    }
    crate::server::inference_models::evict_company_catalogs(runtime.id().as_ref());
}
```

3. Call `restore_previous_key(runtime, &plan.slug, previous_key.as_deref()).await;`
   immediately **after** each rollback:
   - after `clear_orphaned_key` (:390);
   - after `roll_back_add` (:443);
   - after `roll_back_add` (:481).

**(e) Health.** No code change. With (a), `worth_probing` is always true
(:409), so health is recorded as `"ok"` (:452) or as the failure class (:489).

### 5.7 `ops/inference.rs`: exactly one row on the wire

Add to `ManagedDto` (:366-390), after `configured`:

```rust
    /// Whether the console renders the separate legacy Managed row. False when
    /// `providers` already lists slug `tinyhumans` (an added row, or entry zero on a
    /// managed config): that row is the one TinyHumans row.
    legacy_row: bool,
```

- In `managed_state` (:977), set `legacy_row: source.resolves(),`.
- In `effective_status_with`, change `let managed` (:878) to `let mut managed`
  and add:

```rust
managed.legacy_row = managed.configured
    && !providers.iter().any(|p| p.slug == inference::MANAGED_SLUG);
```

`managed_resolves` (:936) and `configured` do not change. Routing still asks
whether the chain resolves.

### 5.8 Managed Test after the change

`test_managed` keeps its credential chain (:1515-1542) and writes health to
`inference/health["tinyhumans"]`, as today. Only the shape argument (§5.5) is
new. What it probes:

- **Before 3b:** `…/openai/v1/models`, as today.
- **After 3b:** `…/agent-integrations/openrouter/models?limit=500&offset=0`,
  paged.
- **With `OPENCOMPANY_INFERENCE_URL` set:** that URL, in the shape
  `catalog_shape_for` picks from its path.

### 5.9 Step 3b (GATED, §0): the two constants

**Edit these:**

| `file:line` | Change |
|---|---|
| `src/company/inference.rs:152` | `pub const PLATFORM_BASE_URL: &str = "https://api.tinyhumans.ai/agent-integrations/openrouter";`; doc (:146-151): "The TinyHumans OpenRouter proxy — the legacy managed chain's endpoint." |
| `src/harness/built_in/provider.rs:55` | `pub const DEFAULT_TINYHUMANS_INFERENCE_URL: &str = "https://api.tinyhumans.ai/agent-integrations/openrouter";` |
| `docs/spec/runtime/config.md:122`, `docs/modules/openhuman/README.md:81` | default URL column |

**No edit** needed in any of these:

- **Uses that follow the constant:** `inference.rs:661`, `:682`;
  `provider.rs:193`; `roster_build.rs:194`; `ops/inference.rs:857`, `:862`,
  `:983`; `providers.rs:1547`.
- **Tests naming the constant:** `inference.rs:2808`, `:2991`, `:3665`;
  `provider.rs:2432`, `:4437`; `ops/inference.rs:2499`, `:2569`, `:2939`.
- **Literals that stay:**
  - staging overrides and fixtures: `provider.rs:2443/2449`,
    `ops/inference.rs:2462`, `setup-wizard-finish-gate.test.ts:323/431/465/525`;
  - backend error text: `provider.rs:1749/1757/5368-5535`;
  - non-OpenRouter host tests: `catalogue.rs:1992/2016`;
  - comments: `cost.rs:87/229`, `metering/inference.rs:65`,
    `ports/types.rs:2801`, `run_trace.rs:259`, `provider.rs:623/1205`.

**Never rewrite the injected URL.** `OPENCOMPANY_INFERENCE_URL` stays
`env.get(…)` verbatim (`provider.rs:190-192`, `roster_build.rs:192-194`).
Hosted tenants run on whatever the manager injects, which makes it a
manager-side prerequisite (part 2 §12.1).

**Response parser.** `model_response_from_payload` (`provider.rs:953`) reads
`/choices/0/…` and `usage` through JSON pointers, so extra top-level keys
(`openhuman`, `service_tier`) and extra `usage` keys (`cost`, `is_byok`) are
ignored. The turn path does **not** stream: `send_plan` (`provider.rs:1843`)
reads one JSON body, and `provider.rs` has no `"stream"` request field. A
final SSE frame without `choices` therefore never reaches this parser; say so in
the PR. The parser test is in part 2 §8.1.
