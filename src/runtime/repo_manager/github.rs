//! The real [`RepoHost`]: GitHub's REST API over `reqwest`.
//!
//! Gated behind the `github` feature, exactly like the feedback filer's
//! [`HttpGitHubClient`](crate::feedback::github::HttpGitHubClient) — the trait,
//! the offline fakes and every test in this module compile and run without it,
//! so the default build links no HTTP client on this path and makes no network
//! call of its own.
//!
//! Two properties are worth stating because they are easy to lose in an edit:
//!
//! * **The token is a per-call argument, not client state.** Each bound
//!   repository has its own credential, and one client instance serves every
//!   company on the host. A token cached on the struct would be the wrong
//!   token for the next caller — which on a multi-tenant host is a
//!   cross-company credential leak, not a bug.
//! * **A diff is bounded on the way in.** GitHub will happily stream a
//!   hundred-megabyte diff for a vendored-dependency bump, and nothing
//!   downstream of here wants it in memory. It is truncated with a visible
//!   marker rather than silently cut.

use async_trait::async_trait;

use super::types::{PullRequestView, RepoCoordinates, RepoHost, RepoMeta};
use crate::Result;
use crate::error::OpenCompanyError;

/// The largest diff kept in full, in bytes.
///
/// A code-review tier reading more than this is not reviewing; it is being
/// handed a vendored-dependency bump. The excess is dropped with a marker so a
/// reader can tell a truncated diff from a small one — a silent cut is how a
/// reviewer concludes a change is safe because they never saw the rest of it.
///
/// The follow-up that materializes checkouts into an agent workspace is the
/// right place to spill an oversized diff to a file instead; there is no
/// workspace at this tier to spill into.
const MAX_DIFF_BYTES: usize = 1024 * 1024;

/// How long a single GitHub request may take.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// A [`RepoHost`] backed by the GitHub REST API.
pub struct HttpRepoHost {
    http: reqwest::Client,
    /// The API base, so tests (and GitHub Enterprise, later) can point it
    /// somewhere else.
    base: String,
}

impl Default for HttpRepoHost {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpRepoHost {
    /// Builds a client against `api.github.com`.
    pub fn new() -> Self {
        Self::with_base("https://api.github.com")
    }

    /// Builds a client against an explicit API base.
    pub fn with_base(base: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .user_agent("opencompany")
                .build()
                .unwrap_or_default(),
            base: base.into().trim_end_matches('/').to_string(),
        }
    }

    /// Issues an authenticated GET and returns the body as text.
    ///
    /// Failures are mapped by *what the operator can do about them*, which is
    /// why 401/403/404 all become an `InvalidRequest` naming the credential:
    /// GitHub deliberately answers 404 for a repository a fine-grained token
    /// has not been granted, so "not found" and "not permitted" are the same
    /// message to the person holding the token — and telling them the
    /// repository does not exist, when the truth is that they forgot to tick
    /// Contents, is the single most expensive wrong error this surface could
    /// give.
    async fn get(&self, url: &str, token: &str, accept: &str) -> Result<String> {
        let response = self
            .http
            .get(url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", accept)
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|e| OpenCompanyError::Store(format!("could not reach the GitHub API: {e}")))?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED
            || status == reqwest::StatusCode::FORBIDDEN
            || status == reqwest::StatusCode::NOT_FOUND
        {
            return Err(OpenCompanyError::InvalidRequest(
                "GitHub refused that credential for this repository. A fine-grained token \
                 answers 404 for a repository it was not granted, so check that the token \
                 lists this repository and has read access to Contents, Metadata and Pull \
                 requests — and that it has not expired."
                    .to_string(),
            ));
        }
        if !status.is_success() {
            return Err(OpenCompanyError::Store(format!(
                "the GitHub API answered {status}"
            )));
        }
        // Deliberately not `response.text()`: that buffers whatever arrives.
        let bytes = response
            .bytes()
            .await
            .map_err(|e| OpenCompanyError::Store(format!("reading the GitHub response: {e}")))?;
        Ok(truncate_utf8(&bytes, MAX_DIFF_BYTES))
    }
}

