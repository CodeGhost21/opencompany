# WS7 Inbound — IMAP Receive: Tasks 1–4 (transport)

Split out of [`07-workload-email-inbound-plan.md`](07-workload-email-inbound-plan.md),
which was over the repository's 500-line ceiling. The parent carries the goal,
architecture and global constraints; this page is the transport half — the
Cargo feature, the `MailReceiver` seam and mock, the injected per-tenant
mailbox config, and the real `async-imap` client. Tasks 5–7 (runtime wiring)
are in [`07-workload-email-inbound-runtime.md`](07-workload-email-inbound-runtime.md).

**Status:** shipped — see the parent for the record.

---

### Task 1: `imap` Cargo feature + deps

**Files:** Modify `Cargo.toml`

**Interfaces:** Produces the `imap` feature enabling `async-imap` + `mail-parser`.

- [ ] **Step 1: Add optional deps** to `[dependencies]` (near the `lettre` line, matching its comment style):

```toml
# IMAP inbound transport + RFC822 parsing for per-teammate mail receiving. Only
# link under the `imap` feature; the `MailReceiver` trait + offline mock compile
# without it. Pure-Rust rustls TLS so no system libs are required.
async-imap = { version = "0.10", default-features = false, features = ["runtime-tokio"], optional = true }
mail-parser = { version = "0.9", optional = true }
```

- [ ] **Step 2: Add the feature** to `[features]` (beside `smtp`):

```toml
# Real IMAP receive for the email surface. The `MailReceiver` trait and the
# offline `RecordingMailReceiver` compile without this; only `AsyncImapReceiver`
# and the RFC822 parser are gated here.
imap = ["dep:async-imap", "dep:mail-parser"]
```

- [ ] **Step 3: Verify both feature states resolve**

Run: `cargo build --all-targets` and `cargo build --all-targets --features imap`
Expected: both succeed (async-imap/mail-parser download + compile under the feature). If the default toolchain fails on a dep, switch to `cargo +1.96.1` (see Global Constraints) and use it for all later steps.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "build(email): add imap feature (async-imap + mail-parser)"
```

---

### Task 2: `ImapCredentials` + `InboundEmail` + `MailReceiver` trait + mock

**Files:** Create `src/server/ops/imap.rs`; Modify `src/server/ops/mailer.rs`, `src/server/ops/mod.rs`

**Interfaces:**
- Produces: `ImapCredentials { host, port, username, password }`; `InboundEmail { from_name, from_email, subject, body }`; `trait MailReceiver { async fn fetch_new(&self, creds: &ImapCredentials) -> Result<Vec<InboundEmail>, OpenCompanyError> }`; `RecordingMailReceiver` (queued messages, records call count).

- [ ] **Step 1: Create `src/server/ops/imap.rs`** with the always-compiled credentials type (network client comes in Task 4):

```rust
//! IMAP inbound: credentials (always compiled) + the async-imap transport
//! (feature-gated in Task 4). Mirrors `smtp.rs`, where `SmtpCredentials` is
//! always compiled and only `LettreMailSender` is gated behind `smtp`.
use serde::{Deserialize, Serialize};

/// Credentials for polling one IMAP mailbox — **secret** (`password`).
#[derive(Clone, Serialize, Deserialize)]
pub struct ImapCredentials {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for ImapCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the password.
        f.debug_struct("ImapCredentials")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}
```

- [ ] **Step 2: Register the module** — add to `src/server/ops/mod.rs`: `pub mod imap;`

- [ ] **Step 3: Add `InboundEmail` + `MailReceiver` + mock** to `src/server/ops/mailer.rs` (near `MailSender`), with the failing test:

```rust
use std::sync::Mutex;
use crate::server::ops::imap::ImapCredentials;

/// One inbound message produced by a [`MailReceiver`]. Plain-text body (v1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InboundEmail {
    pub from_name: String,
    pub from_email: String,
    pub subject: String,
    pub body: String,
}

/// The inbound-fetch seam. Implementations fetch *new* (unseen) messages and
/// mark them seen, so a subsequent call returns only newer mail. Mockable so the
/// poller is exercised offline; the real transport is feature-gated.
#[async_trait]
pub trait MailReceiver: Send + Sync {
    async fn fetch_new(
        &self,
        creds: &ImapCredentials,
    ) -> Result<Vec<InboundEmail>, OpenCompanyError>;
}

