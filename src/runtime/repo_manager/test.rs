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

/// A forge that answers from a script, and records the token it was handed.
struct FakeHost {
    meta: RepoMeta,
    seen_tokens: StdMutex<Vec<String>>,
    fail: bool,
}

impl FakeHost {
    fn new(size_kb: u64) -> Self {
        Self {
            meta: RepoMeta {
                default_branch: "main".into(),
                size_kb,
            },
            seen_tokens: StdMutex::new(Vec::new()),
            fail: false,
        }
    }

    fn failing() -> Self {
        Self {
            fail: true,
            ..Self::new(1)
        }
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

/// Runs git in `cwd`, panicking with its stderr on failure.
fn git_at(cwd: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?}: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
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
    );
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
            url: "https://github.com/acme/widgets".into(),
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
            .get_now("acme", &repo_token_key("acme-widgets"))
            .is_none()
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
    mgr.bind_local(&url, "acme-widgets", vec!["main".into()])
        .await
        .unwrap();

    let err = mgr
        .bind(BindRequest {
            url: "https://github.com/acme/widgets".into(),
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
            .get_now("acme", &repo_token_key("acme-widgets"))
            .is_none(),
        "a refused rebind must not have touched the stored credential"
    );
    assert_eq!(mgr.read_index().await.unwrap().bindings.len(), 1);
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
            url: "https://github.com/acme/widgets".into(),
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
            .get_now("acme", &repo_token_key("acme-widgets"))
            .as_deref(),
        Some(""),
        "the credential written before the failure must be blanked"
    );
    assert!(!mgr.mirror_path("acme-widgets").exists());
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
            url: "https://github.com/acme/widgets".into(),
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
        !mgr.mirror_path("acme-widgets").exists(),
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
