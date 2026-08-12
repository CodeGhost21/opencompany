//! Canonical per-instance workspace layout under the data directory.
//!
//! `OPENCOMPANY_DATA_DIR` (the workspace root — `/data` in a hosted tenant
//! container, `$HOME/.opencompany` by default) holds everything a running
//! instance owns. [`DataLayout`] names the canonical subdirectories so stores,
//! agents, and tools resolve well-known locations instead of ad-hoc paths, and
//! owns their lifecycle: [`ensure`](DataLayout::ensure) creates them on boot and
//! — when asked (`[workspace].clear_tmp_on_startup`, on by default) — clears the
//! ephemeral `tmp/` scratch so none survives a restart.
//!
//! Per-company bundles live under [`companies_dir`](DataLayout::companies_dir)
//! (`companies/<slug>/`), each carrying its own `memory/`/`context/`. The
//! top-level [`memory_dir`](DataLayout::memory_dir) and friends are therefore
//! the *instance-shared* locations, distinct from per-company state, and are
//! created empty as the reserved home for shared artifacts.

use std::path::{Path, PathBuf};

use crate::Result;
use crate::error::OpenCompanyError;

/// The canonical directory layout under one instance's data root.
#[derive(Clone, Debug)]
pub struct DataLayout {
    root: PathBuf,
}

impl DataLayout {
    /// Roots a layout at `root` (the resolved `OPENCOMPANY_DATA_DIR`).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The workspace root (the data directory itself).
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Per-company bundle directories (`companies/<slug>/`). Owned by the fs
    /// store, which creates each company's bundle lazily; listed here so callers
    /// resolve it through the layout rather than a literal.
    pub fn companies_dir(&self) -> PathBuf {
        self.root.join("companies")
    }

    /// One company's repository mirror cache (`companies/<slug>/repos/`).
    ///
    /// **Owned by the repo cache, not by the fs bundle.** It shares the
    /// `companies/<slug>/` prefix with [`companies_dir`](Self::companies_dir)
    /// so a company's whole footprint stays in one subtree, but nothing in the
    /// fs store creates or reads it — and on a mongodb tenant the bundle
    /// directory has no other reason to exist at all, so the cache must create
    /// its own parents rather than assume a bundle put them there.
    ///
    /// It is deliberately **outside** any agent workspace
    /// (`harness/<company>/<agent>/workspace`): the mirrors are fetched
    /// host-side with a credential, and an agent that could write to them could
    /// rewrite what every later checkout sees.
    ///
    /// Nothing extra is needed to keep it inside the soft quota:
    /// [`usage_bytes`](Self::usage_bytes) already sums every regular file under
    /// the root, and this hangs off the root like everything else.
    pub fn company_repos_dir(&self, slug: &str) -> PathBuf {
        self.companies_dir().join(slug).join("repos")
    }

    /// One agent's shell audit sink (`companies/<slug>/audit/<agent_id>/`).
    ///
    /// **Host-owned, not part of the fs bundle**, exactly like
    /// [`company_repos_dir`](Self::company_repos_dir): it shares the
    /// `companies/<slug>/` prefix so a company's whole footprint stays in one
    /// subtree, but nothing in the fs store creates or reads it — and on a
    /// mongodb tenant the bundle directory has no other reason to exist, so
    /// whoever opens the sink must create its own parents rather than assume a
    /// bundle put them there.
    ///
    /// It is deliberately **outside** any agent workspace
    /// (`harness/<company>/<agent>/workspace`), which is also the
    /// `workspace_only` `SecurityPolicy` root the file tools enforce. While the
    /// sink lived inside that root, rewriting the record of an agent's own
    /// commands was a *policy-permitted* write through its ordinary file tools,
    /// not merely something `shell` could reach (issue #775). Moving it here
    /// turns those writes from permitted into refused.
    ///
    /// **One directory per agent, not one shared directory per company.**
    /// OpenHuman's `get_or_create_workspace_audit_logger` caches one logger per
    /// *directory* and the first caller's config wins, so a shared directory
    /// with per-agent file names would silently hand the second agent the first
    /// agent's log file.
    ///
    /// This is not tamper-evidence. Everything in the tenant is one uid and one
    /// process, so a deliberate shell command against this path still succeeds;
    /// see `docs/spec/security/agent-isolation.md`.
    ///
    /// Nothing extra is needed to keep it inside the soft quota:
    /// [`usage_bytes`](Self::usage_bytes) already sums every regular file under
    /// the root, and this hangs off the root like everything else.
    pub fn agent_audit_dir(&self, slug: &str, agent_id: &str) -> PathBuf {
        self.companies_dir()
            .join(slug)
            .join("audit")
            .join(agent_id)
    }

    /// Instance-shared memory artifacts.
    pub fn memory_dir(&self) -> PathBuf {
        self.root.join("memory")
    }

