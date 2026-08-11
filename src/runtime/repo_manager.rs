//! [`RepoManager`]: binding real repositories to a company, and keeping a
//! host-side mirror of each one.
//!
//! Issue #245, operator half. This module owns the whole credential path —
//! stored, used, redacted, revoked — and deliberately exposes **no agent
//! surface**: there is no `repo` grant here, no tool, and nothing that puts a
//! checkout inside an agent workspace during a turn. Splitting it there is the
//! point. A half-wired credential path is the one intermediate state this
//! feature must never ship in, so the tier that *reaches an account* lands
//! complete and alone, and the tier that *hands an agent a checkout* builds on
//! something already whole.
//!
//! ## The one layer that ships here
//!
//! **A bare mirror cache**, one per bound repository, at
//! `<data_dir>/companies/<slug>/repos/<key>.git`
//! ([`DataLayout::company_repos_dir`](crate::store::DataLayout::company_repos_dir)).
//! It is host-owned and outside every agent workspace, fetched incrementally
//! over the network with the operator's credential, and restricted to the
//! branches the binding names plus any `refs/pull/N/head` asked for by name.
//!
//! There is deliberately **no checkout layer here**. Handing an agent a working
//! tree off this cache is not a line of code, it is a confinement problem — a
//! checkout made with `git clone --shared` keeps the mirror's path in
//! `.git/objects/info/alternates` and, if `origin` is left pointing at it, a
//! plain `git push` writes straight back into the host cache. Neither removing
//! `origin` nor a same-uid `chmod` closes that: the path is readable in the
//! checkout and the pushing process runs as the user that owns the mirror. The
//! answer is an isolated object copy or a real filesystem boundary, and it
//! belongs with the tier that actually hands out a checkout. See
//! `docs/spec/runtime/repos.md`, "Not in this tier".
//!
//! ## Where this deliberately departs from the issue text
//!
//! * **Refusal, not eviction.** The issue proposes evicting a mirror that pushes
//!   the cache over quota. A bound repository is operator-configured state, and
//!   silently deleting it converts a disk problem into an inexplicable "the
//!   agent can't see our code any more" problem hours later. An over-quota fetch
//!   fails loudly and says what to do instead.
//! * **Never prune.** Mirrors are configured `gc.auto=0` and
//!   `gc.pruneExpire=never`, and space comes back only when a binding is revoked
//!   and its mirror deleted. Partly because a prune is pure risk on a cache
//!   nothing reads twice, and partly to reserve the property the checkout tier
//!   will need: an alternate object store holds no reference a `gc` in the
//!   mirror can see, so a prune here could delete objects a live checkout is
//!   resolving through.

/// Hardened host-side `git`, and the credential helper that keeps the token out
/// of argv, the environment, and every file on disk. See [`git`].
mod git;
/// The real [`RepoHost`] over GitHub's REST API. Gated behind `github`, so the
/// default build links no HTTP client here. See [`github`].
#[cfg(feature = "github")]
mod github;
#[cfg(test)]
mod test;
/// The value types this surface is made of — request, persisted binding, forge
/// seam — each credential-aware by construction. See [`types`].
pub mod types;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::Result;
use crate::error::OpenCompanyError;
use crate::ports::SecretStore;
use crate::ports::now_millis;
use crate::ports::types::{CompanyId, SecretValue};

#[cfg(feature = "github")]
pub use github::HttpRepoHost;
pub use types::{
    BindRequest, PullRequestView, RepoBinding, RepoCoordinates, RepoHost, RepoMeta, TokenKind,
};
use types::{RepoIndex, classify_token, fingerprint, parse_repo_url};

/// The [`SecretStore`] key holding the JSON index of this company's bindings.
///
/// Metadata only — never a token. Mirrors the `mcp/servers` runtime index for
/// the same reason: a list read touches this document and nothing else.
pub const REPO_INDEX_KEY: &str = "repos/bindings";

