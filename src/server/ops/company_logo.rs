//! Company-logo settings, persisted in the company manifest.

use axum::routing::put;
use axum::{Json, Router};
use serde::Deserialize;

use crate::AppState;
use crate::error::OpenCompanyError;
use crate::runtime::CompanyStatus;
use crate::server::error::ApiError;
use crate::server::ops::{AdminScopedCompany, scoped};

const COMPANY_LOGO_MAX_CHARS: usize = 1_000_000;
const ALLOWED_IMAGE_MIMES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompanyLogoBody {
    #[serde(default)]
    logo_url: Option<String>,
}

/// Builds both `PUT /api/v1/company/logo` addressing variants.
pub fn router() -> Router<AppState> {
    scoped("/logo", put(put_logo))
}

fn invalid_logo(message: impl Into<String>) -> ApiError {
    ApiError(OpenCompanyError::InvalidRequest(message.into()))
}

/// Accepts only bounded, self-contained image data URLs. `None` clears the logo.
fn company_logo_value(value: Option<String>) -> Result<Option<String>, ApiError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.len() > COMPANY_LOGO_MAX_CHARS {
        return Err(invalid_logo(format!(
            "company logo exceeds the {COMPANY_LOGO_MAX_CHARS}-character limit"
        )));
    }

    let (header, payload) = value
        .split_once(',')
        .ok_or_else(|| invalid_logo("company logo must be a base64 image data URL"))?;
    let mime = header
        .strip_prefix("data:")
        .and_then(|header| header.strip_suffix(";base64"))
        .filter(|mime| ALLOWED_IMAGE_MIMES.contains(mime))
        .ok_or_else(|| {
            invalid_logo("company logo must be a base64 PNG, JPEG, GIF, or WebP data URL")
        })?;
    let padding = payload.len() - payload.trim_end_matches('=').len();
    if payload.is_empty()
        || payload.len() % 4 != 0
        || padding > 2
        || !payload[..payload.len() - padding]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/'))
    {
        return Err(invalid_logo(format!(
            "company logo contains invalid base64 data for {mime}"
        )));
    }

    Ok(Some(value))
}

/// `PUT …/logo` — replace or clear the company logo and return fresh status.
async fn put_logo(
    company: AdminScopedCompany,
    Json(body): Json<CompanyLogoBody>,
) -> Result<Json<CompanyStatus>, ApiError> {
    let logo_url = company_logo_value(body.logo_url)?;
    let mut record = company
        .runtime
        .store()
        .load(company.id())
        .await?
        .ok_or_else(|| OpenCompanyError::CompanyNotFound(company.id().to_string()))?;
    record.manifest.company.logo_url = logo_url;
    company.runtime.store().save(&record).await?;
    Ok(Json(company.runtime.status().await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logo_value_accepts_images_and_rejects_external_or_oversized_values() {
        let valid = "data:image/png;base64,iVBORw==".to_string();
        assert_eq!(
            company_logo_value(Some(valid.clone())).unwrap(),
            Some(valid)
        );
        assert!(company_logo_value(Some("https://example.com/logo.png".into())).is_err());
        assert!(company_logo_value(Some("x".repeat(COMPANY_LOGO_MAX_CHARS + 1))).is_err());
    }
}
