//! The TinyHumans proxy's paged model catalog:
//! `{"success":true,"data":{"object":"list","data":[…],"total":N,"limit":L,"offset":O}}`.
//! Pure; no I/O. Readers: [`super::probe::probe_models`],
//! [`crate::server::inference_models::discover_models`].
//!
//! Keys rework, issue #2306, slice 2a. Every id this parser reads is kept
//! exactly as given — nothing here hardcodes, filters, prefers or rejects a
//! model id by vendor or name; any id the endpoint returns is valid.

use std::collections::HashSet;

/// Page size requested. The backend clamps `limit` to `[1, 500]`.
pub const PAGE_LIMIT: usize = 500;
/// Most pages one read follows, so a `total` never reached cannot loop.
pub const MAX_PAGES: usize = 20;
/// Largest success body read for one page.
pub const PAGE_BODY_CAP: usize = 4 * 1024 * 1024;

/// The path (with query) for one page at `offset`.
pub fn page_path(offset: usize) -> String {
    format!("/models?limit={PAGE_LIMIT}&offset={offset}")
}

/// One model the proxy's catalog listed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogEntry {
    /// The id exactly as the endpoint sent it. Any id it returns is valid.
    pub id: String,
    /// `display_name`, else `name`, when present.
    pub name: Option<String>,
    /// The context window, when the endpoint publishes one.
    pub context_length: Option<u64>,
}

/// One parsed page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogPage {
    /// Entries this page carried, after dropping malformed ones.
    pub entries: Vec<CatalogEntry>,
    /// How many entries the page carried, usable or not. Paging advances by
    /// this, not by `entries.len()`, so a malformed entry still moves the
    /// offset forward instead of being re-requested forever.
    pub raw_len: usize,
    /// The envelope's `total`, when it parses as a non-negative integer.
    pub total: Option<usize>,
}

/// What to do after one page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NextPage {
    /// Request the next page at this offset.
    At(usize),
    /// Stop: an empty page, `total` reached, or no `total` at all.
    Done,
    /// Stop: [`MAX_PAGES`] was reached before `total` was.
    Truncated {
        /// How many entries were read before stopping.
        read: usize,
        /// The envelope's own `total`.
        total: usize,
    },
}

