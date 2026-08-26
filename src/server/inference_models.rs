//! OpenAI-compatible model catalog discovery and the OpenRouter registry cache.
//!
//! Both first-run setup and the inference settings picker consume the standard
//! `{ "data": [{ "id": ... }] }` model-list shape. Keeping the fetch and parser
//! here prevents setup from knowing only about the first entry while the picker
//! grows a second interpretation of the same provider response.

use std::collections::BTreeMap;
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

#[derive(Deserialize)]
struct RegistryResponse {
    data: Vec<RegistryModel>,
}

#[derive(Deserialize)]
struct RegistryModel {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    context_length: Option<u64>,
}

/// Parse every concrete model in a standard OpenAI-compatible catalog.
fn parse_models(payload: RegistryResponse) -> Vec<InferenceModel> {
    let mut unique = BTreeMap::new();
    for model in payload.data {
        let id = model.id.trim();
        if id.is_empty() {
            continue;
        }
        unique
            .entry(id.to_string())
            .or_insert_with(|| InferenceModel {
                id: id.to_string(),
                name: model
                    .name
                    .map(|name| name.trim().to_string())
                    .filter(|name| !name.is_empty()),
                context_length: model.context_length,
            });
    }
    unique.into_values().collect()
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
    let mut request = reqwest::Client::new().get(&url);
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
    let models = tokio::time::timeout(MODEL_CATALOG_TIMEOUT, fetch)
        .await
        .map_err(|_| {
            "OpenRouter's model registry did not answer within 10 seconds".to_string()
        })??;
    if models.is_empty() {
        return Err("OpenRouter's model registry returned no models".to_string());
    }
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
    fn catalog_parser_returns_every_non_empty_unique_model() {
        let parsed = parse_models(RegistryResponse {
            data: vec![
                RegistryModel {
                    id: " vendor/zeta ".to_string(),
                    name: Some(" Zeta ".to_string()),
                    context_length: Some(128_000),
                },
                RegistryModel {
                    id: "vendor/alpha".to_string(),
                    name: None,
                    context_length: None,
                },
                RegistryModel {
                    id: "vendor/zeta".to_string(),
                    name: Some("duplicate".to_string()),
                    context_length: None,
                },
                RegistryModel {
                    id: "   ".to_string(),
                    name: None,
                    context_length: None,
                },
            ],
        });

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0], model("vendor/alpha"));
        assert_eq!(parsed[1].id, "vendor/zeta");
        assert_eq!(parsed[1].name.as_deref(), Some("Zeta"));
        assert_eq!(parsed[1].context_length, Some(128_000));
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