/// The [`SecretStore`] key holding one binding's credential.
pub fn repo_token_key(key: &str) -> String {
    format!("repos/token/{key}")
}

/// Binds repositories to one company and keeps their mirrors current.
///
/// One per company runtime, holding that company's secret store — so a mirror
/// and a credential are as company-scoped as everything else the runtime owns.
pub struct RepoManager {
    company: CompanyId,
    /// `<data_dir>/companies/<slug>/repos`.
    root: PathBuf,
    secrets: Arc<dyn SecretStore>,
    /// The cache cap, in bytes. `None` is unlimited — the self-hosted default,
    /// where an operator's disk is their own business.
    quota_bytes: Option<u64>,
    /// The forge REST seam. `None` in a build without it, where
    /// [`pull_request`](Self::pull_request) reports not-wired and the
    /// advertised-size pre-check is skipped.
    host: Option<Arc<dyn RepoHost>>,
    /// Serializes the index's read-modify-write. Two binds landing together
    /// would otherwise each persist a document built from a snapshot taken
    /// before the other's entry existed, and one binding would vanish.
    index_lock: Mutex<()>,
}

impl std::fmt::Debug for RepoManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RepoManager")
            .field("company", &self.company)
            .field("root", &self.root)
            .field("quota_bytes", &self.quota_bytes)
            .field("host", &self.host.is_some())
            .finish()
    }
}

impl RepoManager {
    /// Builds a manager over one company's mirror cache.
    pub fn new(
        company: CompanyId,
        root: impl Into<PathBuf>,
        secrets: Arc<dyn SecretStore>,
    ) -> Self {
        Self {
            company,
            root: root.into(),
            secrets,
            quota_bytes: None,
            host: None,
            index_lock: Mutex::new(()),
        }
    }

    /// Caps the cache. `None` leaves it unlimited.
    pub fn with_quota(mut self, quota_bytes: Option<u64>) -> Self {
        self.quota_bytes = quota_bytes;
        self
    }

    /// Injects the forge REST seam (the real client under `github`, a mock in
    /// tests).
    pub fn with_host(mut self, host: Arc<dyn RepoHost>) -> Self {
        self.host = Some(host);
        self
    }

    /// Whether a forge client is wired, i.e. whether
    /// [`pull_request`](Self::pull_request) can answer.
    pub fn has_host(&self) -> bool {
        self.host.is_some()
    }

    /// The mirror cache root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The bare mirror for one binding.
    pub fn mirror_path(&self, key: &str) -> PathBuf {
        self.root.join(format!("{key}.git"))
    }

    // -- index ---------------------------------------------------------------

    /// Reads the persisted index.
    ///
    /// A document that will not parse is reported rather than silently reset:
    /// the alternative loses every binding an operator configured and gives
    /// them a working empty list as the only evidence.
    async fn read_index(&self) -> Result<RepoIndex> {
        let Some(value) = self.secrets.get(&self.company, REPO_INDEX_KEY).await? else {
            return Ok(RepoIndex::default());
        };
        // The revoke-everything path writes an empty document rather than
        // deleting: `SecretStore` has no delete (see `ops::channels`).
        if value.expose().trim().is_empty() {
            return Ok(RepoIndex::default());
        }
        serde_json::from_str(value.expose()).map_err(|e| {
            OpenCompanyError::Store(format!(
                "the repository binding index for {} is unreadable: {e}",
                self.company
            ))
        })
    }

    async fn write_index(&self, index: &RepoIndex) -> Result<()> {
        let json = serde_json::to_string(index)?;
        self.secrets
            .set(&self.company, REPO_INDEX_KEY, SecretValue(json))
            .await
    }

    /// Every binding this company has, in bind order.
    pub async fn list(&self) -> Result<Vec<RepoBinding>> {
        Ok(self.read_index().await?.bindings)
    }

