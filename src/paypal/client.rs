//! A PayPal REST client: OAuth2 client-credentials, with the access token
//! cached for its lifetime.
//!
//! # Why the token is cached
//!
//! PayPal issues a bearer token from `POST /v1/oauth2/token` that lasts about
//! nine hours. Fetching one per call would double every request and, at any
//! volume, run into PayPal's rate limit on the token endpoint specifically —
//! which fails the *next* call rather than the one that caused it. So a token is
//! held until shortly before it expires and then re-fetched.
//!
//! The cache lives on the client, and the client is built per company, so one
//! company's token can never authenticate another's call.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::Mutex;

use crate::company::paypal::PaypalEnvironment;
use crate::error::{OpenCompanyError, Result};

/// A cached bearer token and when it stops being usable.
#[derive(Clone, Debug)]
struct CachedToken {
    token: String,
    /// When to stop trusting it. Deliberately earlier than PayPal's own expiry
    /// — see [`PaypalClient::token`].
    good_until: Instant,
}

/// One company's PayPal credentials and environment.
#[derive(Clone)]
pub struct PaypalConfig {
    /// The REST app client id.
    pub client_id: String,
    /// The REST app secret.
    pub client_secret: String,
    /// Which PayPal world these belong to.
    pub environment: PaypalEnvironment,
}

/// Prints the environment and **redacts both halves of the credential**.
///
/// Same reasoning as `ChargebeeConfig`: this is reachable from large aggregates
/// that debugging code prints wholesale, and a derived `Debug` would put a live
/// PayPal secret into any log line that formatted one.
impl std::fmt::Debug for PaypalConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaypalConfig")
            .field("environment", &self.environment)
            .field("client_id", &"<redacted>")
            .field("client_secret", &"<redacted>")
            .finish()
    }
}

/// A PayPal API client bound to one company's credentials.
#[derive(Clone)]
pub struct PaypalClient {
    http: reqwest::Client,
    config: PaypalConfig,
    base_url: String,
    cached: Arc<Mutex<Option<CachedToken>>>,
}

impl std::fmt::Debug for PaypalClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaypalClient")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

/// Builds the crate error for a PayPal failure.
fn err(status: u16, code: &str, message: impl Into<String>) -> OpenCompanyError {
    OpenCompanyError::Paypal {
        status,
        code: code.to_string(),
        message: message.into(),
    }
}

impl PaypalClient {
    /// Builds a client against the environment named in `config`.
    pub fn new(config: PaypalConfig) -> Result<Self> {
        let base = config.environment.base_url().to_string();
        Self::with_base_url(config, base)
    }

