//! A company's own SMTP credentials + test send, and the `lettre` transport.
//!
//! This is the SMTP-specific half of outbound mail; the provider-agnostic
//! adapter it plugs into lives in [`mailer`](super::mailer).
//!
//! `PUT …/smtp` stores credentials in [`SecretStore`](crate::ports::SecretStore)
//! under [`SMTP_KEY`](super::SMTP_KEY) and returns a non-secret
//! [`SmtpStatus`] — the password never appears in any response. `POST …/smtp/test`
//! sends a test email through the mockable
//! [`MailSender`](super::mailer::MailSender) seam, pulling the stored
//! credentials per send, and records the sent mail in the company's
//! [`InboxStore`](crate::ports::InboxStore) so the console shows it. The real
//! `lettre` transport is gated behind the `smtp` feature; without an injected
//! sender the test route is "not wired yet" (404).
//!
//! These are the *company's* credentials, distinct from the host-level ones in
//! [`MailConfig`](super::mailer::MailConfig) that platform mail uses.

use std::sync::Arc;

use axum::extract::State;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::company::runtime::CompanyRuntime;
use crate::error::OpenCompanyError;
use crate::ports::inbox::EmailRecord;
use crate::ports::types::SecretValue;
use crate::ports::{generate_id, now_millis};
use crate::server::error::ApiError;
use crate::server::ops::mailer::{MailCredentials, OutboundEmail};
use crate::server::ops::{AdminScopedCompany, SMTP_KEY, ScopedCompany, scoped};

/// The SMTP security mode. Mirrors the console's `SmtpSecurity`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SmtpSecurity {
    /// No transport security.
    None,
    /// Opportunistic STARTTLS on the submission port.
    #[default]
    Starttls,
    /// Implicit TLS (SMTPS).
    Ssl,
}

/// The full SMTP credentials — **secret**. Persisted only to
/// [`SecretStore`](crate::ports::SecretStore); never serialized into a route
/// response (the password would leak).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SmtpCredentials {
    /// SMTP server host.
    pub host: String,
    /// SMTP server port.
    pub port: u16,
    /// Transport security mode.
    #[serde(default)]
    pub security: SmtpSecurity,
    /// Login username.
    pub username: String,
    /// Login password (secret).
    pub password: String,
    /// Display name on the `From` header.
    #[serde(default)]
    pub from_name: String,
    /// Envelope/from address.
    pub from_email: String,
}

/// The non-secret status of a company's SMTP configuration. The password is
/// intentionally absent — a response never carries credential material.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SmtpStatus {
    /// Whether SMTP credentials are stored.
    pub configured: bool,
    /// SMTP host, if configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// SMTP port, if configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// Security mode, if configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security: Option<SmtpSecurity>,
    /// Login username, if configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// From display name, if configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_name: Option<String>,
    /// From address, if configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_email: Option<String>,
}

impl SmtpStatus {
    /// Projects credentials to their non-secret status. Drops the password.
    pub fn from_credentials(creds: &SmtpCredentials) -> Self {
        Self {
            configured: true,
            host: Some(creds.host.clone()),
            port: Some(creds.port),
            security: Some(creds.security),
            username: Some(creds.username.clone()),
            from_name: Some(creds.from_name.clone()),
            from_email: Some(creds.from_email.clone()),
        }
    }

    /// The "nothing stored" status.
    pub fn unconfigured() -> Self {
        Self {
            configured: false,
            host: None,
            port: None,
            security: None,
            username: None,
            from_name: None,
            from_email: None,
        }
    }
}

/// Builds the SMTP route fragment.
pub fn router() -> Router<AppState> {
    scoped("/smtp", get(get_smtp).put(put_smtp)).merge(scoped("/smtp/test", post(test_smtp)))
}

// -- GET smtp ---------------------------------------------------------------