/// Parses one page. `Err` (never an empty page) on: not JSON; `success:
/// false`; no object `data`; no array `data.data` — so a plain
/// `{"data":[…]}` body (the OpenAI shape) is an error, not a page with no
/// entries. Every id is kept exactly as given. Unknown fields are ignored.
pub fn parse_page(body: &str) -> Result<CatalogPage, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("the model catalog was not JSON: {e}"))?;
    if value.get("success").and_then(serde_json::Value::as_bool) == Some(false) {
        let reason = value
            .get("error")
            .or_else(|| value.get("message"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("no reason given");
        return Err(format!("the model catalog reported a failure: {reason}"));
    }
    let Some(data) = value.get("data").filter(|d| d.is_object()) else {
        return Err("the model catalog was not in the `{success, data}` envelope".to_string());
    };
    let Some(raw) = data.get("data").and_then(serde_json::Value::as_array) else {
        return Err("the model catalog envelope carried no `data` list".to_string());
    };
    let text = |e: &serde_json::Value, k: &str| {
        e.get(k)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let entries = raw
        .iter()
        .filter_map(|e| {
            Some(CatalogEntry {
                id: text(e, "id")?,
                name: text(e, "display_name").or_else(|| text(e, "name")),
                context_length: e.get("context_length").and_then(serde_json::Value::as_u64),
            })
        })
        .collect();
    let total = data
        .get("total")
        .and_then(serde_json::Value::as_u64)
        .and_then(|t| usize::try_from(t).ok());
    Ok(CatalogPage {
        entries,
        raw_len: raw.len(),
        total,
    })
}

/// Pages collected, deduplicated by id, in listing order.
#[derive(Debug, Default)]
pub struct Collector {
    seen: HashSet<String>,
    entries: Vec<CatalogEntry>,
    offset: usize,
    pages: usize,
}

impl Collector {
    /// The offset the next page should request.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Folds one page in and says what to do next. Stops on: an empty page;
    /// reaching `total`; no `total` at all; [`MAX_PAGES`].
    pub fn push(&mut self, page: CatalogPage) -> NextPage {
        self.pages += 1;
        for entry in page.entries {
            if self.seen.insert(entry.id.clone()) {
                self.entries.push(entry);
            }
        }
        if page.raw_len == 0 {
            return NextPage::Done;
        }
        self.offset += page.raw_len;
        match page.total {
            Some(total) if self.offset < total && self.pages >= MAX_PAGES => NextPage::Truncated {
                read: self.offset,
                total,
            },
            Some(total) if self.offset < total => NextPage::At(self.offset),
            _ => NextPage::Done,
        }
    }

    /// The entries collected so far, consuming the collector.
    pub fn finish(self) -> Vec<CatalogEntry> {
        self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_path_asks_for_the_maximum_page() {
        assert_eq!(page_path(0), "/models?limit=500&offset=0");
        assert_eq!(page_path(500), "/models?limit=500&offset=500");
    }

    #[test]
    fn a_page_is_unwrapped_and_unknown_fields_are_ignored() {
        let body = serde_json::json!({
            "success": true,
            "data": {
                "object": "list",
                "data": [
                    {
                        "id": "acme/test-model",
                        "display_name": "Test Model",
                        "context_length": 8192,
                        "pricing": {"prompt": "0.0001"},
                        "supports_tools": true,
                        "input_modalities": ["text"],
                    },
                    {"id": "acme/other-model", "name": "Other Model"},
                ],
                "total": 2,
                "limit": 500,
                "offset": 0,
            },
        })
        .to_string();
        let page = parse_page(&body).expect("parses");
        assert_eq!(page.raw_len, 2);
        assert_eq!(page.total, Some(2));
        assert_eq!(
            page.entries,
            vec![
                CatalogEntry {
                    id: "acme/test-model".to_string(),
                    name: Some("Test Model".to_string()),
                    context_length: Some(8192),
                },
                CatalogEntry {
                    id: "acme/other-model".to_string(),
                    name: Some("Other Model".to_string()),
                    context_length: None,
                },
            ]
        );
    }

    #[test]
    fn every_id_is_kept_as_given() {
        let body = serde_json::json!({
            "success": true,
            "data": {"data": [{"id": "acme/test-model"}, {"id": "x"}, {"id": "a:b:c"}]},
        })
        .to_string();
        let page = parse_page(&body).expect("parses");
        let ids: Vec<&str> = page.entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["acme/test-model", "x", "a:b:c"]);
    }

    #[test]
    fn an_openai_shaped_body_is_an_error_not_an_empty_page() {
        let body = serde_json::json!({"data": [{"id": "acme/test-model"}]}).to_string();
        let err = parse_page(&body).expect_err("not the envelope");
        assert!(err.contains("envelope"), "{err}");
    }

    #[test]
    fn success_false_is_an_error_carrying_the_reason() {
        let body = serde_json::json!({"success": false, "error": "rate limited"}).to_string();
        let err = parse_page(&body).expect_err("failure");
        assert!(err.contains("rate limited"), "{err}");
    }

    #[test]
    fn a_malformed_entry_is_dropped_but_still_advances_paging() {
        let body = serde_json::json!({
            "success": true,
            "data": {
                "data": [
                    {"id": 42},
                    {"id": "   "},
                    {"id": "acme/test-model", "context_length": "not-a-number"},
                ],
                "total": 3,
            },
        })
        .to_string();
        let page = parse_page(&body).expect("parses");
        assert_eq!(
            page.raw_len, 3,
            "raw_len counts every entry, dropped or not"
        );
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].id, "acme/test-model");
        assert_eq!(page.entries[0].context_length, None);
    }

    #[test]
    fn paging_follows_total_and_stops_there() {
        let mut collector = Collector::default();
        let first = parse_page(
            &serde_json::json!({"success": true, "data": {"data": [{"id": "a"}, {"id": "b"}], "total": 3}})
                .to_string(),
        )
        .unwrap();
        assert_eq!(collector.push(first), NextPage::At(2));
        let second = parse_page(
            &serde_json::json!({"success": true, "data": {"data": [{"id": "c"}], "total": 3}})
                .to_string(),
        )
        .unwrap();
        assert_eq!(collector.push(second), NextPage::Done);
        assert_eq!(collector.finish().len(), 3);
    }

    #[test]
    fn a_clamped_limit_costs_requests_not_models() {
        let mut collector = Collector::default();
        let first = parse_page(
            &serde_json::json!({"success": true, "data": {"data": [{"id": "a"}], "total": 2}})
                .to_string(),
        )
        .unwrap();
        assert_eq!(collector.push(first), NextPage::At(1));
        let second = parse_page(
            &serde_json::json!({"success": true, "data": {"data": [{"id": "b"}], "total": 2}})
                .to_string(),
        )
        .unwrap();
        assert_eq!(collector.push(second), NextPage::Done);
    }

    #[test]
    fn an_empty_page_ends_a_read_whose_total_is_never_reached() {
        let mut collector = Collector::default();
        let page = parse_page(
            &serde_json::json!({"success": true, "data": {"data": [], "total": 100}}).to_string(),
        )
        .unwrap();
        assert_eq!(collector.push(page), NextPage::Done);
    }

    #[test]
    fn a_page_with_no_total_is_the_whole_answer() {
        let mut collector = Collector::default();
        let page = parse_page(
            &serde_json::json!({"success": true, "data": {"data": [{"id": "a"}]}}).to_string(),
        )
        .unwrap();
        assert_eq!(collector.push(page), NextPage::Done);
    }

    #[test]
    fn duplicates_across_pages_are_kept_once() {
        let mut collector = Collector::default();
        let first = parse_page(
            &serde_json::json!({"success": true, "data": {"data": [{"id": "a"}], "total": 2}})
                .to_string(),
        )
        .unwrap();
        collector.push(first);
        let second = parse_page(
            &serde_json::json!({"success": true, "data": {"data": [{"id": "a"}], "total": 2}})
                .to_string(),
        )
        .unwrap();
        collector.push(second);
        let ids: Vec<String> = collector.finish().into_iter().map(|e| e.id).collect();
        assert_eq!(ids, vec!["a".to_string()]);
    }

    #[test]
    fn the_page_bound_reports_truncation_instead_of_looping() {
        let mut collector = Collector::default();
        let mut outcome = NextPage::Done;
        for i in 0..20 {
            let page = parse_page(
                &serde_json::json!({
                    "success": true,
                    "data": {"data": [{"id": format!("m{i}")}], "total": 1_000_000},
                })
                .to_string(),
            )
            .unwrap();
            outcome = collector.push(page);
        }
        assert_eq!(
            outcome,
            NextPage::Truncated {
                read: 20,
                total: 1_000_000
            }
        );
    }
}