    /// Builds a client against an explicit base URL, with no trailing slash.
    pub fn with_base_url(config: PaypalConfig, base_url: String) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            // The token call carries HTTP Basic and every other call a bearer;
            // reqwest re-sends both across redirects, so a 30x to `http://`
            // would leak them. PayPal's API does not redirect.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| err(0, "client_build_failed", e.to_string()))?;
        Ok(Self {
            http,
            config,
            base_url: base_url.trim_end_matches('/').to_string(),
            cached: Arc::new(Mutex::new(None)),
        })
    }

    /// A usable bearer token, from cache when one is still good.
    async fn token(&self) -> Result<String> {
        let mut slot = self.cached.lock().await;
        if let Some(cached) = slot.as_ref()
            && Instant::now() < cached.good_until
        {
            return Ok(cached.token.clone());
        }

        let response = self
            .http
            .post(format!("{}/v1/oauth2/token", self.base_url))
            .basic_auth(&self.config.client_id, Some(&self.config.client_secret))
            .form(&[("grant_type", "client_credentials")])
            .send()
            .await
            .map_err(|e| err(0, "transport_error", e.to_string()))?;

        let status = response.status().as_u16();
        let body = response
            .text()
            .await
            .map_err(|e| err(status, "unreadable_body", e.to_string()))?;
        let parsed: Value = serde_json::from_str(&body).unwrap_or(Value::Null);

        if !(200..300).contains(&status) {
            // PayPal answers bad credentials with `invalid_client`, which reads
            // like a typo. Naming the environment turns the commonest actual
            // cause — sandbox keys against live, or the reverse — into
            // something an operator can see.
            return Err(err(
                status,
                parsed
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("token_failed"),
                format!(
                    "could not obtain a PayPal token for the `{}` environment: {}",
                    self.config.environment.as_str(),
                    parsed
                        .get("error_description")
                        .and_then(Value::as_str)
                        .unwrap_or(&body.chars().take(200).collect::<String>())
                ),
            ));
        }

        let token = parsed
            .get("access_token")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                err(
                    status,
                    "unexpected_response",
                    "no `access_token` in the reply",
                )
            })?
            .to_string();

        // Re-fetch before PayPal's own expiry rather than at it: a token that
        // expires mid-flight fails the call that carried it, and a minute of
        // margin costs nothing against a nine-hour lifetime.
        let lifetime = parsed
            .get("expires_in")
            .and_then(Value::as_u64)
            .unwrap_or(300)
            .saturating_sub(60)
            .max(30);
        *slot = Some(CachedToken {
            token: token.clone(),
            good_until: Instant::now() + Duration::from_secs(lifetime),
        });
        Ok(token)
    }

    /// `GET path` with query parameters, returning the decoded JSON.
    pub async fn get(&self, path: &str, query: &[(String, String)]) -> Result<Value> {
        let token = self.token().await?;
        let response = self
            .http
            .get(format!("{}{path}", self.base_url))
            .bearer_auth(token)
            .query(query)
            .send()
            .await
            .map_err(|e| err(0, "transport_error", e.to_string()))?;

        let status = response.status().as_u16();
        let body = response
            .text()
            .await
            .map_err(|e| err(status, "unreadable_body", e.to_string()))?;
        let parsed: Value = serde_json::from_str(&body).unwrap_or(Value::Null);

        if (200..300).contains(&status) {
            return Ok(parsed);
        }
        Err(err(
            status,
            parsed
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("unknown"),
            parsed
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| body.chars().take(300).collect()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> PaypalConfig {
        PaypalConfig {
            client_id: "AY_client".to_string(),
            client_secret: "EL_secret".to_string(),
            environment: PaypalEnvironment::Sandbox,
        }
    }

    #[test]
    fn debug_never_renders_either_half_of_the_credential() {
        let rendered = format!("{:?}", config());
        assert!(!rendered.contains("AY_client"), "{rendered}");
        assert!(!rendered.contains("EL_secret"), "{rendered}");
        assert!(rendered.contains("Sandbox"), "{rendered}");
    }

    #[test]
    fn a_client_debug_does_not_reach_into_its_config() {
        let client = PaypalClient::new(config()).expect("builds");
        let rendered = format!("{client:?}");
        assert!(!rendered.contains("EL_secret"), "{rendered}");
        assert!(rendered.contains("sandbox.paypal.com"), "{rendered}");
    }

    #[tokio::test]
    async fn a_token_is_reused_rather_than_refetched() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let hits = Arc::new(AtomicUsize::new(0));
        let seen = hits.clone();

        let app = axum::Router::new().fallback(axum::routing::any(move || {
            let seen = seen.clone();
            async move {
                seen.fetch_add(1, Ordering::SeqCst);
                (
                    axum::http::StatusCode::OK,
                    [("content-type", "application/json")],
                    r#"{"access_token":"tok_1","expires_in":32400}"#,
                )
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let client =
            PaypalClient::with_base_url(config(), format!("http://{addr}")).expect("builds");
        assert_eq!(client.token().await.expect("first"), "tok_1");
        assert_eq!(client.token().await.expect("second"), "tok_1");
        assert_eq!(client.token().await.expect("third"), "tok_1");
        // Three calls, ONE trip: without the cache every API call would pay for
        // a token, and PayPal rate-limits that endpoint separately.
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn bad_credentials_name_the_environment_not_just_invalid_client() {
        let app = axum::Router::new().fallback(axum::routing::any(|| async {
            (
                axum::http::StatusCode::UNAUTHORIZED,
                [("content-type", "application/json")],
                r#"{"error":"invalid_client","error_description":"Client Authentication failed"}"#,
            )
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let client =
            PaypalClient::with_base_url(config(), format!("http://{addr}")).expect("builds");
        let message = client
            .token()
            .await
            .expect_err("401 is an error")
            .to_string();
        // "invalid_client" alone reads as a typo; the commonest real cause is
        // sandbox keys pointed at live, which only the environment reveals.
        assert!(message.contains("sandbox"), "{message}");
        assert!(
            message.contains("Client Authentication failed"),
            "{message}"
        );
        server.abort();
    }
}
