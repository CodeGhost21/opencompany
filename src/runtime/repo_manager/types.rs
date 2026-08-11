//! The value types the repository binding surface is made of: what an operator
//! submits, what is persisted, what is handed back, and the host seam the
//! GitHub REST calls go through.
//!
//! Everything here is credential-aware by construction. [`BindRequest`] is the
//! only type that carries a token, it is never serialized, and its `Debug`
//! redacts. [`RepoBinding`] — the shape that is persisted and listed — carries a
//! [fingerprint](RepoBinding::token_fingerprint) instead, so the console can say
//! *which* credential is installed and notice a rotation without any read path
//! ever loading token bytes.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::Result;
use crate::error::OpenCompanyError;

/// The only forge this tier speaks to. Kept as a constant rather than inlined
/// so the check and the error message cannot drift apart.
pub const SUPPORTED_HOST: &str = "github.com";

/// The prefix GitHub gives a **classic** personal access token.
///
/// Classic PATs are scoped to *every* repository the user can reach, which is
/// the blast radius this whole surface exists to avoid, so the bind route
/// refuses one outright. See [`classify_token`].
const CLASSIC_PAT_PREFIX: &str = "ghp_";

/// The prefix GitHub gives a **fine-grained** personal access token — the one
/// this tier is designed around (single repository, read-only contents).
const FINE_GRAINED_PAT_PREFIX: &str = "github_pat_";

/// Where a repository is bound from, parsed out of the submitted URL.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoCoordinates {
    /// The owning user or organization.
    pub owner: String,
    /// The repository name, without the `.git` suffix.
    pub repo: String,
}

impl RepoCoordinates {
    /// The canonical `https://github.com/<owner>/<repo>` form. Deliberately
    /// rebuilt from the parsed parts rather than echoing the submitted string:
    /// what is persisted is then exactly what was validated.
    pub fn canonical_url(&self) -> String {
        format!("https://{SUPPORTED_HOST}/{}/{}", self.owner, self.repo)
    }

    /// The stable cache key (`owner-repo`, lowercased). Used as a single path
    /// component under the mirror cache and as the id in the REST surface, so
    /// it is constrained to `[a-z0-9-]` — see [`parse_repo_url`], which is the
    /// only constructor.
    pub fn key(&self) -> String {
        format!("{}-{}", slugify(&self.owner), slugify(&self.repo))
    }
}

/// Lowercases and replaces every character outside `[a-z0-9]` with `-`.
fn slugify(part: &str) -> String {
    part.chars()
        .map(|c| {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_alphanumeric() { c } else { '-' }
        })
        .collect()
}

/// Parses and validates a submitted repository URL.
///
/// Deliberately strict, and strict in one direction: the only accepted shape is
/// an `https://github.com/<owner>/<repo>` with nothing else in it. Anything
/// that is not that — another forge, `http`, `ssh`, a userinfo section, a
/// port, a path with extra segments, a `..` — is refused rather than
/// normalized. A URL is the thing a credential is about to be sent to, so the
/// parser's job is to be boring, not accommodating.
pub fn parse_repo_url(raw: &str) -> Result<RepoCoordinates> {
    let refuse = |why: &str| {
        Err(OpenCompanyError::InvalidRequest(format!(
            "repository URL must be https://{SUPPORTED_HOST}/<owner>/<repo> — {why}"
        )))
    };
    let trimmed = raw.trim();
    let Some(rest) = trimmed.strip_prefix("https://") else {
        return refuse("only https is accepted");
    };
    // A userinfo section would smuggle a credential into the stored URL.
    if rest.contains('@') {
        return refuse("the URL must not carry a username or password");
    }
    let Some((host, path)) = rest.split_once('/') else {
        return refuse("no repository path");
    };
    let host = host.to_ascii_lowercase();
    if host != SUPPORTED_HOST && host != format!("www.{SUPPORTED_HOST}") {
        return refuse(&format!(
            "this tier only binds repositories on {SUPPORTED_HOST}"
        ));
    }
    // Query strings and fragments are not part of a clone URL.
    if path.contains('?') || path.contains('#') {
        return refuse("the URL must carry no query string or fragment");
    }
    let path = path.trim_end_matches('/');
    let Some((owner, repo)) = path.split_once('/') else {
        return refuse("no repository name");
    };
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    if repo.contains('/') {
        return refuse("the path must be exactly <owner>/<repo>");
    }
    // GitHub's own rule for both parts, restated so a name that would escape a
    // path component (`.`, `..`, a slash, a NUL) cannot reach the filesystem.
    let ok = |part: &str| {
        !part.is_empty()
            && part.len() <= 100
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
            && part != "."
            && part != ".."
    };
    if !ok(owner) || !ok(repo) {
        return refuse("owner and repository may only contain letters, digits, '-', '_' and '.'");
    }
    Ok(RepoCoordinates {
        owner: owner.to_string(),
        repo: repo.to_string(),
    })
}