    /// One binding by key.
    pub async fn get(&self, key: &str) -> Result<RepoBinding> {
        self.list()
            .await?
            .into_iter()
            .find(|b| b.key == key)
            .ok_or_else(|| OpenCompanyError::NotFound(format!("no repository bound as {key}")))
    }

    /// The stored credential for a binding, if it has one.
    ///
    /// `None` is a real answer, not a failure: a binding whose
    /// [`token_fingerprint`](RepoBinding::token_fingerprint) is empty was made
    /// against a remote that needs no credential (a local fixture), and asking
    /// git to authenticate to it would be wrong. A binding that *does* record a
    /// fingerprint but has no bytes behind it is the broken case — a cleared
    /// secret with a live binding — and says so rather than failing later
    /// inside git with an authentication error nobody can act on.
    async fn token_for(&self, binding: &RepoBinding) -> Result<Option<String>> {
        let value = self
            .secrets
            .get(&self.company, &repo_token_key(&binding.key))
            .await?;
        let token = value.map(|v| v.expose().to_string()).unwrap_or_default();
        if !token.is_empty() {
            return Ok(Some(token));
        }
        if binding.token_fingerprint.is_empty() {
            return Ok(None);
        }
        Err(OpenCompanyError::InvalidRequest(format!(
            "the credential for {} is missing — revoke the binding and add it again",
            binding.key
        )))
    }

    // -- bind ----------------------------------------------------------------

    /// Binds a repository: validates the URL and credential, stores the token,
    /// creates the mirror and performs the first fetch.
    ///
    /// **Atomic from the operator's point of view.** Everything after the token
    /// is written is rolled back on any failure — the credential is blanked and
    /// the half-built mirror removed — so a bad token or an unreachable
    /// repository leaves no entry in the index and no secret behind it. The
    /// alternative is the state that makes this class of feature miserable: a
    /// binding that exists, cannot fetch, and holds a credential nobody
    /// remembers installing.
    pub async fn bind(&self, request: BindRequest) -> Result<RepoBinding> {
        let coords = parse_repo_url(&request.url)?;
        let key = coords.key();
        let token = request.token.trim().to_string();
        if token.is_empty() {
            return Err(OpenCompanyError::InvalidRequest(
                "a personal access token is required to bind a repository".to_string(),
            ));
        }
        if classify_token(&token) == TokenKind::Classic {
            return Err(OpenCompanyError::InvalidRequest(
                "that is a classic personal access token (ghp_…), which can read every \
                 repository the account can reach. Create a fine-grained token instead \
                 (GitHub → Settings → Developer settings → Personal access tokens → \
                 Fine-grained tokens), scope it to this one repository, and give it \
                 read-only access to Contents, Metadata and Pull requests."
                    .to_string(),
            ));
        }
        let branches = normalize_branches(&request.branches)?;

        if self.list().await?.iter().any(|b| b.key == key) {
            return Err(OpenCompanyError::Conflict(format!(
                "{} is already bound as {key}; revoke it first to rebind",
                coords.canonical_url()
            )));
        }

        tokio::fs::create_dir_all(&self.root).await.map_err(|e| {
            OpenCompanyError::Store(format!(
                "creating the repository cache {}: {e}",
                self.root.display()
            ))
        })?;

        self.secrets
            .set(
                &self.company,
                &repo_token_key(&key),
                SecretValue(token.clone()),
            )
            .await?;

        // The advertised size, when a forge client is wired. Advisory: a
        // packfile or LFS bomb need not match what the API reports, which is
        // why the post-fetch cap in `install` exists regardless.
        if let (Some(host), Some(quota)) = (self.host.as_ref(), self.quota_bytes) {
            match host.repo_meta(&coords, &token).await {
                Ok(meta) => {
                    let advertised = meta.size_kb.saturating_mul(1024);
                    let used = dir_bytes(&self.root).await?;
                    if used.saturating_add(advertised) > quota {
                        self.roll_back(&key).await;
                        return Err(OpenCompanyError::WorkspaceQuota(format!(
                            "{} advertises {} and the repository cache is capped at {} \
                             ({} already used) — raise [workspace].tree_quota_gb or revoke a \
                             binding first",
                            coords.canonical_url(),
                            human_bytes(advertised),
                            human_bytes(quota),
                            human_bytes(used),
                        )));
                    }
                }
                Err(err) => {
                    self.roll_back(&key).await;
                    return Err(err);
                }
            }
        }

        // What is fetched is the canonical URL rebuilt from the validated
        // parts, never the submitted string.
        let url = coords.canonical_url();
        match self
            .install(
                &url,
                &key,
                &coords.owner,
                &coords.repo,
                Some(&token),
                branches,
            )
            .await
        {
            Ok(binding) => Ok(binding),
            Err(err) => {
                self.roll_back(&key).await;
                Err(err)
            }
        }
    }

