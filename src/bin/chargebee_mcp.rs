//! The Chargebee MCP server binary (issue #788).
//!
//! Runs standalone rather than inside the tenant container: the tenant workload
//! is an MCP *client*, and OpenCompany's registration for this server is a plain
//! `https://` endpoint. Point a company's `[[default_mcp_server]]` (or a console
//! registration) at wherever this is deployed.
//!
//! ```text
//! CHARGEBEE_SITE=acme-test CHARGEBEE_API_KEY=… chargebee-mcp --bind 127.0.0.1:8790
//! ```

use clap::Parser;
use opencompany::chargebee::{ChargebeeClient, ChargebeeConfig, ServerState, router};

#[derive(Debug, Parser)]
#[command(author, version, about = "Chargebee MCP server for OpenCompany")]
struct Cli {
    /// Address to bind. Falls back to `CHARGEBEE_MCP_BIND`, then 127.0.0.1:8790.
    #[arg(long)]
    bind: Option<String>,
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    let config = match ChargebeeConfig::from_env() {
        Ok(config) => config,
        Err(missing) => {
            // Naming the missing variables is the difference between a one-line
            // fix and a support round trip; the key itself is never echoed.
            eprintln!(
                "chargebee-mcp: missing required environment: {}",
                missing.join(", ")
            );
            return std::process::ExitCode::FAILURE;
        }
    };

    let site = config.site.clone();
    let client = match ChargebeeClient::new(config) {
        Ok(client) => client,
        Err(e) => {
            eprintln!("chargebee-mcp: could not build the HTTP client: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };

    // Optional: unset leaves the server open, which is what a `[[default_mcp_server]]`
    // registration requires (a default may name no `auth_secret`). Set it for any
    // deployment that is not behind a private network boundary.
    let bearer = std::env::var("CHARGEBEE_MCP_BEARER").ok();
    if bearer.as_deref().map(str::trim).is_none_or(str::is_empty) {
        tracing::warn!(
            "CHARGEBEE_MCP_BEARER is unset — this server accepts unauthenticated MCP requests \
             and can create real invoices. Acceptable only behind a private network boundary."
        );
    }

    let bind = cli
        .bind
        .or_else(|| std::env::var("CHARGEBEE_MCP_BIND").ok())
        .unwrap_or_else(|| "127.0.0.1:8790".to_string());

    let listener = match tokio::net::TcpListener::bind(&bind).await {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("chargebee-mcp: could not bind {bind}: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };

    tracing::info!(%bind, %site, "chargebee-mcp listening on /mcp");

    let state = ServerState::new(client, bearer);
    if let Err(e) = axum::serve(listener, router(state)).await {
        eprintln!("chargebee-mcp: server error: {e}");
        return std::process::ExitCode::FAILURE;
    }

    std::process::ExitCode::SUCCESS
}
