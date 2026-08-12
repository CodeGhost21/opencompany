//! Host-side `git`, run the only way this surface is allowed to run it.
//!
//! Two things live here, and neither is optional.
//!
//! ## The credential never becomes an argument or a variable
//!
//! A token handed to `git` the obvious ways is readable by anything running as
//! the same user for as long as the fetch lasts: `/proc/<pid>/cmdline` for an
//! argument, `/proc/<pid>/environ` for an environment variable, and the file
//! itself forever for a persisted `credential.helper` or a URL with userinfo in
//! it. So none of those are used. [`run`] spawns `git` with a
//! [`GIT_ASKPASS`](https://git-scm.com/docs/git#Documentation/git.txt-codeGITASKPASScode)
//! helper — a fixed six-line script containing no secret — and writes the token
//! into the child's **stdin**, which is an anonymous pipe. The helper reads one
//! line from it and prints it back to `git`. The bytes exist in a pipe buffer
//! and in two process memories, and nowhere else: not in argv, not in the
//! environment, not on disk, not in any git config.
//!
//! Two deliberate details:
//!
//! * **stdin, rather than a dedicated inherited descriptor.** Passing `git` an
//!   extra open descriptor means `pre_exec` and a `dup2`, i.e. `unsafe` plus a
//!   new `libc` dependency, to obtain a pipe with exactly the properties stdin
//!   already has. The commands run here (`init`, `remote`, `config`,
//!   `ls-remote`, `fetch`) read nothing from stdin, so it is free, and the
//!   helper inherits it because `git` passes its own stdio through.
//! * **`printf`/`echo` are shell builtins.** The helper never passes the token
//!   to an external program, which would put it back in an argv.
//!
//! Honest limit, restated because it is easy to oversell this: the agent shell
//! and this fetch run as the same user in the same container. This closes the
//! *incidental* exposures — a token in a log, in a config file, in a process
//! listing, in a workspace `.git/config` — which is a real and worthwhile
//! class. It is not a boundary against a determined prompt-injected agent that
//! holds `shell` and attacks `/proc` during a fetch window. See
//! `docs/spec/runtime/repos.md`.
//!
//! ## Repository configuration decides what runs
//!
//! Issue #459 reclassified `read_workspace_state` for exactly this: it shells
//! out to `git` inside a directory an agent can write, and a repository's own
//! configuration can name programs for git to execute. Every invocation here
//! therefore pins `core.hooksPath` at `/dev/null` and clears `core.fsmonitor`,
//! and starts from an empty environment with `GIT_CONFIG_NOSYSTEM` and a
//! `$HOME` that has no `.gitconfig` in it — so neither the operator's own git
//! configuration nor a cloned repository's can inject a command into a
//! host-side fetch.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::io::AsyncWriteExt;

use crate::Result;
use crate::error::OpenCompanyError;

/// The username git is answered with. Not a secret — it is the fixed literal
/// GitHub documents for token authentication, and the token is the password.
///
/// Test-only as a Rust item: the running answer is the literal inside
/// [`ASKPASS_SCRIPT`], and this exists so the tests below can assert the two
/// agree rather than restating the string a third time.
#[cfg(test)]
const TOKEN_USERNAME: &str = "x-access-token";

/// How long any one `git` invocation may take before it is killed.
///
/// Not a nicety. Every caller here sits on an HTTP request thread, and a `git`
/// that reaches a network it cannot finish talking to does not fail — it waits.
/// A credential the remote will neither accept nor refuse (an expired
/// fine-grained token against a repository that has since gone private is the
/// easy one) leaves `git-remote-https` parked indefinitely, and without this
/// the route parks with it. Generous enough for a first mirror of a large
/// repository over a slow link; finite, which is the point.
const GIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// The askpass helper. Contains no credential: the password branch reads one
/// line from stdin, which the parent wrote the token into.
///
/// `read` and `echo` are POSIX shell builtins, so the token never appears in
/// any process's argument list. GitHub tokens are `[A-Za-z0-9_]` only, so
/// `echo` cannot mangle one.
const ASKPASS_SCRIPT: &str = r#"#!/bin/sh
# Answers git's credential prompts for an OpenCompany host-side fetch.
# Holds no secret: the password is read from stdin, an anonymous pipe.
case "$1" in
  Username*|username*) echo "x-access-token" ;;
  *) IFS= read -r secret || exit 1; echo "$secret" ;;
esac
"#;