    /// Creates the mirror, resolves the branches, performs the first fetch and
    /// records the binding.
    ///
    /// The part of [`bind`](Self::bind) that can fail after the credential is
    /// stored, factored out so exactly one place rolls back — and taking the
    /// fetch URL as a parameter rather than deriving it, so the mirror layer
    /// can be exercised against a local `file://` fixture with no network and
    /// no credential at all.
    async fn install(
        &self,
        url: &str,
        key: &str,
        owner: &str,
        repo: &str,
        token: Option<&str>,
        branches: Vec<String>,
    ) -> Result<RepoBinding> {
        let mirror = self.mirror_path(key);
        // A directory left by an interrupted earlier attempt would otherwise be
        // adopted with whatever configuration it had.
        remove_dir(&mirror).await?;
        self.init_mirror(&mirror, url).await?;

        // Resolving the default branch is also the credential probe: a token
        // that cannot list refs cannot do anything else either, and this fails
        // before a single object is transferred.
        let branches = if branches.is_empty() {
            vec![self.default_branch(&mirror, url, token).await?]
        } else {
            branches
        };

        let now = now_millis();
        let mut binding = RepoBinding {
            key: key.to_string(),
            url: url.to_string(),
            owner: owner.to_string(),
            repo: repo.to_string(),
            branches,
            token_fingerprint: token.map(fingerprint).unwrap_or_default(),
            last_fetched_millis: None,
            size_bytes: 0,
            bound_at_millis: now,
        };
        self.fetch_into(&mirror, &binding, token, &[]).await?;
        binding.size_bytes = dir_bytes(&mirror).await?;
        binding.last_fetched_millis = Some(now_millis());
        self.enforce_quota(&binding).await?;

        let _guard = self.index_lock.lock().await;
        let mut index = self.read_index().await?;
        if index.bindings.iter().any(|b| b.key == binding.key) {
            return Err(OpenCompanyError::Conflict(format!(
                "{} was bound concurrently by another request",
                binding.key
            )));
        }
        index.bindings.push(binding.clone());
        self.write_index(&index).await?;
        Ok(binding)
    }

    /// Undoes a failed bind: blanks the credential and removes the mirror.
    ///
    /// Failures here are logged rather than raised — the caller already has the
    /// error that matters, and replacing it with "cleanup failed" would hide
    /// the reason the bind was refused.
    async fn roll_back(&self, key: &str) {
        // `SecretStore` has no delete, so a cleared credential is the empty
        // string; `token()` treats empty as missing. Same convention as
        // `ops::channels`.
        if let Err(err) = self
            .secrets
            .set(
                &self.company,
                &repo_token_key(key),
                SecretValue(String::new()),
            )
            .await
        {
            tracing::warn!(company = %self.company, key, "could not clear the repository credential after a failed bind: {err}");
        }
        if let Err(err) = remove_dir(&self.mirror_path(key)).await {
            tracing::warn!(company = %self.company, key, "could not remove the half-built mirror: {err}");
        }
    }

