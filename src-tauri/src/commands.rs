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
///
/// **Takes no device token.** The console cannot supply one, because it has
/// never seen one: a paired device's session is resolved from the keychain by
/// `connection_id`. That is the difference between "the webview does not
/// normally hold the secret" and "the webview cannot hold the secret", and only
/// the second survives a script injected into rendered agent markdown.
#[tauri::command]
pub async fn oc_connect(
    proxy: State<'_, SharedProxy>,
    connection_id: String,
    base_url: String,
    platform_token: Option<String>,
) -> Result<(), String> {
    // Device first: a paired device is a *person* on this machine, and the
    // journal records their name. A platform bearer is a machine credential
    // that writes anonymously, so preferring it would silently un-attribute
    // every write the desktop makes.
    let credential = match (
        crate::keychain::device_session(&connection_id),
        platform_token,
    ) {
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

/// What the console learns after pairing. Carries no secret.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairedDevice {
    pub company: String,
    pub device_id: String,
    pub expires_at_millis: u64,
}

/// What the host answers a claim with. The token half never leaves this module.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaimedDevice {
    token: String,
    company: String,
    device_id: String,
    expires_at_millis: u64,
}

/// Redeems a pairing code, keeping the session token out of the webview.
///
/// The whole flow lives in Rust for one reason: the token exists for exactly
/// one HTTP response, and the console must not be on the path it takes. So this
/// command performs the claim, writes the result to the keychain, re-registers
/// the connection with the resolved credential, and returns only what a person
/// needs to see — which company, which device, how long it lasts.
///
/// Deliberately does its own request rather than going through
/// `ProxyRegistry::request`: this runs *before* the connection has a credential
/// worth attaching, and routing it through the proxy would mean a code path
/// where the claim response body — the one place a raw token appears — passes
/// through the same machinery that serialises bodies back to the webview.
#[tauri::command]
pub async fn oc_pair_device(
    proxy: State<'_, SharedProxy>,
    connection_id: String,
    base_url: String,
    code: String,
    label: Option<String>,
) -> Result<PairedDevice, String> {
    let base = base_url.trim_end_matches('/');
    let response = reqwest::Client::new()
        .post(format!("{base}/api/v1/devices/claim"))
        .json(&serde_json::json!({ "code": code, "label": label }))
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if !response.status().is_success() {
        // The host's own wording, which is deliberately one indistinguishable
        // message for every way a claim can fail. Passing it through keeps that
        // property instead of inventing a more specific one here.
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let message = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_string))
            .unwrap_or_else(|| format!("pairing failed with {status}"));
        return Err(message);
    }

    let claimed: ClaimedDevice = response.json().await.map_err(|error| error.to_string())?;
    // `<company>.<token>` is the header carrier's form, and the only form
    // anything downstream needs.
    crate::keychain::remember_device(
        &connection_id,
        &format!("{}.{}", claimed.company, claimed.token),
    )
    .map_err(|error| error.to_string())?;

    // Re-register so the credential takes effect without waiting for a reload.
    // The console cannot do this itself — it has nothing to pass.
    if let Ok(existing) = proxy.connection(&connection_id).await {
        proxy
            .upsert(
                connection_id.clone(),
                Connection {
                    base_url: existing.base_url,
                    credential: Credential::Device(
                        crate::keychain::device_session(&connection_id).unwrap_or_default(),
                    ),
                },
            )
            .await;
    }

    Ok(PairedDevice {
        company: claimed.company,
        device_id: claimed.device_id,
        expires_at_millis: claimed.expires_at_millis,
    })
}

/// Forgets this machine's stored session for a connection.
///
/// Local only. The session record on the host outlives it — revoking that is
/// the operator's action from the devices list, and doing both here would mean
/// removing a row from one machine silently cut off another.
#[tauri::command]
pub async fn oc_forget_device(
    proxy: State<'_, SharedProxy>,
    connection_id: String,
) -> Result<(), String> {
    crate::keychain::forget_device(&connection_id).map_err(|error| error.to_string())?;
    if let Ok(existing) = proxy.connection(&connection_id).await {
        proxy
            .upsert(
                connection_id,
                Connection {
                    base_url: existing.base_url,
                    credential: Credential::None,
                },
            )
            .await;
    }
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
