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

    /// The stable cache key: a readable slug plus a short hash of the
    /// coordinates. Used as a single path component under the mirror cache and
    /// as the id in the REST surface, so it is constrained to `[a-z0-9-]` — see
    /// [`parse_repo_url`], which is the only constructor.
    ///
    /// **The hash is not decoration.** The slug alone is not injective, and this
    /// key names the credential: `repos/token/<key>`, the mirror directory, the
    /// index entry and the revoke route. Owner and repository names legitimately
    /// carry `.`, `_` and `-`, all of which the slug flattens to `-`, so
    /// `acme/data.pipeline`, `acme/data_pipeline` and `acme/data-pipeline` all
    /// slug to `acme-data-pipeline` — and so do `a-b/c` and `a/b-c`, from
    /// opposite sides of the separator. Two different repositories sharing one
    /// key means one operator's token is used to fetch the other's source.
    ///
    /// [`repo_identity`] is what restores the distinction: it hashes the
    /// coordinates *before* any flattening, over a string only one pair of
    /// coordinates can produce.
    ///
    /// Bounded by construction: [`parse_repo_url`] caps each part at 100
    /// characters, so a key is at most 214 bytes and `<key>.git` at most 218 —
    /// inside every filesystem's component limit.
    pub fn key(&self) -> String {
        format!(
            "{}-{}-{}",
            slugify(&self.owner),
            slugify(&self.repo),
            repo_identity(&self.owner, &self.repo)
        )
    }
}

/// Lowercases and replaces every character outside `[a-z0-9]` with `-`.
///
/// Lossy on purpose — this is the *readable* half of a key, and a mirror
/// directory an operator may have to look at should be legible. [`key`] carries
/// the distinguishing half separately.
///
/// [`key`]: RepoCoordinates::key
fn slugify(part: &str) -> String {
    part.chars()
        .map(|c| {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_alphanumeric() { c } else { '-' }
        })
        .collect()
}

/// A short, stable, URL-safe identifier for one repository's coordinates.
///
/// The hashed string is `<owner>/<repo>`, ASCII-lowercased. Both properties are
/// load-bearing:
///
/// * **The `/`.** [`parse_repo_url`] refuses a `/` inside either part, so
///   exactly one pair of coordinates can produce any given string here. That is
///   what makes distinct repositories hash distinctly — `a-b/c` and `a/b-c` are
///   different inputs, where their slugs are not.
/// * **Lowercased.** GitHub resolves owner and repository case-insensitively,
///   so `Acme/Widgets` and `acme/widgets` are one repository and must land on
///   one key — otherwise binding the same repository twice, spelled differently,
///   would install two credentials and two mirrors instead of being refused as
///   the duplicate it is.
///
/// Twelve hex characters is 48 bits, which is not a security parameter and does
/// not need to be: a collision does not merge two bindings, it makes the second
/// bind fail as a [`Conflict`](crate::error::OpenCompanyError::Conflict) that
/// names the key. The failure mode is a refusal an operator can see, not a
/// credential quietly shared.
fn repo_identity(owner: &str, repo: &str) -> String {
    short_sha256(&format!(
        "{}/{}",
        owner.to_ascii_lowercase(),
        repo.to_ascii_lowercase()
    ))
}

