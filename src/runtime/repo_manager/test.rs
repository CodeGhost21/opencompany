//! [`RepoManager`] against a real `git`, and no network anywhere.
//!
//! Every test that exercises the mirror builds a bare repository in a temp
//! directory and binds it over `file://`. That is a deliberate choice about
//! what is worth proving: mocking `git` would test a mock, and the bugs this
//! module can actually have — a refspec that fetches everything, a mirror that
//! prunes objects out from under an alternate, a token that lands in
//! `.git/config` — are all bugs in how git is *driven*, and only a real git can
//! catch them.
//!
//! The credential tests use the literal token `SENTINEL`, then walk every byte
//! this module wrote looking for it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use async_trait::async_trait;

use super::*;
use crate::ports::types::CompanyId;

/// The token every credential test looks for afterwards.
const SENTINEL: &str = "github_pat_SENTINEL";

/// The URL the `bind` tests submit, and the key it derives to.
///
/// Derived rather than written out: the key carries a hash of the coordinates,
/// and a test that hard-codes it asserts nothing when the derivation changes —
/// `get_now(…).is_none()` on a key no code produces passes for the wrong reason.
const WIDGETS_URL: &str = "https://github.com/acme/widgets";

fn widgets_key() -> String {
    parse_repo_url(WIDGETS_URL).unwrap().key()
}

/// An in-memory [`SecretStore`]. Also the second half of the credential audit:
/// what a test asserts about the filesystem is only half the story, and this
/// makes the *stored* side inspectable too.
#[derive(Default)]
struct MemSecrets {
    values: StdMutex<HashMap<(String, String), String>>,
}

#[async_trait]
impl SecretStore for MemSecrets {
    async fn get(&self, company: &CompanyId, key: &str) -> Result<Option<SecretValue>> {
        Ok(self
            .values
            .lock()
            .unwrap()
            .get(&(company.to_string(), key.to_string()))
            .cloned()
            .map(SecretValue))
    }

    async fn set(&self, company: &CompanyId, key: &str, value: SecretValue) -> Result<()> {
        self.values
            .lock()
            .unwrap()
            .insert((company.to_string(), key.to_string()), value.0);
        Ok(())
    }
}

impl MemSecrets {
    fn get_now(&self, company: &str, key: &str) -> Option<String> {
        self.values
            .lock()
            .unwrap()
            .get(&(company.to_string(), key.to_string()))
            .cloned()
    }
}

/// One `create_pull_request` call, recorded so a test can assert what was sent.
#[derive(Clone, Debug)]
struct CreatedPr {
    head: String,
    base: String,
    title: String,
    body: String,
}

/// A forge that answers from a script, and records the token it was handed.
struct FakeHost {
    meta: RepoMeta,
    seen_tokens: StdMutex<Vec<String>>,
    fail: bool,
    /// When set, `create_pull_request` errors — for the honest-degradation test
    /// where a push succeeds but the PR does not open (issue #736).
    fail_pr: bool,
    /// Every `create_pull_request` call, in order.
    created_prs: StdMutex<Vec<CreatedPr>>,
}

impl FakeHost {
    fn new(size_kb: u64) -> Self {
        Self {
            meta: RepoMeta {
                default_branch: "main".into(),
                size_kb,
                // Read-only by default: the write tier fails closed, so a test
                // that wants a push-capable credential opts in with `pushable`.
                can_push: false,
            },
            seen_tokens: StdMutex::new(Vec::new()),
            fail: false,
            fail_pr: false,
            created_prs: StdMutex::new(Vec::new()),
        }
    }

    fn failing() -> Self {
        Self {
            fail: true,
            ..Self::new(1)
        }
    }

    /// A forge whose credential the `permissions.push` probe reports as
    /// push-capable.
    fn pushable(mut self) -> Self {
        self.meta.can_push = true;
        self
    }

    /// A forge that accepts a push but refuses to open the pull request.
    fn failing_pr(mut self) -> Self {
        self.fail_pr = true;
        self
    }
}

#[async_trait]
impl RepoHost for FakeHost {
    async fn repo_meta(&self, _coords: &RepoCoordinates, token: &str) -> Result<RepoMeta> {
        self.seen_tokens.lock().unwrap().push(token.to_string());
        if self.fail {
            return Err(OpenCompanyError::InvalidRequest("bad credential".into()));
        }
        Ok(self.meta.clone())
    }

    async fn pull_request(
        &self,
        _coords: &RepoCoordinates,
        number: u64,
        token: &str,
    ) -> Result<PullRequestView> {
        self.seen_tokens.lock().unwrap().push(token.to_string());
        Ok(PullRequestView {
            number,
            title: "a change".into(),
            state: "open".into(),
            head_sha: "cafe".into(),
            base_ref: "main".into(),
            diff: "--- a\n+++ b\n".into(),
        })
    }