/// What kind of credential was submitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    /// A fine-grained PAT (`github_pat_…`) — what this tier is designed for.
    FineGrained,
    /// A classic PAT (`ghp_…`) — refused.
    Classic,
    /// Anything else: a GitHub App installation token (`ghs_…`), or a shape
    /// GitHub has not shipped yet. Accepted, because installation tokens are
    /// already repository-scoped and refusing an unrecognized prefix would
    /// break this surface the next time GitHub changes its token format.
    Other,
}

/// Classifies a submitted credential by its prefix.
pub fn classify_token(token: &str) -> TokenKind {
    if token.starts_with(FINE_GRAINED_PAT_PREFIX) {
        TokenKind::FineGrained
    } else if token.starts_with(CLASSIC_PAT_PREFIX) {
        TokenKind::Classic
    } else {
        TokenKind::Other
    }
}

/// A non-reversible, non-secret identifier for a stored credential: the first
/// 12 hex characters of its SHA-256.
///
/// This is what makes the read surface useful without making it dangerous. An
/// operator can see that the token changed (a rotation landed) and that two
/// bindings share a credential, and a support conversation can name a token
/// without anyone pasting one. Twelve characters is far too short to attack a
/// 40-plus-character random token by brute force, and far too short to be
/// mistaken for one.
pub fn fingerprint(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    digest.iter().take(6).map(|b| format!("{b:02x}")).collect()
}

/// What an operator submits to bind a repository. **Carries the credential**,
/// so it is deliberately not `Serialize`: there is no way to accidentally write
/// one into a response body, a log line or the index document.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BindRequest {
    /// The repository URL (`https://github.com/<owner>/<repo>`).
    pub url: String,
    /// A fine-grained, read-only, single-repository personal access token.
    pub token: String,
    /// Branches to mirror. Empty means the repository's default branch, which
    /// is resolved from the remote rather than guessed.
    #[serde(default)]
    pub branches: Vec<String>,
}

impl std::fmt::Debug for BindRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BindRequest")
            .field("url", &self.url)
            .field("token", &"<redacted>")
            .field("branches", &self.branches)
            .finish()
    }
}

/// One bound repository, as persisted in the index document and returned by
/// every read. **Never carries a token** — only its
/// [fingerprint](Self::token_fingerprint).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoBinding {
    /// The cache key — a single safe path component, and the REST path id.
    pub key: String,
    /// The canonical `https://github.com/<owner>/<repo>` URL.
    pub url: String,
    /// The owning user or organization.
    pub owner: String,
    /// The repository name.
    pub repo: String,
    /// The branches the mirror fetches. Never empty once bound: an empty
    /// submission is resolved to the remote's default branch at bind time.
    pub branches: Vec<String>,
    /// A non-secret identifier for the stored credential. See [`fingerprint`].
    pub token_fingerprint: String,
    /// When the mirror last completed a fetch, in epoch milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_fetched_millis: Option<u64>,
    /// The mirror's size on disk after the last fetch, in bytes.
    #[serde(default)]
    pub size_bytes: u64,
    /// When the binding was created, in epoch milliseconds.
    pub bound_at_millis: u64,
}

/// The persisted index: every binding for one company, in one document.
///
/// Mirrors the `mcp/servers` runtime index — one JSON document under a
/// well-known [`SecretStore`](crate::ports::SecretStore) key, with the actual
/// credentials in sibling keys. That split is the point: listing bindings reads
/// this document and nothing else, so a read path has no token bytes to leak
/// even if it wanted to.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RepoIndex {
    /// The bound repositories, in bind order.
    #[serde(default)]
    pub bindings: Vec<RepoBinding>,
}

/// A repository's non-secret metadata, as the forge advertises it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoMeta {
    /// The repository's default branch.
    pub default_branch: String,
    /// The size the forge advertises, in kilobytes. Checked against the quota
    /// *before* the first fetch — advisory only, because a packfile or LFS bomb
    /// does not have to match what is advertised, which is why there is a
    /// post-fetch cap as well.
    pub size_kb: u64,
}

/// A pull request's metadata plus its unified diff.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestView {
    /// The pull request number.
    pub number: u64,
    /// Its title.
    pub title: String,
    /// Its state (`open` / `closed`).
    pub state: String,
    /// The head commit SHA.
    pub head_sha: String,
    /// The branch it merges into.
    pub base_ref: String,
    /// The unified diff, as `application/vnd.github.diff` returns it.
    pub diff: String,
}

