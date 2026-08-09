use std::{fs, path::PathBuf, process::ExitStatus};

use serde::Serialize;
use tokio::process::Command;

use crate::{OpenCompanyError, Result};

/// OpenHuman target to launch from a sibling checkout.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum LaunchMode {
    /// Launch the core Rust binary (`openhuman-core`).
    Core,
    /// Launch the Tauri desktop host, driving OpenHuman's own `pnpm` scripts.
    Desktop,
}

/// Desktop windowing backend. CEF is OpenHuman's primary surface on macOS;
/// `wry` is the cross-platform fallback that runs on Linux/Windows.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum DesktopBackend {
    /// Chromium Embedded Framework — the `dev:app`/`macos:build:*` scripts.
    /// macOS-only: they assume the Keychain, `APPLE_SIGNING_IDENTITY`, and the
    /// vendored CEF-aware `tauri-cli`.
    Cef,
    /// Native system webview (WebKitGTK on Linux, WebView2 on Windows) — the
    /// `dev:wry`/`tauri:build:ui` scripts, selected on every non-macOS host.
    Wry,
}

impl DesktopBackend {
    /// The backend OpenHuman's own scripts expect on this host: CEF on macOS,
    /// `wry` everywhere else.
    pub fn for_host() -> Self {
        if cfg!(target_os = "macos") {
            Self::Cef
        } else {
            Self::Wry
        }
    }
}

/// Describes an OpenHuman launch request.
#[derive(Clone, Debug)]
pub struct OpenHumanLaunch {
    root: PathBuf,
    mode: LaunchMode,
    release: bool,
    args: Vec<String>,
}