/// A completed `git` invocation.
#[derive(Debug)]
pub(crate) struct GitOutput {
    /// Whether git exited zero.
    pub(crate) ok: bool,
    /// Captured stdout, lossily decoded.
    pub(crate) stdout: String,
    /// Captured stderr, lossily decoded.
    pub(crate) stderr: String,
}

impl GitOutput {
    /// The trimmed stdout, or a `Store` error naming `what` and carrying git's
    /// stderr, when git failed.
    pub(crate) fn require(self, what: &str) -> Result<String> {
        if self.ok {
            return Ok(self.stdout.trim().to_string());
        }
        Err(OpenCompanyError::Store(format!(
            "{what} failed: {}",
            first_lines(&self.stderr, 4)
        )))
    }
}

/// The first `n` non-empty lines of git's stderr, joined — enough to say what
/// went wrong without pasting a wall of progress output into an API response.
fn first_lines(text: &str, n: usize) -> String {
    let joined: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(n)
        .collect();
    if joined.is_empty() {
        "git wrote nothing to stderr".to_string()
    } else {
        joined.join("; ")
    }
}

/// A temporary directory holding the askpass helper (and standing in as the
/// child's `$HOME`), removed when dropped.
///
/// Not `tempfile`: that crate is a dev-dependency here, and pulling it into the
/// shipped build to create one 0700 directory would be a poor trade.
pub(crate) struct AskpassDir {
    path: PathBuf,
}

impl AskpassDir {
    /// Creates a private scratch directory under `base` and writes the helper
    /// into it.
    ///
    /// A fresh one per invocation, rather than one written once at bind time:
    /// the script is a program this host is about to execute, and re-writing it
    /// means nothing that happened between two fetches can have edited it.
    pub(crate) fn create(base: &Path) -> Result<Self> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);

        let io_err = |p: &Path, e: std::io::Error| {
            OpenCompanyError::Store(format!(
                "preparing git credential helper {}: {e}",
                p.display()
            ))
        };
        let path = base.join(format!(
            ".askpass-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).map_err(|e| io_err(&path, e))?;
        set_mode(&path, 0o700)?;
        let script = path.join("askpass.sh");
        std::fs::write(&script, ASKPASS_SCRIPT).map_err(|e| io_err(&script, e))?;
        set_mode(&script, 0o700)?;
        Ok(Self { path })
    }

    /// The helper script's path, for `GIT_ASKPASS`.
    fn script(&self) -> PathBuf {
        self.path.join("askpass.sh")
    }
}