/// `GET …/smtp` (both scope forms) — the non-secret status of what is stored.
///
/// `ScopedCompany`, and the asymmetry with its neighbours is deliberate. The
/// admin line on this plane guards the company's outward identity: `PUT …/smtp`
/// sets the credentials its mail goes out under, and `POST …/smtp/test` sends
/// real mail to a recipient the caller names in the body. This read does
/// neither — [`SmtpStatus`] is host, port, username and from-address, with the
/// password absent by construction — so it stays open to any member, the same
/// rule `docs/modules/server/authority.md` already states for reads on these
/// surfaces. Admin-only, it would `403` a member on the Settings screen while
/// the identical projection stayed readable to them over GraphQL as
/// `Company.smtp`.
///
/// [`SmtpStatus::unconfigured`] rather than `null` when nothing is stored: the
/// type already carries a `configured` flag to say so, and the GraphQL field is
/// the non-null `SmtpStatus!`.
async fn get_smtp(company: ScopedCompany) -> Result<Json<SmtpStatus>, ApiError> {
    let stored = load_credentials(&company.runtime).await?;
    Ok(Json(stored.as_ref().map_or_else(
        SmtpStatus::unconfigured,
        SmtpStatus::from_credentials,
    )))
}

// -- PUT smtp ---------------------------------------------------------------

/// Persists credentials and returns the non-secret status.
async fn store_credentials(
    runtime: Arc<CompanyRuntime>,
    creds: SmtpCredentials,
) -> Result<Json<SmtpStatus>, ApiError> {
    let json = serde_json::to_string(&creds)?;
    runtime
        .secrets()
        .set(runtime.id(), SMTP_KEY, SecretValue(json))
        .await?;
    Ok(Json(SmtpStatus::from_credentials(&creds)))
}

/// The save-credentials body: [`SmtpCredentials`], except that the password is
/// a **patch**.
///
/// Same shape and same field names on the wire — a body carrying a password
/// behaves exactly as it always did. What is new is that a body *without* one
/// keeps the stored password, mirroring the patch semantics
/// [`hosting`](super::hosting) already uses for its API key. The reason is the
/// same: the password is write-only, so a form can never render it back, and
/// without this every correction to a from-name would cost the operator a
/// credential they would have to go and look up again.
///
/// Field names stay `snake_case` (`from_name`, `from_email`) because
/// [`SmtpCredentials`] has no `rename_all` and the console mirrors it as-is.
#[derive(Debug, Deserialize)]
struct SmtpConfigBody {
    /// SMTP server host.
    host: String,
    /// SMTP server port.
    port: u16,
    /// Transport security mode.
    #[serde(default)]
    security: SmtpSecurity,
    /// Login username.
    username: String,
    /// Login password. Omit (or send empty) to keep the stored one.
    #[serde(default)]
    password: Option<String>,
    /// Display name on the `From` header.
    #[serde(default)]
    from_name: String,
    /// Envelope/from address.
    from_email: String,
}

/// `PUT …/smtp` (both scope forms).
///
/// Requires authority over the company (issue #403): these credentials are the
/// address the company's mail goes out as.
///
/// Every field but the password replaces what is stored. The password is kept
/// when the body omits it, so "stored — leave blank to keep" is a save the
/// console can actually offer; with nothing supplied and nothing stored, the
/// request is refused rather than persisting credentials that could never
/// authenticate.
async fn put_smtp(
    company: AdminScopedCompany,
    Json(body): Json<SmtpConfigBody>,
) -> Result<Json<SmtpStatus>, ApiError> {
    let supplied = body
        .password
        .as_deref()
        .map(str::trim)
        .filter(|password| !password.is_empty())
        .map(str::to_string);
    let password = match supplied {
        Some(password) => password,
        None => load_credentials(&company.runtime)
            .await?
            .map(|stored| stored.password)
            .ok_or_else(|| {
                ApiError(OpenCompanyError::InvalidRequest(
                    "an SMTP password is required".to_string(),
                ))
            })?,
    };
    let creds = SmtpCredentials {
        host: body.host,
        port: body.port,
        security: body.security,
        username: body.username,
        password,
        from_name: body.from_name,
        from_email: body.from_email,
    };
    store_credentials(company.runtime, creds).await
}

// -- POST smtp/test ---------------------------------------------------------

/// The optional test-send override.
#[derive(Debug, Default, Deserialize)]
struct TestSend {
    /// Recipient; defaults to the configured `from_email` (loopback test).
    #[serde(default)]
    to: Option<String>,
}

/// The test-send result.
#[derive(Debug, Serialize)]
struct TestResult {
    /// Whether the send was accepted.
    ok: bool,
    /// A prosumer-friendly description of the outcome.
    message: String,
}

