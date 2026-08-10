//! Exclusive ownership of an instance data root.
//!
//! ## Why this exists
//!
//! `docs/spec/runtime/storage.md` is explicit that the runtime journal is
//! single-writer and that two processes sharing one home write over each
//! other's companies. Until now nothing enforced it: [`resolve_home`] hands any
//! number of processes the same directory and every one of them opens it.
//!
//! On a server that was survivable, because starting a second `opencompany
//! serve` against one data root is a deliberate act. A desktop application is
//! different in kind — it is launched by double-clicking, and being launched
//! twice is ordinary rather than exceptional. The same is true of the common
//! development shape: `opencompany serve` in a terminal against
//! `~/.opencompany`, and then the desktop app opening the same root.
//!
//! So the second instance has to be refused, and refused with something an
//! operator can act on rather than a corrupted store discovered later.
//!
//! ## Why an OS advisory lock and not a pid file
//!
//! A pid file has to answer "is the process that wrote this still alive?", and
//! every answer is platform-specific and racy — a pid is reused, `/proc` is
//! Linux-only, and a crash leaves a file that looks exactly like a live one. An
//! operator then has to be told to delete a lock file, which is a footgun
//! pointed at the data the lock protects.
//!
//! `flock`/`LockFileEx` has none of that: the lock is held by the *open file
//! description*, so the kernel drops it when the process exits for any reason —
//! clean exit, panic, `SIGKILL`, or power loss. There is no stale state to
//! reason about and nothing for anyone to delete by hand.
//!
//! `fs2` is already in `Cargo.lock` as a transitive dependency, so declaring it
//! directly adds no new download — the same reasoning the crate applies to
//! `base64` and `url`.
//!
//! ## What it does not do
//!
//! This is a *process* boundary on one machine, not a distributed one. Two
//! hosts sharing a network filesystem are outside what `flock` promises, and
//! the layout was never safe to share that way regardless.
//!
//! ## The fork window
//!
//! The lock belongs to the open file description, and between `fork()` and
//! `exec()` a child shares every descriptor its parent held. So a process that
//! spawns subprocesses — the desktop does, one per ACP harness — can, for the
//! microseconds of that window, keep a released root locked. The descriptor is
//! `O_CLOEXEC` (Rust's default), so it closes the instant the child `exec`s and
//! a spawned harness never holds the root for its lifetime.
//!
//! Stated because it is observable: releasing and immediately re-acquiring in
//! the same instant can fail. Nothing real does that — a person quits and
//! relaunches seconds later — but a test that asserts instantaneous release is
//! asserting something stricter than the lock promises.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use fs2::FileExt;

use crate::Result;
use crate::error::OpenCompanyError;

/// The lock file's name inside the data root.
const LOCK_FILE: &str = ".lock";

/// Exclusive ownership of one data root, released when this value is dropped.
///
/// Must be held for as long as the instance is running. Dropping it early
/// releases the root to another process while this one is still writing, which
/// is the thing the lock exists to prevent — so bind it for the lifetime of the
/// server rather than to a narrower scope.
#[derive(Debug)]
#[must_use = "the lock is released as soon as this is dropped"]
pub struct HomeLock {
    path: PathBuf,
    /// Held open because the lock belongs to the open file description. Closing
    /// the handle releases it.
    _file: File,
}

impl HomeLock {
    /// The lock file backing this guard.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Takes exclusive ownership of `home`, or reports who has it.
///
/// The lock file is created if absent and **never removed**: an empty file is
/// harmless, and deleting it on release would race a second process that has
/// already opened the same path and is about to lock it — the classic
/// unlink-under-a-lock bug, where both processes end up holding a lock on
/// different files with the same name.
pub fn acquire(home: &Path) -> Result<HomeLock> {
    std::fs::create_dir_all(home).map_err(|error| {
        OpenCompanyError::Config(format!(
            "cannot create the instance data root at {}: {error}",
            home.display()
        ))
    })?;

    let path = home.join(LOCK_FILE);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|error| {
            OpenCompanyError::Config(format!(
                "cannot open the instance lock at {}: {error}",
                path.display()
            ))
        })?;

    // Non-blocking on purpose. Waiting would leave a desktop app apparently
    // hung on launch with nothing said, and the honest answer — "something else
    // is already using this data directory" — is available immediately.
    file.try_lock_exclusive().map_err(|_| {
        OpenCompanyError::Config(format!(
            "another OpenCompany instance is already using the data directory at {}. \
             Only one process may write an instance root: two would overwrite each \
             other's companies and journals. Stop the other instance, or point this \
             one somewhere else with OPENCOMPANY_DATA_DIR (or --home).",
            home.display()
        ))
    })?;

    Ok(HomeLock { path, _file: file })
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn a_second_instance_is_refused_while_the_first_holds_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let first = acquire(dir.path()).expect("the first instance takes the root");

        let refused = acquire(dir.path()).expect_err("the second must be refused");
        let message = refused.to_string();
        // The message has to name the directory: an operator seeing this needs
        // to know *which* root is contended, not merely that one is.
        assert!(
            message.contains(&dir.path().display().to_string()),
            "the refusal must name the data root: {message}"
        );
        assert!(
            message.contains("OPENCOMPANY_DATA_DIR"),
            "the refusal must say how to run a second instance anyway: {message}"
        );

        drop(first);
    }

    #[test]
    fn the_root_is_available_again_once_released() {
        let dir = tempfile::tempdir().unwrap();
        let first = acquire(dir.path()).unwrap();
        drop(first);

        // Not merely "does not error": a lock that could not be retaken after a
        // clean shutdown would make every restart of a desktop app fail.
        let _retaken = acquire(dir.path()).expect("a released root is takeable");
    }

    #[test]
    fn two_roots_do_not_contend() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let _first = acquire(a.path()).unwrap();
        let _second = acquire(b.path()).expect("a different root is unaffected");
    }

    #[test]
    fn the_root_is_created_when_it_does_not_exist_yet() {
        // First launch of a desktop app: the platform data directory has never
        // been written to.
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("does/not/exist/yet");
        let _lock = acquire(&nested).expect("a fresh root is created and locked");
        assert!(nested.join(LOCK_FILE).is_file());
    }

    #[test]
    fn an_existing_lock_file_is_reused_rather_than_truncated() {
        // The file is never deleted on release, so the next run opens one that
        // already exists. Truncating would be harmless today and a data loss the
        // moment anything is ever written into it.
        let dir = tempfile::tempdir().unwrap();
        drop(acquire(dir.path()).unwrap());
        std::fs::write(dir.path().join(LOCK_FILE), b"marker").unwrap();

        let _lock = acquire(dir.path()).unwrap();
        assert_eq!(
            std::fs::read(dir.path().join(LOCK_FILE)).unwrap(),
            b"marker"
        );
    }
}
