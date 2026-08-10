//! The desktop's HTTP and event lane, in Rust rather than in the webview.
//!
//! ## Why the console does not talk to hosts directly
//!
//! Three reasons, in the order they bite:
//!
//! 1. **CORS.** A webview origin (`tauri://localhost`) is cross-origin with
//!    every host. Fetching directly would require each operator to add that
//!    origin to `OPENCOMPANY_CORS_ORIGINS` before their desktop could connect —
//!    a per-server configuration step standing in front of the product's
//!    headline feature. Requests made from Rust are not subject to CORS at all.
//! 2. **The credential.** A device token attached here never enters the
//!    webview, so a cross-site-scripting bug in rendered agent markdown cannot
//!    exfiltrate N hosts' credentials. It lives in the OS keychain and is read
//!    by this process.
//! 3. **Streaming.** `EventSource` cannot set a request header, so it cannot
//!    carry the session header at all, and a `SameSite=Lax` cookie is never
//!    sent cross-site. There is no way for a desktop webview to open the event
//!    stream on its own.
//!
//! ## The shape, and the one rule
//!
//! [`ProxyRegistry`] holds a `HashMap<ConnectionId, Connection>`. **Never a
//! single "active connection".** That is the field that stops block/buzz from
//! holding more than one relay at a time, and every command here takes an
//! explicit `connection_id` for the same reason — an implicit current
//! connection would reintroduce it through the back door.

pub mod sse;

use std::collections::HashMap;
use std::sync::Arc;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

pub use sse::SseDecoder;

/// A connection's id, minted by the console and opaque here.
pub type ConnectionId = String;

/// How this client authenticates to one host.
#[derive(Clone, Debug, Default)]
pub enum Credential {
    /// Nothing to present. A host that needs one answers 401 and the console
    /// shows its sign-in.
    #[default]
    None,
    /// A paired device session, presented in the header carrier as
    /// `<company>.<token>`.
    ///
    /// Held as the fully-rendered header value rather than as its parts,
    /// because that is the only form anything here needs — and a struct with a
    /// `token` field invites something to log it.
    Device(String),
    /// A platform bearer, for the hosting layer.
    Platform(String),
}

/// One host this client is connected to.
#[derive(Clone, Debug)]
pub struct Connection {
    pub base_url: String,
    pub credential: Credential,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyRequest {
    pub method: String,
    /// Origin-relative, e.g. `/api/v1/company/tasks`. The host is the
    /// connection's, so a caller cannot aim one connection's credential at
    /// another origin by passing an absolute URL.
    pub path: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub body: Option<String>,
}

/// Mirrors the console's `TransportResponse` exactly.
///
/// Field-for-field with what `BrowserTransport` produces, because the console's
/// error handling reads all of it: the status decides success, `statusText`
/// becomes the message when the host sends no envelope, and the header map
/// answers the one `content-disposition` read. A shape that merely resembled it
/// would produce subtly different `ApiError`s on one transport.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyResponse {
    pub status: u16,
    pub status_text: String,
    pub url: String,
    pub text: String,
    /// Lowercased response header names to values.
    pub headers: HashMap<String, String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error("no such connection: {0}")]
    UnknownConnection(ConnectionId),
    #[error("{0}")]
    Transport(String),
}

impl serde::Serialize for ProxyErrorWire {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

/// The wire form of a [`ProxyError`]: a plain string.
///
/// The console turns *any* rejection into its own `network_error` — the
/// transport's own words are deliberately not surfaced, because they differ per
/// transport and the message is shown to a person. So there is nothing to
/// structure here.
#[derive(Debug)]
pub struct ProxyErrorWire(pub String);

impl From<ProxyError> for ProxyErrorWire {
    fn from(error: ProxyError) -> Self {
        Self(error.to_string())
    }
}

/// Every host this client is connected to.
#[derive(Debug)]
pub struct ProxyRegistry {
    connections: RwLock<HashMap<ConnectionId, Connection>>,
    http: reqwest::Client,
}

impl Default for ProxyRegistry {
    fn default() -> Self {
        Self {
            connections: RwLock::new(HashMap::new()),
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                // A host that accepts a connection but never answers must not
                // hold a console command open forever. Deliberately *not* a
                // client-wide `timeout`: that would also cut the long-lived
                // `subscribe` event stream.
                .connect_timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("reqwest client with fixed settings cannot fail"),
        }
    }
}

impl ProxyRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers or replaces a connection.
    pub async fn upsert(&self, id: ConnectionId, connection: Connection) {
        self.connections.write().await.insert(id, connection);
    }