/// Sends a test email through the injected sender and records it as outbound.
async fn run_test(
    state: &AppState,
    runtime: Arc<CompanyRuntime>,
    body: TestSend,
) -> Result<Json<TestResult>, Response> {
    use axum::response::IntoResponse;
    // Not wired without a sender (default build / no `smtp` feature).
    let Some(sender) = state.connections().mail.clone() else {
        return Err(super::not_wired("smtp test send"));
    };
    let creds = load_credentials(&runtime)
        .await
        .map_err(|e| ApiError(e).into_response())?;
    let Some(creds) = creds else {
        return Err(ApiError(OpenCompanyError::InvalidRequest(
            "no SMTP credentials configured".to_string(),
        ))
        .into_response());
    };
    let to = body.to.unwrap_or_else(|| creds.from_email.clone());
    let email = OutboundEmail {
        to: to.clone(),
        subject: "OpenCompany SMTP test".to_string(),
        body: "This is a test message confirming your outbound email is wired up.".to_string(),
    };
    // The company's stored credentials are SMTP by construction (the route that
    // writes them is `PUT …/smtp`), so tag them for the provider-agnostic seam.
    let tagged = MailCredentials::Smtp(creds.clone());
    match sender.send(&tagged, &email).await {
        Ok(()) => {
            record_outbound(&runtime, &creds, &email).await;
            Ok(Json(TestResult {
                ok: true,
                message: format!("Test email sent to {to}."),
            }))
        }
        Err(err) => Ok(Json(TestResult {
            ok: false,
            message: format!("Send failed: {err}"),
        })),
    }
}

/// Loads and parses stored SMTP credentials, if any.
pub(crate) async fn load_credentials(
    runtime: &CompanyRuntime,
) -> Result<Option<SmtpCredentials>, OpenCompanyError> {
    let Some(value) = runtime.secrets().get(runtime.id(), SMTP_KEY).await? else {
        return Ok(None);
    };
    let creds: SmtpCredentials = serde_json::from_str(value.expose())?;
    Ok(Some(creds))
}

/// Appends a sent email to the sender's inbox so the console shows outbound mail.
pub(crate) async fn record_outbound(
    runtime: &CompanyRuntime,
    creds: &SmtpCredentials,
    email: &OutboundEmail,
) {
    let record = EmailRecord {
        id: generate_id(),
        inbox: local_part(&creds.from_email),
        from_name: creds.from_name.clone(),
        from_email: creds.from_email.clone(),
        subject: email.subject.clone(),
        body: email.body.clone(),
        at_millis: now_millis(),
        read: true,
        outbound: true,
    };
    if let Err(err) = runtime.inbox().append(runtime.id(), &record).await {
        tracing::warn!(company = %runtime.id(), "failed to record outbound email: {err}");
    }
}

/// The local part of an address (`ceo@acme.test` → `ceo`), or the whole string
/// when it carries no `@`.
///
/// `pub` (not `pub(crate)`) so the `opencompany` binary target — a separate
/// crate from this library — can reuse it to scope an injected mailbox to its
/// owning company (see `spawn_mailbox_poller`/`register_company` in
/// `src/bin/opencompany.rs`).
pub fn local_part(address: &str) -> String {
    address
        .split_once('@')
        .map(|(local, _)| local.to_string())
        .unwrap_or_else(|| address.to_string())
}

/// `POST …/smtp/test` (both scope forms).
///
/// Requires authority over the company (issue #403). Grouped with the write
/// rather than with the read-only probes because the caller chooses the
/// recipient: it sends real mail from the company's address to an address
/// supplied in the request body.
async fn test_smtp(
    company: AdminScopedCompany,
    State(state): State<AppState>,
    body: Option<Json<TestSend>>,
) -> Result<Json<TestResult>, Response> {
    run_test(
        &state,
        company.runtime,
        body.map(|b| b.0).unwrap_or_default(),
    )
    .await
}

/// The real `lettre` SMTP transport. Gated behind the `smtp` feature so the
/// default build links no SMTP crate.
#[cfg(feature = "smtp")]
pub struct LettreMailSender;

/// Upper bound on one delivery (connect through the final response).
///
/// `lettre`'s own timeout bounds individual phases and not reliably all of
/// them, so a relay that accepts a connection and then stalls — mid-TLS, mid
/// -write, or before its reply — can hold a caller far longer than the caller
/// budgeted for. Every send here happens inside an HTTP request an operator is
/// waiting on, so the bound lives next to the socket rather than at each call
/// site: one place, and no call site can forget it. Same placement, and same
/// reason, as `IMAP_TIMEOUT` in `imap.rs`.
#[cfg(feature = "smtp")]
const SMTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

