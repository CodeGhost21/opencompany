//! The guarantee, tested: no analytics payload can carry caller-supplied text.
//!
//! These run at **default features**, in the build every lane compiles, because
//! the leak they guard against would not be introduced in the gated transport —
//! it would be introduced at a call site, in a payload field, on any build.

use super::*;
use crate::analytics::types::{OpaqueId, provider_slug, sample_kind_slug};
use crate::app::deployment::Deployment;
use crate::error::OpenCompanyError;
use crate::ports::brain::{Cognition, UsageMetering};
use crate::ports::usage::{SampleKind, UsageSample};

/// Strings that must never appear in a payload, standing in for the four kinds
/// of content #1739 names: a customer's brand, an operator's own text, a host
/// path, and an address.
const HOSTILE: &[&str] = &[
    "AcmeCorp Holdings",
    "please summarise the merger memo",
    "/Users/someone/companies/acme/secrets",
    "founder@acme.example",
    "sk-not-a-real-key",
    "project-titan",
];

fn envelope() -> Envelope {
    Envelope::new(
        OpaqueId::instance("0123456789abcdef0123456789abcdef"),
        Deployment::HostedTenant,
        Cognition {
            path: "harness",
            provider: "openrouter",
            metering: UsageMetering::PerTurn,
        },
    )
}

/// Every event kind, each built from the most hostile input its constructor
/// will accept.
fn hostile_events() -> Vec<Event> {
    let sample = UsageSample {
        at_millis: 1,
        agent: "AcmeCorp Holdings".into(),
        provider: "mcp:project-titan".into(),
        input_tokens: 10,
        output_tokens: 4,
        cached_input_tokens: 0,
        cost_usd: 0.25,
        kind: SampleKind::Inference,
        run_id: Some("/Users/someone/companies/acme/secrets".into()),
    };

    let err = OpenCompanyError::Store(
        "could not write /Users/someone/companies/acme/secrets for founder@acme.example".into(),
    );

    // A second sample whose provider is neither MCP-prefixed nor a known slug:
    // it must reach the `other` fallback rather than the `mcp` branch. Without
    // it the whole fallback path went untested by the two assertions below —
    // found by mutating `provider_slug`'s `_` arm into a `Box::leak`, which the
    // MCP-prefixed sample alone did not notice.
    let unknown_provider = UsageSample {
        provider: "AcmeCorp Holdings".into(),
        kind: SampleKind::OauthCall,
        ..sample.clone()
    };

    vec![
        Event::InstanceStarted {
            companies: 3,
            storage: "mongodb",
            setup_complete: true,
        },
        Event::TurnFinished {
            trigger: Trigger::OperatorMessage,
            outcome: Outcome::Failed,
            failure: Some(FailureCode::of(&err)),
            duration_ms: 1_234,
            effects_executed: 2,
            approvals_parked: 1,
        },
        Event::metered(&sample),
        Event::metered(&unknown_provider),
    ]
}

/// **Issue #1739's third acceptance criterion**, from the outside: render every
/// event from hostile inputs and assert none of that text survives.
///
/// It is the blunt half of the guarantee. The structural half is that
/// [`PropValue`] has no `String` variant at all — but a blunt test is what
/// fails loudly if someone adds one, so both are here.
#[test]
fn a_payload_carries_no_caller_supplied_text() {
    let envelope = envelope();
    for event in hostile_events() {
        // Case-insensitively: a classifier that lowercases what it passes
        // through has still passed it through, and an exact-case search would
        // read that as clean. Found by mutation — the first version of this
        // test missed exactly that.
        let rendered = payload(&envelope, &event).to_string().to_ascii_lowercase();
        for needle in HOSTILE {
            assert!(
                !rendered.contains(&needle.to_ascii_lowercase()),
                "the {} payload leaked {needle:?}: {rendered}",
                event.name()
            );
        }
    }
}

