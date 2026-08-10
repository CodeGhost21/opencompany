//! The `#[tauri::command]` surface the console calls.
//!
//! Thin by design: every one of these delegates to [`crate::proxy`] or
//! [`crate::embedded`], which are plain Rust and testable without a webview.
//! Logic that lives in a command is logic that can only be exercised by
//! starting a GUI.
//!
//! **Every command takes an explicit `connection_id`.** None of them reads an
//! "active connection" from application state — that single-valued field is
//! exactly what stops block/buzz from holding more than one workspace at a
//! time, and a command that defaulted it would reintroduce the limit invisibly.

use tauri::State;
use tauri::ipc::Channel;

use crate::proxy::{Connection, Credential, ProxyRequest, ProxyResponse, SharedProxy};

/// What the console needs to construct a connection record.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddedInfo {
    pub base_url: String,
    pub data_dir: String,
}

/// Registers (or re-registers) a host this client talks to.
#[tauri::command]
pub async fn oc_connect(
    proxy: State<'_, SharedProxy>,
    connection_id: String,
    base_url: String,
    device_session: Option<String>,
    platform_token: Option<String>,
) -> Result<(), String> {
    // Device first: a paired device is a *person* on this machine, and the
    // journal records their name. A platform bearer is a machine credential
    // that writes anonymously, so preferring it would silently un-attribute
    // every write the desktop makes.
    let credential = match (device_session, platform_token) {
        (Some(session), _) => Credential::Device(session),
        (None, Some(token)) => Credential::Platform(token),
        (None, None) => Credential::None,
    };
    proxy
        .upsert(
            connection_id,
            Connection {
                base_url,
                credential,
            },
        )
        .await;
    Ok(())
}

#[tauri::command]
pub async fn oc_disconnect(
    proxy: State<'_, SharedProxy>,
    connection_id: String,
) -> Result<(), String> {
    proxy.remove(&connection_id).await;
    Ok(())
}

#[tauri::command]
pub async fn oc_connections(proxy: State<'_, SharedProxy>) -> Result<Vec<String>, String> {
    Ok(proxy.ids().await)
}

/// One HTTP request against a named connection.
#[tauri::command]
pub async fn oc_request(
    proxy: State<'_, SharedProxy>,
    connection_id: String,
    request: ProxyRequest,
) -> Result<ProxyResponse, String> {
    proxy
        .request(&connection_id, request)
        .await
        .map_err(|error| error.to_string())
}

/// Subscribes to a connection's event stream, pushing payloads down `channel`.
///
/// One channel per subscription rather than one shared bus: a chatty company's
/// turn events must not be able to starve another connection's, and dropping
/// the channel is how the console unsubscribes.
#[tauri::command]
pub async fn oc_subscribe(
    proxy: State<'_, SharedProxy>,
    connection_id: String,
    path: String,
    channel: Channel<String>,
) -> Result<(), String> {
    let proxy = proxy.inner().clone();
    tokio::spawn(async move {
        let result = proxy
            .subscribe(&connection_id, &path, |event| {
                // A send failure means the console dropped the channel, i.e.
                // unsubscribed. Not an error worth reporting.
                let _ = channel.send(event);
            })
            .await;
        if let Err(error) = result {
            tracing::debug!(%error, "event stream ended");
        }
    });
    Ok(())
}

/// Where the in-process host is listening, if one is running.
#[tauri::command]
pub fn oc_embedded(state: State<'_, crate::AppHandleState>) -> Option<EmbeddedInfo> {
    state.embedded.as_ref().map(|host| EmbeddedInfo {
        base_url: host.base_url(),
        data_dir: state.data_dir.display().to_string(),
    })
}
