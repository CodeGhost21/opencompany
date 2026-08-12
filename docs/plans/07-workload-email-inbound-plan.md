# WS7 Inbound — IMAP Receive: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add IMAP-poll receiving so a running company reads its own `<slug>@opencompany.work` mailbox and each new message drives an agent cycle — reusing the existing `InboxStore` + `WebhookReceived` path.

**Architecture:** A `MailReceiver` trait (mock + a feature-gated `async-imap` impl), a `MailboxPoller` structured like the existing `CompanyScheduler`, and a shared `file_and_notify` helper factored out of the existing webhook `ingest()`. Outbound send is a separate plan (deferred pending the openhuman effect-model spike).

**Tech Stack:** Rust 2024, `async_trait`, `tokio`, `async-imap` + `mail-parser` (new, behind a new `imap` Cargo feature), the crate error type `OpenCompanyError`.

**Design source:** [`07-workload-email-send-receive.md`](07-workload-email-send-receive.md) — this plan implements its **inbound** half only.

## Global Constraints

- Before **every** commit: `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo build --all-targets`, `cargo test` must pass (workload `CLAUDE.md`). **Toolchain note:** this machine's default `stable` is 1.93.0 and may fail to build a transitive dep (`cfg_select`); if a plain `cargo build` fails on a dependency, use `cargo +1.96.1 …` (installed) as the manager side did. Confirm which toolchain builds cleanly before Task 1 and use it throughout.
- All async traits use the external **`#[async_trait]`** macro (never native `async fn` in trait) — every port here is stored as `Arc<dyn …>`.
- Errors use `OpenCompanyError` variants (`Config(String)`, `Store(String)`, `InvalidRequest(String)`, `Serde(#[from])`). No `anyhow`.
- **Feature-gate the network crate:** `async-imap` + `mail-parser` link only under a new `imap` feature (mirroring how `lettre` is gated behind `smtp`). The `MailReceiver` trait, `InboundEmail`, `ImapCredentials`, `RecordingMailReceiver`, `MailboxPoller`, and the shared helper all compile in the default build; only `AsyncImapReceiver` + the RFC822 parse fn are `#[cfg(feature = "imap")]`.
- **Secrets:** the IMAP password is a secret — never logged / never `#[derive(Debug)]`-printed in a way that leaks it (follow `SmtpCredentials`' handling; if a `Debug` would print the password, hand-write it like `MailCredentials` does at `mailer.rs:120`).
- Reuse, don't reinvent: `EmailRecord`/`InboxStore` (`src/ports/inbox.rs`), `CompanyEvent::WebhookReceived` (`src/ports/types.rs:233`), `Clock`/spawn pattern (`src/runtime/scheduler.rs`), `local_part` (`src/server/ops/smtp.rs:249`).

## File Structure

- Modify: `Cargo.toml` — new `imap` feature + optional deps.
- Modify: `src/server/ops/mailer.rs` — `InboundEmail`, `MailReceiver` trait, `RecordingMailReceiver`, `TenantMailboxConfig` + `from_env`.
- Create: `src/server/ops/imap.rs` — `ImapCredentials` (always compiled) + `AsyncImapReceiver` + `parse_message` (feature-gated).
- Modify: `src/server/ops/mod.rs` — declare `pub mod imap;` and re-exports.
- Modify: `src/server/ops/inbox.rs` — extract `file_and_notify`; `ingest()` calls it.
- Create: `src/runtime/mailbox_poller.rs` — `MailboxPoller`.
- Modify: `src/runtime/mod.rs` — `pub mod mailbox_poller;`.
- Modify: `src/bin/opencompany.rs` — construct config + receiver, spawn a poller per company.

---

### Task 5: Shared `file_and_notify` helper

**Files:** Modify `src/server/ops/inbox.rs`

**Interfaces:** Produces `pub(crate) async fn file_and_notify(runtime: &CompanyRuntime, to: &str, record: EmailRecord) -> Result<()>` — appends the record then, if running, fires `WebhookReceived{channel:"email"}`. `ingest()` is refactored to call it (behavior-preserving).

- [ ] **Step 1: Extract the helper** (the tail of `ingest()` at `inbox.rs:139-159`):

```rust
/// File an inbound email and, if the company is running, drive one cycle so the
/// addressed teammate can act on it. Shared by the ingest webhook and the IMAP
/// poller. `to` is the full recipient address (for the event body); the record
/// already carries the local-part `inbox`.
pub(crate) async fn file_and_notify(
    runtime: &CompanyRuntime,
    to: &str,
    record: EmailRecord,
) -> crate::Result<()> {
    runtime.inbox().append(runtime.id(), &record).await?;
    if runtime.ensure_running().await.is_ok() {
        let event = CompanyEvent::WebhookReceived {
            channel: "email".to_string(),
            body: serde_json::json!({
                "from": record.from_email,
                "to": to,
                "inbox": record.inbox,
                "subject": record.subject,
                "body": record.body,
            }),
        };
        if let Err(err) = runtime.run_cycle(vec![event]).await {
            tracing::warn!(company = %runtime.id(), "email cycle failed: {err}");
        }
    }
    Ok(())
}
```

- [ ] **Step 2: Rewrite `ingest()`'s tail** to build the `EmailRecord` then call the helper, preserving the current behavior (return `ApiError` if `file_and_notify` errors on append):

```rust
    let record = EmailRecord {
        id: generate_id(),
        inbox: inbox.clone(),
        from_name: String::new(),
        from_email: email.from.clone(),
        subject: email.subject.clone(),
        body: email.body.clone(),
        at_millis: now_millis(),
        read: false,
        outbound: false,
    };
    if let Err(err) = file_and_notify(&runtime, &email.to, record).await {
        return crate::server::error::ApiError(err).into_response();
    }
    (StatusCode::ACCEPTED, Json(IngestAck { ok: true, inbox })).into_response()
```

- [ ] **Step 3: Run the existing inbox tests** (they must still pass — behavior preserved)

Run: `cargo test inbox:: && cargo clippy --all-targets -- -D warnings && cargo fmt --all -- --check`
Expected: PASS (existing ingest tests unchanged in behavior).

- [ ] **Step 4: Commit**

```bash
git add src/server/ops/inbox.rs
git commit -m "refactor(email): extract file_and_notify shared by ingest + poller"
```

---

### Task 6: `MailboxPoller`

**Files:** Create `src/runtime/mailbox_poller.rs`; Modify `src/runtime/mod.rs`

**Interfaces:**
- Consumes: `MailReceiver`, `ImapCredentials`, `file_and_notify`, `Clock` (`scheduler.rs`), `CompanyRuntime`.
- Produces: `MailboxPoller::new(runtime, receiver, creds, address, interval_secs)`, `async fn tick(&self) -> Result<usize>`, `fn spawn(self, shutdown: Arc<Notify>) -> JoinHandle<()>`.

- [ ] **Step 1: Write the failing test** — one tick with a mock receiver files N records + fires N cycles:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    // Build a CompanyRuntime in the "running" state via the existing test
    // harness/builder used by scheduler.rs tests (mirror its setup), inject a
    // RecordingMailReceiver with one queued batch of 2 messages.
    #[tokio::test]
    async fn tick_files_and_notifies_each_message() {
        let runtime = /* test CompanyRuntime, running */;
        let rx = std::sync::Arc::new(RecordingMailReceiver::new());
        rx.push_batch(vec![
            InboundEmail { from_name: "A".into(), from_email: "a@x".into(), subject: "s1".into(), body: "b1".into() },
            InboundEmail { from_name: "B".into(), from_email: "b@x".into(), subject: "s2".into(), body: "b2".into() },
        ]);
        let creds = ImapCredentials { host: "h".into(), port: 993, username: "u".into(), password: "p".into() };
        let poller = MailboxPoller::new(runtime.clone(), rx.clone(), creds, "acme@opencompany.work".into(), 60);
        let n = poller.tick().await.unwrap();
        assert_eq!(n, 2);
        // both filed to the inbox
        assert_eq!(runtime.inbox().messages(runtime.id(), "acme", 10, 0).await.unwrap().len(), 2);
    }
}
```

(Model the `CompanyRuntime` construction on `scheduler.rs`'s own tests — reuse the same in-memory builder + a `"running"` lifecycle.)

Run: `cargo test mailbox_poller` → FAIL.

- [ ] **Step 2: Implement `MailboxPoller`**:

```rust
//! Per-company IMAP poller. Structured like `CompanyScheduler`: an injectable
//! interval loop that, per tick, fetches new mail via a `MailReceiver` and files
//! it through the shared `file_and_notify`. Skips while the company is asleep
//! (scale-to-zero: unseen mail waits in Stalwart and is picked up on wake).
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::company::runtime::CompanyRuntime;
use crate::server::ops::imap::ImapCredentials;
use crate::server::ops::inbox::file_and_notify;
use crate::server::ops::mailer::MailReceiver;
use crate::ports::inbox::EmailRecord;
use crate::{now_millis, generate_id}; // adjust import paths to the crate's helpers
use crate::server::ops::smtp::local_part;