/// Every literal any classifier or enum in this module can produce.
///
/// The vocabulary is enumerated by hand **on purpose**: it is the list a
/// reviewer reads to answer "what can this product send?", and a list derived
/// from the code would answer that question with itself.
fn vocabulary() -> Vec<&'static str> {
    let mut words = vec![
        // Deployment kinds.
        "desktop",
        "self-hosted",
        "hosted-tenant",
        // Outcomes and triggers.
        "ok",
        "failed",
        "operator-message",
        "task-dispatch",
        "approval-continuation",
        "agent-reply",
        // Failures.
        "none",
        "store",
        "manifest",
        "refused",
        "not-found",
        "cognition",
        "workflow",
        "config",
        "upstream",
        // Metering.
        "per-turn",
        "per-cycle",
        // Cognition paths (`ports::brain::Cognition::path`).
        "harness",
        "hosted",
        "echo",
        "sidecar",
        "custom",
        // Storage kinds.
        "fs",
        "sqlite",
        "mongodb",
        // The catch-all.
        types::OTHER,
    ];
    // Provider slugs and sample kinds, taken from the classifiers themselves so
    // a new one cannot be added without appearing here.
    for provider in [
        "openrouter",
        "subscription",
        "managed",
        "ollama",
        "byok",
        "openai_compatible",
        "echo",
        "hosted",
        "github",
        "google",
        "slack",
        "notion",
        "composio",
        "unknown",
        "mcp:anything",
        "something nobody anticipated",
    ] {
        words.push(provider_slug(provider));
    }
    for kind in [
        SampleKind::Inference,
        SampleKind::OauthCall,
        SampleKind::SearchCall,
        SampleKind::PlanningCall,
        SampleKind::TriageCall,
        SampleKind::SetupCall,
    ] {
        words.push(sample_kind_slug(kind));
    }
    words
}

/// The structural claim, asserted rather than described: **every string value in
/// every payload is either the opaque identity, a platform fact fixed at compile
/// time, or a word from the vocabulary above.**
///
/// This is the test that fails the moment someone adds a `String`-carrying
/// property. A free-form value has, by definition, no entry in a hand-written
/// list.
#[test]
fn every_string_in_a_payload_comes_from_the_compiled_vocabulary() {
    let envelope = envelope();
    let vocabulary = vocabulary();

    // The three values that are strings but are not vocabulary: the opaque id,
    // and the two platform facts `std::env::consts` supplies. All three are
    // fixed for the life of the process and none originates with a user.
    let allowed_platform = [
        envelope.id.as_str().to_string(),
        envelope.app_version.to_string(),
        envelope.os.to_string(),
        envelope.arch.to_string(),
    ];

    for event in hostile_events() {
        let rendered = payload(&envelope, &event);
        assert_eq!(rendered["event"], event.name());

        let properties = rendered["properties"]
            .as_object()
            .expect("properties is an object");
        assert!(!properties.is_empty(), "an event with no properties");

        for (key, value) in properties {
            let Some(text) = value.as_str() else { continue };
            let known = vocabulary.contains(&text)
                || allowed_platform.iter().any(|allowed| allowed == text);
            assert!(
                known,
                "the property {key:?} carried the string {text:?}, which is not in this \
                 module's compiled vocabulary. Either it is a leak, or a new literal was \
                 added without recording it in `vocabulary()`."
            );
        }
    }
}

/// A tenant slug is a customer's brand. It is hashed, and the test is that the
/// slug cannot be read back out of the id.
#[test]
fn a_tenant_slug_is_hashed_rather_than_carried() {
    let id = OpaqueId::tenant("acmecorp-holdings");
    assert!(!id.as_str().contains("acme"), "{id:?}");
    assert!(
        id.as_str().starts_with("t_"),
        "tenant ids are namespaced apart from instance ids: {id:?}"
    );
    assert_eq!(
        id.as_str(),
        OpaqueId::tenant("acmecorp-holdings").as_str(),
        "the same tenant must map to the same id on every boot, or uniques and \
         funnels mean nothing"
    );
    assert_ne!(
        id.as_str(),
        OpaqueId::tenant("acmecorp-holdings-2").as_str()
    );
}

/// `Display` on this crate's error type embeds absolute paths, company ids, tool
/// names and agent text — it is the richest source of user content in the tree.
/// A failure property is the coarse class and nothing else.
#[test]
fn an_error_reaches_a_payload_only_as_a_coarse_class() {
    let err =
        OpenCompanyError::Store("could not write /Users/someone/companies/acme/secrets".into());
    assert!(
        err.to_string().contains("/Users/someone"),
        "the premise of this test: Display really does carry the path"
    );
    assert_eq!(FailureCode::of(&err), FailureCode::Store);
    assert_eq!(FailureCode::of(&err).as_str(), "store");

    // And an upstream's own code is folded to the family, never carried.
    let upstream = OpenCompanyError::Chargebee {
        status: 404,
        code: "customer_acme_holdings_not_found".into(),
        message: "nope".into(),
    };
    assert_eq!(FailureCode::of(&upstream), FailureCode::Upstream);
    assert!(!FailureCode::of(&upstream).as_str().contains("acme"));
}

