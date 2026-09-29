//! What an MCP server says about itself: the `serverInfo` block of its
//! `initialize` reply, kept per server beside its health record.
//!
//! The protocol leaves `serverInfo` open-ended, so every field here is optional
//! and coverage is patchy in practice — Context7 answers with a title, a
//! description, a website and icons, DeepWiki with none of them. Absent has to
//! stay representable: a placeholder would be this host asserting something the
//! server never said.
//!
//! ## Icons never become a request from the operator's browser
//!
//! An icon URL is chosen by whoever runs the remote server. Put in an `src=`
//! attribute it is a beacon that fires for every operator who opens the
//! Connections page and reports to that host who looked and when — the same
//! reasoning that closed the avatar grammar to URLs
//! ([`super::avatar`]). So the host fetches the icon itself, during the probe it
//! already performs, and stores the bytes inline as a `data:` URI: rendering one
//! reaches nothing at all, which is the strongest form of "served from our own
//! origin". A fetch that fails leaves [`McpServerInfo::icon_data_url`] `None`
//! and the console draws its letter tile.
//!
//! [`load`] therefore refuses a stored `icon_data_url` that is not a `data:`
//! image, so a tampered store cannot turn the field back into a remote URL.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Result;
use crate::error::OpenCompanyError;
use crate::ports::SecretStore;
use crate::ports::types::{CompanyId, SecretValue};

/// The longest stored title.
const MAX_TITLE_CHARS: usize = 120;

/// The longest stored description. Sized for the sentence a console renders
/// under a server's name, not for prose a server would like to send.
const MAX_DESCRIPTION_CHARS: usize = 400;

/// The largest icon the host will fetch and store.
///
/// The console draws these at tens of pixels, so this is generous for the job
/// and mean for anything else — the bytes ride inline in the Connections read,
/// which is the reason the ceiling is far below
/// [`MAX_AVATAR_BYTES`](super::avatar::MAX_AVATAR_BYTES).
pub const MAX_ICON_BYTES: usize = 64 * 1024;

/// How long the host waits for an icon before giving up on it.
#[cfg(feature = "mcp")]
const ICON_FETCH_TIMEOUT_SECS: u64 = 5;

/// The [`SecretStore`] key holding a declared server's [`McpServerInfo`].
pub fn server_info_key(name: &str) -> String {
    format!("mcp/{name}/server_info")
}

/// What a server said about itself on its last successful handshake.
///
/// Every field is absent-by-default, and absent means the server did not say.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerInfo {
    /// The display name the server prefers for itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The server's own description of what it does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The server's home page, as an `http(s)` URL. Rendered as a link, never
    /// fetched by this host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub website_url: Option<String>,
    /// The server's icon, inlined as a `data:` URI by [`fetch_icon`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_data_url: Option<String>,
}

impl McpServerInfo {
    /// Whether the server said anything worth storing.
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.description.is_none()
            && self.website_url.is_none()
            && self.icon_data_url.is_none()
    }
}

/// Reads the `title`, `description` and `websiteUrl` a server reported.
///
/// The icon is not filled here — it takes a network fetch, which [`fetch_icon`]
/// performs against the URL [`icon_source`] picks out.
pub fn from_server_info(server_info: &Value) -> McpServerInfo {
    McpServerInfo {
        title: text(server_info, "title", MAX_TITLE_CHARS),
        description: text(server_info, "description", MAX_DESCRIPTION_CHARS),
        website_url: server_info
            .get("websiteUrl")
            .and_then(Value::as_str)
            .and_then(http_url),
        icon_data_url: None,
    }
}

/// The first `http(s)` icon source a server advertised, if any.
///
/// `icons` is an array of objects carrying a `src`; the first usable one wins
/// rather than the largest, because the sizes a server declares are its own
/// claim and every accepted icon is capped on the way in regardless.
pub fn icon_source(server_info: &Value) -> Option<String> {
    server_info
        .get("icons")?
        .as_array()?
        .iter()
        .filter_map(|icon| icon.get("src").and_then(Value::as_str))
        .find_map(http_url)
}

/// One field of the remote block, bounded and stripped of control characters.
///
/// Remote text reaches an operator's screen from here, so the length ceiling is
/// the host's rather than the server's, and a control character — which could
/// reorder or overwrite what is rendered around it — is dropped rather than
/// escaped.
fn text(server_info: &Value, field: &str, max_chars: usize) -> Option<String> {
    let raw = server_info.get(field)?.as_str()?;
    let cleaned: String = raw
        .chars()
        .filter(|c| !c.is_control())
        .take(max_chars)
        .collect();
    let trimmed = cleaned.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// A URL the console may render as a link: `http(s)` and nothing else.
///
/// `javascript:` and `data:` are the reason this is a scheme allow-list rather
/// than a check for what is obviously hostile.
fn http_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.chars().any(char::is_whitespace) {
        return None;
    }
    (trimmed.starts_with("https://") || trimmed.starts_with("http://")).then(|| trimmed.to_string())
}