    // -- mirror --------------------------------------------------------------

    /// Creates a bare mirror and configures it.
    async fn init_mirror(&self, mirror: &Path, url: &str) -> Result<()> {
        tokio::fs::create_dir_all(mirror).await.map_err(|e| {
            OpenCompanyError::Store(format!("creating the mirror {}: {e}", mirror.display()))
        })?;
        git::run(mirror, &["init", "--bare", "--quiet"], None, None)
            .await?
            .require("git init")?;
        // A bare remote URL. The credential is answered for per invocation and
        // never written here — so this file, which outlives every fetch, holds
        // nothing worth stealing.
        git::run(mirror, &["remote", "add", "origin", url], None, None)
            .await?
            .require("git remote add")?;
        for (name, value) in [
            // Checkouts reference this mirror through `objects/info/alternates`,
            // and a prune here cannot see those references. Never prune. Space
            // is reclaimed by revoking a binding, which deletes the whole
            // mirror at once. See this module's header.
            ("gc.auto", "0"),
            ("gc.pruneExpire", "never"),
            // The mirror holds no working tree, so nothing should ever try to
            // filter, smudge or normalize on the way in.
            ("core.autocrlf", "false"),
        ] {
            git::run(mirror, &["config", name, value], None, None)
                .await?
                .require("git config")?;
        }
        Ok(())
    }