/// The first 12 hex characters of the SHA-256 of `input`.
fn short_sha256(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    digest.iter().take(6).map(|b| format!("{b:02x}")).collect()
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
    short_sha256(token)
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
    /// Whether the bound credential can push, as the forge's `permissions.push`
    /// advertised it when last probed (issue #734). Probed at bind time and
    /// re-probed on every fetch, so a token whose scope changed heals without a
    /// re-bind.
    ///
    /// `None` means **unknown** — the binding predates this field, or no forge
    /// client was wired to probe it — and it **reads as "cannot push"**
    /// everywhere the write tier gates on it: unknown is fail-closed, never
    /// "allow". Serde-additive (`default`) so bindings written before this field
    /// existed round-trip to `None` rather than failing to load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_push: Option<bool>,
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
    /// Whether the probing credential can push to this repository, as the
    /// forge's `permissions.push` reports it (issue #734). A fine-grained
    /// read-only PAT reports `false`; absent permissions (an unauthenticated or
    /// malformed response) also read as `false` — the write tier fails closed on
    /// this, so an unknown answer must never read as "can push".
    pub can_push: bool,
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

/// A newly opened pull request, as the create call answers (issue #736): the
/// two facts an operator needs — which PR, and where to open it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestRef {
    /// The pull request number.
    pub number: u64,
    /// Its browser URL.
    pub html_url: String,
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

    /// Opens a pull request from `head` into `base`, host-side, and returns the
    /// created PR's number and browser URL (issue #736). The one *write* verb on
    /// this seam — every other reaches the forge read-only.
    async fn create_pull_request(
        &self,
        coords: &RepoCoordinates,
        token: &str,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<PullRequestRef>;
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn accepts_a_plain_github_url() {
        let coords = parse_repo_url("https://github.com/tinyhumansai/opencompany").unwrap();
        assert_eq!(coords.owner, "tinyhumansai");
        assert_eq!(coords.repo, "opencompany");
        assert_eq!(coords.key(), "tinyhumansai-opencompany-065c771079f9");
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
        assert_eq!(key, "acme-corp-my-widgets-v2-86dbcf155254");
        assert!(
            key.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "{key} must be [a-z0-9-]"
        );

        // The longest key the parser can admit still fits a path component,
        // with the mirror's `.git` suffix on it. 100 + 1 + 100 + 1 + 12 = 214.
        let long = "a".repeat(100);
        let longest = parse_repo_url(&format!("https://github.com/{long}/{long}"))
            .unwrap()
            .key();
        assert_eq!(longest.len(), 214);
        assert!(
            longest.len() + ".git".len() <= 255,
            "a mirror directory must be a legal filename: {}",
            longest.len()
        );
    }

    #[test]
    fn distinct_repositories_never_share_a_key() {
        // Every pair here slugs to one string, and the key names the *token*:
        // `repos/token/<key>`. Collapsing two repositories into one key means
        // fetching one repository under the other's credential.
        let colliding = [
            // Punctuation the slug flattens.
            "https://github.com/acme/data.pipeline",
            "https://github.com/acme/data_pipeline",
            "https://github.com/acme/data-pipeline",
            // The separator itself: `a-b/c` and `a/b-c` both slug to `a-b-c`.
            "https://github.com/a-b/c",
            "https://github.com/a/b-c",
        ];
        let keys: Vec<String> = colliding
            .iter()
            .map(|url| parse_repo_url(url).unwrap().key())
            .collect();
        let mut unique = keys.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(
            unique.len(),
            colliding.len(),
            "these repositories must not share a key: {keys:?}"
        );

        // The readable half really does collide — this is what the hash is for,
        // stated so a future edit cannot conclude the suffix is redundant.
        assert!(keys[0].starts_with("acme-data-pipeline-"), "{keys:?}");
        assert!(keys[1].starts_with("acme-data-pipeline-"), "{keys:?}");
        assert!(keys[3].starts_with("a-b-c-"), "{keys:?}");
        assert!(keys[4].starts_with("a-b-c-"), "{keys:?}");
    }

    #[test]
    fn one_repository_spelled_two_ways_is_one_key() {
        // GitHub resolves owner and repository case-insensitively, so these are
        // the same repository. They must land on one key, or a rebind installs a
        // second credential and a second mirror instead of being refused.
        assert_eq!(
            parse_repo_url("https://github.com/Acme/Widgets")
                .unwrap()
                .key(),
            parse_repo_url("https://github.com/acme/widgets")
                .unwrap()
                .key(),
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
            can_push: None,
        };
        let json = serde_json::to_string(&binding).unwrap();
        assert!(!json.contains("SENTINEL"), "{json}");
        assert!(json.contains("tokenFingerprint"), "{json}");
        // An unknown capability is omitted rather than written as `null`, so an
        // older build reading this document is unaffected.
        assert!(!json.contains("canPush"), "{json}");
    }

    /// A binding persisted before `can_push` existed (issue #734) must load with
    /// the field absent → `None` → cannot-push: the document round-trips rather
    /// than failing to deserialize, and an unknown capability never reads as
    /// "allow". Re-probe on the next fetch is what then heals it to a definite
    /// answer.
    #[test]
    fn a_binding_written_before_can_push_deserializes_as_unknown() {
        // Exactly the shape an older build wrote: no `canPush` key at all.
        let legacy = r#"{
            "key": "acme-widgets",
            "url": "https://github.com/acme/widgets",
            "owner": "acme",
            "repo": "widgets",
            "branches": ["main"],
            "tokenFingerprint": "0f1e2d3c4b5a",
            "lastFetchedMillis": 1,
            "sizeBytes": 10,
            "boundAtMillis": 1
        }"#;
        let binding: RepoBinding = serde_json::from_str(legacy).unwrap();
        assert_eq!(
            binding.can_push, None,
            "an absent capability must read as unknown (cannot-push), not a load failure"
        );
    }
}