    /// Instance-shared durable-store artifacts.
    pub fn store_dir(&self) -> PathBuf {
        self.root.join("store")
    }

    /// Instance-shared file artifacts (exports, attachments).
    pub fn files_dir(&self) -> PathBuf {
        self.root.join("files")
    }

    /// Instance logs.
    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    /// Ephemeral scratch, cleared on startup.
    pub fn tmp_dir(&self) -> PathBuf {
        self.root.join("tmp")
    }

    /// The canonical shared subdirectories, in creation order.
    fn shared_dirs(&self) -> [PathBuf; 5] {
        [
            self.memory_dir(),
            self.store_dir(),
            self.files_dir(),
            self.logs_dir(),
            self.tmp_dir(),
        ]
    }

    /// Materializes the layout: clears the ephemeral `tmp/` scratch (when
    /// `clear_tmp`) so nothing stale survives a restart, then creates every
    /// canonical shared subdirectory. Idempotent — existing directories are
    /// left in place.
    ///
    /// The per-company `companies/` tree is intentionally not pre-created: the
    /// fs store owns it and mints each bundle on demand.
    pub async fn ensure(&self, clear_tmp: bool) -> Result<()> {
        if clear_tmp {
            match tokio::fs::remove_dir_all(self.tmp_dir()).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(OpenCompanyError::Store(format!(
                        "clearing tmp {}: {e}",
                        self.tmp_dir().display()
                    )));
                }
            }
        }
        for dir in self.shared_dirs() {
            tokio::fs::create_dir_all(&dir)
                .await
                .map_err(|e| OpenCompanyError::Store(format!("creating {}: {e}", dir.display())))?;
        }
        Ok(())
    }

    /// Total size in bytes of everything under the workspace root, for the
    /// soft-quota check. Used by `serve` to alert when a workspace exceeds its
    /// configured `[workspace].storage_quota_gb`.
    pub async fn usage_bytes(&self) -> Result<u64> {
        dir_bytes(self.root.clone()).await
    }

    /// Size in bytes of the ephemeral `tmp/` scratch directory.
    pub async fn tmp_bytes(&self) -> Result<u64> {
        dir_bytes(self.tmp_dir()).await
    }
}