    pub async fn remove(&self, id: &str) {
        self.connections.write().await.remove(id);
    }

    pub async fn ids(&self) -> Vec<ConnectionId> {
        self.connections.read().await.keys().cloned().collect()
    }

    async fn get(&self, id: &str) -> Result<Connection, ProxyError> {
        self.connections
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| ProxyError::UnknownConnection(id.to_string()))
    }

    /// Performs one request against `id`'s host.
    ///
    /// Resolves for every HTTP status, including 5xx — deciding what a status
    /// means is the console's job, and a proxy that errored on 401 would move
    /// session handling into Rust where only half of it lives.
    pub async fn request(
        &self,
        id: &str,
        request: ProxyRequest,
    ) -> Result<ProxyResponse, ProxyError> {
        let connection = self.get(id).await?;
        let url = join(&connection.base_url, &request.path);

        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|_| ProxyError::Transport(format!("bad method {:?}", request.method)))?;
        let mut builder = self
            .http
            .request(method, &url)
            // Per-request, not client-wide: `subscribe` must stay long-lived.
            .timeout(std::time::Duration::from_secs(30));
        for (name, value) in &request.headers {
            // The proxy owns the session/authority headers; a caller-supplied
            // copy would be appended rather than replaced, leaving the host to
            // arbitrate between two session values. See `RESERVED_HEADERS`.
            if RESERVED_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
                continue;
            }
            builder = builder.header(name, value);
        }
        builder = apply_credential(builder, &connection.credential);
        if let Some(body) = request.body {
            builder = builder.body(body);
        }

        let response = builder
            .send()
            .await
            .map_err(|error| ProxyError::Transport(error.to_string()))?;

        let status = response.status();
        let final_url = response.url().to_string();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_ascii_lowercase(), value.to_string()))
            })
            .collect();
        let text = response
            .text()
            .await
            .map_err(|error| ProxyError::Transport(error.to_string()))?;

        Ok(ProxyResponse {
            status: status.as_u16(),
            // `canonical_reason` is absent for a non-standard status; the
            // console falls back to `HTTP <status>` when this is empty, which is
            // the same thing `BrowserTransport` does with an empty `statusText`.
            status_text: status.canonical_reason().unwrap_or_default().to_string(),
            url: final_url,
            text,
            headers,
        })
    }

    /// Opens `id`'s event stream, handing each payload to `on_event`.
    ///
    /// Runs until the stream ends or the host goes away; the caller spawns it
    /// and drops the task to unsubscribe. Reconnection is deliberately absent —
    /// the console already polls as its safety net, and a retry loop here would
    /// be a second, invisible one.
    pub async fn subscribe<F>(
        &self,
        id: &str,
        path: &str,
        mut on_event: F,
    ) -> Result<(), ProxyError>
    where
        F: FnMut(String) + Send,
    {
        let connection = self.get(id).await?;
        let url = join(&connection.base_url, path);

        let mut builder = self.http.get(&url).header("accept", "text/event-stream");
        builder = apply_credential(builder, &connection.credential);
        let response = builder
            .send()
            .await
            .map_err(|error| ProxyError::Transport(error.to_string()))?;

        if !response.status().is_success() {
            return Err(ProxyError::Transport(format!(
                "event stream refused with {}",
                response.status()
            )));
        }

        let mut decoder = SseDecoder::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| ProxyError::Transport(error.to_string()))?;
            // Bytes, not `from_utf8_lossy` per chunk: a read boundary can fall
            // inside a multi-byte codepoint, and decoding each chunk on its own
            // would turn both halves into U+FFFD. The decoder holds the
            // incomplete tail until the chunk that finishes it. Genuinely
            // malformed bytes are still lossy — a stream is not a place to fail
            // over one bad sequence, and the payloads are JSON the console
            // re-parses.
            for event in decoder.push_bytes(&chunk) {
                on_event(event);
            }
        }
        Ok(())
    }
}