    async fn create_pull_request(
        &self,
        _coords: &RepoCoordinates,
        token: &str,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<PullRequestRef> {
        self.seen_tokens.lock().unwrap().push(token.to_string());
        if self.fail_pr {
            return Err(OpenCompanyError::Store(
                "the forge refused the pull request".into(),
            ));
        }
        self.created_prs.lock().unwrap().push(CreatedPr {
            head: head.to_string(),
            base: base.to_string(),
            title: title.to_string(),
            body: body.to_string(),
        });
        Ok(PullRequestRef {
            number: 42,
            html_url: "https://github.com/acme/fixture/pull/42".into(),
        })
    }
}

/// A scratch directory removed when the test ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "oc-repo-{}-{}-{tag}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn join(&self, part: &str) -> PathBuf {
        self.0.join(part)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

/// Runs git in `cwd`, panicking with its exit status and output on failure.
///
/// **Starts from an empty environment**, the same posture [`super::git::run`]
/// takes for a host-side fetch (see its `env_clear` and the module header).
/// These fixtures inherited the developer's whole environment, so every one of
/// these tests read that machine's global git config — they were strictly less
/// isolated than the code they exercise. On the machine where issue #748's
/// flake was recorded, that config supplied both a `core.hooksPath` (the exact
/// setting #727 showed silently disables hooks repo-wide) and the git-lfs
/// filter triplet, so each fixture checkout also span up a `git-lfs
/// filter-process` child. Under a full-suite run that is many such children at
/// once, which fits #748's signature — a failure carrying *empty* stderr — far
/// better than any git refusal does.
///
/// Treat that as the motivation, not a proven cause: what is verified is that
/// the fixtures no longer read any of it, and that the flake did not recur in
/// five consecutive full-suite runs against a baseline of three failures in
/// four. The reason to make the change regardless is that a fixture reading the
/// developer's config is testing the developer.
///
/// `PATH` is the one thing carried over, because `git` itself has to be
/// findable. Everything else is supplied here:
///
/// * `GIT_CONFIG_NOSYSTEM` / `GIT_CONFIG_GLOBAL=/dev/null` — no system or user
///   config, so no `core.hooksPath`, no `init.templateDir`, no alias.
/// * `HOME` inside the scratch tree — anything that still resolves a home finds
///   an empty one rather than the developer's.
/// * `GIT_TERMINAL_PROMPT=0` — a fixture must fail, never block on a prompt.
/// * A fixed identity, so a machine without `user.email` set is not a failure.
///
/// The panic reports the **status** as well as both streams: the failures in
/// #748 arrived with empty stderr, which says nothing on its own but is exactly
/// what a signal-killed child looks like — `status` is what distinguishes
/// "git refused" from "git was killed".
fn git_at(cwd: &Path, args: &[&str]) -> String {
    let mut cmd = std::process::Command::new("git");
    cmd.current_dir(cwd).args(args);
    cmd.env_clear();
    if let Some(path) = std::env::var_os("PATH") {
        cmd.env("PATH", path);
    }
    cmd.env("HOME", cwd);
    cmd.env("GIT_CONFIG_NOSYSTEM", "1");
    cmd.env("GIT_CONFIG_GLOBAL", "/dev/null");
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.env("GIT_AUTHOR_NAME", "oc-test");
    cmd.env("GIT_AUTHOR_EMAIL", "oc-test@example.invalid");
    cmd.env("GIT_COMMITTER_NAME", "oc-test");
    cmd.env("GIT_COMMITTER_EMAIL", "oc-test@example.invalid");
    let out = cmd.output().unwrap_or_else(|e| panic!("git {args:?}: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed ({}): stderr={} stdout={}",
        out.status,
        String::from_utf8_lossy(&out.stderr).trim(),
        String::from_utf8_lossy(&out.stdout).trim()
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Builds a bare fixture repository with a `main` branch, a `topic` branch and
/// a `refs/pull/7/head` — the three ref shapes this module fetches.
fn fixture_remote(scratch: &Scratch) -> String {
    let work = scratch.join("origin-work");
    let bare = scratch.join("origin.git");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&bare).unwrap();

    git_at(
        &bare,
        &["init", "--bare", "--quiet", "--initial-branch=main"],
    );
    git_at(&work, &["init", "--quiet", "--initial-branch=main"]);
    for (k, v) in [
        ("user.email", "fixture@example.test"),
        ("user.name", "Fixture"),
        ("commit.gpgsign", "false"),
    ] {
        git_at(&work, &["config", k, v]);
    }
    std::fs::write(work.join("README.md"), "# fixture\n").unwrap();
    git_at(&work, &["add", "README.md"]);
    git_at(&work, &["commit", "--quiet", "-m", "initial"]);

    git_at(&work, &["checkout", "--quiet", "-b", "topic"]);
    std::fs::write(work.join("topic.txt"), "topic\n").unwrap();
    git_at(&work, &["add", "topic.txt"]);
    git_at(&work, &["commit", "--quiet", "-m", "topic"]);
    git_at(&work, &["checkout", "--quiet", "main"]);

    let bare_str = bare.to_string_lossy().to_string();
    git_at(&work, &["remote", "add", "origin", &bare_str]);
    git_at(&work, &["push", "--quiet", "origin", "main", "topic"]);
    // A pull-request head ref, exactly as GitHub publishes one.
    git_at(
        &work,
        &["push", "--quiet", "origin", "topic:refs/pull/7/head"],
    );
    // Make `main` the bare repo's HEAD so the default-branch probe has an
    // answer, as a real forge would.
    git_at(&bare, &["symbolic-ref", "HEAD", "refs/heads/main"]);

    format!("file://{bare_str}")
}

fn manager(scratch: &Scratch) -> (RepoManager, Arc<MemSecrets>) {
    let secrets = Arc::new(MemSecrets::default());
    let mgr = RepoManager::new(
        CompanyId::new("acme"),
        scratch.join("data/companies/acme/repos"),
        secrets.clone(),
    )
    // Issue #752: `bind` refuses outright on a backend that keeps secrets as
    // plaintext on the container's disk, and `RepoManager::new` defaults to
    // that refusing side. These tests exercise the credentialed path *past*
    // that gate, so they stand in a deployment that clears it — which is also
    // what `MemSecrets` actually is: secrets that never touch the disk.
    .with_storage_kind(crate::store::StorageKind::Mongodb);
    (mgr, secrets)
}

/// Every regular file under `dir`, as `(path, bytes)`.
fn all_files(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(path) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                let bytes = std::fs::read(entry.path()).unwrap_or_default();
                out.push((entry.path(), bytes));
            }
        }
    }
    out
}

// -- binding -----------------------------------------------------------------

#[tokio::test]
async fn binding_mirrors_only_the_named_branches() {
    let scratch = Scratch::new("branches");
    let url = fixture_remote(&scratch);
    let (mgr, _) = manager(&scratch);

    let binding = mgr
        .bind_local(&url, "fixture", vec!["main".into()])
        .await
        .unwrap();
    assert_eq!(binding.branches, vec!["main".to_string()]);
    assert!(binding.size_bytes > 0, "the mirror measured zero");
    assert!(binding.last_fetched_millis.is_some());

    let mirror = mgr.mirror_path("fixture");
    let refs = git_at(&mirror, &["for-each-ref", "--format=%(refname)"]);
    assert!(refs.contains("refs/heads/main"), "{refs}");
    // The restriction is the whole point of the refspec: a mirror that fetched
    // everything is most of what the quota exists to bound.
    assert!(!refs.contains("refs/heads/topic"), "{refs}");
    assert!(!refs.contains("refs/pull/7"), "{refs}");
}

#[tokio::test]
async fn an_empty_branch_list_resolves_the_remote_default() {
    let scratch = Scratch::new("default-branch");
    let url = fixture_remote(&scratch);
    let (mgr, _) = manager(&scratch);

    let binding = mgr.bind_local(&url, "fixture", vec![]).await.unwrap();
    assert_eq!(
        binding.branches,
        vec!["main".to_string()],
        "the default branch is read off the remote, not guessed"
    );
}