impl Drop for AskpassDir {
    fn drop(&mut self) {
        // Best effort: the directory holds no secret, so a failure to clean up
        // is untidy rather than dangerous, and there is no caller to report to.
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Restricts a path to the owning user. A no-op off unix, where this surface
/// does not run.
fn set_mode(path: &Path, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .map_err(|e| OpenCompanyError::Store(format!("restricting {}: {e}", path.display())))?;
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}

/// The hardening flags every invocation carries, in front of the subcommand.
///
/// `-c` rather than repository config: these must hold for a repository this
/// host did not author, whose `.git/config` it has not read.
fn hardening_flags() -> [&'static str; 6] {
    [
        // A repository's hooks are programs. Not on a host-side invocation.
        "-c",
        "core.hooksPath=/dev/null",
        // `core.fsmonitor` is likewise a command git will run.
        "-c",
        "core.fsmonitor=",
        // Drop any inherited credential helper, including the OS keychain ones.
        // This surface answers for its own credential and nothing else.
        "-c",
        "credential.helper=",
    ]
}

/// Exactly what a `git` child is given: its arguments and its whole
/// environment.
///
/// Built as data rather than by poking a `Command`, because "the credential is
/// in neither of these" is the central claim this module makes, and a claim
/// that cannot be inspected is a claim that cannot be tested. A test asserts it
/// over the real plan for a real token; a `Command` would only have let one
/// assert it indirectly, over whatever the child happened to print.
#[derive(Debug)]
struct SpawnPlan {
    /// The full argument vector, hardening flags first.
    args: Vec<String>,
    /// The complete environment. Not a delta — the child is spawned with
    /// `env_clear` and exactly these.
    env: Vec<(String, String)>,
}

impl SpawnPlan {
    fn build(args: &[&str], askpass: Option<&AskpassDir>) -> Self {
        let mut argv: Vec<String> = hardening_flags().iter().map(|f| f.to_string()).collect();
        argv.extend(args.iter().map(|a| a.to_string()));

        let mut env: Vec<(String, String)> = Vec::new();
        for key in [
            "PATH",
            // TLS trust, for the platforms that locate it by variable.
            "SSL_CERT_FILE",
            "SSL_CERT_DIR",
            "GIT_SSL_CAINFO",
            // git resolves the current user through this on some minimal images.
            "TZ",
        ] {
            if let Ok(value) = std::env::var(key) {
                env.push((key.to_string(), value));
            }
        }
        // A prompt git cannot answer must fail, not wait on a tty nobody is at.
        env.push(("GIT_TERMINAL_PROMPT".into(), "0".into()));
        env.push(("GIT_CONFIG_NOSYSTEM".into(), "1".into()));
        env.push(("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()));
        // Large binaries are not what a code-reading tier is for, and an LFS
        // smudge would fetch them through a filter this host has not vetted.
        env.push(("GIT_LFS_SKIP_SMUDGE".into(), "1".into()));
        if let Some(dir) = askpass {
            // `$HOME` points at the scratch directory so nothing resolves an
            // operator's `~/.gitconfig` even if `GIT_CONFIG_GLOBAL` were
            // ignored.
            env.push(("HOME".into(), dir.path.to_string_lossy().into_owned()));
            env.push((
                "GIT_ASKPASS".into(),
                dir.script().to_string_lossy().into_owned(),
            ));
        }
        Self { args: argv, env }
    }
}

/// Runs `git` in `cwd` with the hardening flags and a minimal environment.
///
/// `token` is `Some` only for the invocations that talk to the network. When it
/// is `None` no askpass is installed at all, and `GIT_TERMINAL_PROMPT=0` turns
/// what would have been an interactive password prompt into a clean failure
/// instead of a hung fetch.
pub(crate) async fn run(
    cwd: &Path,
    args: &[&str],
    token: Option<&str>,
    askpass: Option<&AskpassDir>,
) -> Result<GitOutput> {
    run_bounded(cwd, args, token, askpass, GIT_TIMEOUT).await
}

/// [`run`], with the deadline supplied. Separate only so the timeout path is
/// reachable from a test without one taking [`GIT_TIMEOUT`] to run.
async fn run_bounded(
    cwd: &Path,
    args: &[&str],
    token: Option<&str>,
    askpass: Option<&AskpassDir>,
    limit: std::time::Duration,
) -> Result<GitOutput> {
    let plan = SpawnPlan::build(args, askpass);
    let mut cmd = tokio::process::Command::new("git");
    cmd.current_dir(cwd);
    cmd.args(&plan.args);
    // Start from nothing, then add back only what git needs. Building the
    // environment up rather than filtering it down is what makes "the token is
    // not in the environment" a property of the code instead of a claim about
    // it — and it also keeps an operator's `GIT_*` variables from steering a
    // host-side fetch.
    cmd.env_clear();
    cmd.envs(plan.env.iter().map(|(k, v)| (k, v)));

    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    // What makes the timeout below actually stop anything: dropping the child
    // on the way out of a cancelled future reaps the process instead of
    // orphaning it. Without this, a timed-out fetch leaves `git` and its
    // `git-remote-https` helper running, still holding the credential.
    cmd.kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| {
        OpenCompanyError::Store(format!(
            "could not run git (is it installed on this host?): {e}"
        ))
    })?;

    // The token goes down the pipe before anything is awaited. It is a few
    // dozen bytes against a pipe buffer measured in kilobytes, so this cannot
    // block, and closing the pipe afterwards is what lets the helper's `read`
    // terminate if git never asks.
    if let Some(mut stdin) = child.stdin.take() {
        if let Some(token) = token {
            let mut line = String::with_capacity(token.len() + 1);
            line.push_str(token);
            line.push('\n');
            let write = stdin.write_all(line.as_bytes()).await;
            // Deliberately not `?`: a git that exited before reading its stdin
            // gives a broken pipe here, and its own exit status is the better
            // error to report.
            if let Err(err) = write {
                tracing::debug!("git stdin closed before the credential was written: {err}");
            }
        }
        drop(stdin);
    }

    let output = match tokio::time::timeout(limit, child.wait_with_output()).await {
        Ok(result) => {
            result.map_err(|e| OpenCompanyError::Store(format!("git did not finish: {e}")))?
        }
        Err(_) => {
            // `child` is dropped here, and `kill_on_drop` reaps it.
            return Err(OpenCompanyError::Store(format!(
                "git {} did not finish within {}s and was stopped",
                args.first().copied().unwrap_or("command"),
                limit.as_secs(),
            )));
        }
    };
    Ok(GitOutput {
        ok: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn the_askpass_script_carries_no_placeholder_for_a_secret() {
        // The script is written verbatim to disk and executed. If a future edit
        // ever interpolates a token into it, this is the line that objects.
        assert!(!ASKPASS_SCRIPT.contains("{}"), "no interpolation");
        assert!(ASKPASS_SCRIPT.contains("read -r secret"));
        assert!(ASKPASS_SCRIPT.contains(TOKEN_USERNAME));
    }

    #[tokio::test]
    async fn the_helper_emits_the_token_once_and_the_username_without_consuming_it() {
        // Drives the helper exactly as git does — two separate invocations
        // sharing one stdin pipe — and proves the username answer does not eat
        // the password line.
        let base = std::env::temp_dir().join(format!("oc-askpass-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let dir = AskpassDir::create(&base).unwrap();
        let script = dir.script();

        let run_helper = |prompt: &str| {
            let script = script.clone();
            let prompt = prompt.to_string();
            async move {
                let mut child = tokio::process::Command::new(&script)
                    .arg(&prompt)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .spawn()
                    .unwrap();
                let mut stdin = child.stdin.take().unwrap();
                // The username invocation answers a fixed literal *without*
                // reading stdin — the very property this test exists to prove —
                // so the helper can exit before this write lands and the pipe
                // is already closed. `run_git` tolerates the same `EPIPE`
                // deliberately for the same reason; a test that unwraps here
                // passes only where the pipe buffer happens to win the race
                // (macOS) and fails where the child does (Linux CI).
                //
                // Still asserted, not swallowed: any error other than a broken
                // pipe is a real failure.
                if let Err(err) = stdin.write_all(b"SENTINEL\n").await {
                    assert_eq!(
                        err.kind(),
                        std::io::ErrorKind::BrokenPipe,
                        "writing the credential failed for a reason other than the helper having already exited: {err}"
                    );
                }
                drop(stdin);
                let out = child.wait_with_output().await.unwrap();
                String::from_utf8_lossy(&out.stdout).trim().to_string()
            }
        };

        assert_eq!(
            run_helper("Username for 'https://github.com': ").await,
            TOKEN_USERNAME,
            "the username answer is a fixed literal, not the token"
        );
        assert_eq!(
            run_helper("Password for 'https://x-access-token@github.com': ").await,
            "SENTINEL",
            "the password answer is the line read off stdin"
        );

        drop(dir);
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn neither_the_argv_nor_the_environment_can_carry_the_token() {
        // The central claim of this module, asserted over the exact data the
        // child is spawned with rather than over something it printed.
        let base = std::env::temp_dir().join(format!("oc-gitenv-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let dir = AskpassDir::create(&base).unwrap();

        let plan = SpawnPlan::build(
            &[
                "fetch",
                "--quiet",
                "origin",
                "+refs/heads/main:refs/heads/main",
            ],
            Some(&dir),
        );

        // There is no parameter here that *could* carry it — `SpawnPlan::build`
        // is not given the token at all, which is the structural half of the
        // guarantee. These assertions are the regression guard for an edit that
        // changes that.
        for arg in &plan.args {
            assert!(!arg.contains("SENTINEL"), "argv leaked the token: {arg}");
        }
        for (key, value) in &plan.env {
            assert!(
                !key.contains("SENTINEL") && !value.contains("SENTINEL"),
                "env leaked the token: {key}={value}"
            );
        }

        // And the environment really is the whole environment, not a delta on
        // the parent's — the child is spawned with `env_clear`. A variable the
        // operator exported cannot steer a host-side fetch.
        let names: Vec<&str> = plan.env.iter().map(|(k, _)| k.as_str()).collect();
        assert!(names.contains(&"GIT_TERMINAL_PROMPT"), "{names:?}");
        assert!(names.contains(&"GIT_CONFIG_NOSYSTEM"), "{names:?}");
        assert!(names.contains(&"GIT_ASKPASS"), "{names:?}");
        assert!(names.contains(&"HOME"), "{names:?}");
        assert!(
            names.iter().all(|n| n.starts_with("GIT_")
                || matches!(
                    *n,
                    "PATH" | "HOME" | "TZ" | "SSL_CERT_FILE" | "SSL_CERT_DIR"
                )),
            "an unexpected variable reached git: {names:?}"
        );

        // The hardening flags come first, so a subcommand cannot displace them.
        assert_eq!(plan.args[0], "-c");
        assert_eq!(plan.args[1], "core.hooksPath=/dev/null");
        assert_eq!(plan.args[6], "fetch");

        drop(dir);
        std::fs::remove_dir_all(&base).ok();
    }

    #[tokio::test]
    async fn a_credentialed_invocation_prints_nothing_of_the_token() {
        // The end-to-end companion to the plan assertion above: a real spawn,
        // a real token on the pipe, and neither stream carrying it back.
        let base = std::env::temp_dir().join(format!("oc-gitrun-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let dir = AskpassDir::create(&base).unwrap();

        let out = run(&base, &["var", "GIT_EDITOR"], Some("SENTINEL"), Some(&dir))
            .await
            .unwrap();
        assert!(!out.stdout.contains("SENTINEL"), "{}", out.stdout);
        assert!(!out.stderr.contains("SENTINEL"), "{}", out.stderr);

        drop(dir);
        std::fs::remove_dir_all(&base).ok();
    }

    #[tokio::test]
    async fn a_git_that_overruns_its_deadline_is_stopped_and_reported() {
        // Drive the deadline against a git that deterministically blocks, not a
        // real `git version` under virtual time. `tokio::time::timeout` polls
        // its inner future first and only consults the deadline once that future
        // returns `Pending`; a fast `git version` finished and had its output
        // buffered before the first `Pending`, so the old `start_paused` version
        // returned `Ok` — exactly what failed on Linux CI. Auto-advancing the
        // clock does not help while the runtime is still waiting on a real
        // child's I/O, so the deadline never reached the timeout arm reliably.
        //
        // So this test puts a fake `git` on `PATH` whose only special case is to
        // block, checks that the deadline arm fires, and relies on `kill_on_drop`
        // to reap the child instead of leaking it. The fake delegates everything
        // else to the real git, so its presence on `PATH` cannot perturb sibling
        // tests running concurrently in the same binary.
        let base = std::env::temp_dir().join(format!("oc-gittimeout-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();

        let real_git = std::process::Command::new("sh")
            .arg("-c")
            .arg("command -v git")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .expect("real git must be on PATH");
        let bin = base.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let fake_git = bin.join("git");
        std::fs::write(
            &fake_git,
            format!(
                // `exec sleep`, not `sleep`: `kill_on_drop` stops the direct
                // child and nothing below it, so a plain `sleep` outlives the
                // shell that spawned it and keeps running for its full minute
                // after the deadline arm has fired. Replacing the shell makes
                // the blocking process the one Tokio holds.
                "#!/bin/sh\ncase \" $* \" in\n  *\" __deadline__ \"*) exec sleep 60;;\nesac\nexec {real_git} \"$@\"\n"
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake_git, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        // `run_bounded` rebuilds the child environment from this process's, so a
        // test-local `PATH` prepend is enough to route `git` to the fake for the
        // duration of the run. The marker still reaches the fake's `$*` scan
        // because `hardening_flags()` prepends its own `-c` arguments ahead of
        // it in the argv.
        let original_path = std::env::var("PATH").unwrap_or_default();
        // SAFETY: a single-threaded test lifetime; we restore the variable on
        // every exit path below. Edition-2024 marks this unsafe.
        unsafe { std::env::set_var("PATH", format!("{}:{original_path}", bin.display())) };

        // A 500ms deadline against a 60s sleep is a ~120x margin, not a race.
        let err = run_bounded(
            &base,
            &["__deadline__"],
            None,
            None,
            std::time::Duration::from_millis(500),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("did not finish"), "{err}");
        assert!(err.contains("was stopped"), "{err}");

        unsafe { std::env::set_var("PATH", original_path) };
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn hardening_flags_pin_hooks_and_clear_inherited_helpers() {
        let flags = hardening_flags().join(" ");
        assert!(flags.contains("core.hooksPath=/dev/null"), "{flags}");
        assert!(flags.contains("core.fsmonitor="), "{flags}");
        assert!(flags.contains("credential.helper="), "{flags}");
    }

    #[test]
    fn failure_reports_git_stderr_without_the_whole_transcript() {
        let out = GitOutput {
            ok: false,
            stdout: String::new(),
            stderr: "fatal: repository not found\nhint: check the URL\n\n\nline3\nline4\nline5"
                .into(),
        };
        let err = out.require("fetch").unwrap_err().to_string();
        assert!(err.contains("repository not found"), "{err}");
        assert!(!err.contains("line5"), "{err}");
    }
}