/// Decodes at most `limit` bytes, cutting on a character boundary and marking
/// the cut.
///
/// The boundary walk is not decoration: slicing a byte buffer at a fixed offset
/// splits a multi-byte character about one time in four on any diff containing
/// non-ASCII, and `String::from_utf8_lossy` would then quietly substitute a
/// replacement character into source code a reviewer is reading.
fn truncate_utf8(bytes: &[u8], limit: usize) -> String {
    if bytes.len() <= limit {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let mut end = limit;
    while end > 0 && (bytes[end] & 0b1100_0000) == 0b1000_0000 {
        end -= 1;
    }
    let mut out = String::from_utf8_lossy(&bytes[..end]).into_owned();
    out.push_str(&format!(
        "\n\n[truncated: {} more bytes not shown]\n",
        bytes.len() - end
    ));
    out
}

#[async_trait]
impl RepoHost for HttpRepoHost {
    async fn repo_meta(&self, coords: &RepoCoordinates, token: &str) -> Result<RepoMeta> {
        let url = format!("{}/repos/{}/{}", self.base, coords.owner, coords.repo);
        let body = self.get(&url, token, "application/vnd.github+json").await?;
        let json: serde_json::Value = serde_json::from_str(&body)?;
        Ok(RepoMeta {
            default_branch: json
                .get("default_branch")
                .and_then(|v| v.as_str())
                .unwrap_or("main")
                .to_string(),
            size_kb: json.get("size").and_then(|v| v.as_u64()).unwrap_or(0),
        })
    }

    async fn pull_request(
        &self,
        coords: &RepoCoordinates,
        number: u64,
        token: &str,
    ) -> Result<PullRequestView> {
        let url = format!(
            "{}/repos/{}/{}/pulls/{number}",
            self.base, coords.owner, coords.repo
        );
        // Two calls rather than one: the diff media type returns a diff and
        // nothing else, so the metadata has to come from the JSON representation
        // of the same resource.
        let meta = self.get(&url, token, "application/vnd.github+json").await?;
        let meta: serde_json::Value = serde_json::from_str(&meta)?;
        let diff = self.get(&url, token, "application/vnd.github.diff").await?;
        let string_at = |path: &[&str]| -> String {
            let mut node = &meta;
            for key in path {
                match node.get(key) {
                    Some(next) => node = next,
                    None => return String::new(),
                }
            }
            node.as_str().unwrap_or_default().to_string()
        };
        Ok(PullRequestView {
            number,
            title: string_at(&["title"]),
            state: string_at(&["state"]),
            head_sha: string_at(&["head", "sha"]),
            base_ref: string_at(&["base", "ref"]),
            diff,
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn a_short_body_is_returned_whole() {
        assert_eq!(truncate_utf8(b"hello", 1024), "hello");
    }

    #[test]
    fn an_oversized_body_is_cut_on_a_character_boundary_and_says_so() {
        // A three-byte character straddling the limit is the case that matters:
        // cutting mid-character would put a replacement character into source
        // a reviewer reads as real.
        let body = "€".repeat(100);
        let out = truncate_utf8(body.as_bytes(), 10);
        assert!(out.contains("[truncated:"), "{out}");
        assert!(!out.contains('\u{FFFD}'), "cut mid-character: {out}");
        let shown = out.split("\n\n[truncated:").next().unwrap();
        assert_eq!(shown.len() % 3, 0, "cut on a character boundary");
    }

    #[test]
    fn the_api_base_is_normalized() {
        let host = HttpRepoHost::with_base("https://ghe.example.test/api/v3/");
        assert_eq!(host.base, "https://ghe.example.test/api/v3");
    }
}