#[tokio::test]
async fn an_incremental_fetch_picks_up_new_commits_and_named_pull_refs() {
    let scratch = Scratch::new("incremental");
    let url = fixture_remote(&scratch);
    let (mgr, _) = manager(&scratch);
    mgr.bind_local(&url, "fixture", vec!["main".into()])
        .await
        .unwrap();
    let mirror = mgr.mirror_path("fixture");
    let before = git_at(&mirror, &["rev-parse", "refs/heads/main"]);

    // Advance the fixture's `main`.
    let work = scratch.join("origin-work");
    std::fs::write(work.join("second.txt"), "more\n").unwrap();
    git_at(&work, &["add", "second.txt"]);
    git_at(&work, &["commit", "--quiet", "-m", "second"]);
    git_at(&work, &["push", "--quiet", "origin", "main"]);

    let updated = mgr.fetch("fixture", &[7]).await.unwrap();
    let after = git_at(&mirror, &["rev-parse", "refs/heads/main"]);
    assert_ne!(before, after, "the incremental fetch moved main");
    assert!(updated.last_fetched_millis.is_some());

    // The PR ref arrives only because it was asked for by number.
    let refs = git_at(&mirror, &["for-each-ref", "--format=%(refname)"]);
    assert!(refs.contains("refs/pull/7/head"), "{refs}");
}

#[tokio::test]
async fn revoking_removes_the_entry_the_credential_and_the_bytes() {
    let scratch = Scratch::new("revoke");
    let url = fixture_remote(&scratch);
    let (mgr, secrets) = manager(&scratch);
    mgr.bind_local(&url, "fixture", vec!["main".into()])
        .await
        .unwrap();
    // Stand a credential up beside it so the blanking is observable.
    secrets
        .set(
            &CompanyId::new("acme"),
            &repo_token_key("fixture"),
            SecretValue(SENTINEL.into()),
        )
        .await
        .unwrap();
    assert!(mgr.mirror_path("fixture").is_dir());

    mgr.revoke("fixture").await.unwrap();

    assert!(mgr.list().await.unwrap().is_empty());
    assert!(
        !mgr.mirror_path("fixture").exists(),
        "revoke is the only way cache space comes back, so it must delete the mirror"
    );
    assert_eq!(
        secrets
            .get_now("acme", &repo_token_key("fixture"))
            .as_deref(),
        Some(""),
        "SecretStore has no delete, so a cleared credential is the empty string"
    );
    assert!(matches!(
        mgr.revoke("fixture").await,
        Err(OpenCompanyError::NotFound(_))
    ));
}

// -- quota -------------------------------------------------------------------

#[tokio::test]
async fn an_over_quota_fetch_is_refused_and_nothing_is_evicted() {
    let scratch = Scratch::new("quota");
    let url = fixture_remote(&scratch);
    let (mgr, secrets) = manager(&scratch);
    // One byte of headroom: the first fetch cannot fit.
    let mgr = mgr.with_quota(Some(1));
    let _ = secrets;

    let err = mgr
        .bind_local(&url, "fixture", vec!["main".into()])
        .await
        .unwrap_err();
    assert!(
        matches!(err, OpenCompanyError::WorkspaceQuota(_)),
        "expected a quota refusal, got {err:?}"
    );
    let message = err.to_string();
    assert!(
        message.contains("Nothing was evicted"),
        "the refusal must say the operator's bindings were left alone: {message}"
    );
    assert!(
        mgr.list().await.unwrap().is_empty(),
        "a refused bind leaves no binding behind"
    );
}

#[tokio::test]
async fn a_generous_quota_lets_the_bind_through() {
    let scratch = Scratch::new("quota-ok");
    let url = fixture_remote(&scratch);
    let (mgr, _) = manager(&scratch);
    let mgr = mgr.with_quota(Some(64 * 1024 * 1024));
    mgr.bind_local(&url, "fixture", vec!["main".into()])
        .await
        .unwrap();
    assert_eq!(mgr.list().await.unwrap().len(), 1);
}

// -- mirror configuration ----------------------------------------------------

#[tokio::test]
async fn a_mirror_is_configured_never_to_prune() {
    let scratch = Scratch::new("no-prune");
    let url = fixture_remote(&scratch);
    let (mgr, _) = manager(&scratch);
    mgr.bind_local(&url, "fixture", vec!["main".into()])
        .await
        .unwrap();

    // Space comes back by revoking a binding, never by pruning: a prune cannot
    // see an alternate's references, and the checkout tier this cache is built
    // for will hold exactly those.
    let mirror = mgr.mirror_path("fixture");
    assert_eq!(git_at(&mirror, &["config", "gc.pruneExpire"]), "never");
    assert_eq!(git_at(&mirror, &["config", "gc.auto"]), "0");
}

// -- credential non-exposure -------------------------------------------------

#[tokio::test]
async fn the_token_appears_in_no_byte_the_mirror_wrote() {
    let scratch = Scratch::new("sentinel");
    let url = fixture_remote(&scratch);
    let (mgr, secrets) = manager(&scratch);

    mgr.bind_local(&url, "fixture", vec!["main".into()])
        .await
        .unwrap();
    // Install the credential and drive a fetch and a checkout through the
    // credentialed path, exactly as a bound HTTPS repository would.
    secrets
        .set(
            &CompanyId::new("acme"),
            &repo_token_key("fixture"),
            SecretValue(SENTINEL.into()),
        )
        .await
        .unwrap();
    // The binding records a fingerprint, so `token_for` now hands git the
    // sentinel on every network invocation.
    {
        let mut index = mgr.read_index().await.unwrap();
        index.bindings[0].token_fingerprint = super::types::fingerprint(SENTINEL);
        mgr.write_index(&index).await.unwrap();
    }
    mgr.fetch("fixture", &[7]).await.unwrap();

    // Everything this module wrote: the whole mirror cache, including the
    // mirror's `config`, where a credential helper or a userinfo URL would
    // live, and the askpass scratch directory beside it.
    let mut scanned = 0usize;
    for (path, bytes) in all_files(mgr.root()) {
        scanned += 1;
        assert!(
            !bytes
                .windows(SENTINEL.len())
                .any(|w| w == SENTINEL.as_bytes()),
            "the credential leaked into {}",
            path.display()
        );
    }
    assert!(
        scanned > 5,
        "expected to have scanned a real tree, saw {scanned}"
    );

    // The stored index is metadata only: a fingerprint, never the token.
    let index = mgr.read_index().await.unwrap();
    let json = serde_json::to_string(&index).unwrap();
    assert!(!json.contains(SENTINEL), "{json}");
    assert_eq!(
        index.bindings[0].token_fingerprint,
        super::types::fingerprint(SENTINEL)
    );

    // The credential lives in exactly one place, under its own key.
    assert_eq!(
        secrets
            .get_now("acme", &repo_token_key("fixture"))
            .as_deref(),
        Some(SENTINEL)
    );
    assert!(
        !secrets
            .get_now("acme", REPO_INDEX_KEY)
            .unwrap_or_default()
            .contains(SENTINEL)
    );
}

// -- validation --------------------------------------------------------------

