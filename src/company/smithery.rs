//! The company's own Smithery directory credential (issue #1287) — the key that
//! decides whether the MCP directory has anything to show.
//!
//! ## Why a credential decides whether a search returns rows
//!
//! Two upstream directories back the browse surface shipped in #1270. The
//! official `modelcontextprotocol/registry` is always queried, and most of its
//! entries declare no remote endpoint, so the hosted-transport filter discards
//! them — correctly, since this deployment cannot launch a local subprocess.
//! Smithery.ai is the directory that actually carries hosted servers, and
//! upstream's `enabled_registries` adds it **only when a key resolves**. With no
//! key the browse surface works perfectly and has almost nothing to show, which
//! reads on screen as a broken search rather than a missing credential.
//!
//! ## Scope: discovery, not connection
//!
//! This key authenticates the *directory* calls — search, entry lookup, and the
//! fetch an install performs. It is **not** what an installed server connects
//! with: `registry::connections::connect` dials the stored `deployment_url` and
//! builds its auth from that server's own stored env row (the headers declared
//! by the entry, or a captured OAuth token). So clearing this key stops new
//! discovery; it does not break servers already installed through it. Worth
//! stating because the opposite is the intuitive guess, and it would make the
//! clear action look far more destructive than it is.
//!
//! ## Precedence, and why the two tiers are reported apart
//!
//! 1. **The company's own key**, stored here under [`API_KEY_KEY`] — set,
//!    rotated and cleared by an admin through the console, write-only.
//! 2. **[`API_KEY_ENV`]** on the host process — the self-hosting escape hatch,
//!    and upstream's own fallback.
//! 3. Nothing, in which case Smithery is not queried at all.
//!
//! Tier 2 is deliberately surfaced as its own [`DirectoryKeySource`] rather than
//! folded into a `configured: true` boolean. A host process serves **every**
//! company on it, so an env key is one Smithery account shared by all of them —
//! a materially different answer from "this company has its own", and the
//! console has to be able to say which. Collapsing the two is precisely the
//! failure issue #886 was filed about on Composio, where the panel reported no
//! credential while agents were making authenticated calls in the same session.
//!
//! ## Write-only, and read live
//!
//! The value never leaves over any read route — [`resolve`] is called by the
//! routes that *use* it, and the status shape carries only a tier name. It is
//! read from the [`SecretStore`] on every directory call rather than cached at
//! boot, so a console set / rotate / clear takes effect on the next search with
//! no restart.

use crate::Result;
use crate::app::config::EnvSource;
use crate::ports::SecretStore;
use crate::ports::types::{CompanyId, SecretValue};

/// The canonical per-company Smithery credential key. Write-only via the
/// console; the value is the raw API key string.
///
/// Namespaced `smithery/` for the same reason
/// [`company_key::KEY_KEY`](crate::company::company_key::KEY_KEY) is namespaced
/// `tinyhumans/`: it is one vendor's credential, not "the MCP key", and a second
/// directory arriving later must not have to share this slot.
pub const API_KEY_KEY: &str = "smithery/api-key";

/// The host-wide Smithery key environment variable. Upstream's own fallback
/// (`registries::smithery::smithery_api_key` reads it directly), kept here so
/// this module can *report* the tier rather than let it resolve invisibly one
/// layer down.
pub const API_KEY_ENV: &str = "SMITHERY_API_KEY";

/// Which Smithery credential this company's directory calls present.
///
/// The wire spelling is the enum name lowercased and is part of the console
/// contract — the browse surface branches on it to tell "no key, so the hosted
/// directory is off" apart from "searched fine, nothing matched".
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DirectoryKeySource {
    /// This company's own key. What setting one buys.
    Company,
    /// The host process's [`API_KEY_ENV`], shared by every company on this
    /// instance. Works, and is honest about being one shared account.
    Environment,
    /// No key resolves, so Smithery is not queried and the directory is limited
    /// to the official registry's hosted entries.
    None,
}

