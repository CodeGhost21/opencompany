//! `GET {scope}/harnesses` (issue #1245's harness-picker follow-up): the
//! company's declared `[[harness]]` set, read-only.
//!
//! Before this route existed, nothing outside the process could see what
//! harnesses a company had declared — `AgentDetailDto`/`EditAgentInput` could
//! carry a `harness` binding (a manifest agent already could; an overlay
//! teammate gained the same field alongside this route), but the console had
//! no way to know *what it could bind to*, or which one is the default a
//! blank binding falls back to. Settings' Harnesses page and the per-agent
//! Harness picker both read this one list, so the two cannot disagree about
//! what the company has declared.
//!
//! Read-only, and stays that way: a harness is declared in the
//! version-controlled `company.toml`, the same blueprint
//! [`team_agent`](super::team_agent)'s own module docs describe the console as
//! never rewriting. Defining a *new* harness from the console is a
//! meaningfully bigger, separate piece of work — it would need an overlay
//! storage mechanism harnesses do not have today, the way
//! [`OverlayAgent`](crate::ports::types::OverlayAgent) gives a teammate one.

use axum::Json;
use axum::Router;
use axum::routing::get;
use serde::Serialize;

use crate::AppState;
use crate::company::{ACP_AGENTS, Harness};
use crate::error::OpenCompanyError;
use crate::server::error::ApiError;
use crate::server::ops::{ScopedCompany, scoped};

/// Builds the harnesses read route fragment.
pub fn router() -> Router<AppState> {
    scoped("/harnesses", get(list_harnesses))
}

/// One declared `[[harness]]`, as the console renders it in a picker.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct HarnessDto {
    id: String,
    /// One of `HARNESS_KINDS` — `built_in` (managed) or `acp` (external).
    kind: String,
    /// Whether an agent naming no harness runs here. Exactly one entry in the
    /// list sets this.
    default: bool,
    /// `acp` harnesses only: which CLI (`claude`/`codex`), when
    /// `transport = "local"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    agent: Option<String>,
    /// `acp` harnesses only: `local` (spawned on this machine) or `runner` (a
    /// registered remote).
    #[serde(skip_serializing_if = "Option::is_none")]
    transport: Option<String>,
    /// Whether this entry is **declared** in `company.toml` or merely
    /// **detected** — a coding CLI this build can drive that is bindable
    /// without any `[[harness]]` naming it
    /// ([`Harness::implicit_local`](crate::company::Harness::implicit_local)).
    ///
    /// The distinction is the whole contract with the console. A declared
    /// harness is a property of the *company* and is the same wherever the
    /// manifest is opened; a detected one is a property of the *machine*, and
    /// whether it can actually run is answered by that machine's own
    /// `acp::discovery` survey — which this host cannot see and deliberately
    /// does not guess at. The console joins the two on `id`.
    detected: bool,
}

/// `GET {scope}/harnesses` — every harness an agent here can be bound to.
///
/// Declared `[[harness]]` entries first, then every coding CLI this build
/// knows how to drive that the manifest does not already declare. A declared
/// entry always wins on id collision, so a company that pinned a model on its
/// own `claude` harness is never shadowed by the bare detected one.
///
/// Carries **no readiness**: whether a CLI is installed and signed in is a
/// fact about the operator's machine, not about this company, and this route
/// answers for any client on any machine. See [`HarnessDto::detected`].
async fn list_harnesses(company: ScopedCompany) -> Result<Json<Vec<HarnessDto>>, ApiError> {
    let record = company
        .runtime
        .store()
        .load(company.id())
        .await?
        .ok_or_else(|| OpenCompanyError::CompanyNotFound(company.id().to_string()))?;

    let declared = record.manifest.effective_harnesses();
    let detected = ACP_AGENTS
        .iter()
        .filter(|id| !declared.iter().any(|h| h.id == **id))
        .map(|id| Harness::implicit_local(id));

    Ok(Json(
        declared
            .iter()
            .cloned()
            .map(|harness| dto(harness, false))
            .chain(detected.map(|harness| dto(harness, true)))
            .collect(),
    ))
}

fn dto(harness: Harness, detected: bool) -> HarnessDto {
    HarnessDto {
        id: harness.id,
        kind: harness.kind,
        default: harness.default,
        agent: harness.acp.as_ref().and_then(|acp| acp.agent.clone()),
        transport: harness.acp.map(|acp| acp.transport),
        detected,
    }
}

#[cfg(test)]
mod test {
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use serde_json::Value;
    use tower::ServiceExt;

    use crate::company::CompanyManifest;
    use crate::ports::CompanyStore;
    use crate::ports::types::{CompanyId, CompanyRecord};
    use crate::runtime::RuntimeBuilder;
    use crate::server::router;
    use crate::store::FsCompanyStore;
    use crate::{AppConfig, AppState};