/// Recursively sums the byte size of regular files under `dir`. A missing
/// directory is `0`, not an error. Symlinks are not followed (an iterative
/// stack walk, so no recursion depth limit and no symlink loops).
async fn dir_bytes(dir: PathBuf) -> Result<u64> {
    let read_err = |p: &Path, e: std::io::Error| {
        OpenCompanyError::Store(format!("measuring {}: {e}", p.display()))
    };
    let mut total = 0u64;
    let mut stack = vec![dir];
    while let Some(path) = stack.pop() {
        let mut entries = match tokio::fs::read_dir(&path).await {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(read_err(&path, e)),
        };
        while let Some(entry) = entries.next_entry().await.map_err(|e| read_err(&path, e))? {
            // `DirEntry::metadata` does not follow symlinks, so a symlink is
            // neither dir nor file here and is simply skipped.
            let meta = match entry.metadata().await {
                Ok(meta) => meta,
                // A file removed mid-walk (e.g. tmp churn) just isn't counted.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(read_err(&entry.path(), e)),
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

#[cfg(test)]
mod test {
    use super::*;

    fn scratch_root(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("oc-layout-{}-{tag}", std::process::id()))
    }

    #[test]
    fn subdirs_hang_off_the_root() {
        let layout = DataLayout::new("/data");
        assert_eq!(layout.root(), Path::new("/data"));
        assert_eq!(layout.companies_dir(), Path::new("/data/companies"));
        assert_eq!(layout.tmp_dir(), Path::new("/data/tmp"));
        assert_eq!(layout.memory_dir(), Path::new("/data/memory"));
        assert_eq!(
            layout.company_repos_dir("acme"),
            Path::new("/data/companies/acme/repos"),
        );
        assert_eq!(
            layout.agent_audit_dir("acme", "ceo"),
            Path::new("/data/companies/acme/audit/ceo"),
        );
    }

    /// The whole point of issue #775: the audit sink must not be reachable from
    /// the agent workspace subtree, which is the `workspace_only` policy root
    /// the file tools sandbox to. Pinned as a *path* property here, and proven
    /// against the real file tools in `crate::harness::audit`'s tests.
    #[test]
    fn the_audit_sink_is_outside_every_agent_workspace() {
        let layout = DataLayout::new("/data");
        let audit = layout.agent_audit_dir("acme", "ceo");
        // The harness roots every agent workspace at `<root>/harness/...`.
        let workspaces = Path::new("/data/harness");
        assert!(
            !audit.starts_with(workspaces),
            "{} must not sit under the agent-workspace tree {}",
            audit.display(),
            workspaces.display(),
        );
        // Two agents in one company never share a directory — the vendored
        // logger registry caches per directory with first-config-wins, so a
        // shared directory would hand agent B agent A's file.
        assert_ne!(
            layout.agent_audit_dir("acme", "ceo"),
            layout.agent_audit_dir("acme", "cto"),
        );
    }

    #[tokio::test]
    async fn ensure_creates_the_shared_subdirs() {
        let root = scratch_root("create");
        let layout = DataLayout::new(&root);
        layout.ensure(true).await.unwrap();
        for dir in layout.shared_dirs() {
            assert!(dir.is_dir(), "{} should exist", dir.display());
        }
        tokio::fs::remove_dir_all(&root).await.ok();
    }

    #[tokio::test]
    async fn ensure_clears_tmp_but_keeps_it_when_asked() {
        let root = scratch_root("tmp");
        let layout = DataLayout::new(&root);
        layout.ensure(true).await.unwrap();

        let scratch = layout.tmp_dir().join("scratch.txt");
        tokio::fs::write(&scratch, b"stale").await.unwrap();

        // clear_tmp = false keeps the scratch file.
        layout.ensure(false).await.unwrap();
        assert!(scratch.exists(), "clear_tmp=false must keep tmp contents");

        // clear_tmp = true wipes it (but tmp/ itself is recreated).
        layout.ensure(true).await.unwrap();
        assert!(!scratch.exists(), "clear_tmp=true must empty tmp");
        assert!(
            layout.tmp_dir().is_dir(),
            "tmp/ is recreated after clearing"
        );

        tokio::fs::remove_dir_all(&root).await.ok();
    }

    #[tokio::test]
    async fn usage_bytes_sums_files_recursively() {
        let root = scratch_root("usage");
        let layout = DataLayout::new(&root);
        layout.ensure(true).await.unwrap();
        tokio::fs::write(layout.files_dir().join("a.bin"), vec![0u8; 1000])
            .await
            .unwrap();
        tokio::fs::write(layout.tmp_dir().join("scratch.bin"), vec![0u8; 500])
            .await
            .unwrap();

        assert_eq!(
            layout.usage_bytes().await.unwrap(),
            1500,
            "root sums all files"
        );
        assert_eq!(layout.tmp_bytes().await.unwrap(), 500, "tmp/ subtree only");

        // A missing workspace measures zero, not an error.
        let absent = DataLayout::new(scratch_root("absent"));
        assert_eq!(absent.usage_bytes().await.unwrap(), 0);

        tokio::fs::remove_dir_all(&root).await.ok();
    }

    /// The repo mirror cache joins the boot quota measurement for free — it is
    /// a subtree of the root, and `usage_bytes` walks the whole root. Asserted
    /// rather than assumed: a cache that measured zero would let one bad clone
    /// fill a tenant volume with the quota check reporting all clear.
    #[tokio::test]
    async fn usage_bytes_counts_the_repo_cache() {
        let root = scratch_root("repos-usage");
        let layout = DataLayout::new(&root);
        layout.ensure(true).await.unwrap();

        let repos = layout.company_repos_dir("acme");
        // `companies/<slug>/` does not exist yet on a mongodb tenant, so the
        // cache creates its own parents. That is the case measured here.
        tokio::fs::create_dir_all(repos.join("acme-widgets.git/objects"))
            .await
            .unwrap();
        tokio::fs::write(
            repos.join("acme-widgets.git/objects/pack.bin"),
            vec![0u8; 4096],
        )
        .await
        .unwrap();

        assert_eq!(
            layout.usage_bytes().await.unwrap(),
            4096,
            "the repo mirror cache must be inside the measured root"
        );

        tokio::fs::remove_dir_all(&root).await.ok();
    }

    /// The shell audit sink joins the soft quota for free, for the same reason
    /// the repo cache does — it hangs off the measured root. Asserted rather
    /// than assumed: a sink that measured zero would let a runaway command loop
    /// fill a tenant volume with the quota check reporting all clear.
    #[tokio::test]
    async fn usage_bytes_counts_the_agent_audit_sink() {
        let root = scratch_root("audit-usage");
        let layout = DataLayout::new(&root);
        layout.ensure(true).await.unwrap();

        let audit = layout.agent_audit_dir("acme", "ceo");
        // `companies/<slug>/` does not exist yet on a mongodb tenant, so the
        // sink creates its own parents. That is the case measured here.
        tokio::fs::create_dir_all(&audit).await.unwrap();
        tokio::fs::write(audit.join("audit.log"), vec![0u8; 2048])
            .await
            .unwrap();

        assert_eq!(
            layout.usage_bytes().await.unwrap(),
            2048,
            "the audit sink must be inside the measured root"
        );

        tokio::fs::remove_dir_all(&root).await.ok();
    }
}
