//! OpenAI-compatible model catalog discovery and the OpenRouter registry cache.
//!
//! Both first-run setup and the inference settings picker consume the standard
//! `{ "data": [{ "id": ... }] }` model-list shape. Keeping the fetch and parser
//! here prevents setup from knowing only about the first entry while the picker
//! grows a second interpretation of the same provider response.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// How long a successful OpenRouter catalog stays fresh in this process.
pub(crate) const MODEL_CATALOG_TTL: Duration = Duration::from_secs(60 * 60);

/// Maximum time a console page-load waits for the registry on a cache miss.
const MODEL_CATALOG_TIMEOUT: Duration = Duration::from_secs(10);

/// One model exposed to the operator console.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InferenceModel {
    /// Provider model id written unchanged into the tier mapping.
    pub(crate) id: String,
    /// Provider display name, when published.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    /// Maximum context window, when published.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) context_length: Option<u64>,
}

/// Parsed leniently: `data` is decoded as raw JSON values first, so one
/// malformed entry (a non-string `id`, a string-valued `context_length`, …)
/// only drops that entry in [`parse_models`] instead of failing the whole
/// response and hiding every valid model the endpoint actually returned.
#[derive(Deserialize)]
struct RegistryResponse {
    #[serde(default)]
    data: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct RegistryModel {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    context_length: Option<u64>,
}

/// Parse every concrete model in a standard OpenAI-compatible catalog, in the
/// order the provider listed them.
///
/// Each `data` entry is decoded on its own so a single malformed record (bad
/// field type, missing `id`, …) is skipped rather than rejecting entries this
/// same response otherwise reported cleanly.
///
/// Order is preserved rather than sorted here: `src/server/setup.rs`'s probe
/// path takes `.next()` off this list to pick a local/custom endpoint's
/// leading model, the same thing the pre-catalog `discover_local_model` did
/// by taking the provider's first array entry. Sorting only matters for the
/// operator-facing OpenRouter catalog, so [`openrouter_models`] sorts its own
/// copy before caching it rather than this shared parser reordering every
/// caller's result.
fn parse_models(payload: RegistryResponse) -> Vec<InferenceModel> {
    let mut seen = std::collections::HashSet::new();
    let mut models = Vec::new();
    for entry in payload.data {
        let Ok(model) = serde_json::from_value::<RegistryModel>(entry) else {
            continue;
        };
        let id = model.id.trim();
        if id.is_empty() || !seen.insert(id.to_string()) {
            continue;
        }
        models.push(InferenceModel {
            id: id.to_string(),
            name: model
                .name
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty()),
            context_length: model.context_length,
        });
    }
    models
}

/// Fetch every model from an OpenAI-compatible `{base_url}/models` endpoint.
///
/// `bearer` is used by local/custom setup probes; OpenRouter's public registry
/// passes `None`.
pub(crate) async fn discover_models(
    base_url: &str,
    bearer: Option<&str>,
) -> Result<Vec<InferenceModel>, String> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    // Bounded here, not left to each caller: reqwest's async client has no
    // default timeout, so an endpoint that accepts the connection but never
    // responds would otherwise hold this open indefinitely. `setup.rs`'s
    // local/custom probe calls this directly (no wrapping timeout of its
    // own), while `openrouter_models` below also wraps its call in
    // `tokio::time::timeout` for a friendlier, registry-specific message.
    let client = reqwest::Client::builder()
        .timeout(MODEL_CATALOG_TIMEOUT)
        .build()
        .map_err(|error| format!("failed to build the model-discovery client: {error}"))?;
    let mut request = client.get(&url);
    if let Some(bearer) = bearer.filter(|bearer| !bearer.trim().is_empty()) {
        request = request.bearer_auth(bearer);
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("request to {url} failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("request to {url} failed: {error}"))?;
    let payload = response
        .json::<RegistryResponse>()
        .await
        .map_err(|error| format!("model catalog from {url} was invalid: {error}"))?;
    Ok(parse_models(payload))
}

struct CacheEntry {
    at: Instant,
    models: Vec<InferenceModel>,
}

/// Process-wide OpenRouter catalog cache.
#[derive(Default)]
pub(crate) struct ModelCatalogCache {
    entry: Mutex<Option<CacheEntry>>,
}

impl ModelCatalogCache {
    pub(crate) fn lookup(&self, now: Instant) -> Option<Vec<InferenceModel>> {
        let entry = self.entry.lock().ok()?;
        let entry = entry.as_ref()?;
        (now.saturating_duration_since(entry.at) < MODEL_CATALOG_TTL).then(|| entry.models.clone())
    }

    pub(crate) fn store(&self, models: Vec<InferenceModel>, at: Instant) {
        if let Ok(mut entry) = self.entry.lock() {
            *entry = Some(CacheEntry { at, models });
        }
    }
}

pub(crate) fn openrouter_cache() -> &'static ModelCatalogCache {
    static CACHE: OnceLock<ModelCatalogCache> = OnceLock::new();
    CACHE.get_or_init(ModelCatalogCache::default)
}

