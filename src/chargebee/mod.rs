//! A first-party Chargebee MCP server (issue #788), so a company agent can
//! create invoices and manage billing from a natural-language instruction.
//!
//! # Why this exists rather than a vendor integration
//!
//! Chargebee publishes MCP servers, and none of them can do this. The npm
//! `@chargebee/mcp` package is deprecated and exposes only documentation search
//! and a code planner — developer tooling, not billing calls — and it is a
//! stdio server besides, which [`crate::company::McpServer`] rejects. The newer
//! hosted Chargebee servers (Knowledge Base, Data Lookup, Onboarding) are
//! HTTP and therefore transport-compatible, but read-only; Chargebee's own
//! documentation lists write actions as forthcoming. Creating an invoice is not
//! available from any of them, so the server is ours.
//!
//! # Shape
//!
//! - [`types`] — configuration and tool arguments. Every money field is named
//!   `*_in_minor_units`, because Chargebee is cents-based and an agent reading
//!   "$500" from a prompt will otherwise write a $5.00 invoice that succeeds.
//! - [`client`] — the Chargebee REST v2 client: form-encoded writes, HTTP Basic
//!   auth, bracket-array nesting.
//! - [`tools`] — the five operations scoped by the issue, validated locally
//!   before any network call.
//! - [`server`] — JSON-RPC over one HTTP route, the whole MCP surface the
//!   OpenCompany client speaks.
//!
//! # Credentials
//!
//! The Chargebee API key is read from `CHARGEBEE_API_KEY` at startup and never
//! leaves this process — it is not a tool argument, not part of any tool result,
//! and not present in the MCP registration OpenCompany holds. That placement is
//! deliberate and load-bearing: it is what lets the server be listed as a
//! `[[default_mcp_server]]`, which rejects any entry naming an `auth_secret`.
//! See [`server::ServerState`] for the inbound half of the same tradeoff.

pub mod client;
pub mod server;
pub mod tools;
pub mod types;

pub use client::ChargebeeClient;
pub use server::{ServerState, router};
pub use types::ChargebeeConfig;