#[tokio::test]
async fn a_classic_pat_is_refused_with_a_usable_instruction() {
    let scratch = Scratch::new("classic-pat");
    let (mgr, secrets) = manager(&scratch);
    let err = mgr
        .bind(BindRequest {
            url: WIDGETS_URL.into(),
            token: "ghp_classicclassicclassic".into(),
            branches: vec![],
        })
        .await
        .unwrap_err();
    let message = err.to_string();
    assert!(
        matches!(err, OpenCompanyError::InvalidRequest(_)),
        "{err:?}"
    );
    assert!(message.contains("fine-grained"), "{message}");
    assert!(message.contains("read-only"), "{message}");
    // Refused before anything was stored: a rejected credential must not be
    // sitting in the secret store afterwards.
    assert!(
        secrets
            .get_now("acme", &repo_token_key(&widgets_key()))
            .is_none()
    );
}

// -- issue #752: the plaintext-secret-backend gate -----------------------------

/// The attack this gate exists to stop, run as an attack rather than as a unit
/// test of the guard: on a host whose secrets are plaintext files on the
/// container's own disk, install a repository credential — and then go looking
/// for it as the agent shell would, by reading the disk.
///
/// It must not be there, because the bind must not have happened.
#[tokio::test]
async fn a_credential_cannot_be_installed_where_the_agent_shell_could_read_it() {
    let scratch = Scratch::new("plaintext-secret-bind");
    let url = fixture_remote(&scratch);
    // A *real* filesystem secret store rooted in the scratch home — not the
    // in-memory fake — so "the token is on disk" is a claim about actual bytes
    // in an actual file, which is the only version of it worth asserting.
    let home = scratch.join("data");
    let secrets = Arc::new(crate::store::FsSecretStore::new(home.clone()));
    let mgr = RepoManager::new(
        CompanyId::new("acme"),
        scratch.join("data/companies/acme/repos"),
        secrets.clone(),
    )
    .with_storage_kind(crate::store::StorageKind::Fs);

    // Exactly the call the accepted-case test makes, against the same fixture
    // remote. The only difference between the two is which backend is holding
    // the secrets — which is the whole claim.
    let coords = parse_repo_url(WIDGETS_URL).unwrap();
    let err = mgr
        .bind_validated(&coords, &url, SENTINEL, vec!["main".into()])
        .await
        .unwrap_err();

    assert!(matches!(err, OpenCompanyError::Conflict(_)), "{err:?}");
    let message = err.to_string();
    assert!(message.contains("OPENCOMPANY_STORAGE=fs"), "{message}");
    assert!(message.contains("OPENCOMPANY_STORAGE=mongodb"), "{message}");
    assert!(message.contains("`repo` grant"), "{message}");
    // The refusal must not quote the credential back into an error string that
    // ends up in a log line or a console toast.
    assert!(!message.contains(SENTINEL), "{message}");

    // The attack: walk the company's whole on-disk footprint the way a shell
    // with `cat`/`grep` would, and find nothing.
    let leaked: Vec<_> = all_files(&home)
        .into_iter()
        .filter(|(_, bytes)| String::from_utf8_lossy(bytes).contains(SENTINEL))
        .map(|(path, _)| path)
        .collect();
    assert!(
        leaked.is_empty(),
        "a refused credential is readable on disk: {leaked:?}"
    );
    // And no binding exists to hang a later fetch off.
    assert!(mgr.list().await.unwrap().is_empty());
}

/// The other half, without which the test above only proves the feature is off:
/// the identical credential install is *accepted* on the backend that keeps
/// secrets out of the container, and the token really does land. Driven through
/// `bind_validated` against the `file://` fixture — the same way every other
/// successful-bind test here runs the real path with no network — which is also
/// the funnel the gate sits in. `sqlite` is refused alongside `fs`: same disk,
/// same uid.
#[tokio::test]
async fn the_same_bind_is_accepted_where_secrets_leave_the_container() {
    let scratch = Scratch::new("mongodb-secret-bind");
    let url = fixture_remote(&scratch);
    let (mgr, secrets) = manager(&scratch);
    let coords = parse_repo_url(WIDGETS_URL).unwrap();
    let binding = mgr
        .bind_validated(&coords, &url, SENTINEL, vec!["main".into()])
        .await
        .expect("mongodb-backed secrets must clear the #752 gate");
    assert_eq!(
        secrets
            .get_now("acme", &repo_token_key(&binding.key))
            .as_deref(),
        Some(SENTINEL)
    );
    assert_eq!(mgr.list().await.unwrap().len(), 1);

    // Sqlite is on the same disk as fs and is refused with the same message.
    // Driven against the same fixture remote rather than the canonical GitHub
    // URL: if this gate is ever removed, this assertion must fail *fast* on an
    // unexpected success, not sit for five minutes waiting out a `ls-remote` to
    // a host the test suite has no business contacting.
    let sqlite_mgr = RepoManager::new(
        CompanyId::new("acme"),
        scratch.join("data/companies/acme/repos-sqlite"),
        Arc::new(MemSecrets::default()),
    )
    .with_storage_kind(crate::store::StorageKind::Sqlite);
    let err = sqlite_mgr
        .bind_validated(&coords, &url, SENTINEL, vec!["main".into()])
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("OPENCOMPANY_STORAGE=sqlite"),
        "{err}"
    );
}

#[tokio::test]
async fn a_bad_url_or_an_empty_token_is_refused() {
    let scratch = Scratch::new("bad-url");
    let (mgr, _) = manager(&scratch);
    for (url, token) in [
        ("https://gitlab.com/acme/widgets", SENTINEL),
        ("git@github.com:acme/widgets.git", SENTINEL),
        ("https://github.com/acme/widgets", "   "),
    ] {
        let err = mgr
            .bind(BindRequest {
                url: url.into(),
                token: token.into(),
                branches: vec![],
            })
            .await
            .unwrap_err();
        assert!(
            matches!(err, OpenCompanyError::InvalidRequest(_)),
            "{url}: {err:?}"
        );
    }
}

#[tokio::test]
async fn a_branch_name_that_would_become_an_option_is_refused() {
    let scratch = Scratch::new("branch-name");
    let url = fixture_remote(&scratch);
    let (mgr, _) = manager(&scratch);
    for bad in [
        "--upload-pack=touch /tmp/pwned",
        "a..b",
        "/leading",
        "with space",
    ] {
        let err = mgr
            .bind_local(&url, "fixture", vec![bad.into()])
            .await
            .unwrap_err();
        assert!(
            matches!(err, OpenCompanyError::InvalidRequest(_)),
            "{bad}: {err:?}"
        );
    }
}