/// Return the cached OpenRouter catalog, fetching it on a miss.
pub(crate) async fn openrouter_models() -> Result<Vec<InferenceModel>, String> {
    let now = Instant::now();
    if let Some(models) = openrouter_cache().lookup(now) {
        return Ok(models);
    }

    let fetch = discover_models(crate::company::inference::OPENROUTER_BASE_URL, None);
    let mut models = tokio::time::timeout(MODEL_CATALOG_TIMEOUT, fetch)
        .await
        .map_err(|_| {
            "OpenRouter's model registry did not answer within 10 seconds".to_string()
        })??;
    if models.is_empty() {
        return Err("OpenRouter's model registry returned no models".to_string());
    }
    // Sorted here, not in `parse_models`: this is the operator-facing
    // catalog picker's own copy, while `parse_models` also serves
    // `setup.rs`'s local/custom probe, which relies on provider order.
    models.sort_by(|a, b| a.id.cmp(&b.id));
    openrouter_cache().store(models.clone(), now);
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str) -> InferenceModel {
        InferenceModel {
            id: id.to_string(),
            name: None,
            context_length: None,
        }
    }

    #[test]
    fn catalog_parser_returns_every_non_empty_unique_model_in_provider_order() {
        let parsed = parse_models(RegistryResponse {
            data: vec![
                serde_json::json!({"id": " vendor/zeta ", "name": " Zeta ", "context_length": 128_000}),
                serde_json::json!({"id": "vendor/alpha"}),
                serde_json::json!({"id": "vendor/zeta", "name": "duplicate"}),
                serde_json::json!({"id": "   "}),
            ],
        });

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].id, "vendor/zeta");
        assert_eq!(parsed[0].name.as_deref(), Some("Zeta"));
        assert_eq!(parsed[0].context_length, Some(128_000));
        assert_eq!(parsed[1], model("vendor/alpha"));
    }

    /// Regression for a Minor review finding on #1838: the previous
    /// `BTreeMap`-keyed parser returned ids in lexicographic order, so
    /// `src/server/setup.rs` taking `.next()` off the result silently swapped
    /// from "the provider's first-listed model" (what the deleted
    /// `discover_local_model` returned) to "the alphabetically first model" —
    /// an arbitrary pick on any multi-model host whose ids don't already
    /// sort first-to-preferred.
    #[test]
    fn catalog_parser_does_not_alphabetize_a_local_hosts_leading_model() {
        let parsed = parse_models(RegistryResponse {
            data: vec![
                serde_json::json!({"id": "zephyr-preferred"}),
                serde_json::json!({"id": "alpaca-not-preferred"}),
            ],
        });

        assert_eq!(
            parsed.first().map(|m| m.id.as_str()),
            Some("zephyr-preferred"),
            "the provider's leading model must survive `.next()` in setup.rs, not lose to sort order"
        );
    }

    /// Regression for a P2 review finding on #1838: a single malformed
    /// record (here, a numeric `id`) used to fail `RegistryResponse`
    /// deserialization outright — `discover_models` never reached
    /// `parse_models` at all, so a valid model earlier or later in the same
    /// `data` array was lost with it. `data` is now decoded as raw JSON
    /// first, so only the bad entry drops out.
    #[test]
    fn catalog_parser_skips_malformed_entries_instead_of_rejecting_the_response() {
        let payload: RegistryResponse = serde_json::from_str(
            r#"{"data": [
                {"id": "vendor/good-one"},
                {"id": 12345},
                {"id": "vendor/good-two", "context_length": "not-a-number"},
                {"id": "vendor/good-three"}
            ]}"#,
        )
        .expect("RegistryResponse itself must still deserialize leniently");

        let parsed = parse_models(payload);

        let ids: Vec<&str> = parsed.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["vendor/good-one", "vendor/good-three"]);
    }

    #[test]
    fn cache_serves_only_fresh_catalogs() {
        let cache = ModelCatalogCache::default();
        let stored_at = Instant::now();
        cache.store(vec![model("vendor/model")], stored_at);

        assert_eq!(
            cache.lookup(stored_at + MODEL_CATALOG_TTL - Duration::from_secs(1)),
            Some(vec![model("vendor/model")])
        );
        assert_eq!(cache.lookup(stored_at + MODEL_CATALOG_TTL), None);
    }
}
