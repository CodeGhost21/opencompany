// Issue #1298: a deliberate deferral, not a disagreement with the lint.
//
// `clippy::result_large_err` began firing across this module on rustc 1.98.0
// (released 2026-08-18) with no change to this tree. Every finding names the
// same foreign type — 80 of them on the default feature set, and not one names
// `ApiError` or `OpenCompanyError`, both of which are well under the limit.
// `axum::http::Response<Body>` is exactly 128 bytes, which is exactly clippy's
// default `large-error-threshold`, and handlers here return
// `Result<T, Response>` — the pre-rendered-rejection shape.
//
// The lint is correct. `Result<T, E>` is as large as its larger arm, so those
// 128 bytes are moved on the success path too, on every one of these handlers.
//
// It is allowed rather than fixed because `Response` is axum's type, so there
// is no variant of ours to box. `Result<T, Box<Response>>` does not work
// directly — axum requires the `Err` type to implement `IntoResponse`, which
// `Box<Response>` does not — so the real fix is a newtype or a move onto
// `ApiError`, threaded through 80 signatures in 16 files. That is a reviewable
// refactor of the whole server surface, not a CI unblock.
//
// Tracked in #1299, which also records the per-module plan for narrowing this
// allow back down as handlers are converted. Scoped to `server` on purpose:
// the rest of the crate stays covered by the lint.
#![allow(clippy::result_large_err)]

#[cfg(feature = "tinyplace")]
pub mod a2a;
/// The Agent Client Protocol surface (the `acp` feature).
///
/// The module's own docs reason about "a build without the feature"; this is
/// that feature. `routes::RESERVED_PREFIXES` keeps `/acp` 404ing either way, so
/// a client probing a host without it gets an honest answer rather than the
/// console shell.
#[cfg(feature = "acp")]
pub mod acp;
pub(crate) mod approval_visibility;
pub mod chat_history;
pub mod cors;
mod error;
pub mod feedback;
pub mod feedback_board;
pub mod graphql;
pub mod hooks;
pub mod hooks_chargebee;
pub mod hub_identity;
// Console MCP OAuth callback (issue #90): the unauthenticated browser-redirect
// landing route. Gated on `mcp` (it needs the OAuth token-exchange path).
#[cfg(feature = "mcp")]
pub mod mcp_oauth;
pub mod operator;
pub mod ops;
pub mod platform_auth;
/// Who is here, and who is typing — ephemeral, leased, and never journaled.
/// See [`presence`].
pub mod presence;
pub mod provision;
mod routes;
/// The first-run setup flow: one surface that configures an instance.
pub mod setup;
pub mod shutdown;
pub mod users;

#[cfg(test)]
pub(crate) mod test_support;
pub mod webhook;

pub use error::ApiError;
pub use routes::{Serving, bind, router, serve, serve_on, serve_on_until};