pub struct MailboxPoller {
    runtime: Arc<CompanyRuntime>,
    receiver: Arc<dyn MailReceiver>,
    creds: ImapCredentials,
    address: String,
    interval: Duration,
}

impl MailboxPoller {
    pub fn new(
        runtime: Arc<CompanyRuntime>,
        receiver: Arc<dyn MailReceiver>,
        creds: ImapCredentials,
        address: String,
        interval_secs: u64,
    ) -> Self {
        Self { runtime, receiver, creds, address, interval: Duration::from_secs(interval_secs.max(1)) }
    }

    /// Fetch new mail and file each message. Returns the count filed. Skips (0)
    /// when the company is not running.
    pub async fn tick(&self) -> crate::Result<usize> {
        if self.runtime.ensure_running().await.is_err() {
            return Ok(0);
        }
        let messages = self.receiver.fetch_new(&self.creds).await?;
        let filed = messages.len();
        for m in messages {
            let record = EmailRecord {
                id: generate_id(),
                inbox: local_part(&self.address),
                from_name: m.from_name,
                from_email: m.from_email,
                subject: m.subject,
                body: m.body,
                at_millis: now_millis(),
                read: false,
                outbound: false,
            };
            file_and_notify(&self.runtime, &self.address, record).await?;
        }
        Ok(filed)
    }

