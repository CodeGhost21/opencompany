//! `mcp.json` — the company's declared MCP servers as one editable document.
//!
//! The console's MCP page has two ways to say the same thing: a row per
//! connection with its buttons, and this file. Both write the *same* store the
//! per-server routes in [`super::mcp`] write — the runtime index under
//! [`RUNTIME_INDEX_KEY`](crate::company::mcp::RUNTIME_INDEX_KEY) — so an
//! operator who prefers a text editor is not editing a second, shadow copy of
//! the configuration that can disagree with the rows.
//!
//! The document is Claude-shaped, so a server pasted out of a `claude_desktop_
//! config.json`/`.mcp.json` fits without translation:
//!
//! ```json
//! {
//!   "mcpServers": {
//!     "notion": {
//!       "type": "http",
//!       "url": "https://mcp.notion.com/mcp",
//!       "headers": { "Authorization": "Bearer secret" },
//!       "enabled": true,
//!       "allowedTools": [],
//!       "disallowedTools": [],
//!       "readOnlyTools": [],
//!       "timeoutSecs": 30
//!     }
//!   }
//! }
//! ```
//!
//! Two rules the shape cannot express, both inherited rather than invented here:
//!
//! * **Credentials are write-only.** `headers` is accepted on `PUT` and stored
//!   in the secret store under the server's own key; `GET` never echoes it back,
//!   reporting only `authConfigured`. A round-trip therefore does not erase a
//!   stored credential — an entry that arrives without `headers` leaves the
//!   credential alone, exactly as an omitted `token` does on `PUT …/mcp/servers/
//!   {name}`.
//! * **A manifest or install-default server cannot be deleted by deleting its
//!   entry.** Its declaration lives in `company.toml` or the instance
//!   `config.toml`, so dropping the runtime row would not remove it — the next
//!   resolution merges it straight back. Removing one is refused by name with
//!   the same sentence `DELETE …/mcp/servers/{name}` uses; disable it instead
//!   (`"enabled": false`), which *does* persist as an override.
//!
//! Directory (registry) installs are **not** in the document. They live in
//! OpenHuman's own store keyed by a stable `serverId` rather than in this
//! company's index, and a name in this file addresses no such install — so
//! rendering them here would invite an edit that silently does nothing. They
//! stay on the rows, where their own routes manage them.

use axum::Json;
use axum::Router;
use axum::routing::get;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::AppState;
use crate::company::McpServer;
use crate::company::mcp::{
    AuthMaterial, clear_auth, clear_health, effective_mcp_servers, load_runtime_index,
    save_runtime_index, store_auth,
};
use crate::company::runtime::CompanyRuntime;
use crate::error::OpenCompanyError;
use crate::server::error::ApiError;
use crate::server::ops::mcp::{
    McpServerDto, NEXT_TURN_NOTE, manifest_servers, merged_rows, reject_invalid,
};
use crate::server::ops::{AdminScopedCompany, ScopedCompany, scoped};

/// Builds the `mcp.json` document route fragment.
pub fn router() -> Router<AppState> {
    scoped("/mcp/config", get(read_config).put(write_config))
}

/// One server as the document renders it. `headers` is deliberately absent:
/// the credential is write-only and `auth_configured` is all a reader gets.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConfigEntry {
    /// Always `http` — the only transport hosted v1 dials (a `command` entry is
    /// a validation error, not a silent no-op).
    r#type: &'static str,
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    enabled: bool,
    allowed_tools: Vec<String>,
    disallowed_tools: Vec<String>,
    read_only_tools: Vec<String>,
    timeout_secs: u64,
    /// Where the declaration came from — `manifest`, `default` or `runtime`.
    /// Echoed for the editor's benefit and **ignored on write**: provenance is
    /// a property of which layer holds the declaration, not something a
    /// document can assert.
    source: String,
    /// Whether an outbound credential is stored. Never the credential.
    /// Ignored on write.
    auth_configured: bool,
}

/// The document body: `{ "mcpServers": { … } }`.
#[derive(Debug, Serialize)]
struct ConfigDoc {
    #[serde(rename = "mcpServers")]
    mcp_servers: Map<String, Value>,
}