#[tokio::test]
async fn binding_the_same_repository_twice_is_a_conflict() {
    let scratch = Scratch::new("conflict");
    let url = fixture_remote(&scratch);
    let (mgr, secrets) = manager(&scratch);
    // Bound under the key `https://github.com/acme/widgets` derives, so the
    // real `bind` below collides with it.
    mgr.bind_local(&url, &widgets_key(), vec!["main".into()])
        .await
        .unwrap();

    let err = mgr
        .bind(BindRequest {
            url: WIDGETS_URL.into(),
            token: SENTINEL.into(),
            branches: vec![],
        })
        .await
        .unwrap_err();
    assert!(matches!(err, OpenCompanyError::Conflict(_)), "{err:?}");
    assert!(err.to_string().contains("revoke it first"), "{err}");

    // The conflict is detected before the credential is stored, so a rebind
    // attempt cannot overwrite the working binding's token with the new one.
    assert!(
        secrets
            .get_now("acme", &repo_token_key(&widgets_key()))
            .is_none(),
        "a refused rebind must not have touched the stored credential"
    );
    assert_eq!(mgr.read_index().await.unwrap().bindings.len(), 1);
}

/// Two binds of the same repository, racing.
///
/// The duplicate check used to run outside any lock, so both callers passed it,
/// both wrote `repos/token/<key>`, and both built the same mirror. The one that
/// lost the index commit then called `roll_back` — which blanked the credential
/// and deleted the mirror now belonging to the **winner**, whose index entry
/// survived. The operator was left with a binding that reads as installed and
/// cannot fetch anything.
///
/// Driven through `bind_validated` against the `file://` fixture so the real
/// path runs — duplicate check, token write, mirror build, index commit,
/// rollback — with no network.
#[tokio::test]
async fn two_concurrent_binds_of_one_repository_leave_exactly_one_intact() {
    let scratch = Scratch::new("bindrace");
    let url = fixture_remote(&scratch);
    let (mgr, secrets) = manager(&scratch);
    let mgr = Arc::new(mgr);
    let coords = parse_repo_url(WIDGETS_URL).unwrap();

    // Both futures are spawned before either is awaited, so they interleave
    // inside `bind_validated` rather than running end to end in turn.
    let (a, b) = tokio::join!(
        {
            let mgr = mgr.clone();
            let coords = coords.clone();
            let url = url.clone();
            async move {
                mgr.bind_validated(&coords, &url, SENTINEL, vec!["main".into()])
                    .await
            }
        },
        {
            let mgr = mgr.clone();
            let coords = coords.clone();
            let url = url.clone();
            async move {
                mgr.bind_validated(&coords, &url, SENTINEL, vec!["main".into()])
                    .await
            }
        }
    );

    let winners = [&a, &b].iter().filter(|r| r.is_ok()).count();
    assert_eq!(winners, 1, "exactly one bind may win: a={a:?} b={b:?}");
    let loser = match (a, b) {
        (Err(e), Ok(_)) | (Ok(_), Err(e)) => e,
        (a, b) => panic!("exactly one bind may fail: a={a:?} b={b:?}"),
    };
    assert!(
        matches!(loser, OpenCompanyError::Conflict(_)),
        "the loser must lose on the duplicate check, not on a corrupted mirror: {loser:?}"
    );

    // The survivor is whole: one index entry, its credential still stored, and
    // its mirror still on disk. Before the lock, the loser's rollback took the
    // last two while leaving the first.
    let index = mgr.read_index().await.unwrap();
    assert_eq!(index.bindings.len(), 1, "{index:?}");
    let key = index.bindings[0].key.clone();
    assert!(
        secrets.get_now("acme", &repo_token_key(&key)).is_some(),
        "the winner's credential was blanked by the loser's rollback"
    );
    assert!(
        scratch
            .join(&format!("data/companies/acme/repos/{key}.git"))
            .exists(),
        "the winner's mirror was deleted by the loser's rollback"
    );
}

// -- rollback ----------------------------------------------------------------

#[tokio::test]
async fn a_failed_bind_leaves_no_credential_and_no_mirror() {
    let scratch = Scratch::new("rollback");
    let (mgr, secrets) = manager(&scratch);
    // A forge that rejects the credential, so the bind fails after the token
    // has already been written.
    let mgr = mgr
        .with_host(Arc::new(FakeHost::failing()))
        .with_quota(Some(1024 * 1024 * 1024));

    let err = mgr
        .bind(BindRequest {
            url: WIDGETS_URL.into(),
            token: SENTINEL.into(),
            branches: vec!["main".into()],
        })
        .await
        .unwrap_err();
    assert!(
        matches!(err, OpenCompanyError::InvalidRequest(_)),
        "{err:?}"
    );

    assert!(mgr.list().await.unwrap().is_empty(), "no binding persisted");
    assert_eq!(
        secrets
            .get_now("acme", &repo_token_key(&widgets_key()))
            .as_deref(),
        Some(""),
        "the credential written before the failure must be blanked"
    );
    assert!(!mgr.mirror_path(&widgets_key()).exists());
}

#[tokio::test]
async fn an_advertised_size_over_quota_is_refused_before_any_transfer() {
    let scratch = Scratch::new("advertised");
    let (mgr, _) = manager(&scratch);
    let mgr = mgr
        // 100 MiB advertised against a 1 MiB cap.
        .with_host(Arc::new(FakeHost::new(100 * 1024)))
        .with_quota(Some(1024 * 1024));

    let err = mgr
        .bind(BindRequest {
            url: WIDGETS_URL.into(),
            token: SENTINEL.into(),
            branches: vec!["main".into()],
        })
        .await
        .unwrap_err();
    assert!(
        matches!(err, OpenCompanyError::WorkspaceQuota(_)),
        "{err:?}"
    );
    assert!(
        !mgr.mirror_path(&widgets_key()).exists(),
        "nothing should have been transferred"
    );
}

// -- pull requests -----------------------------------------------------------

#[tokio::test]
async fn pull_request_reads_through_the_forge_seam_with_the_stored_token() {
    let scratch = Scratch::new("pr");
    let url = fixture_remote(&scratch);
    let (mgr, secrets) = manager(&scratch);
    let host = Arc::new(FakeHost::new(1));
    let mgr = mgr.with_host(host.clone());

    mgr.bind_local(&url, "fixture", vec!["main".into()])
        .await
        .unwrap();
    secrets
        .set(
            &CompanyId::new("acme"),
            &repo_token_key("fixture"),
            SecretValue(SENTINEL.into()),
        )
        .await
        .unwrap();

    let view = mgr.pull_request("fixture", 7).await.unwrap();
    assert_eq!(view.number, 7);
    assert!(view.diff.contains("+++"));
    assert_eq!(
        host.seen_tokens.lock().unwrap().as_slice(),
        [SENTINEL.to_string()],
        "the stored credential is what reaches the forge"
    );
}