    /// Asks the remote which branch it considers default.
    async fn default_branch(
        &self,
        mirror: &Path,
        url: &str,
        token: Option<&str>,
    ) -> Result<String> {
        let askpass = git::AskpassDir::create(&self.root)?;
        let out = git::run(
            mirror,
            &["ls-remote", "--symref", url, "HEAD"],
            token,
            Some(&askpass),
        )
        .await?;
        if !out.ok {
            return Err(OpenCompanyError::InvalidRequest(format!(
                "could not read {url} with that credential. Check the token has read access \
                 to this repository (Contents: read-only) and has not expired."
            )));
        }
        // `ref: refs/heads/main\tHEAD`
        let branch = out
            .stdout
            .lines()
            .find_map(|line| line.strip_prefix("ref: "))
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|r| r.strip_prefix("refs/heads/"))
            .map(str::to_string);
        branch.ok_or_else(|| {
            OpenCompanyError::InvalidRequest(format!(
                "{url} advertises no default branch — name the branches to mirror explicitly"
            ))
        })
    }

    /// Refreshes a binding's mirror, optionally pulling named pull-request refs
    /// in the same round trip.
    pub async fn fetch(&self, key: &str, pull_requests: &[u64]) -> Result<RepoBinding> {
        let binding = self.get(key).await?;
        let token = self.token_for(&binding).await?;
        let mirror = self.mirror_path(key);
        if !mirror.is_dir() {
            return Err(OpenCompanyError::NotFound(format!(
                "the mirror for {key} is missing from the cache — revoke and rebind"
            )));
        }
        self.fetch_into(&mirror, &binding, token.as_deref(), pull_requests)
            .await?;

        let _guard = self.index_lock.lock().await;
        let mut index = self.read_index().await?;
        let size = dir_bytes(&mirror).await?;
        let fetched = now_millis();
        let Some(entry) = index.bindings.iter_mut().find(|b| b.key == key) else {
            return Err(OpenCompanyError::NotFound(format!(
                "no repository bound as {key}"
            )));
        };
        entry.size_bytes = size;
        entry.last_fetched_millis = Some(fetched);
        let updated = entry.clone();
        self.write_index(&index).await?;
        // After the index is current, so an operator reading the console sees
        // the size that pushed it over rather than the previous one.
        self.enforce_quota(&updated).await?;
        Ok(updated)
    }

    /// The fetch itself: the binding's branches, plus any pull-request head
    /// refs named by number.
    async fn fetch_into(
        &self,
        mirror: &Path,
        binding: &RepoBinding,
        token: Option<&str>,
        pull_requests: &[u64],
    ) -> Result<()> {
        let mut refspecs: Vec<String> = binding
            .branches
            .iter()
            .map(|b| format!("+refs/heads/{b}:refs/heads/{b}"))
            .collect();
        for n in pull_requests {
            refspecs.push(format!("+refs/pull/{n}/head:refs/pull/{n}/head"));
        }
        // Never a bare `git fetch`: the refspecs are the restriction. A mirror
        // that fetched everything would carry every branch and every tag of a
        // large repository, which is most of what the quota exists to bound.
        let mut args: Vec<&str> = vec![
            "fetch",
            "--quiet",
            "--no-tags",
            // Objects a checkout may still be alternating through must not be
            // removed. See this module's header.
            "--no-auto-gc",
            "origin",
        ];
        args.extend(refspecs.iter().map(String::as_str));

        let askpass = git::AskpassDir::create(&self.root)?;
        let out = git::run(mirror, &args, token, Some(&askpass)).await?;
        if !out.ok {
            let refs = binding.branches.join(", ");
            return Err(OpenCompanyError::Store(format!(
                "fetching {} ({refs}) failed: {}",
                binding.url,
                out.stderr
                    .lines()
                    .map(str::trim)
                    .find(|l| !l.is_empty())
                    .unwrap_or("git wrote nothing to stderr")
            )));
        }
        Ok(())
    }

    /// Refuses — loudly — when the cache has outgrown its cap.
    ///
    /// Deliberately not eviction. See this module's header: a bound repository
    /// is operator-configured state, and deleting one to make room turns a disk
    /// problem into a mystery.
    async fn enforce_quota(&self, binding: &RepoBinding) -> Result<()> {
        let Some(quota) = self.quota_bytes else {
            return Ok(());
        };
        let used = dir_bytes(&self.root).await?;
        if used <= quota {
            return Ok(());
        }
        Err(OpenCompanyError::WorkspaceQuota(format!(
            "fetching {} put the repository cache at {} against a {} cap. Nothing was evicted \
             — raise [workspace].tree_quota_gb, mirror fewer branches, or revoke a binding.",
            binding.url,
            human_bytes(used),
            human_bytes(quota),
        )))
    }

    // -- pull requests -------------------------------------------------------

    /// A pull request's metadata and unified diff, fetched host-side.
    ///
    /// Errors with [`Unimplemented`](OpenCompanyError::Unimplemented) when no
    /// forge client is wired — the honest answer for a build that links no HTTP
    /// client, rather than an empty diff a caller would read as "no changes".
    pub async fn pull_request(&self, key: &str, number: u64) -> Result<PullRequestView> {
        let binding = self.get(key).await?;
        let Some(host) = self.host.as_ref() else {
            return Err(OpenCompanyError::Unimplemented(
                "pull-request fetch needs a forge client; rebuild with the `github` feature",
            ));
        };
        let Some(token) = self.token_for(&binding).await? else {
            return Err(OpenCompanyError::InvalidRequest(format!(
                "{} is bound without a credential, so its pull requests cannot be read",
                binding.url
            )));
        };
        let coords = RepoCoordinates {
            owner: binding.owner,
            repo: binding.repo,
        };
        host.pull_request(&coords, number, &token).await
    }

    /// Binds a repository from a URL this surface does not otherwise accept —
    /// a local `file://` fixture — with no credential.
    ///
    /// Test-only, and narrow on purpose: it skips [`parse_repo_url`] and
    /// nothing else, so every test below drives the *real* mirror, fetch,
    /// quota and revoke paths. Widening the URL validator or
    /// standing up a fake HTTPS forge to reach the same code would each be
    /// worse — one weakens a boundary for the benefit of tests, the other
    /// tests a different program.
    #[cfg(test)]
    pub(crate) async fn bind_local(
        &self,
        url: &str,
        key: &str,
        branches: Vec<String>,
    ) -> Result<RepoBinding> {
        tokio::fs::create_dir_all(&self.root).await.map_err(|e| {
            OpenCompanyError::Store(format!("creating {}: {e}", self.root.display()))
        })?;
        self.install(
            url,
            key,
            "fixture",
            key,
            None,
            normalize_branches(&branches)?,
        )
        .await
    }

    // -- revoke --------------------------------------------------------------

    /// Removes a binding: drops the index entry, blanks the credential, deletes
    /// the mirror.
    ///
    /// This is also the only way cache space comes back, since the mirrors never
    /// prune.
    pub async fn revoke(&self, key: &str) -> Result<()> {
        let guard = self.index_lock.lock().await;
        let mut index = self.read_index().await?;
        let before = index.bindings.len();
        index.bindings.retain(|b| b.key != key);
        if index.bindings.len() == before {
            return Err(OpenCompanyError::NotFound(format!(
                "no repository bound as {key}"
            )));
        }
        self.write_index(&index).await?;
        drop(guard);

        // Index first, then the credential, then the bytes. Every ordering
        // leaves *some* window; this one's window is a stored token nothing
        // references, which is strictly better than a live binding whose
        // credential has already been blanked.
        self.secrets
            .set(
                &self.company,
                &repo_token_key(key),
                SecretValue(String::new()),
            )
            .await?;
        remove_dir(&self.mirror_path(key)).await?;
        Ok(())
    }
}

