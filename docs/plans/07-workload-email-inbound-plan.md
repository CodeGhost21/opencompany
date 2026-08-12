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

## Post-implementation (not code tasks)

- **Live smoke test** (with `--features imap` + a real tenant, `OPENCOMPANY_MAIL_*` injected): send mail to `<slug>@opencompany.work`; confirm the poller files an `EmailRecord` and drives a cycle.
- **Outbound** (`send_email`) is the separate deferred half — pending the `vendor/openhuman` effect-model spike; it will reuse `MailSender`/`SmtpCredentials`/`file_and_notify`-adjacent `record_outbound` and `TenantMailboxConfig.smtp`.
- **Docs:** update `docs/spec/runtime/ports.md` / `docs/modules/` to note the IMAP receive path alongside the webhook ingest.