#[cfg(feature = "smtp")]
#[async_trait::async_trait]
impl crate::server::ops::mailer::MailSender for LettreMailSender {
    async fn send(
        &self,
        creds: &MailCredentials,
        email: &OutboundEmail,
    ) -> Result<(), OpenCompanyError> {
        match tokio::time::timeout(SMTP_TIMEOUT, Self::send_inner(creds, email)).await {
            Ok(result) => result,
            // The same variant a refused send reports, because it means the
            // same thing to every caller: the message was not accepted, and
            // nothing may be recorded as delivered.
            Err(_) => Err(OpenCompanyError::Store("smtp send: timed out".into())),
        }
    }
}

#[cfg(feature = "smtp")]
impl LettreMailSender {
    async fn send_inner(
        creds: &MailCredentials,
        email: &OutboundEmail,
    ) -> Result<(), OpenCompanyError> {
        use lettre::transport::smtp::authentication::Credentials;
        use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

        // Selecting the transport by variant is what makes this an adapter: a
        // future provider adds a variant, and this match stops compiling until
        // someone decides what this sender does with it.
        let MailCredentials::Smtp(creds) = creds;

        let from = if creds.from_name.is_empty() {
            creds.from_email.clone()
        } else {
            format!("{} <{}>", creds.from_name, creds.from_email)
        };
        let message = Message::builder()
            .from(from.parse().map_err(|e| {
                OpenCompanyError::InvalidRequest(format!("invalid from address: {e}"))
            })?)
            .to(email.to.parse().map_err(|e| {
                OpenCompanyError::InvalidRequest(format!("invalid to address: {e}"))
            })?)
            .subject(&email.subject)
            .body(email.body.clone())
            .map_err(|e| OpenCompanyError::Store(format!("build message: {e}")))?;

        let builder = match creds.security {
            SmtpSecurity::None => {
                AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&creds.host)
                    .port(creds.port)
            }
            SmtpSecurity::Starttls => {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&creds.host)
                    .map_err(|e| OpenCompanyError::Store(format!("smtp starttls: {e}")))?
                    .port(creds.port)
            }
            SmtpSecurity::Ssl => AsyncSmtpTransport::<Tokio1Executor>::relay(&creds.host)
                .map_err(|e| OpenCompanyError::Store(format!("smtp relay: {e}")))?
                .port(creds.port),
        };
        let transport = builder
            .credentials(Credentials::new(
                creds.username.clone(),
                creds.password.clone(),
            ))
            .build();
        transport
            .send(message)
            .await
            .map_err(|e| OpenCompanyError::Store(format!("smtp send: {e}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn status_drops_password() {
        let creds = SmtpCredentials {
            host: "smtp.example.com".into(),
            port: 587,
            security: SmtpSecurity::Starttls,
            username: "user".into(),
            password: "s3cret-pw".into(),
            from_name: "Acme".into(),
            from_email: "ceo@acme.test".into(),
        };
        let status = SmtpStatus::from_credentials(&creds);
        let json = serde_json::to_string(&status).unwrap();
        assert!(!json.contains("s3cret-pw"), "password leaked into status");
        assert!(json.contains("smtp.example.com"));
        assert!(status.configured);
    }

    #[test]
    fn local_part_splits_address() {
        assert_eq!(local_part("ceo@acme.test"), "ceo");
        assert_eq!(local_part("bare"), "bare");
    }

    #[tokio::test]
    async fn recording_sender_captures_send() {
        use crate::server::ops::mailer::{MailSender, RecordingMailSender};

        let sender = RecordingMailSender::new();
        let creds = MailCredentials::Smtp(SmtpCredentials {
            host: "h".into(),
            port: 25,
            security: SmtpSecurity::None,
            username: "u".into(),
            password: "p".into(),
            from_name: String::new(),
            from_email: "from@x.test".into(),
        });
        let email = OutboundEmail {
            to: "to@x.test".into(),
            subject: "s".into(),
            body: "b".into(),
        };
        sender.send(&creds, &email).await.unwrap();
        assert_eq!(sender.sent().len(), 1);
        assert_eq!(sender.sent()[0].0, "from@x.test");
    }
}