/// The forge REST seam.
///
/// Dependency-inverted for the same reason as the DNS resolver and the mail
/// sender on the connections surface: the default build must make no network
/// call, and every test in this module must run with no network at all. The
/// real implementation is `HttpRepoHost`, compiled only under the `github`
/// feature; without one the manager reports the PR surface as not wired and
/// skips the advertised-size pre-check.
#[async_trait]
pub trait RepoHost: Send + Sync {
    /// Non-secret repository metadata. Also the bind-time credential probe: a
    /// token that cannot read this cannot read anything.
    async fn repo_meta(&self, coords: &RepoCoordinates, token: &str) -> Result<RepoMeta>;

    /// One pull request's metadata and unified diff.
    async fn pull_request(
        &self,
        coords: &RepoCoordinates,
        number: u64,
        token: &str,
    ) -> Result<PullRequestView>;
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn accepts_a_plain_github_url() {
        let coords = parse_repo_url("https://github.com/tinyhumansai/opencompany").unwrap();
        assert_eq!(coords.owner, "tinyhumansai");
        assert_eq!(coords.repo, "opencompany");
        assert_eq!(coords.key(), "tinyhumansai-opencompany");
        assert_eq!(
            coords.canonical_url(),
            "https://github.com/tinyhumansai/opencompany"
        );
    }

    #[test]
    fn tolerates_the_git_suffix_and_a_trailing_slash() {
        for raw in [
            "https://github.com/acme/widgets.git",
            "https://github.com/acme/widgets/",
            "  https://www.github.com/acme/widgets  ",
        ] {
            let coords = parse_repo_url(raw).unwrap_or_else(|e| panic!("{raw}: {e}"));
            assert_eq!(coords.repo, "widgets", "{raw}");
            assert_eq!(coords.owner, "acme", "{raw}");
        }
    }

    #[test]
    fn refuses_everything_that_is_not_a_bare_github_https_url() {
        for raw in [
            "http://github.com/acme/widgets",
            "git@github.com:acme/widgets.git",
            "ssh://git@github.com/acme/widgets",
            "https://gitlab.com/acme/widgets",
            "https://user:pw@github.com/acme/widgets",
            "https://github.com/acme",
            "https://github.com/acme/widgets/tree/main",
            "https://github.com/../../etc/passwd",
            "https://github.com/acme/widgets?token=x",
            "https://github.com//widgets",
        ] {
            assert!(parse_repo_url(raw).is_err(), "{raw} should be refused");
        }
    }

    #[test]
    fn a_parsed_key_is_always_one_safe_path_component() {
        let coords = parse_repo_url("https://github.com/Acme.Corp/My_Widgets.v2").unwrap();
        let key = coords.key();
        assert_eq!(key, "acme-corp-my-widgets-v2");
        assert!(
            key.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "{key} must be [a-z0-9-]"
        );
    }

    #[test]
    fn classic_pats_are_distinguishable_from_fine_grained_ones() {
        assert_eq!(classify_token("ghp_abcdef"), TokenKind::Classic);
        assert_eq!(
            classify_token("github_pat_11ABCDEF_xyz"),
            TokenKind::FineGrained
        );
        assert_eq!(classify_token("ghs_installation"), TokenKind::Other);
    }

    #[test]
    fn a_fingerprint_identifies_without_revealing() {
        let fp = fingerprint("SENTINEL");
        assert_eq!(fp.len(), 12);
        assert!(!fp.contains("SENTINEL"));
        assert_eq!(fp, fingerprint("SENTINEL"), "stable for one token");
        assert_ne!(fp, fingerprint("SENTINEL2"), "a rotation is visible");
    }

    #[test]
    fn a_bind_request_never_debug_prints_its_token() {
        let req = BindRequest {
            url: "https://github.com/acme/widgets".into(),
            token: "github_pat_SENTINEL".into(),
            branches: vec![],
        };
        let rendered = format!("{req:?}");
        assert!(!rendered.contains("SENTINEL"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
    }

    #[test]
    fn a_binding_serializes_without_credential_material() {
        let binding = RepoBinding {
            key: "acme-widgets".into(),
            url: "https://github.com/acme/widgets".into(),
            owner: "acme".into(),
            repo: "widgets".into(),
            branches: vec!["main".into()],
            token_fingerprint: fingerprint("github_pat_SENTINEL"),
            last_fetched_millis: Some(1),
            size_bytes: 10,
            bound_at_millis: 1,
        };
        let json = serde_json::to_string(&binding).unwrap();
        assert!(!json.contains("SENTINEL"), "{json}");
        assert!(json.contains("tokenFingerprint"), "{json}");
    }
}