    fn home() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("oc-harnesses-")
            .tempdir()
            .expect("tempdir")
    }

    async fn state_with_manifest(home: &std::path::Path, manifest_toml: &str) -> AppState {
        let manifest: CompanyManifest = toml::from_str(manifest_toml).unwrap();
        let store = FsCompanyStore::new(home.to_path_buf());
        let id = CompanyId::new("acme");
        store
            .save(&CompanyRecord {
                id: id.clone(),
                manifest: manifest.clone(),
                ledger: Vec::new(),
                lifecycle: "running".to_string(),
                overlay_agents: Vec::new(),
                // Added on `main` by #1530's override layer while this test was
                // on `feat/external-acp`; empty is the right seed for a fixture
                // that declares its roster in the manifest.
                overlay_agent_edits: Vec::new(),
                overlay_retired_agents: Vec::new(),
                overlay_desk_members: Vec::new(),
                overlay_desk_order: Vec::new(),
                overlay_desks: Vec::new(),
                overlay_workflows: Vec::new(),
                overlay_budgets: Vec::new(),
                overlay_policy: None,
                overlay_desk_tools: Default::default(),
                disabled_workflows: Vec::new(),
                template_provenance: None,
                setup: None,
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

    async fn get(state: &AppState, path: &str) -> (StatusCode, Value) {
        let request = Request::builder()
            .method("GET")
            .uri(path)
            .header("cookie", crate::server::test_support::fixed_cookie("acme"))
            .body(Body::empty())
            .unwrap();
        let response = router(state.clone()).oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, body)
    }

    const BASE: &str = "[company]\nname = \"Acme\"\n\n[[agent]]\nid = \"ceo\"\nrole = \"CEO\"\n";

    #[tokio::test]
    async fn a_company_with_no_harness_block_lists_the_implicit_default() {
        let home = home();
        let state = state_with_manifest(home.path(), BASE).await;

        let (status, body) = get(&state, "/api/v1/company/harnesses").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let list = body.as_array().unwrap();
        assert_eq!(list[0]["kind"], "built_in");
        assert_eq!(list[0]["default"], true);
        assert_eq!(list[0]["detected"], false, "declared, not detected");
    }

    /// Issue #1245's detected-harness follow-up: every coding CLI this build
    /// drives is bindable without a `[[harness]]`, so the picker can offer it
    /// — marked `detected` so the console knows its availability is a fact
    /// about the operator's machine, which this route cannot answer.
    #[tokio::test]
    async fn every_coding_cli_is_listed_as_detected_and_never_default() {
        let home = home();
        let state = state_with_manifest(home.path(), BASE).await;

        let (status, body) = get(&state, "/api/v1/company/harnesses").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let list = body.as_array().unwrap();

        for id in ["claude", "codex"] {
            let found = list.iter().find(|h| h["id"] == id).expect("cli listed");
            assert_eq!(found["kind"], "acp", "{id}");
            assert_eq!(found["transport"], "local", "{id}");
            assert_eq!(found["agent"], id);
            assert_eq!(found["detected"], true, "{id}");
            assert_eq!(
                found["default"], false,
                "a detected CLI must never become the company default"
            );
        }
    }

    #[tokio::test]
    async fn a_declared_acp_harness_carries_its_agent_and_transport() {
        let home = home();
        let toml = format!(
            "{BASE}\n[[harness]]\nid = \"laptop\"\nkind = \"acp\"\ndefault = true\n\n\
             [harness.acp]\ntransport = \"local\"\nagent = \"claude\"\n"
        );
        let state = state_with_manifest(home.path(), &toml).await;

        let (status, body) = get(&state, "/api/v1/company/harnesses").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let laptop = body
            .as_array()
            .unwrap()
            .iter()
            .find(|h| h["id"] == "laptop")
            .expect("laptop harness listed");
        assert_eq!(laptop["kind"], "acp");
        assert_eq!(laptop["agent"], "claude");
        assert_eq!(laptop["transport"], "local");
        assert_eq!(laptop["default"], true);
    }

    #[tokio::test]
    async fn several_harnesses_are_all_listed() {
        let home = home();
        let toml = format!(
            "{BASE}\n[[harness]]\nid = \"main\"\nkind = \"built_in\"\ndefault = true\n\n\
             [[harness]]\nid = \"laptop\"\nkind = \"acp\"\n\n\
             [harness.acp]\ntransport = \"local\"\nagent = \"codex\"\n"
        );
        let state = state_with_manifest(home.path(), &toml).await;

        let (status, body) = get(&state, "/api/v1/company/harnesses").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let ids: Vec<&str> = body
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            &ids[..2],
            &["main", "laptop"],
            "declared entries come first, in manifest order: {ids:?}"
        );
    }

    /// A declared harness is never shadowed by the detected entry of the same
    /// id — otherwise a company that pinned a model on its own `claude`
    /// harness would read back the bare synthesized one instead.
    #[tokio::test]
    async fn a_declared_harness_wins_over_the_detected_entry_of_the_same_id() {
        let home = home();
        let toml = format!(
            "{BASE}\n[[harness]]\nid = \"main\"\nkind = \"built_in\"\ndefault = true\n\n\
             [[harness]]\nid = \"claude\"\nkind = \"acp\"\n\n\
             [harness.acp]\ntransport = \"local\"\nagent = \"claude\"\nmodel = \"opus-4-5\"\n"
        );
        let state = state_with_manifest(home.path(), &toml).await;

        let (status, body) = get(&state, "/api/v1/company/harnesses").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let claude: Vec<_> = body
            .as_array()
            .unwrap()
            .iter()
            .filter(|h| h["id"] == "claude")
            .collect();
        assert_eq!(claude.len(), 1, "listed once, not twice: {body}");
        assert_eq!(claude[0]["detected"], false, "the declared one");
    }
}