/// Header names the proxy owns, matched case-insensitively.
///
/// The console passes its request headers straight through
/// (`ProxyTransport.request`), and `RequestBuilder::header` **appends** rather
/// than replaces — so without this filter a header the webview supplied and the
/// credential attached alongside it would both go on the wire under one name.
/// Axum reads `HeaderMap::get`, which returns the *first*, so the webview's
/// value would win and the connection's own credential would be the one
/// ignored.
///
/// That inverts the property this module exists for. The token never entering
/// the webview is worth little if the webview can still say what the request
/// authenticates as — a script injected into rendered agent markdown could
/// present a credential of its choosing to every connected host.
///
/// Dropped rather than rejected: these are not headers a legitimate console
/// call sets, so refusing the whole request would turn a defence into a way to
/// break one.
const RESERVED_HEADERS: [&str; 4] = [
    // The session carrier `apply_credential` sets for a paired device.
    "x-opencompany-session",
    // The platform bearer, via `bearer_auth`.
    "authorization",
    // Neither is set here, but both are ambient authority all the same, and a
    // caller has no reason to hand-roll one through the proxy.
    "cookie",
    "proxy-authorization",
];

/// Attaches whatever this connection authenticates with.
fn apply_credential(
    builder: reqwest::RequestBuilder,
    credential: &Credential,
) -> reqwest::RequestBuilder {
    match credential {
        Credential::None => builder,
        // The header carrier, which exists precisely because a desktop cannot
        // use the cookie: `SameSite=Lax` means a browser never sends one
        // cross-site, and a webview is cross-site with every host.
        Credential::Device(value) => builder.header("x-opencompany-session", value),
        Credential::Platform(token) => builder.bearer_auth(token),
    }
}

/// Joins a base URL and an origin-relative path.
///
/// Both halves are normalised so that a base with a trailing slash and a path
/// with a leading one cannot produce `//api/v1`, which some hosts route
/// differently and some proxies rewrite.
fn join(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    if path.starts_with('/') {
        format!("{base}{path}")
    } else {
        format!("{base}/{path}")
    }
}

/// A registry shared with the Tauri command layer.
pub type SharedProxy = Arc<ProxyRegistry>;

#[cfg(test)]
mod test {
    use super::*;