#[tokio::test]
async fn pull_request_without_a_forge_client_says_so() {
    let scratch = Scratch::new("pr-unwired");
    let url = fixture_remote(&scratch);
    let (mgr, _) = manager(&scratch);
    mgr.bind_local(&url, "fixture", vec!["main".into()])
        .await
        .unwrap();
    assert!(!mgr.has_host());
    let err = mgr.pull_request("fixture", 7).await.unwrap_err();
    assert!(
        matches!(err, OpenCompanyError::Unimplemented(_)),
        "an unwired forge must not read as an empty diff: {err:?}"
    );
}

// -- push capability (issue #734) --------------------------------------------

/// A fetch probes `permissions.push` and records a push-capable credential, and
/// in doing so **heals a binding that predates the field** without a re-bind:
/// `bind_local` stores no capability (unknown → cannot-push), and the fetch is
/// where the recorded answer becomes `Some(true)`.
#[tokio::test]
async fn a_fetch_probes_and_records_a_push_capable_credential() {
    let scratch = Scratch::new("push-capable");
    let url = fixture_remote(&scratch);
    let (mgr, secrets) = manager(&scratch);
    let mgr = mgr.with_host(Arc::new(FakeHost::new(1).pushable()));

    let bound = mgr
        .bind_local(&url, "fixture", vec!["main".into()])
        .await
        .unwrap();
    // The migration precondition: a binding with no probed capability reads as
    // cannot-push, never as "unknown, allow".
    assert_eq!(
        bound.can_push, None,
        "an unprobed binding must carry no push capability"
    );

    // A credential the forge answers for is what makes the re-probe run.
    secrets
        .set(
            &CompanyId::new("acme"),
            &repo_token_key("fixture"),
            SecretValue(SENTINEL.into()),
        )
        .await
        .unwrap();

    let updated = mgr.fetch("fixture", &[]).await.unwrap();
    assert_eq!(
        updated.can_push,
        Some(true),
        "the fetch must record the probed push capability"
    );
    // And it is persisted, not merely returned.
    let listed = mgr.get("fixture").await.unwrap();
    assert_eq!(listed.can_push, Some(true));
}

/// A read-only credential is recorded as `Some(false)` — a definite
/// cannot-push, distinct from the unknown `None` a pre-field binding carries.
/// This is the value the write tier fails closed on.
#[tokio::test]
async fn a_fetch_records_a_read_only_credential_as_cannot_push() {
    let scratch = Scratch::new("read-only");
    let url = fixture_remote(&scratch);
    let (mgr, secrets) = manager(&scratch);
    // `FakeHost::new` is read-only unless made `pushable`.
    let mgr = mgr.with_host(Arc::new(FakeHost::new(1)));

    mgr.bind_local(&url, "fixture", vec!["main".into()])
        .await
        .unwrap();
    secrets
        .set(
            &CompanyId::new("acme"),
            &repo_token_key("fixture"),
            SecretValue(SENTINEL.into()),
        )
        .await
        .unwrap();

    let updated = mgr.fetch("fixture", &[]).await.unwrap();
    assert_eq!(
        updated.can_push,
        Some(false),
        "a read-only credential must be recorded as a definite cannot-push"
    );
}

/// With no forge client wired, a fetch cannot probe and must leave the recorded
/// capability untouched — an unknown stays unknown (fail-closed), never
/// silently promoted.
#[tokio::test]
async fn a_fetch_without_a_forge_client_leaves_push_capability_unknown() {
    let scratch = Scratch::new("no-host-probe");
    let url = fixture_remote(&scratch);
    let (mgr, _) = manager(&scratch);

    mgr.bind_local(&url, "fixture", vec!["main".into()])
        .await
        .unwrap();
    let updated = mgr.fetch("fixture", &[]).await.unwrap();
    assert_eq!(
        updated.can_push, None,
        "with nothing to probe against, the capability must stay unknown"
    );
}

/// Once the capability is known, a fetch must NOT re-probe. `fetch` runs on the
/// agent checkout path (before every `repo_checkout`), so re-probing a known
/// capability would add a GitHub round trip — and burn rate limit — on every
/// checkout. Only an unknown (pre-field) capability heals; a known one is left
/// alone. Counting the tokens the forge saw is what distinguishes "healed once"
/// from "re-probes every time".
#[tokio::test]
async fn a_known_capability_is_not_re_probed_on_every_fetch() {
    let scratch = Scratch::new("no-reprobe");
    let url = fixture_remote(&scratch);
    let (mgr, secrets) = manager(&scratch);
    let host = Arc::new(FakeHost::new(1).pushable());
    let mgr = mgr.with_host(host.clone());

    mgr.bind_local(&url, "fixture", vec!["main".into()])
        .await
        .unwrap();
    secrets
        .set(
            &CompanyId::new("acme"),
            &repo_token_key("fixture"),
            SecretValue(SENTINEL.into()),
        )
        .await
        .unwrap();

    // First fetch: the capability is unknown (bind_local records none), so it
    // probes exactly once and records the answer.
    let first = mgr.fetch("fixture", &[]).await.unwrap();
    assert_eq!(first.can_push, Some(true));
    assert_eq!(
        host.seen_tokens.lock().unwrap().len(),
        1,
        "the first fetch heals the unknown capability with one probe"
    );

    // Second fetch: the capability is now known, so no further probe is made.
    mgr.fetch("fixture", &[]).await.unwrap();
    assert_eq!(
        host.seen_tokens.lock().unwrap().len(),
        1,
        "a known capability must not be re-probed on a subsequent fetch"
    );
}

// -- publish (issue #735) ----------------------------------------------------

/// Clones `mirror` into `dest` and commits one file, returning the new HEAD SHA.
/// Stands in for an agent's task-scoped checkout with committed work — the thing
/// `stage_publish` fetches from.
fn checkout_with_commit(
    scratch: &Scratch,
    mirror: &Path,
    dest: &Path,
    file: &str,
    body: &str,
) -> String {
    git_at(
        &scratch.0,
        &[
            "clone",
            "--quiet",
            mirror.to_str().unwrap(),
            dest.to_str().unwrap(),
        ],
    );
    for (k, v) in [
        ("user.email", "agent@acme.test"),
        ("user.name", "Agent Seat"),
        ("commit.gpgsign", "false"),
    ] {
        git_at(dest, &["config", k, v]);
    }
    std::fs::write(dest.join(file), body).unwrap();
    git_at(dest, &["add", file]);
    git_at(dest, &["commit", "--quiet", "-m", "agent work"]);
    git_at(dest, &["rev-parse", "HEAD"])
}