// -- helpers -----------------------------------------------------------------

/// Validates and de-duplicates the submitted branch list.
///
/// A branch name reaches a `git fetch` refspec, so it is held to what a ref can
/// actually be — and specifically may not start with `-`, which is how a name
/// becomes an option.
fn normalize_branches(branches: &[String]) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for raw in branches {
        let name = validate_ref(raw)?;
        if !out.contains(&name) {
            out.push(name);
        }
    }
    Ok(out)
}

/// Validates one ref name.
fn validate_ref(raw: &str) -> Result<String> {
    let name = raw.trim();
    let refuse = |why: &str| {
        Err(OpenCompanyError::InvalidRequest(format!(
            "{name:?} is not a usable branch name — {why}"
        )))
    };
    if name.is_empty() {
        return refuse("it is empty");
    }
    if name.len() > 255 {
        return refuse("it is too long");
    }
    if name.starts_with('-') {
        return refuse("a leading '-' would be read as a command-line option");
    }
    if name.contains("..")
        || name.starts_with('/')
        || name.ends_with('/')
        || name.ends_with(".lock")
    {
        return refuse("git does not accept that shape");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
    {
        return refuse("only letters, digits, '-', '_', '.' and '/' are accepted");
    }
    Ok(name.to_string())
}

/// Removes a directory if it exists. Absent is success.
async fn remove_dir(path: &Path) -> Result<()> {
    match tokio::fs::remove_dir_all(path).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(OpenCompanyError::Store(format!(
            "removing {}: {e}",
            path.display()
        ))),
    }
}

/// Sums the regular files under `dir`. Absent is zero.
///
/// The same shape as [`DataLayout::usage_bytes`](crate::store::DataLayout::usage_bytes)
/// and for the same reasons — iterative so there is no recursion limit, and
/// symlinks are not followed so a link cannot be counted twice or walked out of
/// the tree.
async fn dir_bytes(dir: &Path) -> Result<u64> {
    let mut total = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(path) = stack.pop() {
        let mut entries = match tokio::fs::read_dir(&path).await {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                return Err(OpenCompanyError::Store(format!(
                    "measuring {}: {e}",
                    path.display()
                )));
            }
        };
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| OpenCompanyError::Store(format!("measuring {}: {e}", path.display())))?
        {
            let Ok(meta) = entry.metadata().await else {
                continue;
            };
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                total += meta.len();
            }
        }
    }
    Ok(total)
}

/// Renders a byte count for an operator-facing message.
fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