    /// A one-shot host that answers with the request head it received.
    ///
    /// Deliberately raw TCP rather than a framework: what is under test is
    /// which bytes leave this process, and anything that parses the request
    /// into a map on the way in could normalise away the very duplicate the
    /// test exists to catch.
    async fn reflector() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            // Read to the end of the head only; a body would block a reader
            // that does not know the content length.
            while !head.ends_with(b"\r\n\r\n") {
                use tokio::io::AsyncReadExt as _;
                if socket.read_exact(&mut byte).await.is_err() {
                    break;
                }
                head.push(byte[0]);
            }
            use tokio::io::AsyncWriteExt as _;
            let body = String::from_utf8_lossy(&head).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
        });
        format!("http://{addr}")
    }

    /// The webview must not be able to choose what a request authenticates as.
    ///
    /// `RequestBuilder::header` appends, and axum reads `HeaderMap::get`, which
    /// takes the first value — so a caller-supplied session header that reached
    /// the wire would be the one the host honoured, and the connection's own
    /// credential would be the one ignored.
    #[tokio::test]
    async fn a_caller_cannot_supply_the_credential_headers() {
        let base = reflector().await;
        let registry = ProxyRegistry::new();
        registry
            .upsert(
                "primary".into(),
                Connection {
                    base_url: base,
                    credential: Credential::Device("acme.the-real-token".into()),
                },
            )
            .await;

        let mut headers = HashMap::new();
        headers.insert(
            "x-opencompany-session".to_string(),
            "evil.stolen".to_string(),
        );
        headers.insert("Authorization".to_string(), "Bearer evil".to_string());
        headers.insert("cookie".to_string(), "oc_session_acme=evil".to_string());
        // An ordinary header must still get through; this is a filter, not a
        // seal.
        headers.insert("content-type".to_string(), "application/json".to_string());

        let reflected = registry
            .request(
                "primary",
                ProxyRequest {
                    method: "POST".into(),
                    path: "/api/v1/anything".into(),
                    headers,
                    body: None,
                },
            )
            .await
            .expect("the reflector answers")
            .text
            .to_ascii_lowercase();

        assert!(
            reflected.contains("x-opencompany-session: acme.the-real-token"),
            "the connection's own credential must be on the wire: {reflected}"
        );
        assert_eq!(
            reflected.matches("x-opencompany-session:").count(),
            1,
            "exactly one session header, or the host reads the wrong one: {reflected}"
        );
        for forbidden in ["evil.stolen", "bearer evil", "oc_session_acme=evil"] {
            assert!(
                !reflected.contains(forbidden),
                "{forbidden:?} must not reach the host: {reflected}"
            );
        }
        assert!(
            reflected.contains("content-type: application/json"),
            "an ordinary header must still pass: {reflected}"
        );
    }

    #[test]
    fn credential_header_matching_ignores_case() {
        // A caller sends whatever casing it likes; HTTP header names are
        // case-insensitive, so a case-sensitive filter would be no filter.
        let reserved = |name: &str| RESERVED_HEADERS.contains(&name.to_ascii_lowercase().as_str());
        for name in ["Authorization", "AUTHORIZATION", "X-OpenCompany-Session"] {
            assert!(reserved(name), "{name} must be filtered");
        }
        for name in ["content-type", "accept", "x-request-id"] {
            assert!(!reserved(name), "{name} must pass");
        }
    }

    #[test]
    fn joining_never_doubles_a_slash() {
        assert_eq!(join("http://h.test", "/api/v1"), "http://h.test/api/v1");
        assert_eq!(join("http://h.test/", "/api/v1"), "http://h.test/api/v1");
        assert_eq!(join("http://h.test/", "api/v1"), "http://h.test/api/v1");
        // Same-origin, which is what the embedded host looks like before its
        // port is known.
        assert_eq!(join("", "/api/v1"), "/api/v1");
    }

    #[tokio::test]
    async fn an_unknown_connection_is_named_rather_than_guessed() {
        let registry = ProxyRegistry::new();
        let error = registry
            .request(
                "nope",
                ProxyRequest {
                    method: "GET".into(),
                    path: "/healthz".into(),
                    headers: HashMap::new(),
                    body: None,
                },
            )
            .await
            .expect_err("an unregistered connection has no host to reach");
        assert!(error.to_string().contains("nope"));
    }

    #[tokio::test]
    async fn connections_are_independent_entries_not_one_active_slot() {
        // The buzz regression, at the Rust boundary: two hosts coexist, and
        // removing one leaves the other addressable.
        let registry = ProxyRegistry::new();
        registry
            .upsert(
                "a".into(),
                Connection {
                    base_url: "http://a.test".into(),
                    credential: Credential::None,
                },
            )
            .await;
        registry
            .upsert(
                "b".into(),
                Connection {
                    base_url: "http://b.test".into(),
                    credential: Credential::None,
                },
            )
            .await;

        let mut ids = registry.ids().await;
        ids.sort();
        assert_eq!(ids, vec!["a".to_string(), "b".to_string()]);

        registry.remove("a").await;
        assert_eq!(registry.ids().await, vec!["b".to_string()]);
    }
}