/// Offline mock: returns queued batches, one per `fetch_new` call, and counts calls.
pub struct RecordingMailReceiver {
    batches: Mutex<std::collections::VecDeque<Vec<InboundEmail>>>,
    calls: std::sync::atomic::AtomicUsize,
}

impl RecordingMailReceiver {
    pub fn new() -> Self {
        Self { batches: Mutex::new(std::collections::VecDeque::new()), calls: Default::default() }
    }
    /// Queue a batch to be returned by the next `fetch_new`.
    pub fn push_batch(&self, batch: Vec<InboundEmail>) {
        self.batches.lock().expect("poisoned").push_back(batch);
    }
    pub fn calls(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl Default for RecordingMailReceiver {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl MailReceiver for RecordingMailReceiver {
    async fn fetch_new(&self, _creds: &ImapCredentials) -> Result<Vec<InboundEmail>, OpenCompanyError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(self.batches.lock().expect("poisoned").pop_front().unwrap_or_default())
    }
}
```

Test (in `mailer.rs`'s test module):

```rust
#[tokio::test]
async fn recording_receiver_returns_queued_batches_in_order() {
    let creds = ImapCredentials { host: "h".into(), port: 993, username: "u".into(), password: "p".into() };
    let rx = RecordingMailReceiver::new();
    rx.push_batch(vec![InboundEmail { from_name: "A".into(), from_email: "a@x".into(), subject: "s".into(), body: "b".into() }]);
    assert_eq!(rx.fetch_new(&creds).await.unwrap().len(), 1);
    assert_eq!(rx.fetch_new(&creds).await.unwrap().len(), 0); // drained
    assert_eq!(rx.calls(), 2);
}
```

- [ ] **Step 4: Run + gate**

Run: `cargo test mailer:: && cargo clippy --all-targets -- -D warnings && cargo fmt --all -- --check`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/server/ops/imap.rs src/server/ops/mailer.rs src/server/ops/mod.rs
git commit -m "feat(email): MailReceiver trait + ImapCredentials + offline mock"
```

---

### Task 3: `TenantMailboxConfig::from_env`

**Files:** Modify `src/server/ops/mailer.rs`

**Interfaces:**
- Consumes: `SmtpCredentials` (`smtp.rs`), `ImapCredentials` (Task 2).
- Produces: `TenantMailboxConfig { address, smtp: SmtpCredentials, imap: ImapCredentials }`; `pub fn from_env() -> Result<Option<Self>, OpenCompanyError>` reading the manager-injected `OPENCOMPANY_MAIL_ADDRESS/SMTP_HOST/SMTP_PORT/IMAP_HOST/IMAP_PORT/USER/PASSWORD`. `None` when `OPENCOMPANY_MAIL_ADDRESS` is unset; a partial set is an error.

- [ ] **Step 1: Write the failing test** (uses a scoped env guard; run serially):

```rust
#[test]
fn tenant_mailbox_config_parses_injected_env() {
    // Serialize env access; set the 7 injected vars.
    let _g = ENV_LOCK.lock().unwrap();
    for (k, v) in [
        ("OPENCOMPANY_MAIL_ADDRESS", "acme@opencompany.work"),
        ("OPENCOMPANY_MAIL_SMTP_HOST", "mail.opencompany.work"),
        ("OPENCOMPANY_MAIL_SMTP_PORT", "465"),
        ("OPENCOMPANY_MAIL_IMAP_HOST", "mail.opencompany.work"),
        ("OPENCOMPANY_MAIL_IMAP_PORT", "993"),
        ("OPENCOMPANY_MAIL_USER", "acme@opencompany.work"),
        ("OPENCOMPANY_MAIL_PASSWORD", "secret"),
    ] { unsafe { std::env::set_var(k, v) }; }

    let cfg = TenantMailboxConfig::from_env().unwrap().expect("configured");
    assert_eq!(cfg.address, "acme@opencompany.work");
    assert_eq!(cfg.imap.host, "mail.opencompany.work");
    assert_eq!(cfg.imap.port, 993);
    assert_eq!(cfg.smtp.from_email, "acme@opencompany.work");

    for k in ["OPENCOMPANY_MAIL_ADDRESS","OPENCOMPANY_MAIL_SMTP_HOST","OPENCOMPANY_MAIL_SMTP_PORT","OPENCOMPANY_MAIL_IMAP_HOST","OPENCOMPANY_MAIL_IMAP_PORT","OPENCOMPANY_MAIL_USER","OPENCOMPANY_MAIL_PASSWORD"] {
        unsafe { std::env::remove_var(k) };
    }
}
```

Add near the test module: `static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());` (guard shared by env-reading tests). Verify FAIL: `cargo test tenant_mailbox_config_parses_injected_env` → does not compile / not found.

- [ ] **Step 2: Implement `TenantMailboxConfig`** (mirror `MailConfig::from_env`'s `var`/`missing` idiom at `mailer.rs:181`; note the separate host-level `OPENCOMPANY_MAIL_HOST/PORT/...` remain for platform mail — document the split):

```rust
use crate::server::ops::smtp::SmtpCredentials;

/// A managed tenant's OWN mailbox identity, injected by the manager as
/// `OPENCOMPANY_MAIL_*`. Distinct from the host-level `OPENCOMPANY_MAIL_HOST/...`
/// platform-mail read by `MailConfig` (login links). Seeds the company's SMTP
/// send credentials AND the IMAP poller config.
#[derive(Clone, Debug)]
pub struct TenantMailboxConfig {
    pub address: String,
    pub smtp: SmtpCredentials,
    pub imap: ImapCredentials,
}

impl TenantMailboxConfig {
    /// `Ok(None)` when unconfigured (no `OPENCOMPANY_MAIL_ADDRESS`); a *partial*
    /// injection is a hard error.
    pub fn from_env() -> Result<Option<Self>, OpenCompanyError> {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        let Some(address) = var("OPENCOMPANY_MAIL_ADDRESS") else { return Ok(None) };
        let need = |k: &str| var(k).ok_or_else(|| OpenCompanyError::Config(
            format!("{k} is required when OPENCOMPANY_MAIL_ADDRESS is set")));
        let port = |k: &str| -> Result<u16, OpenCompanyError> {
            need(k)?.parse::<u16>().map_err(|_| OpenCompanyError::Config(format!("{k} must be a port number")))
        };
        let user = need("OPENCOMPANY_MAIL_USER")?;
        let password = need("OPENCOMPANY_MAIL_PASSWORD")?;
        let smtp = SmtpCredentials {
            host: need("OPENCOMPANY_MAIL_SMTP_HOST")?,
            port: port("OPENCOMPANY_MAIL_SMTP_PORT")?,
            security: crate::server::ops::smtp::SmtpSecurity::default(),
            username: user.clone(),
            password: password.clone(),
            from_name: String::new(),
            from_email: address.clone(),
        };
        let imap = ImapCredentials {
            host: need("OPENCOMPANY_MAIL_IMAP_HOST")?,
            port: port("OPENCOMPANY_MAIL_IMAP_PORT")?,
            username: user,
            password,
        };
        Ok(Some(Self { address, smtp, imap }))
    }
}
```

- [ ] **Step 3: Add the partial-config + absent tests**

```rust
#[test]
fn tenant_mailbox_config_absent_is_none() {
    let _g = ENV_LOCK.lock().unwrap();
    unsafe { std::env::remove_var("OPENCOMPANY_MAIL_ADDRESS") };
    assert!(TenantMailboxConfig::from_env().unwrap().is_none());
}

#[test]
fn tenant_mailbox_config_partial_is_error() {
    let _g = ENV_LOCK.lock().unwrap();
    unsafe { std::env::set_var("OPENCOMPANY_MAIL_ADDRESS", "acme@opencompany.work") };
    unsafe { std::env::remove_var("OPENCOMPANY_MAIL_PASSWORD") };
    assert!(TenantMailboxConfig::from_env().is_err());
    unsafe { std::env::remove_var("OPENCOMPANY_MAIL_ADDRESS") };
}
```

- [ ] **Step 4: Run + gate**

Run: `cargo test mailer:: -- --test-threads=1 && cargo clippy --all-targets -- -D warnings && cargo fmt --all -- --check`
Expected: PASS (env tests run single-threaded via the shared `ENV_LOCK`).

- [ ] **Step 5: Commit**

```bash
git add src/server/ops/mailer.rs
git commit -m "feat(email): TenantMailboxConfig::from_env for injected per-tenant creds"
```

---

### Task 4: `AsyncImapReceiver` + `parse_message` (feature-gated)

**Files:** Modify `src/server/ops/imap.rs`

**Interfaces:** Produces `AsyncImapReceiver` (`impl MailReceiver`, `#[cfg(feature="imap")]`) and a testable pure `parse_message(raw: &[u8]) -> InboundEmail`.

- [ ] **Step 1: Write the failing parse test** (feature-gated; run with `--features imap`):

```rust
#[cfg(all(test, feature = "imap"))]
mod imap_tests {
    use super::*;
    #[test]
    fn parse_message_extracts_headers_and_text_body() {
        let raw = b"From: Alice <alice@example.com>\r\nSubject: Hi\r\n\r\nHello world\r\n";
        let msg = parse_message(raw);
        assert_eq!(msg.from_email, "alice@example.com");
        assert_eq!(msg.from_name, "Alice");
        assert_eq!(msg.subject, "Hi");
        assert!(msg.body.contains("Hello world"));
    }
}
```

Run: `cargo test --features imap parse_message` → FAIL (undefined).

- [ ] **Step 2: Implement `parse_message` + `AsyncImapReceiver`** in `imap.rs`:

```rust
#[cfg(feature = "imap")]
use crate::server::ops::mailer::{InboundEmail, MailReceiver};
#[cfg(feature = "imap")]
use crate::error::OpenCompanyError;

/// Parse one RFC822 message into an `InboundEmail` (plain-text body, v1).
#[cfg(feature = "imap")]
pub(crate) fn parse_message(raw: &[u8]) -> InboundEmail {
    use mail_parser::MessageParser;
    let parsed = MessageParser::default().parse(raw);
    let (from_name, from_email) = parsed
        .as_ref()
        .and_then(|m| m.from())
        .and_then(|a| a.first())
        .map(|addr| (
            addr.name().unwrap_or_default().to_string(),
            addr.address().unwrap_or_default().to_string(),
        ))
        .unwrap_or_default();
    let subject = parsed.as_ref().and_then(|m| m.subject()).unwrap_or_default().to_string();
    let body = parsed.as_ref().and_then(|m| m.body_text(0)).map(|c| c.to_string()).unwrap_or_default();
    InboundEmail { from_name, from_email, subject, body }
}

/// Real IMAP poller: connect over TLS, SELECT INBOX, SEARCH UNSEEN, FETCH,
/// parse, then mark the fetched messages `\Seen`.
#[cfg(feature = "imap")]
pub struct AsyncImapReceiver;

#[cfg(feature = "imap")]
#[async_trait::async_trait]
impl MailReceiver for AsyncImapReceiver {
    async fn fetch_new(&self, creds: &ImapCredentials) -> Result<Vec<InboundEmail>, OpenCompanyError> {
        use futures::TryStreamExt;
        let stream = async_imap::connect_tls((creds.host.as_str(), creds.port))
            .await
            .map_err(|e| OpenCompanyError::Store(format!("imap connect: {e}")))?;
        let mut session = stream
            .login(&creds.username, &creds.password)
            .await
            .map_err(|(e, _)| OpenCompanyError::Store(format!("imap login: {e}")))?;
        session.select("INBOX").await.map_err(|e| OpenCompanyError::Store(format!("imap select: {e}")))?;
        let unseen = session.search("UNSEEN").await.map_err(|e| OpenCompanyError::Store(format!("imap search: {e}")))?;
        let mut out = Vec::new();
        if !unseen.is_empty() {
            let set = unseen.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(",");
            // RFC822 marks \Seen on fetch; that's the intended dedup (Decision 5).
            let mut fetches = session.fetch(&set, "RFC822").await
                .map_err(|e| OpenCompanyError::Store(format!("imap fetch: {e}")))?;
            while let Some(f) = fetches.try_next().await
                .map_err(|e| OpenCompanyError::Store(format!("imap fetch stream: {e}")))? {
                if let Some(body) = f.body() { out.push(parse_message(body)); }
            }
        }
        let _ = session.logout().await;
        Ok(out)
    }
}
```

Add `futures` if not already a dep (check `Cargo.toml`; the crate already uses `BoxStream` per the channel port, so `futures` is present — confirm and only add under the `imap` feature if missing).

- [ ] **Step 3: Run the parse test + both build states**

Run: `cargo test --features imap parse_message` (PASS) and `cargo build --all-targets` (default, no imap) and `cargo build --all-targets --features imap`, then `cargo clippy --all-targets --features imap -- -D warnings` and `cargo fmt --all -- --check`.
Expected: all PASS. (The live `fetch_new` path is validated by the poller's mock tests in Task 6 + a manual/integration send later; there is no offline IMAP server here.)

- [ ] **Step 4: Commit**

```bash
git add src/server/ops/imap.rs Cargo.toml Cargo.lock
git commit -m "feat(email): AsyncImapReceiver + RFC822 parse (imap feature)"
```