/// One server as the document accepts it. Every field but the URL is optional,
/// so the smallest useful entry is `{"url": "https://…"}`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfigEntryIn {
    /// `http`/`https`/`sse` are all HTTP transports here; anything else (notably
    /// `stdio`) is refused by name rather than ignored.
    #[serde(default)]
    r#type: Option<String>,
    #[serde(default)]
    url: Option<String>,
    /// Accepted as a synonym so a body copied off `GET …/mcp/servers` fits.
    #[serde(default)]
    endpoint: Option<String>,
    /// A stdio command. Present only so the refusal can name the real problem.
    #[serde(default)]
    command: Option<Value>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
    /// Claude's spelling of a disabled entry; `enabled` wins when both appear.
    #[serde(default)]
    disabled: Option<bool>,
    #[serde(default)]
    allowed_tools: Option<Vec<String>>,
    #[serde(default)]
    disallowed_tools: Option<Vec<String>>,
    #[serde(default)]
    read_only_tools: Option<Vec<String>>,
    #[serde(default)]
    timeout_secs: Option<u64>,
    /// The outbound credential, write-only. Exactly one header is supported —
    /// `Authorization: Bearer …` is stored as a bearer token, anything else as
    /// that named header. Omit to leave the stored credential unchanged.
    #[serde(default)]
    headers: Option<Map<String, Value>>,
}

/// The write body.
#[derive(Debug, Deserialize)]
struct ConfigDocIn {
    #[serde(rename = "mcpServers", default)]
    mcp_servers: Map<String, Value>,
}

/// A write's answer: the resulting rows (the same shape `GET …/mcp/servers`
/// serves, so the console re-renders from one reading) and the rebuild note.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WriteResponse {
    servers: Vec<McpServerDto>,
    note: String,
}

/// The company's declared MCP servers (manifest ∪ install defaults ∪ runtime),
/// merged and projected into the three layers' effective bodies.
async fn declared(
    runtime: &CompanyRuntime,
) -> Result<Vec<crate::company::mcp::McpServerDecl>, ApiError> {
    let manifest = manifest_servers(runtime).await?;
    let index = load_runtime_index(runtime.id(), runtime.secrets().as_ref())
        .await
        .map_err(ApiError)?;
    Ok(effective_mcp_servers(
        runtime.default_mcp_servers(),
        &manifest,
        &index,
    ))
}

/// `GET …/mcp/config` — the company's declared MCP servers as `mcp.json`.
async fn read_config(company: ScopedCompany) -> Result<Json<ConfigDoc>, ApiError> {
    let runtime = company.runtime.as_ref();
    let decls = declared(runtime).await?;
    // `auth_configured` is read off the resolved rows rather than the decls,
    // which carry `AuthMaterial::None` until `resolve_effective` fills them.
    let rows = merged_rows(runtime).await?;
    let mut out = Map::new();
    for decl in decls {
        let auth_configured = rows
            .iter()
            .find(|row| row.name == decl.name)
            .is_some_and(|row| row.auth_configured);
        let entry = ConfigEntry {
            r#type: "http",
            url: decl.endpoint.clone(),
            description: decl.description.clone(),
            enabled: decl.enabled,
            allowed_tools: decl.allowed_tools.clone(),
            disallowed_tools: decl.disallowed_tools.clone(),
            read_only_tools: decl.read_only_tools.clone(),
            timeout_secs: decl.timeout_secs,
            source: source_label(decl.source).to_string(),
            auth_configured,
        };
        out.insert(
            decl.name.clone(),
            serde_json::to_value(entry).map_err(|err| {
                ApiError(OpenCompanyError::Config(format!(
                    "serializing mcp.json entry `{}`: {err}",
                    decl.name
                )))
            })?,
        );
    }
    Ok(Json(ConfigDoc { mcp_servers: out }))
}

/// The document's word for a provenance.
fn source_label(source: crate::company::mcp::McpSource) -> &'static str {
    use crate::company::mcp::McpSource;
    match source {
        McpSource::Manifest => "manifest",
        McpSource::Runtime => "runtime",
        McpSource::Default => "default",
        McpSource::Registry => "registry",
    }
}

