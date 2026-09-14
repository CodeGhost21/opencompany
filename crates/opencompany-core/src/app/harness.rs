//! Attaching the embedded agent harness to a company runtime.
//!
//! Every process that builds a [`RuntimeBuilder`] and expects its companies to
//! be able to reach a model has to make the same four decisions: attach the
//! pool, and then wire whichever of the managed media, search and inference
//! backends the environment supplies. `serve` in `src/bin/opencompany.rs` used
//! to hold the only copy of that sequence, which meant the desktop shell —
//! which links this crate rather than spawning the binary — built companies
//! with no harness at all even in a build that compiled one in.
//!
//! So the sequence lives here, in [`attach`], for the same reason
//! [`prepare_instance`](crate::app::prepare_instance) does: one copy, shared by
//! the command line and by every embedder, rather than two that drift.
//!
//! Without the `openhuman` feature this is the identity function, so a default
//! build is byte-for-byte unaffected.

use crate::runtime::RuntimeBuilder;

/// Attaches the harness pool and every managed backend the environment offers.
///
/// The pool is attached **unconditionally**, so cognition routes through a live
/// company agent whenever *any* inference source is configured — the managed
/// env default (`TINYHUMANS_API_KEY` / `OPENCOMPANY_INFERENCE_*`), a manifest
/// `[inference]` section, or a runtime console override (issue #56 — BYOK).
/// That is what unblocks a BYOK-only tenant with no platform credential: the
/// builder still constructs the harness brain from its manifest/runtime config.
/// With no source at all, the runtime keeps its hosted/echo brain.
///
/// Call this on any builder whose companies should be able to think.
#[cfg(not(feature = "openhuman"))]
pub fn attach(builder: RuntimeBuilder) -> RuntimeBuilder {
    builder
}

#[cfg(feature = "openhuman")]
pub fn attach(builder: RuntimeBuilder) -> RuntimeBuilder {
    use std::sync::Arc;

    use crate::app::config::ProcessEnv;
    use crate::harness::HarnessPool;
    use crate::harness::provider::{
        PlatformCredentialStatus, harness_inference_from_env, media_backend_from_env,
        search_backend_from_env,
    };

    // Issue #879: every managed surface below fails closed and says nothing at
    // boot, so a tenant provisioned without its platform token comes up looking
    // healthy and only reveals the gap when an agent is built or a workflow node
    // 500s. Say it once, here, where an operator reading the first lines of the
    // log will see it.
    if let Some(warning) = PlatformCredentialStatus::resolve(&ProcessEnv).boot_warning() {
        tracing::warn!("[boot] {warning}");
    }

    let builder = builder.with_harness(Arc::new(HarnessPool::new()));
    // Issue #109: the MANAGED media-generation backend, resolved from the
    // environment only (never a tenant secret). Absent ⇒ media tools stay unwired
    // even for a company that grants `media` (fail-closed).
    let builder = match media_backend_from_env(&ProcessEnv) {
        Some(media_backend) => builder.with_media_backend(media_backend),
        None => builder,
    };
    // Issue #238: the MANAGED web-search backend, on the same platform identity
    // as managed inference and resolved from the environment only. Absent ⇒
    // `web_search` stays unwired even for a company that grants `search`.
    let builder = match search_backend_from_env(&ProcessEnv) {
        Some(search_backend) => builder.with_search_backend(search_backend),
        None => builder,
    };
    // The managed env default is an *optional*, lowest-precedence source; a
    // BYOK-only tenant supplies none and still gets a harness brain from its
    // manifest/runtime config.
    match harness_inference_from_env(&ProcessEnv) {
        Some((config, model_override)) => builder.with_harness_inference(config, model_override),
        None => builder,
    }
}