/// An unrecognised provider is folded to `other`, not passed through. This is
/// the direction that matters: the leak would arrive as a value nobody
/// anticipated, which is the only way such a leak ever arrives.
#[test]
fn an_unknown_provider_folds_to_other() {
    assert_eq!(provider_slug("mcp:acme-internal-crm"), "mcp");
    assert_eq!(provider_slug("acme-internal-crm"), types::OTHER);
    assert_eq!(provider_slug("OpenRouter"), "openrouter");
}

/// The default tracker in every build sends nothing and records nothing.
#[tokio::test]
async fn the_null_tracker_is_a_no_op() {
    let tracker = null_tracker();
    tracker.track(Event::InstanceStarted {
        companies: 1,
        storage: "fs",
        setup_complete: false,
    });
    tracker.flush().await;
}

/// A default build resolves to silence for the two deployments that must never
/// report, whatever else is configured. The transport-level proof is in
/// `mixpanel.rs`; this is the same decision asserted where every lane runs it.
#[test]
fn the_default_build_chooses_silence_for_desktop_and_self_hosted() {
    use crate::analytics::config::{Silence, TOKEN_ENV};
    use crate::app::config::MapEnv;

    let env = MapEnv::new([(TOKEN_ENV, "not-a-real-token")]);
    for deployment in [Deployment::Desktop, Deployment::SelfHosted] {
        assert_eq!(
            resolve(deployment, &env),
            Decision::Silent(Silence::NotHosted),
            "{deployment:?} must be silent"
        );
    }
}

/// **The deferred handle holds what it is given before installation and replays
/// it, in order, when the real tracker arrives.**
///
/// It used to drop those events, on the reasoning that the pre-install window
/// at boot contains nothing. It does not: `CompanyScheduler::spawn` runs its
/// restart catch-up immediately, so a company with a cron occurrence missed
/// during downtime finishes a real cycle inside that window, and its
/// `turn_finished` and `turn_metered` went nowhere.
#[tokio::test]
async fn a_deferred_tracker_holds_before_install_and_forwards_after() {
    let event = |companies| Event::InstanceStarted {
        companies,
        storage: "fs",
        setup_complete: false,
    };

    let deferred = DeferredTracker::new();
    deferred.track(event(1));
    deferred.track(event(2));

    let recorder = std::sync::Arc::new(RecordingTracker::new());
    assert!(deferred.install(recorder.clone()));
    deferred.track(event(3));
    deferred.flush().await;

    assert_eq!(
        recorder.events(),
        vec![event(1), event(2), event(3)],
        "both held events arrive, in order, ahead of the one tracked after"
    );
    assert_eq!(recorder.flushes(), 1);

    // A second install is refused rather than splitting the stream in two.
    let second = std::sync::Arc::new(RecordingTracker::new());
    assert!(!deferred.install(second.clone()));
    deferred.track(event(4));
    assert!(second.events().is_empty());
    assert_eq!(recorder.events().len(), 4);
}

/// The held buffer is bounded. A handle nobody ever installs — every embedder
/// that wires no analytics — must not grow without limit, so the oldest are
/// dropped rather than the process.
#[tokio::test]
async fn the_held_buffer_is_bounded() {
    let event = |companies| Event::InstanceStarted {
        companies,
        storage: "fs",
        setup_complete: false,
    };

    let deferred = DeferredTracker::new();
    for n in 0..5_000u64 {
        deferred.track(event(n));
    }

    let recorder = std::sync::Arc::new(RecordingTracker::new());
    assert!(deferred.install(recorder.clone()));

    let held = recorder.events();
    assert!(
        held.len() < 5_000,
        "the buffer must be bounded, not unbounded: {}",
        held.len()
    );
    assert!(!held.is_empty(), "and it must not be zero either");
    assert_eq!(
        held.last(),
        Some(&event(4_999)),
        "the newest survives; it is the oldest that is dropped"
    );
}
