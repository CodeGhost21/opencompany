//! The OpenCompany desktop shell.
//!
//! Three responsibilities, and nothing else:
//!
//! - **[`proxy`]** — every host's HTTP and event traffic, in Rust so that CORS
//!   does not apply and the credential never enters the webview.
//! - **[`embedded`]** — the host running in this process, for someone with no
//!   server to point at.
//! - **[`keychain`]** — where a paired device's token lives, so the webview
//!   holds a handle and never the secret.
//! - **[`commands`]** — the thin Tauri surface over all three.
//!
//! The console itself is unchanged: it is the same `frontend/` bundle the web
//! deployment serves, and it reaches all of the above through the `Transport`
//! seam it already had.

pub mod acp;
pub mod commands;
pub mod embedded;
pub mod keychain;
pub mod proxy;

use std::path::PathBuf;

use crate::embedded::EmbeddedHost;

/// Process-wide state the commands read.
pub struct AppHandleState {
    pub data_dir: PathBuf,
    /// `None` when the embedded host could not start — most often because
    /// another instance already holds the data root. That is a state the
    /// console renders, not a reason to refuse to launch: the whole point of
    /// the desktop is that it can also talk to *remote* hosts, and a second
    /// window being unable to serve locally must not stop it doing that.
    pub embedded: Option<EmbeddedHost>,
}

/// The platform directory this instance keeps its data in.
///
/// Resolved from the OS rather than from `HOME`. The crate's own fallback ends
/// at a *relative* `.opencompany` when neither `HOME` nor `USERPROFILE` is set,
/// which for a double-clicked application resolves against whatever working
/// directory the launcher gave it — plausibly unwritable, and plausibly
/// different between launches.
pub fn default_data_dir() -> PathBuf {
    // `OPENCOMPANY_DATA_DIR` still wins, so a developer can point one build at
    // a scratch root without touching the installed app's.
    if let Some(explicit) = std::env::var_os("OPENCOMPANY_DATA_DIR")
        && !explicit.is_empty()
    {
        return PathBuf::from(explicit);
    }
    let base = dirs_next_data_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("ai.tinyhumans.opencompany")
}

/// The per-OS application-data directory, without taking a `dirs` dependency
/// for four lines of `std`.
fn dirs_next_data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(PathBuf::from)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "opencompany_desktop_lib=info,opencompany=info".into()),
        )
        .init();

    let data_dir = default_data_dir();

    // The runtime starts here, before the webview, because the embedded host
    // has to be listening before the console asks for its address.
    let runtime = tokio::runtime::Runtime::new().expect("start an async runtime");
    let embedded = runtime.block_on(async {
        match embedded::start(data_dir.clone()).await {
            Ok(host) => Some(host),
            Err(error) => {
                // Not fatal. Most often this is the data root already being held
                // — by another window, or by an `opencompany serve` in a
                // terminal — and the right answer is to launch anyway and let
                // the operator connect to that host instead of this one.
                tracing::warn!(%error, "no embedded host; remote connections still work");
                None
            }
        }
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(proxy::SharedProxy::default())
        .manage(AppHandleState { data_dir, embedded })
        .invoke_handler(tauri::generate_handler![
            commands::oc_connect,
            commands::oc_pair_device,
            commands::oc_forget_device,
            commands::oc_disconnect,
            commands::oc_connections,
            commands::oc_request,
            commands::oc_subscribe,
            commands::oc_embedded,
        ])
        .run(tauri::generate_context!())
        .expect("run the desktop shell");
}
