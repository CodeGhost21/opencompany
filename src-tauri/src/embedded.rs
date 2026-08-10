//! The host, running inside the desktop process.
//!
//! ## Why it still binds a real socket
//!
//! The obvious optimisation is to skip the network: hold the axum `Router` and
//! drive it in-process through `tower::Service`. It is rejected on purpose.
//!
//! With a real listener, the embedded path exercises the identical
//! serialisation, auth extractors (`ScopedCompany`, the session carrier), CORS
//! branch, error envelopes and event framing as a remote host. Every Playwright
//! spec, every future ACP conformance test, and the proxy's own tests are then
//! valid evidence about embedded mode too. Skipping the socket saves perhaps
//! 50µs per request and buys a *second code path* that will diverge — and
//! divergence in an auth extractor is precisely the class of bug that cannot be
//! afforded.
//!
//! ## Loopback and an ephemeral port
//!
//! `127.0.0.1:0`, never `0.0.0.0`: an embedded instance is this machine's, and
//! binding a routable address would quietly publish someone's company to their
//! network. Port `0` because a fixed 8080 collides with a dev server or a second
//! app — a support case that reads "it works unless I have a terminal open".
//! The OS picks; [`EmbeddedHost::address`] reports what it picked.

use std::path::PathBuf;

use opencompany::app::EmbeddedInstance;
use opencompany::{AppConfig, AppState};

/// A running in-process host.
pub struct EmbeddedHost {
    address: std::net::SocketAddr,
    /// Holds the data root's exclusive lock for as long as the host runs.
    /// Dropping it would release the root while this process kept writing.
    _instance: EmbeddedInstance,
    server: tokio::task::JoinHandle<()>,
}

impl EmbeddedHost {
    /// The loopback address the console should point at.
    pub fn address(&self) -> std::net::SocketAddr {
        self.address
    }

    /// The base URL for a connection record.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.address)
    }
}

impl Drop for EmbeddedHost {
    fn drop(&mut self) {
        // The task owns the listener; aborting it closes the port. Without this
        // a restarted embedded host would leak a listener per restart.
        self.server.abort();
    }
}

/// Boots a host over `data_dir`.
///
/// `data_dir` is passed explicitly rather than resolved from the environment.
/// A desktop app knows its platform data directory and should say so — and the
/// crate's own fallback resolves a *relative* path when neither `HOME` nor
/// `USERPROFILE` is set, which for a double-clicked application is wherever the
/// launcher happened to put it.
pub async fn start(data_dir: PathBuf) -> opencompany::Result<EmbeddedHost> {
    // Resolve, lock, migrate, and prove the journal root is writable — the same
    // sequence `serve` runs, shared rather than copied so the two cannot drift.
    // The lock is what refuses a second instance over one data root, including
    // the very ordinary case of a terminal already running `opencompany serve`
    // against the same default.
    let instance = opencompany::app::prepare_instance(Some(data_dir)).await?;

    let config = AppConfig {
        bind: "127.0.0.1:0".to_string(),
        ..AppConfig::default()
    };
    let state = AppState::new(config).with_home(instance.home().to_path_buf());

    let (address, serving) = opencompany::server::bind("127.0.0.1:0", state).await?;
    let server = tokio::spawn(async move {
        if let Err(error) = serving.run().await {
            tracing::error!(%error, "the embedded host stopped");
        }
    });

    tracing::info!(%address, home = %instance.home().display(), "embedded host listening");
    Ok(EmbeddedHost {
        address,
        _instance: instance,
        server,
    })
}

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn an_embedded_host_answers_on_loopback() {
        let dir = tempfile::tempdir().unwrap();
        let host = start(dir.path().to_path_buf()).await.expect("host starts");

        assert!(host.address().ip().is_loopback(), "must never be routable");
        assert_ne!(
            host.address().port(),
            0,
            "the OS-chosen port must be reported"
        );

        let health = reqwest::get(format!("{}/healthz", host.base_url()))
            .await
            .expect("the reported address is reachable");
        assert!(health.status().is_success());
    }

    #[tokio::test]
    async fn the_embedded_host_holds_its_data_root() {
        // The desktop being launched twice is ordinary rather than exceptional,
        // and two hosts over one root overwrite each other's companies.
        let dir = tempfile::tempdir().unwrap();
        let _first = start(dir.path().to_path_buf())
            .await
            .expect("the first starts");

        let second = start(dir.path().to_path_buf()).await;
        assert!(
            second.is_err(),
            "a second host over one root must be refused"
        );
    }

    #[tokio::test]
    async fn two_embedded_hosts_over_different_roots_coexist() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let first = start(a.path().to_path_buf()).await.unwrap();
        let second = start(b.path().to_path_buf()).await.unwrap();
        assert_ne!(first.address().port(), second.address().port());
    }

    #[tokio::test]
    async fn stopping_a_host_frees_its_root_and_its_port() {
        let dir = tempfile::tempdir().unwrap();
        let host = start(dir.path().to_path_buf()).await.unwrap();
        drop(host);

        // Both resources come back: a desktop restarted after a clean quit must
        // start, and it must not leak a listener per restart.
        //
        // Retried briefly, and the reason is worth recording rather than
        // hiding behind a sleep. `flock` belongs to the *open file
        // description*, and between `fork()` and `exec()` a concurrently
        // spawned child shares every descriptor its parent had. So if anything
        // else in this process spawns a subprocess in the same instant this
        // host releases its root — and the suite does, constantly: `git` in the
        // worktree tests, `python3` in the ACP ones — the lock survives until
        // that child reaches `exec` and `O_CLOEXEC` closes it. Microseconds,
        // and reproducible here about one run in five.
        //
        // Not worth engineering away: the production shape is a person quitting
        // and relaunching seconds later, and a harness the desktop spawned
        // always execs. Asserting instantaneous release would be asserting
        // something stricter than the product needs, so this asserts what it
        // does need — that the root comes back promptly.
        let mut last = None;
        for _ in 0..50 {
            match start(dir.path().to_path_buf()).await {
                Ok(host) => {
                    assert_ne!(host.address().port(), 0);
                    return;
                }
                Err(error) => last = Some(error),
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("a released root must become takeable: {last:?}");
    }
}