/// Whether a stored `icon_data_url` is what this module writes.
fn is_inline_image(value: &str) -> bool {
    value.starts_with("data:image/") && value.contains(";base64,")
}

/// Reads a server's stored self-description, or the empty one when it has never
/// been probed.
///
/// A malformed record degrades to the empty description rather than erroring —
/// a missing subtitle is never worth failing a status read over. A stored
/// `icon_data_url` that is not an inline image is dropped on the way out, so the
/// field the console puts in an `src=` cannot have become a remote URL.
pub async fn load(company: &CompanyId, name: &str, secrets: &dyn SecretStore) -> McpServerInfo {
    let raw = match secrets.get(company, &server_info_key(name)).await {
        Ok(Some(SecretValue(raw))) => raw,
        Ok(None) => return McpServerInfo::default(),
        Err(err) => {
            tracing::warn!(
                company = %company,
                server = %name,
                error = %err,
                "reading MCP server info failed; this server describes itself with nothing"
            );
            return McpServerInfo::default();
        }
    };
    if raw.trim().is_empty() {
        return McpServerInfo::default();
    }
    let mut info: McpServerInfo = serde_json::from_str(&raw).unwrap_or_default();
    if !info
        .icon_data_url
        .as_deref()
        .is_none_or(|value| is_inline_image(value))
    {
        info.icon_data_url = None;
    }
    info
}

/// Persists a server's self-description.
pub async fn save(
    company: &CompanyId,
    name: &str,
    info: &McpServerInfo,
    secrets: &dyn SecretStore,
) -> Result<()> {
    let raw = serde_json::to_string(info)
        .map_err(|e| OpenCompanyError::Store(format!("serializing mcp server info: {e}")))?;
    secrets
        .set(company, &server_info_key(name), SecretValue(raw))
        .await
}

/// Fetches `url` and returns it as an inline `data:` image, or `None`.
///
/// Four things bound what a remote server can make this host do: the SSRF guard
/// the outbound tools apply (so an icon cannot address the instance's own
/// network or a cloud metadata endpoint), no redirect following (so a public URL
/// cannot hand the request onward to a private one), a byte ceiling read off the
/// body rather than off the declared length, and a media type taken from the
/// bytes' own signature rather than from the `Content-Type` the server claimed.
/// Anything that fails one of them yields `None`, and the console falls back to
/// its letter tile.
#[cfg(feature = "mcp")]
pub async fn fetch_icon(url: &str) -> Option<String> {
    use futures::StreamExt;

    if openhuman_core::tools::validate_url(url, &[]).is_err() {
        return None;
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(ICON_FETCH_TIMEOUT_SECS))
        .build()
        .ok()?;
    let response = client.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let mut bytes: Vec<u8> = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        bytes.extend_from_slice(&chunk.ok()?);
        if bytes.len() > MAX_ICON_BYTES {
            return None;
        }
    }
    inline_image(&bytes)
}

/// Fetched bytes as the inline image they will be stored and served as, or
/// `None` when they are not an image this host will re-serve.
///
/// The media type comes from the bytes' own signature rather than from the
/// `Content-Type` the server claimed, because what is claimed is a claim and
/// what is served has to be a fact — and because the one format that could
/// carry script, SVG, has no signature to match and lands here as `None`
/// whatever it was labelled. The decoded size is held to the avatar
/// decompression-bomb check, so a header promising 65535×65535 in a few hundred
/// bytes is refused rather than handed to every operator who opens the page.
#[cfg(feature = "mcp")]
fn inline_image(bytes: &[u8]) -> Option<String> {
    use base64::Engine;

    if bytes.len() > MAX_ICON_BYTES {
        return None;
    }
    let media_type = super::avatar::sniff_image(bytes)?;
    super::avatar::check_image_dimensions(bytes).ok()?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Some(format!("data:{media_type};base64,{encoded}"))
}

/// Without the `mcp` feature no agent in the build can call an MCP server, so
/// there is no console row to draw an icon on and nothing to fetch.
#[cfg(not(feature = "mcp"))]
pub async fn fetch_icon(_url: &str) -> Option<String> {
    None
}

#[cfg(test)]
#[path = "mcp_server_info_tests.rs"]
mod tests;