impl OpenHumanLaunch {
    /// Creates a launch request for the OpenHuman core binary.
    pub fn core(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            mode: LaunchMode::Core,
            release: false,
            args: Vec::new(),
        }
    }

    /// Creates a launch request for the OpenHuman desktop host.
    pub fn desktop(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            mode: LaunchMode::Desktop,
            release: false,
            args: Vec::new(),
        }
    }

    /// Switch from a dev launch to a release build (a bundled `.app`/dmg on
    /// macOS, a deb/AppImage elsewhere). For Core this adds `--release` to the
    /// `cargo run`; for Desktop it selects OpenHuman's `*build*` script.
    pub fn release(mut self) -> Self {
        self.release = true;
        self
    }

    /// Adds passthrough arguments forwarded after `--` to the OpenHuman core
    /// binary. Desktop mode ignores these — it drives OpenHuman's own
    /// opinionated pnpm scripts, which do not accept passthrough args.
    pub fn with_args(mut self, args: impl IntoIterator<Item = String>) -> Self {
        self.args = args.into_iter().collect();
        self
    }

    /// The OpenHuman `pnpm` script Desktop mode drives, picked from the host
    /// backend and whether this is a dev or release build.
    fn desktop_script(&self) -> &'static str {
        match (self.release, DesktopBackend::for_host()) {
            (false, DesktopBackend::Cef) => "dev:app",
            (false, DesktopBackend::Wry) => "dev:wry",
            (true, DesktopBackend::Cef) => "macos:build:release",
            (true, DesktopBackend::Wry) => "tauri:build:ui",
        }
    }

    /// Returns the command OpenCompany will spawn, without spawning it.
    ///
    /// Desktop mode drives OpenHuman's own pnpm scripts (`dev:app`/`dev:wry`/
    /// `macos:build:release`/`tauri:build:ui`) rather than `cargo run --bin
    /// OpenHuman` directly, because those scripts are the only thing that wires
    /// up the vendored CEF-aware `tauri-cli`, the CEF runtime, the Vite dev
    /// server, the macOS signing identity, and `.env` — without them the Tauri
    /// window opens blank or panics inside `cef::library_loader`.
    pub fn command_preview(&self) -> Vec<String> {
        match self.mode {
            LaunchMode::Core => {
                let mut command = vec!["cargo".to_string(), "run".to_string()];
                if self.release {
                    command.push("--release".to_string());
                }
                command.extend([
                    "--manifest-path".to_string(),
                    self.root.join("Cargo.toml").display().to_string(),
                    "--bin".to_string(),
                    "openhuman-core".to_string(),
                    "--".to_string(),
                ]);
                command.extend(self.args.clone());
                command
            }
            LaunchMode::Desktop => {
                vec![
                    "pnpm".to_string(),
                    "--filter".to_string(),
                    "openhuman-app".to_string(),
                    "run".to_string(),
                    self.desktop_script().to_string(),
                ]
            }
        }
    }

    /// Starts OpenHuman and waits for it to exit.
    pub async fn run(self) -> Result<ExitStatus> {
        if !self.root.exists() {
            return Err(OpenCompanyError::MissingOpenHumanRoot(self.root));
        }

        if matches!(self.mode, LaunchMode::Desktop) && !self.args.is_empty() {
            return Err(OpenCompanyError::OpenHuman {
                code: 400,
                message: format!(
                    "desktop mode drives OpenHuman's own pnpm scripts ({}) and does \
                     not accept passthrough args; use --mode core for binary args",
                    self.desktop_script()
                ),
            });
        }

        // OpenHuman's `dev:*`/build scripts source `<root>/.env` via
        // `load-dotenv.sh`, which hard-exits when the file is absent. Seed it
        // from the vendored `.env.example` so a fresh checkout launches
        // out-of-the-box without a manual copy step.
        if matches!(self.mode, LaunchMode::Desktop) {
            self.ensure_env()?;
        }

        let preview = self.command_preview();
        let mut command = Command::new(&preview[0]);
        command.args(&preview[1..]);
        // pnpm resolves the workspace relative to its cwd, and the OpenHuman
        // scripts are written to run from the checkout root — anchor there.
        if matches!(self.mode, LaunchMode::Desktop) {
            command.current_dir(&self.root);
        }
        let status = command.status().await?;
        Ok(status)
    }

    /// Copy `<root>/.env.example` to `<root>/.env` when the latter is missing,
    /// so OpenHuman's `load-dotenv.sh` does not abort. No-op once it exists or
    /// when no example is present (OpenHuman's script then surfaces the error).
    fn ensure_env(&self) -> Result<()> {
        let env = self.root.join(".env");
        if env.exists() {
            return Ok(());
        }
        let example = self.root.join(".env.example");
        if !example.exists() {
            return Ok(());
        }
        fs::copy(&example, &env).map_err(OpenCompanyError::from)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_preview_points_to_openhuman_core() {
        let preview = OpenHumanLaunch::core("vendor/openhuman")
            .with_args(["status".to_string()])
            .command_preview();

        assert!(preview.contains(&"openhuman-core".to_string()));
        assert!(preview.contains(&"vendor/openhuman/Cargo.toml".to_string()));
        assert_eq!(preview.last(), Some(&"status".to_string()));
    }

    #[test]
    fn core_release_adds_release_flag_before_manifest() {
        let preview = OpenHumanLaunch::core("vendor/openhuman")
            .release()
            .command_preview();

        let run = preview.iter().position(|p| p == "run").unwrap();
        assert_eq!(preview[run + 1], "--release");
        assert!(preview.contains(&"openhuman-core".to_string()));
    }

    #[test]
    fn desktop_preview_drives_pnpm_script_not_cargo_run() {
        let preview = OpenHumanLaunch::desktop("vendor/openhuman").command_preview();

        // Drives pnpm, not a raw `cargo run --bin OpenHuman` (which opens a
        // blank window because it skips the Vite dev server, CEF runtime, and
        // vendored CEF-aware tauri-cli).
        assert_eq!(preview.first().map(String::as_str), Some("pnpm"));
        assert!(preview.contains(&"--filter".to_string()));
        assert!(preview.contains(&"openhuman-app".to_string()));
        assert!(preview.contains(&"run".to_string()));
        assert!(
            !preview.contains(&"cargo".to_string()),
            "desktop must not shell out to cargo directly: {preview:?}"
        );

        let script = preview.last().unwrap();
        assert!(
            matches!(
                script.as_str(),
                "dev:app" | "dev:wry" | "macos:build:release" | "tauri:build:ui"
            ),
            "unexpected desktop script: {script}"
        );
    }

    #[test]
    fn desktop_release_switches_to_a_build_script() {
        let dev = OpenHumanLaunch::desktop("vendor/openhuman").command_preview();
        let rel = OpenHumanLaunch::desktop("vendor/openhuman")
            .release()
            .command_preview();

        assert_ne!(dev.last(), rel.last());
        let script = rel.last().unwrap();
        assert!(
            script.starts_with("macos:build") || script == "tauri:build:ui",
            "release should select a build script, got {script}"
        );
    }

    #[test]
    fn desktop_backend_matches_host_os() {
        // CEF on macOS, wry everywhere else — the same split OpenHuman's own
        // script set assumes.
        assert_eq!(
            DesktopBackend::for_host(),
            if cfg!(target_os = "macos") {
                DesktopBackend::Cef
            } else {
                DesktopBackend::Wry
            }
        );
    }
}