/// The happy path end to end: the agent's committed HEAD is staged onto the
/// host-owned `oc/<company>/<task>` ref in the mirror, then pushed to the remote
/// as exactly that branch and commit.
#[tokio::test]
async fn a_publish_stages_the_agents_commit_and_pushes_the_namespaced_branch() {
    let scratch = Scratch::new("publish");
    let url = fixture_remote(&scratch);
    let (mgr, secrets) = manager(&scratch);
    mgr.bind_local(&url, "fixture", vec!["main".into()])
        .await
        .unwrap();
    secrets
        .set(
            &CompanyId::new("acme"),
            &repo_token_key("fixture"),
            SecretValue(SENTINEL.into()),
        )
        .await
        .unwrap();

    let mirror = mgr.mirror_path("fixture");
    let checkout = scratch.join("checkout");
    let head = checkout_with_commit(&scratch, &mirror, &checkout, "FIX.md", "the fix\n");

    // Stage: the agent's HEAD lands on the host-owned branch, host-side, and the
    // exact staged commit is returned so the approval can be bound to it.
    let (branch, staged_head) = mgr
        .stage_publish("fixture", &checkout, "task-1")
        .await
        .unwrap();
    assert_eq!(branch, "oc/acme/task-1", "the branch is host-namespaced");
    assert_eq!(staged_head, head, "stage_publish returns the staged commit");
    let staged = git_at(&mirror, &["rev-parse", "refs/heads/oc/acme/task-1"]);
    assert_eq!(staged, head, "the mirror ref points at the agent's commit");

    // Push: the remote receives exactly that branch and commit.
    mgr.push_published("fixture", &branch, &staged_head)
        .await
        .unwrap();
    let bare = scratch.join("origin.git");
    let pushed = git_at(&bare, &["rev-parse", "refs/heads/oc/acme/task-1"]);
    assert_eq!(pushed, head, "the remote branch is the agent's commit");
}

/// The task id is the one part of the branch that comes from outside the
/// manager, so an unsafe one is refused before any git runs — no `/` (it may not
/// add path segments), no `..`, no leading `-`, no odd characters.
#[tokio::test]
async fn a_publish_task_id_that_is_not_a_safe_segment_is_refused() {
    let scratch = Scratch::new("bad-task");
    let url = fixture_remote(&scratch);
    let (mgr, _) = manager(&scratch);
    mgr.bind_local(&url, "fixture", vec!["main".into()])
        .await
        .unwrap();

    // The task is validated before the checkout is even looked at, so the path
    // does not need to exist for this to refuse.
    let checkout = scratch.join("unused");
    for bad in ["../evil", "a/b", "-rf", "", "with space", "has..dots"] {
        let err = mgr
            .stage_publish("fixture", &checkout, bad)
            .await
            .unwrap_err();
        assert!(
            matches!(err, OpenCompanyError::InvalidRequest(_)),
            "task {bad:?} should be refused: {err:?}"
        );
    }
}

/// The push refuses any branch that is not a publish branch this company owns —
/// the default branch, another company's namespace, the bare prefix, or a
/// traversal — enforced in `RepoManager`, not by the tool description.
#[tokio::test]
async fn a_push_to_anything_but_this_companys_namespace_is_refused() {
    let scratch = Scratch::new("bad-push");
    let url = fixture_remote(&scratch);
    let (mgr, secrets) = manager(&scratch);
    mgr.bind_local(&url, "fixture", vec!["main".into()])
        .await
        .unwrap();
    secrets
        .set(
            &CompanyId::new("acme"),
            &repo_token_key("fixture"),
            SecretValue(SENTINEL.into()),
        )
        .await
        .unwrap();

    for bad in [
        "main",            // the default branch
        "oc/other/task-1", // a foreign company's namespace
        "oc/acme/",        // the bare prefix, no task
        "oc/acme/../main", // a traversal out of the namespace
        "refs/heads/main", // a fully-qualified default ref
    ] {
        // A valid-shaped commit id, so it is the branch that is refused — the
        // branch is re-validated before the head is even looked at.
        let err = mgr
            .push_published("fixture", bad, &"a".repeat(40))
            .await
            .unwrap_err();
        assert!(
            matches!(err, OpenCompanyError::InvalidRequest(_)),
            "branch {bad:?} should be refused: {err:?}"
        );
    }
}

/// The push is never a force push: a branch that already exists on the remote
/// and would not fast-forward is refused by the remote, leaving the earlier
/// commit in place, rather than being overwritten.
#[tokio::test]
async fn a_non_fast_forward_publish_is_refused_never_forced() {
    let scratch = Scratch::new("no-force");
    let url = fixture_remote(&scratch);
    let (mgr, secrets) = manager(&scratch);
    mgr.bind_local(&url, "fixture", vec!["main".into()])
        .await
        .unwrap();
    secrets
        .set(
            &CompanyId::new("acme"),
            &repo_token_key("fixture"),
            SecretValue(SENTINEL.into()),
        )
        .await
        .unwrap();
    let mirror = mgr.mirror_path("fixture");
    let bare = scratch.join("origin.git");

    // First publish: commit A lands on the remote branch.
    let c1 = scratch.join("checkout1");
    let head_a = checkout_with_commit(&scratch, &mirror, &c1, "A.md", "A\n");
    mgr.stage_publish("fixture", &c1, "task-1").await.unwrap();
    mgr.push_published("fixture", "oc/acme/task-1", &head_a)
        .await
        .unwrap();

    // A divergent commit B (a sibling of A, not its descendant) staged onto the
    // same branch and pushed. Since the push never forces, the remote refuses
    // the non-fast-forward.
    let c2 = scratch.join("checkout2");
    let head_b = checkout_with_commit(&scratch, &mirror, &c2, "B.md", "B\n");
    mgr.stage_publish("fixture", &c2, "task-1").await.unwrap();
    let err = mgr
        .push_published("fixture", "oc/acme/task-1", &head_b)
        .await
        .unwrap_err();
    assert!(
        matches!(err, OpenCompanyError::Store(_)),
        "a non-fast-forward publish must be refused, not forced: {err:?}"
    );

    // And the remote still holds A — nothing was clobbered.
    let remote = git_at(&bare, &["rev-parse", "refs/heads/oc/acme/task-1"]);
    assert_eq!(
        remote, head_a,
        "the remote branch must still point at the first commit"
    );
}