impl DirectoryKeySource {
    /// The stable wire spelling (`company` / `environment` / `none`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Company => "company",
            Self::Environment => "environment",
            Self::None => "none",
        }
    }

    /// Whether *some* key resolves, whichever tier answered.
    pub fn configured(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// The resolved directory credential: the value to hand upstream, and the tier
/// that produced it.
///
/// The value is `Some` exactly when [`source`](Self::source) is not
/// [`DirectoryKeySource::None`] — the two cannot disagree, because both come out
/// of the same match in [`resolve`].
#[derive(Debug, Clone)]
pub struct DirectoryKey {
    value: Option<String>,
    source: DirectoryKeySource,
}

impl DirectoryKey {
    /// The tier that answered. Safe on any read plane — a name, never a secret.
    pub fn source(&self) -> DirectoryKeySource {
        self.source
    }

    /// The key itself, for handing to the upstream registry config. **Never**
    /// serialise this: it is the credential.
    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }

    /// Consumes into the owned key, for the upstream `Config` field.
    pub fn into_value(self) -> Option<String> {
        self.value
    }
}

/// Store (or rotate / clear) the company's Smithery key. A non-empty value sets
/// or rotates it; an empty string clears it, falling back to whatever the host
/// environment offers. Write-only — never read back over the API.
pub async fn store_key(company: &CompanyId, secrets: &dyn SecretStore, key: &str) -> Result<()> {
    secrets
        .set(company, API_KEY_KEY, SecretValue(key.trim().to_string()))
        .await
}

/// Whether a non-empty key is stored in **this company's own** slot — never the
/// key itself, and never the environment tier.
///
/// This answers "did this company set one", not "will the directory work". For
/// the latter ask [`resolve`] and read its [`DirectoryKeySource`]; a host with
/// [`API_KEY_ENV`] set has a working directory and `false` here.
pub async fn key_configured(company: &CompanyId, secrets: &dyn SecretStore) -> Result<bool> {
    Ok(company_key(company, secrets).await?.is_some())
}

/// This company's own stored key, trimmed, `None` when unset or blank.
async fn company_key(company: &CompanyId, secrets: &dyn SecretStore) -> Result<Option<String>> {
    Ok(secrets
        .get(company, API_KEY_KEY)
        .await?
        .map(|SecretValue(key)| key.trim().to_string())
        .filter(|key| !key.is_empty()))
}

/// The Smithery credential this company's directory calls present — **the** seam.
///
/// The company's own key wins; failing that the host's [`API_KEY_ENV`]; failing
/// that nothing. Every directory call resolves through here so the tier the
/// console reports cannot drift from the key the search actually sent.
///
/// ## Why a store error propagates
///
/// Mapping an unreadable store to "no key" would silently downgrade a company
/// that **has** its own key onto the host's shared one — a different Smithery
/// account, and the same class of misattribution
/// [`company_key::resolve`](crate::company::company_key::resolve) refuses for
/// the same reason. A failed read is not an answer.
/// `env` is `+ Sync` because this future is held across an await inside an axum
/// handler, and a bare `&dyn EnvSource` would make it `!Send`. Every real
/// implementor already satisfies it.
pub async fn resolve(
    company: &CompanyId,
    secrets: &dyn SecretStore,
    env: &(dyn EnvSource + Sync),
) -> Result<DirectoryKey> {
    if let Some(key) = company_key(company, secrets).await? {
        return Ok(DirectoryKey {
            value: Some(key),
            source: DirectoryKeySource::Company,
        });
    }
    Ok(match env.get(API_KEY_ENV).map(|k| k.trim().to_string()) {
        Some(key) if !key.is_empty() => DirectoryKey {
            value: Some(key),
            source: DirectoryKeySource::Environment,
        },
        _ => DirectoryKey {
            value: None,
            source: DirectoryKeySource::None,
        },
    })
}

#[cfg(test)]
mod test;