    /// Spawn the interval loop; stops on `shutdown`.
    pub fn spawn(self, shutdown: Arc<Notify>) -> JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown.notified() => break,
                    _ = tokio::time::sleep(self.interval) => {
                        if let Err(err) = self.tick().await {
                            tracing::warn!(company = %self.runtime.id(), %err, "mailbox poll failed");
                        }
                    }
                }
            }
        })
    }
}
```

Adjust the exact import paths for `now_millis`/`generate_id`/`local_part` to wherever the crate exposes them (grep: `fn now_millis`, `fn generate_id`). Make `local_part` reachable (it's `pub(crate)` in `smtp.rs`).

- [ ] **Step 3: Register + run**

Add `pub mod mailbox_poller;` to `src/runtime/mod.rs`.
Run: `cargo test mailbox_poller && cargo clippy --all-targets -- -D warnings && cargo fmt --all -- --check`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/runtime/mailbox_poller.rs src/runtime/mod.rs
git commit -m "feat(email): MailboxPoller (IMAP poll -> file_and_notify)"
```

---

### Task 7: Wire the poller into the serve command

**Files:** Modify `src/bin/opencompany.rs`

**Interfaces:** Consumes `TenantMailboxConfig::from_env`, `AsyncImapReceiver`/`MailReceiver`, `MailboxPoller`. Starts one poller per running company under the shared `shutdown`.

- [ ] **Step 1: Add a start helper** near `spawn_scheduler` (`:209`):

```rust
fn spawn_mailbox_poller(
    state: &AppState,
    id: &str,
    shutdown: &Arc<Notify>,
    handles: &mut Vec<JoinHandle<()>>,
) {
    // Only when the manager injected this tenant's mailbox creds, and only when
    // the imap transport is compiled in.
    let cfg = match opencompany::server::ops::mailer::TenantMailboxConfig::from_env() {
        Ok(Some(cfg)) => cfg,
        Ok(None) => return,
        Err(err) => { eprintln!("mailbox config error: {err}"); return; }
    };
    #[cfg(feature = "imap")]
    {
        let Some(runtime) = state.registry().get(&CompanyId::new(id)) else { return };
        let receiver: Arc<dyn opencompany::server::ops::mailer::MailReceiver> =
            Arc::new(opencompany::server::ops::imap::AsyncImapReceiver);
        let interval = std::env::var("OPENCOMPANY_MAIL_POLL_SECONDS")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(60);
        let poller = opencompany::runtime::mailbox_poller::MailboxPoller::new(
            runtime, receiver, cfg.imap.clone(), cfg.address.clone(), interval);
        handles.push(poller.spawn(shutdown.clone()));
    }
    #[cfg(not(feature = "imap"))]
    { let _ = (state, id, shutdown, handles, cfg); }
}
```

- [ ] **Step 2: Call it** in the `for dir in &companies` loop right after `spawn_scheduler(&state, &id, &schedules, &shutdown)` (`:552-574`):

```rust
        spawn_scheduler(&state, &id, &schedules, &shutdown);
        spawn_mailbox_poller(&state, &id, &shutdown, &mut scheduler_handles);
```

- [ ] **Step 3: Build both feature states + full suite**

Run: `cargo build --all-targets` and `cargo build --all-targets --features imap` and `cargo test` and `cargo clippy --all-targets --features imap -- -D warnings` and `cargo fmt --all -- --check`.
Expected: all PASS.

- [ ] **Step 4: Commit**

```bash
git add src/bin/opencompany.rs
git commit -m "feat(email): start a per-company IMAP poller in serve"
```

---

## Post-implementation (not code tasks)

- **Live smoke test** (with `--features imap` + a real tenant, `OPENCOMPANY_MAIL_*` injected): send mail to `<slug>@opencompany.work`; confirm the poller files an `EmailRecord` and drives a cycle.
- **Outbound** (`send_email`) is the separate deferred half — pending the `vendor/openhuman` effect-model spike; it will reuse `MailSender`/`SmtpCredentials`/`file_and_notify`-adjacent `record_outbound` and `TenantMailboxConfig.smtp`.
- **Docs:** update `docs/spec/runtime/ports.md` / `docs/modules/` to note the IMAP receive path alongside the webhook ingest.