/// An approval is bound to the exact commit it was staged for (issue #735). A
/// second publish on the same task force-updates the mirror's branch ref, but
/// approving the first still publishes the first commit — the later re-stage
/// cannot ride in on the earlier approval.
#[tokio::test]
async fn a_publish_pushes_the_approved_commit_even_after_a_restage() {
    let scratch = Scratch::new("bound-commit");
    let url = fixture_remote(&scratch);
    let (mgr, secrets) = manager(&scratch);
    mgr.bind_local(&url, "fixture", vec!["main".into()])
        .await
        .unwrap();
    secrets
        .set(
            &CompanyId::new("acme"),
            &repo_token_key("fixture"),
            SecretValue(SENTINEL.into()),
        )
        .await
        .unwrap();
    let mirror = mgr.mirror_path("fixture");
    let bare = scratch.join("origin.git");

    // First publish stages commit A and records its head.
    let c1 = scratch.join("checkout1");
    checkout_with_commit(&scratch, &mirror, &c1, "A.md", "A\n");
    let (_branch, head_a) = mgr.stage_publish("fixture", &c1, "task-1").await.unwrap();

    // Before it is approved, a second publish on the SAME task stages commit B,
    // force-updating the mirror's branch ref to B.
    let c2 = scratch.join("checkout2");
    let head_b = checkout_with_commit(&scratch, &mirror, &c2, "B.md", "B\n");
    mgr.stage_publish("fixture", &c2, "task-1").await.unwrap();
    assert_ne!(head_a, head_b);
    assert_eq!(
        git_at(&mirror, &["rev-parse", "refs/heads/oc/acme/task-1"]),
        head_b,
        "the mirror ref now points at the re-staged commit B"
    );

    // Approving the FIRST publish pushes A — the commit that approval was bound
    // to — not whatever the branch ref points at now.
    mgr.push_published("fixture", "oc/acme/task-1", &head_a)
        .await
        .unwrap();
    let remote = git_at(&bare, &["rev-parse", "refs/heads/oc/acme/task-1"]);
    assert_eq!(
        remote, head_a,
        "the approved commit reached the remote, not the re-staged one"
    );
}

// -- pull-request creation (issue #736) --------------------------------------

/// A manager with a push-capable binding: `bind_local` records no capability, so
/// a `pushable` host + a fetch heals it to `Some(true)`, the state
/// `open_pull_request` requires.
async fn pushable_bound(scratch: &Scratch, host: Arc<FakeHost>) -> RepoManager {
    let url = fixture_remote(scratch);
    let (mgr, secrets) = manager(scratch);
    let mgr = mgr.with_host(host);
    mgr.bind_local(&url, "fixture", vec!["main".into()])
        .await
        .unwrap();
    secrets
        .set(
            &CompanyId::new("acme"),
            &repo_token_key("fixture"),
            SecretValue(SENTINEL.into()),
        )
        .await
        .unwrap();
    // Heals can_push from None to Some(true) via the pushable host.
    mgr.fetch("fixture", &[]).await.unwrap();
    mgr
}

/// A PR is opened from the published branch into the repository's **default**
/// branch, carrying the title and body the caller built.
#[tokio::test]
async fn open_pull_request_targets_the_default_branch_with_the_given_body() {
    let scratch = Scratch::new("open-pr");
    let host = Arc::new(FakeHost::new(1).pushable());
    let mgr = pushable_bound(&scratch, host.clone()).await;

    let pr = mgr
        .open_pull_request("fixture", "oc/acme/card-1", "the fix", "body with #card-1")
        .await
        .unwrap();
    assert_eq!(pr.number, 42);

    let created = host.created_prs.lock().unwrap();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].head, "oc/acme/card-1");
    assert_eq!(
        created[0].base, "main",
        "the base is the repository's default branch"
    );
    assert_eq!(created[0].title, "the fix");
    assert!(created[0].body.contains("#card-1"));
}

/// With no forge client wired, opening a PR is honestly unavailable rather than a
/// silent success — the shape `pull_request` already uses.
#[tokio::test]
async fn open_pull_request_without_a_forge_client_says_so() {
    let scratch = Scratch::new("open-pr-unwired");
    let url = fixture_remote(&scratch);
    let (mgr, _) = manager(&scratch);
    mgr.bind_local(&url, "fixture", vec!["main".into()])
        .await
        .unwrap();
    let err = mgr
        .open_pull_request("fixture", "oc/acme/card-1", "t", "b")
        .await
        .unwrap_err();
    assert!(matches!(err, OpenCompanyError::Unimplemented(_)), "{err:?}");
}

/// A read-only binding is refused — PR creation rides the same push-capable
/// credential the publish did.
#[tokio::test]
async fn open_pull_request_refuses_a_read_only_binding() {
    let scratch = Scratch::new("open-pr-readonly");
    let url = fixture_remote(&scratch);
    let (mgr, _) = manager(&scratch);
    // A read-only host: bind_local leaves can_push None, and nothing heals it.
    let mgr = mgr.with_host(Arc::new(FakeHost::new(1)));
    mgr.bind_local(&url, "fixture", vec!["main".into()])
        .await
        .unwrap();
    let err = mgr
        .open_pull_request("fixture", "oc/acme/card-1", "t", "b")
        .await
        .unwrap_err();
    assert!(
        matches!(err, OpenCompanyError::InvalidRequest(_)),
        "a read-only binding must be refused: {err:?}"
    );
}

/// When the forge accepts the push but refuses the PR, `open_pull_request`
/// returns the error — the caller (`perform_effect`) is what keeps that from
/// failing the whole publish, reporting it on the task instead.
#[tokio::test]
async fn open_pull_request_surfaces_a_forge_refusal() {
    let scratch = Scratch::new("open-pr-fail");
    let host = Arc::new(FakeHost::new(1).pushable().failing_pr());
    let mgr = pushable_bound(&scratch, host).await;
    let err = mgr
        .open_pull_request("fixture", "oc/acme/card-1", "t", "b")
        .await
        .unwrap_err();
    assert!(matches!(err, OpenCompanyError::Store(_)), "{err:?}");
}

// -- index -------------------------------------------------------------------

#[tokio::test]
async fn an_unreadable_index_is_reported_rather_than_silently_reset() {
    let scratch = Scratch::new("bad-index");
    let (mgr, secrets) = manager(&scratch);
    secrets
        .set(
            &CompanyId::new("acme"),
            REPO_INDEX_KEY,
            SecretValue("{ not json".into()),
        )
        .await
        .unwrap();
    let err = mgr.list().await.unwrap_err();
    assert!(matches!(err, OpenCompanyError::Store(_)), "{err:?}");
}

#[tokio::test]
async fn an_absent_or_blank_index_lists_nothing() {
    let scratch = Scratch::new("empty-index");
    let (mgr, secrets) = manager(&scratch);
    assert!(mgr.list().await.unwrap().is_empty());
    secrets
        .set(
            &CompanyId::new("acme"),
            REPO_INDEX_KEY,
            SecretValue(String::new()),
        )
        .await
        .unwrap();
    assert!(mgr.list().await.unwrap().is_empty());
}

#[test]
fn byte_counts_read_as_sizes() {
    assert_eq!(human_bytes(512), "512 B");
    assert_eq!(human_bytes(2048), "2.0 KiB");
    assert_eq!(human_bytes(3 * 1024 * 1024 * 1024), "3.0 GiB");
}