/// `PUT …/mcp/config` — replace the company's declared MCP servers with the
/// document.
///
/// The document is the whole declared set, so the write is a replace and not a
/// merge: a runtime server absent from it is removed (credential and health
/// wiped with it), exactly as `DELETE …/mcp/servers/{name}` would. A manifest or
/// default server absent from it is a `409` — that layer is not this file's to
/// delete.
///
/// An entry that matches its manifest/default declaration exactly writes **no**
/// override row, so saving an unedited document is a genuine no-op rather than a
/// silent conversion of every declared server into an operator override.
async fn write_config(
    company: AdminScopedCompany,
    Json(body): Json<ConfigDocIn>,
) -> Result<Json<WriteResponse>, ApiError> {
    let runtime = company.runtime.as_ref();
    let manifest = manifest_servers(runtime).await?;
    let defaults = runtime.default_mcp_servers().to_vec();
    let previous = load_runtime_index(runtime.id(), runtime.secrets().as_ref())
        .await
        .map_err(ApiError)?;

    let mut index: Vec<McpServer> = Vec::new();
    let mut credentials: Vec<(String, AuthMaterial)> = Vec::new();
    let mut seen: Vec<String> = Vec::new();

    for (name, value) in &body.mcp_servers {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(ApiError(OpenCompanyError::InvalidRequest(
                "an MCP server needs a name — the empty key has none.".to_string(),
            )));
        }
        if seen.iter().any(|s| s == &name) {
            return Err(ApiError(OpenCompanyError::InvalidRequest(format!(
                "`{name}` is declared twice in mcp.json."
            ))));
        }
        seen.push(name.clone());

        let entry: ConfigEntryIn = serde_json::from_value(value.clone()).map_err(|err| {
            ApiError(OpenCompanyError::InvalidRequest(format!(
                "mcp server `{name}`: {err}."
            )))
        })?;
        let lower = manifest
            .iter()
            .find(|m| m.name.trim() == name)
            .or_else(|| defaults.iter().find(|d| d.name.trim() == name));
        let server = server_from(&name, &entry, lower)?;
        reject_invalid(&format!("mcp server `{name}`"), &server)?;

        if let Some(headers) = &entry.headers {
            credentials.push((name.clone(), auth_from_headers(&name, headers)?));
        }

        // Only a body that differs from the lower layer earns an override row.
        match lower {
            Some(own) if !differs(&server, own) => {}
            _ => index.push(server),
        }
    }

    // A lower-layer declaration cannot be deleted by omission.
    let mut undeletable: Vec<String> = Vec::new();
    for declared in manifest.iter().chain(defaults.iter()) {
        let name = declared.name.trim();
        if !name.is_empty() && !seen.iter().any(|s| s == name) {
            undeletable.push(name.to_string());
        }
    }
    if !undeletable.is_empty() {
        undeletable.dedup();
        return Err(ApiError(OpenCompanyError::Conflict(format!(
            "{} is declared outside mcp.json (company.toml or the install defaults) — set \
             \"enabled\": false instead of removing the entry.",
            undeletable
                .iter()
                .map(|n| format!("`{n}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ))));
    }

    save_runtime_index(runtime.id(), runtime.secrets().as_ref(), &index)
        .await
        .map_err(ApiError)?;

    // A runtime-only server the document dropped loses its credential and badge
    // with it, so a later server of the same name inherits neither.
    for gone in &previous {
        let name = gone.name.trim();
        if name.is_empty() || seen.iter().any(|s| s == name) {
            continue;
        }
        clear_auth(runtime.id(), name, runtime.secrets().as_ref())
            .await
            .map_err(ApiError)?;
        clear_health(runtime.id(), name, runtime.secrets().as_ref())
            .await
            .map_err(ApiError)?;
    }

    for (name, material) in &credentials {
        store_auth(runtime.id(), name, material, runtime.secrets().as_ref())
            .await
            .map_err(ApiError)?;
    }

    Ok(Json(WriteResponse {
        servers: merged_rows(runtime).await?,
        note: NEXT_TURN_NOTE.to_string(),
    }))
}

/// Builds the declaration one document entry describes, filling anything it
/// left out from the lower-layer declaration (when there is one) so an entry
/// that only flips `enabled` keeps the tool lists it never mentioned.
fn server_from(
    name: &str,
    entry: &ConfigEntryIn,
    lower: Option<&McpServer>,
) -> Result<McpServer, ApiError> {
    if entry.command.is_some() {
        return Err(ApiError(OpenCompanyError::InvalidRequest(format!(
            "mcp server `{name}`: this deployment dials MCP servers over HTTP only — a \
             `command` entry would need a local subprocess. Give it a `url`."
        ))));
    }
    if let Some(kind) = entry.r#type.as_deref()
        && !matches!(
            kind.trim().to_ascii_lowercase().as_str(),
            "http" | "https" | "sse" | "http_remote" | ""
        )
    {
        return Err(ApiError(OpenCompanyError::InvalidRequest(format!(
            "mcp server `{name}`: unsupported transport `{kind}` — this deployment dials MCP \
             servers over HTTP only."
        ))));
    }
    let url = entry
        .url
        .as_deref()
        .or(entry.endpoint.as_deref())
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .map(str::to_string)
        .or_else(|| lower.map(|l| l.endpoint.trim().to_string()))
        .ok_or_else(|| {
            ApiError(OpenCompanyError::InvalidRequest(format!(
                "mcp server `{name}`: needs a `url`."
            )))
        })?;
    // `enabled` is authoritative; `disabled` is Claude's spelling of the same
    // switch and applies only when `enabled` is absent.
    let enabled = entry
        .enabled
        .or_else(|| entry.disabled.map(|d| !d))
        .or_else(|| lower.map(|l| l.enabled))
        .unwrap_or(true);
    let pick = |own: Option<&Vec<String>>, fallback: fn(&McpServer) -> &Vec<String>| {
        own.cloned()
            .or_else(|| lower.map(|l| fallback(l).clone()))
            .unwrap_or_default()
    };
    Ok(McpServer {
        name: name.to_string(),
        endpoint: url,
        description: entry
            .description
            .clone()
            .or_else(|| lower.and_then(|l| l.description.clone())),
        command: None,
        allowed_tools: pick(entry.allowed_tools.as_ref(), |l| &l.allowed_tools),
        disallowed_tools: pick(entry.disallowed_tools.as_ref(), |l| &l.disallowed_tools),
        read_only_tools: pick(entry.read_only_tools.as_ref(), |l| &l.read_only_tools),
        timeout_secs: entry
            .timeout_secs
            .or_else(|| lower.map(|l| l.timeout_secs))
            .unwrap_or(30),
        enabled,
        // An override always resolves the canonical per-server credential key,
        // the same way `PUT …/mcp/servers/{name}` writes one.
        auth_secret: None,
    })
}

/// Whether a document entry says anything the lower-layer declaration does not.
/// Compares the fields the runtime layer can actually override — `auth_secret`
/// and `command` are not among them.
fn differs(candidate: &McpServer, lower: &McpServer) -> bool {
    let norm = |v: &Vec<String>| {
        v.iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
    };
    candidate.endpoint.trim() != lower.endpoint.trim()
        || candidate.enabled != lower.enabled
        || candidate.timeout_secs != lower.timeout_secs
        || candidate.description.as_deref().map(str::trim)
            != lower.description.as_deref().map(str::trim)
        || norm(&candidate.allowed_tools) != norm(&lower.allowed_tools)
        || norm(&candidate.disallowed_tools) != norm(&lower.disallowed_tools)
        || norm(&candidate.read_only_tools) != norm(&lower.read_only_tools)
}

/// Reads the one header a document entry carries into stored auth material.
///
/// `Authorization: Bearer …` is stored as a bearer token rather than as a
/// literal header, so a credential pasted in this form rotates through the same
/// slot the console's "Add token" button writes — two spellings of one
/// credential, not two credentials.
fn auth_from_headers(name: &str, headers: &Map<String, Value>) -> Result<AuthMaterial, ApiError> {
    let mut entries = headers.iter().filter_map(|(key, value)| {
        value
            .as_str()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| (key.trim().to_string(), v.to_string()))
    });
    let (header, value) = entries.next().ok_or_else(|| {
        ApiError(OpenCompanyError::InvalidRequest(format!(
            "mcp server `{name}`: `headers` carries no header value. Omit it to leave the \
             stored credential unchanged."
        )))
    })?;
    if entries.next().is_some() {
        return Err(ApiError(OpenCompanyError::InvalidRequest(format!(
            "mcp server `{name}`: one outbound credential header is supported, and this entry \
             has several."
        ))));
    }
    if header.eq_ignore_ascii_case("authorization")
        && let Some(token) = value.strip_prefix("Bearer ").map(str::trim)
        && !token.is_empty()
    {
        return Ok(AuthMaterial::Bearer(token.to_string()));
    }
    Ok(AuthMaterial::Header {
        name: header,
        value,
    })
}

#[cfg(test)]
#[path = "mcp_config/test.rs"]
mod test;

#[cfg(test)]
mod http_tests {
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use crate::company::CompanyManifest;
    use crate::ports::CompanyStore;
    use crate::ports::types::{CompanyId, CompanyRecord};
    use crate::runtime::RuntimeBuilder;
    use crate::server::router;
    use crate::store::FsCompanyStore;
    use crate::{AppConfig, AppState};

    const MANIFEST: &str = "[company]\nname = \"Acme\"\n\
         [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
         [[mcp_server]]\nname = \"notion\"\n\
         endpoint = \"https://mcp.notion.com/mcp\"\n\
         description = \"Notion workspace\"\n\
         allowed_tools = [\"search\"]\n\
         timeout_secs = 45\n\
         enabled = true\n";

    async fn state(home: &std::path::Path) -> AppState {
        let manifest: CompanyManifest = toml::from_str(MANIFEST).unwrap();
        let store = FsCompanyStore::new(home.to_path_buf());
        let id = CompanyId::new("acme");
        store
            .save(&CompanyRecord {
                overlay_desk_hive: Vec::new(),
                overlay_retired_agents: Vec::new(),
                overlay_agent_edits: Vec::new(),
                id: id.clone(),
                manifest: manifest.clone(),
                ledger: Vec::new(),
                lifecycle: "running".to_string(),
                overlay_agents: Vec::new(),
                overlay_desk_members: Vec::new(),
                overlay_desk_order: Vec::new(),
                overlay_desks: Vec::new(),
                overlay_workflows: Vec::new(),
                overlay_budgets: Vec::new(),
                overlay_policy: None,
                overlay_tool_grants: None,
                overlay_desk_tools: Default::default(),
                disabled_workflows: Vec::new(),
                template_provenance: None,
                setup: None,
                name_confirmed: false,
                activation_completed_at: None,
                created_at_millis: None,
            })
            .await
            .unwrap();
        let runtime = RuntimeBuilder::new(home.to_path_buf(), manifest)
            .with_id(id.clone())
            .build()
            .await
            .unwrap();
        let state = AppState::new(AppConfig::default());
        state.registry().insert(id, std::sync::Arc::new(runtime));
        crate::server::test_support::seed_fixed_admin(&state, "acme").await;
        state
    }

    /// `GET …/mcp/config` over the real router: the manifest's `[[mcp_server]]`
    /// entry must come back as a `mcpServers` document entry, not merely as
    /// something [`effective_mcp_servers`] agrees with in isolation.
    ///
    /// Every helper `declared`/`read_config` calls (`manifest_servers`,
    /// `load_runtime_index`, `effective_mcp_servers`) has its own unit
    /// coverage; this is the one test that proves the handler actually wires
    /// them together behind the HTTP route the console calls, with the
    /// auth-matrix's admitted request reaching a real manifest-backed runtime
    /// rather than stopping at the authorization verdict.
    #[tokio::test]
    async fn read_config_reports_a_manifest_declared_server() {
        let home = tempfile::tempdir().unwrap();
        let state = state(home.path()).await;

        let request = Request::builder()
            .method("GET")
            .uri("/api/v1/company/mcp/config")
            .header("cookie", crate::server::test_support::fixed_cookie("acme"))
            .body(Body::empty())
            .unwrap();
        let response = router(state).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let notion = &body["mcpServers"]["notion"];

        assert_eq!(notion["url"], "https://mcp.notion.com/mcp");
        assert_eq!(notion["description"], "Notion workspace");
        assert_eq!(notion["enabled"], true);
        assert_eq!(notion["allowedTools"], serde_json::json!(["search"]));
        assert_eq!(notion["timeoutSecs"], 45);
        assert_eq!(
            notion["source"], "manifest",
            "a company.toml-declared server reports its real provenance"
        );
        assert_eq!(
            notion["authConfigured"], false,
            "no auth_secret was declared and none was stored"
        );
    }
}
