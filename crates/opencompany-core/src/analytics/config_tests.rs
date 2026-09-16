use super::*;
use crate::app::config::MapEnv;

/// A collector address that resolves nowhere. Every reporting test needs
/// one now: there is no default endpoint left to fall back to.
const TEST_ENDPOINT: &str = "https://collector.invalid/track";

/// A fully configured reporting environment, which `pairs` then overrides.
///
/// It takes three variables where it used to take one, and that is the
/// shape of the change: an OpenPanel deployment configures a client id, a
/// client secret and the address of the collector it self-hosts.
fn configured(pairs: &[(&str, &str)]) -> MapEnv {
    let mut all = vec![
        (CLIENT_ID_ENV, "not-a-real-client-id"),
        (CLIENT_SECRET_ENV, "not-a-real-client-secret"),
        (ENDPOINT_ENV, TEST_ENDPOINT),
    ];
    all.extend_from_slice(pairs);
    MapEnv::new(all)
}

/// **The decision the GPL posture rests on.** A self-hosted instance that
/// has been handed a working credential — which is the easiest way to get
/// this wrong, because a credential looks like consent — still sends
/// nothing.
#[test]
fn a_self_hosted_instance_is_silent_even_with_a_credential() {
    assert_eq!(
        resolve(Deployment::SelfHosted, &configured(&[])),
        Decision::Silent(Silence::NotHosted)
    );
}

#[test]
fn a_desktop_instance_is_silent_even_with_a_credential() {
    assert_eq!(
        resolve(Deployment::Desktop, &configured(&[])),
        Decision::Silent(Silence::NotHosted)
    );
}

#[test]
fn a_hosted_tenant_with_a_credential_reports() {
    let decision = resolve(Deployment::HostedTenant, &configured(&[]));
    assert!(decision.reports(), "{decision:?}");
    match decision {
        Decision::Report {
            endpoint,
            credentials,
        } => {
            assert_eq!(endpoint, TEST_ENDPOINT);
            assert_eq!(credentials.expose_id(), "not-a-real-client-id");
            assert_eq!(credentials.expose_secret(), "not-a-real-client-secret");
        }
        other => panic!("{other:?}"),
    }
}

/// A hosted tenant with nothing configured is misconfigured, not reporting
/// to nowhere — and the reason says so.
#[test]
fn a_hosted_tenant_without_a_credential_is_silent() {
    assert_eq!(
        resolve(Deployment::HostedTenant, &MapEnv::default()),
        Decision::Silent(Silence::NoCredentials)
    );
}

/// **Half a credential is a misconfiguration, and the reason names the half
/// that is missing.**
///
/// OpenPanel authenticates a write client with an id *and* a secret, so
/// there is no useful state in between. This is the shape a half-finished
/// secret rollout has — the id is in the manifest, the secret is still in
/// the vault — and telling that operator "no credential is configured"
/// while `OPENCOMPANY_ANALYTICS_CLIENT_ID` is plainly set in their env file
/// sends them to look at the wrong variable.
#[test]
fn half_a_credential_says_which_half_is_missing() {
    let only_id = MapEnv::new([
        (CLIENT_ID_ENV, "not-a-real-client-id"),
        (ENDPOINT_ENV, TEST_ENDPOINT),
    ]);
    assert_eq!(
        resolve(Deployment::HostedTenant, &only_id),
        Decision::Silent(Silence::NoClientSecret)
    );
    assert!(
        Silence::NoClientSecret
            .as_str()
            .contains("OPENCOMPANY_ANALYTICS_CLIENT_SECRET"),
        "the reason must name the variable to set: {}",
        Silence::NoClientSecret.as_str()
    );

    let only_secret = MapEnv::new([
        (CLIENT_SECRET_ENV, "not-a-real-client-secret"),
        (ENDPOINT_ENV, TEST_ENDPOINT),
    ]);
    assert_eq!(
        resolve(Deployment::HostedTenant, &only_secret),
        Decision::Silent(Silence::NoClientId)
    );
    assert!(
        Silence::NoClientId
            .as_str()
            .contains("OPENCOMPANY_ANALYTICS_CLIENT_ID"),
        "the reason must name the variable to set: {}",
        Silence::NoClientId.as_str()
    );
}

/// Blank is absent for both halves, and for the same reason it is for the
/// switch: a secret mounted from a file arrives with a trailing newline
/// more often than not, and a launcher that exports an empty variable has
/// configured nothing.
#[test]
fn a_blank_half_is_no_half() {
    for blank in ["   ", "\n", "\t\n "] {
        assert_eq!(
            resolve(
                Deployment::HostedTenant,
                &configured(&[(CLIENT_SECRET_ENV, blank)])
            ),
            Decision::Silent(Silence::NoClientSecret),
            "a secret of {blank:?} must not read as configured"
        );
        assert_eq!(
            resolve(
                Deployment::HostedTenant,
                &configured(&[(CLIENT_ID_ENV, blank)])
            ),
            Decision::Silent(Silence::NoClientId),
            "an id of {blank:?} must not read as configured"
        );
    }
}

/// And a credential that merely *arrived* with surrounding whitespace is
/// used, trimmed, rather than put into a header with a newline in it — which
/// `reqwest` rejects outright when it builds the request.
#[test]
fn a_credential_is_trimmed() {
    match resolve(
        Deployment::HostedTenant,
        &configured(&[
            (CLIENT_ID_ENV, "  not-a-real-client-id\n"),
            (CLIENT_SECRET_ENV, "\tnot-a-real-client-secret\n"),
        ]),
    ) {
        Decision::Report { credentials, .. } => {
            assert_eq!(credentials.expose_id(), "not-a-real-client-id");
            assert_eq!(credentials.expose_secret(), "not-a-real-client-secret");
        }
        other => panic!("{other:?}"),
    }
}

/// **A credential that cannot go in a header is silence with a reason.**
///
/// This is new with OpenPanel and is a consequence of where the credential
/// now travels. Mixpanel's token rode in the JSON body, where any string is
/// legal, so a mangled one was simply refused by the collector. These two
/// ride in `openpanel-client-id` / `openpanel-client-secret` headers, and
/// `reqwest` refuses to *build* a request whose header value holds a control
/// byte — so a secret with an embedded newline (`kubectl create secret` over
/// a wrapped file is the usual way one arrives) would install a tracker that
/// never constructs a single request, forever, behind a `debug!` nobody has
/// enabled. Trimming does not save it: the newline is in the middle.
#[test]
fn a_credential_that_cannot_go_in_a_header_is_silence() {
    for mangled in [
        "not-a-real\nclient-secret",
        "not-a-real\rclient-secret",
        "not a real client secret",
        "not-a-real-client-secret\u{0}",
        "not-a-r\u{e9}al-client-secret",
    ] {
        assert_eq!(
            resolve(
                Deployment::HostedTenant,
                &configured(&[(CLIENT_SECRET_ENV, mangled)])
            ),
            Decision::Silent(Silence::UnusableCredential),
            "a secret of {mangled:?} must not resolve to a report that cannot be built"
        );
        assert_eq!(
            resolve(
                Deployment::HostedTenant,
                &configured(&[(CLIENT_ID_ENV, mangled)])
            ),
            Decision::Silent(Silence::UnusableCredential),
            "an id of {mangled:?} must not resolve to a report that cannot be built"
        );
    }

    // The control, without which "reject everything" would pass: the shapes
    // an OpenPanel client actually has still report. Opaque generated
    // tokens — hex, base64url, a uuid, a prefixed key.
    for real_shaped in [
        "0f8b1c2d3e4f5a6b7c8d9e0f1a2b3c4d",
        "op_sk_9zQx-4Kd_7Yb2Lp0",
        "550e8400-e29b-41d4-a716-446655440000",
        "YWJjZGVmZ2hpamtsbW5vcHFyc3R1dnd4eXo=",
    ] {
        assert!(
            resolve(
                Deployment::HostedTenant,
                &configured(&[
                    (CLIENT_ID_ENV, real_shaped),
                    (CLIENT_SECRET_ENV, real_shaped)
                ])
            )
            .reports(),
            "{real_shaped:?} is the shape a real credential has and must still report"
        );
    }
}

/// And the reason never quotes the credential it rejected, for the same
/// reason the endpoint reason does not quote the endpoint.
#[test]
fn the_unusable_credential_reason_never_quotes_the_credential() {
    let reason = Silence::UnusableCredential.as_str();
    let printed = format!("{:?} {reason}", Silence::UnusableCredential);
    assert!(
        !printed.to_ascii_lowercase().contains("not-a-real"),
        "the reason leaked the credential: {printed}"
    );
    assert!(
        reason.contains("header"),
        "the reason must say what is wrong with it: {reason}"
    );
}

/// **There is no default endpoint, and an absent one is silence with its
/// own reason.**
///
/// This replaced `https://api.mixpanel.com/track`, and dropping the default
/// rather than re-pointing it is the deliberate half of that. OpenPanel is
/// self-hosted: its address is whatever the operator runs it at, and any
/// address this crate picked would be somebody else's collector. A tenant
/// that configured a credential but no endpoint would then have shipped its
/// telemetry to a third party nobody named — which is the accident
/// `Silence::UnusableEndpoint` already refuses to make from the other
/// direction.
#[test]
fn an_absent_endpoint_is_silence_rather_than_a_default() {
    let decision = resolve(
        Deployment::HostedTenant,
        &MapEnv::new([
            (CLIENT_ID_ENV, "not-a-real-client-id"),
            (CLIENT_SECRET_ENV, "not-a-real-client-secret"),
        ]),
    );
    assert_eq!(decision, Decision::Silent(Silence::NoEndpoint));
    assert!(!decision.reports());
    assert!(
        Silence::NoEndpoint
            .as_str()
            .contains("OPENCOMPANY_ANALYTICS_ENDPOINT"),
        "the reason must name the variable to set: {}",
        Silence::NoEndpoint.as_str()
    );
}

/// A blank endpoint is an absent one, not a broken one: a launcher that
/// exports an empty variable has configured nothing, and the reason it gets
/// should send it to set the variable rather than to fix its value.
#[test]
fn a_blank_endpoint_is_absent_rather_than_unusable() {
    assert_eq!(
        resolve(
            Deployment::HostedTenant,
            &configured(&[(ENDPOINT_ENV, "  \n")])
        ),
        Decision::Silent(Silence::NoEndpoint)
    );
}

/// `off` outranks the deployment kind. The platform can switch a tenant off
/// without rebuilding it.
#[test]
fn off_outranks_a_hosted_deployment() {
    assert_eq!(
        resolve(
            Deployment::HostedTenant,
            &configured(&[(ENABLE_ENV, "off")])
        ),
        Decision::Silent(Silence::OptedOut)
    );
}

/// The self-hoster's opt-in, which is the only way a non-hosted install ever
/// reports.
#[test]
fn a_self_hoster_can_opt_in() {
    assert!(resolve(Deployment::SelfHosted, &configured(&[(ENABLE_ENV, "on")])).reports());
}

/// A typo must not opt anybody in.
#[test]
fn a_misspelled_switch_does_not_opt_in() {
    assert_eq!(
        resolve(Deployment::SelfHosted, &configured(&[(ENABLE_ENV, "onn")])),
        Decision::Silent(Silence::Unreadable)
    );
}

/// **And a typo must not fail to opt anybody out.** This is the direction
/// that used to leak: an unreadable value fell through to the deployment
/// default, so a hosted tenant whose operator meant `off` and typed `of`
/// carried on reporting, with a boot line that said "reporting to …" and
/// gave them no reason to look again.
#[test]
fn a_misspelled_opt_out_does_not_keep_a_hosted_tenant_reporting() {
    for typo in ["of", "offf", "disabled", "0.0", "nope"] {
        let decision = resolve(Deployment::HostedTenant, &configured(&[(ENABLE_ENV, typo)]));
        assert_eq!(
            decision,
            Decision::Silent(Silence::Unreadable),
            "{typo:?} must not leave a hosted tenant reporting"
        );
        assert!(!decision.reports(), "{typo:?}");
    }
}

/// **A switch that is set but is not text fails closed too.**
///
/// `EnvSource::get` maps a non-Unicode value to `None`, so reading through
/// it would have treated `OPENCOMPANY_ANALYTICS=<invalid bytes>` as an
/// absent switch and left a hosted tenant reporting — the same leak as the
/// unreadable spelling, by a different route.
#[cfg(unix)]
#[test]
fn a_non_unicode_switch_is_unreadable_rather_than_absent() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    struct NonUnicodeSwitch;
    impl EnvSource for NonUnicodeSwitch {
        fn get_os(&self, key: &str) -> Option<OsString> {
            match key {
                ENABLE_ENV => Some(OsString::from_vec(vec![0xff, 0xfe, 0x6f, 0x6e])),
                CLIENT_ID_ENV => Some(OsString::from("not-a-real-client-id")),
                CLIENT_SECRET_ENV => Some(OsString::from("not-a-real-client-secret")),
                ENDPOINT_ENV => Some(OsString::from(TEST_ENDPOINT)),
                _ => None,
            }
        }
    }

    // The premise: this really is a value `get` cannot see at all.
    assert_eq!(NonUnicodeSwitch.get(ENABLE_ENV), None);
    assert!(NonUnicodeSwitch.get_os(ENABLE_ENV).is_some());

    assert_eq!(
        resolve(Deployment::HostedTenant, &NonUnicodeSwitch),
        Decision::Silent(Silence::Unreadable),
        "a switch set to bytes this process cannot read must not read as unset"
    );
}

/// The near-miss control: `off` really is matched case-insensitively and
/// after trimming, so the test above is finding typos rather than finding
/// every value that is not lowercase and bare.
#[test]
fn an_off_switch_is_trimmed_and_case_folded() {
    assert_eq!(
        resolve(
            Deployment::HostedTenant,
            &configured(&[(ENABLE_ENV, "  ofF\n")])
        ),
        Decision::Silent(Silence::OptedOut)
    );
}

/// The control for the two above: an **absent** switch still falls to the
/// deployment default, in both directions. Without this, "everything is
/// silent now" would pass the tests above just as well.
#[test]
fn an_absent_switch_still_falls_to_the_deployment_default() {
    assert!(resolve(Deployment::HostedTenant, &configured(&[])).reports());
    assert_eq!(
        resolve(Deployment::SelfHosted, &configured(&[])),
        Decision::Silent(Silence::NotHosted)
    );
}

/// A whitespace-only switch is an absent switch, not an unreadable one —
/// consistent with the credential and endpoint, and it must not flip a
/// hosted tenant into silence just because a launcher exported an empty
/// variable.
#[test]
fn a_blank_switch_is_treated_as_absent() {
    assert!(
        resolve(
            Deployment::HostedTenant,
            &configured(&[(ENABLE_ENV, "   ")])
        )
        .reports(),
        "a blank switch must not read as unreadable"
    );
}

/// The positive control for the endpoint group, and deliberately
/// **insensitive** to the trim: no surrounding whitespace, so this test
/// passes both with the filter and without it. Without such a control,
/// "every test in the group fails when I revert the fix" would be evidence
/// that the group asserts the implementation rather than the behaviour.
#[test]
fn a_configured_endpoint_is_reported_to_exactly() {
    match resolve(
        Deployment::HostedTenant,
        &configured(&[(ENDPOINT_ENV, "http://127.0.0.1:9/track")]),
    ) {
        Decision::Report { endpoint, .. } => assert_eq!(endpoint, "http://127.0.0.1:9/track"),
        other => panic!("{other:?}"),
    }
}

/// **A malformed endpoint is silence with a reason, not reporting.**
///
/// `collector.internal/track` — a hostname written without a scheme, which
/// is how anyone would first write one — used to resolve to
/// `Decision::Report`. Boot printed "reporting to collector.internal/track",
/// the tracker was installed, and every send died inside `reqwest` behind a
/// `debug!` line no operator has enabled. The product said something
/// true-sounding and then did nothing, which is the one failure this module
/// exists to make impossible — and it matters more now that every reporting
/// deployment types this variable by hand.
#[test]
fn a_malformed_endpoint_is_silence_rather_than_a_broken_report() {
    for unusable in [
        "collector.internal/track",
        "collector.internal",
        "/track",
        "://collector.internal/track",
        "ftp://collector.internal/track",
        "file:///tmp/track",
        "https://",
        "http://someone:hunter2@/track",
        "http://collector internal/track",
    ] {
        let decision = resolve(
            Deployment::HostedTenant,
            &configured(&[(ENDPOINT_ENV, unusable)]),
        );
        assert_eq!(
            decision,
            Decision::Silent(Silence::UnusableEndpoint),
            "{unusable:?} must not resolve to a report that cannot be sent"
        );
        assert!(!decision.reports(), "{unusable:?}");
    }
}

/// The reason names the variable and **never the value**: a collector
/// fronted by an authenticated proxy carries its key in the very URL that
/// was rejected, so quoting the bad value would put a credential in the boot
/// line of every misconfigured tenant. Asserted case-insensitively, because
/// a guard that matched exact case would read a lowercased leak as clean.
#[test]
fn the_unusable_endpoint_reason_never_quotes_the_endpoint() {
    const SECRET: &str = "NotARealCollectorKey";
    let reason = Silence::UnusableEndpoint.as_str();
    assert!(
        reason.contains("OPENCOMPANY_ANALYTICS_ENDPOINT"),
        "the reason must name the variable to act on: {reason}"
    );

    // Rejected for having no scheme, and carrying a credential while it is
    // rejected — which is exactly the case that would leak.
    let raw = format!("collector.internal/track?key={SECRET}");
    assert_eq!(
        resolve(
            Deployment::HostedTenant,
            &configured(&[(ENDPOINT_ENV, raw.as_str())])
        ),
        Decision::Silent(Silence::UnusableEndpoint)
    );
    let printed = format!("{:?} {}", Silence::UnusableEndpoint, reason);
    assert!(
        !printed
            .to_ascii_lowercase()
            .contains(&SECRET.to_ascii_lowercase()),
        "the reason leaked the endpoint credential: {printed}"
    );
    // The self-check: the needle really is findable in the unredacted
    // value, in whatever case it comes back, or the guard above is vacuous.
    assert!(
        raw.to_ascii_lowercase()
            .contains(&SECRET.to_ascii_lowercase())
            && raw
                .to_ascii_uppercase()
                .to_ascii_lowercase()
                .contains(&SECRET.to_ascii_lowercase()),
        "the needle must be findable before redaction: {raw}"
    );
}

/// **A non-Unicode endpoint is unusable, not absent.**
///
/// It reads through `get_os` rather than `get` so that the two stay
/// distinguishable. `get` maps unreadable bytes to `None`, which would tell
/// an operator who mistyped their collector address that they had never set
/// one — sending them to add a variable that is already there.
///
/// Under the endpoint default this replaced, the same confusion was
/// materially worse: unreadable bytes fell back to `api.mixpanel.com`, so a
/// tenant that pointed analytics at its own collector and mistyped it
/// reported to a **third party** instead. There is no default left for it
/// to fall into, so this is now a diagnostic distinction rather than a
/// containment one — but it is the same read, kept for the same reason.
#[cfg(unix)]
#[test]
fn a_non_unicode_endpoint_is_unusable_rather_than_absent() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    struct NonUnicodeEndpoint;
    impl EnvSource for NonUnicodeEndpoint {
        fn get_os(&self, key: &str) -> Option<OsString> {
            match key {
                ENDPOINT_ENV => Some(OsString::from_vec(
                    [b"https://collector.invalid/".as_slice(), &[0xff, 0xfe]].concat(),
                )),
                CLIENT_ID_ENV => Some(OsString::from("not-a-real-client-id")),
                CLIENT_SECRET_ENV => Some(OsString::from("not-a-real-client-secret")),
                _ => None,
            }
        }
    }

    // The premise: a value `get` cannot see at all.
    assert_eq!(NonUnicodeEndpoint.get(ENDPOINT_ENV), None);
    assert!(NonUnicodeEndpoint.get_os(ENDPOINT_ENV).is_some());

    let decision = resolve(Deployment::HostedTenant, &NonUnicodeEndpoint);
    assert_eq!(decision, Decision::Silent(Silence::UnusableEndpoint));
    match &decision {
        Decision::Report { endpoint, .. } => {
            panic!("reported to {endpoint} — an endpoint the operator never configured")
        }
        Decision::Silent(_) => {}
    }
}

/// **The endpoint check agrees with what `reqwest` can actually send to.**
///
/// Every row was measured against reqwest 0.12.28 — `Url::parse`,
/// `Client::post(..).build()`, and for the scheme, what the send does — not
/// reasoned about. The rows marked below are the ones a hand-rolled grammar
/// check accepted and `reqwest` rejects; they resolved to `Decision::Report`
/// and then dropped every event, which is the very failure
/// `is_usable_endpoint` exists to prevent.
#[test]
fn the_endpoint_check_matches_what_the_transport_accepts() {
    // (endpoint, refusal) — `None` reports, `Some(reason)` is silence with
    // that reason. `UnusableEndpoint` means `reqwest` cannot send to it at
    // all; `InsecureEndpoint` means it could, and must not, because the
    // credential would be readable on the way.
    let measured: &[(&str, Option<Silence>)] = &[
        // Rejected by `Url::parse`. Each of these was accepted by the
        // hand-rolled check this replaced.
        ("http://[::1/track", Some(Silence::UnusableEndpoint)), // unclosed IPv6 bracket
        ("http://]::1[/track", Some(Silence::UnusableEndpoint)), // brackets inside out
        (
            "http://collector.internal:99999/track",
            Some(Silence::UnusableEndpoint),
        ), // port out of range
        (
            "http://collector.internal:65536/track",
            Some(Silence::UnusableEndpoint),
        ), // one past the top
        (
            "http://collector.internal:abc/track",
            Some(Silence::UnusableEndpoint),
        ), // port not a number
        (
            "http://host:8080:9090/track",
            Some(Silence::UnusableEndpoint),
        ), // two ports
        ("http://127.0.0.1.5/track", Some(Silence::UnusableEndpoint)), // IPv4-shaped, invalid
        (
            "http://999.999.999.999/track",
            Some(Silence::UnusableEndpoint),
        ), // IPv4-shaped, invalid
        // Rejected by `Url::parse` and by the hand-rolled check alike.
        ("collector.internal/track", Some(Silence::UnusableEndpoint)),
        ("collector.internal", Some(Silence::UnusableEndpoint)),
        ("/track", Some(Silence::UnusableEndpoint)),
        (
            "://collector.internal/track",
            Some(Silence::UnusableEndpoint),
        ),
        ("https://", Some(Silence::UnusableEndpoint)),
        (
            "http://someone:hunter2@/track",
            Some(Silence::UnusableEndpoint),
        ),
        (
            "http://collector internal/track",
            Some(Silence::UnusableEndpoint),
        ),
        // Parsed happily by `url` — and even built by `reqwest` — but not
        // sendable, so checked on top of the parse.
        (
            "ftp://collector.internal/track",
            Some(Silence::UnusableEndpoint),
        ), // scheme refused at send
        ("file:///tmp/track", Some(Silence::UnusableEndpoint)),
        // NOT here: `http:///track`. It looks like an empty host and is
        // not one — `url` normalizes it to `http://track/`, taking the
        // first path segment as the host, and `reqwest` sends to it. A
        // collector named `track` that does not resolve is an unreachable
        // collector like any other, which #1739 makes a no-op on purpose.
        //
        // Sendable, but plain `http` to a host that is not loopback: the
        // client secret is a header on every request, so these would put it
        // on the wire in the clear. Each was accepted before the
        // `InsecureEndpoint` rule.
        (
            "http://collector.internal:65535/track",
            Some(Silence::InsecureEndpoint),
        ), // the top of the range
        (
            "http://collector.internal:/track",
            Some(Silence::InsecureEndpoint),
        ), // empty port is legal
        ("http://exa_mple.com/track", Some(Silence::InsecureEndpoint)),
        ("http://-example.com/track", Some(Silence::InsecureEndpoint)),
        (
            "http://\u{4f8b}\u{3048}.jp/track",
            Some(Silence::InsecureEndpoint),
        ),
        // A host that merely *starts* with a loopback address is not one.
        (
            "http://127.0.0.1.evil.example/track",
            Some(Silence::InsecureEndpoint),
        ),
        (
            "http://localhost.evil.example/track",
            Some(Silence::InsecureEndpoint),
        ),
        // Accepted, and the ones a deployment actually uses: `https`
        // anywhere, and `http` only to loopback.
        (TEST_ENDPOINT, None),
        ("http://127.0.0.1:9/track", None),
        ("http://127.0.0.1:9", None),
        ("http://localhost:9/track", None),
        ("http://LOCALHOST:9/track", None),
        ("https://collector.internal/track", None),
        ("HTTPS://collector.internal/track", None),
        (
            "https://collector.internal/track?key=NotARealCollectorKey",
            None,
        ),
        (
            "https://someone:NotARealCollectorKey@collector.internal/track",
            None,
        ),
        ("https://[::1]:8443/track", None),
        ("http://[::1]/track", None),
        ("https://collector.internal:8443/track#frag", None),
    ];

    for (endpoint, refusal) in measured {
        let decision = resolve(
            Deployment::HostedTenant,
            &configured(&[(ENDPOINT_ENV, endpoint)]),
        );
        match refusal {
            None => match decision {
                Decision::Report { endpoint: got, .. } => assert_eq!(&got, endpoint),
                other => panic!("{endpoint:?} must still report: {other:?}"),
            },
            Some(reason) => assert_eq!(
                decision,
                Decision::Silent(*reason),
                "{endpoint:?} must resolve to silence with {reason:?}"
            ),
        }
    }
}

/// **A plain `http` endpoint to a non-loopback host is silence, not a
/// credential in the clear.**
///
/// The OpenPanel client secret is a request header on *every* request, so
/// `OPENCOMPANY_ANALYTICS_ENDPOINT=http://collector.internal/track` writes a
/// long-lived write credential to the network in cleartext once per event
/// for the life of the tenant (CWE-319). Mixpanel had no equivalent
/// exposure: its token rode in the body of a request to one fixed `https`
/// address this crate chose, and no configuration could downgrade it.
///
/// Silence rather than a warning-and-send, because a warning is a line
/// nobody reads while the secret ships anyway, and a disclosed credential
/// cannot be un-disclosed once noticed. The reason names the variable and
/// the two ways out.
#[test]
fn a_cleartext_endpoint_is_silence_rather_than_a_credential_on_the_wire() {
    for insecure in [
        "http://collector.internal/track",
        "http://collector.internal:8080/track",
        "http://10.0.0.5:3000/track",
        "http://192.168.1.10/track",
        "http://[2001:db8::1]/track",
        "http://collector.example.com/api/track",
        // Not loopback, however much it looks like it.
        "http://127.0.0.1.evil.example/track",
        "http://localhost.evil.example/track",
    ] {
        let decision = resolve(
            Deployment::HostedTenant,
            &configured(&[(ENDPOINT_ENV, insecure)]),
        );
        assert_eq!(
            decision,
            Decision::Silent(Silence::InsecureEndpoint),
            "{insecure:?} would send the client secret in the clear"
        );
        assert!(!decision.reports(), "{insecure:?}");
    }
}

/// The control that keeps the test above from passing by rejecting every
/// `http` URL: **loopback `http` is the documented exception and still
/// reports.**
///
/// It is not a concession — it is the only `http` case that is actually
/// safe, because that traffic does not leave the host and so never crosses
/// a network between machines. It is also
/// how the collector is run beside the workload in development, and how
/// every gated transport test in this crate points at its own local
/// collector; without this arm those tests would be asserting against a
/// tracker that resolve had already silenced.
#[test]
fn loopback_http_is_the_one_cleartext_endpoint_that_still_reports() {
    for loopback in [
        "http://127.0.0.1:3000/track",
        "http://127.0.0.1/track",
        "http://127.1.2.3:9/track",
        "http://[::1]:3000/track",
        "http://[::1]/track",
        "http://localhost:3000/track",
        "http://LocalHost:3000/track",
    ] {
        match resolve(
            Deployment::HostedTenant,
            &configured(&[(ENDPOINT_ENV, loopback)]),
        ) {
            Decision::Report { endpoint, .. } => assert_eq!(endpoint, loopback),
            other => panic!("{loopback:?} is loopback and must still report: {other:?}"),
        }
    }
}

/// The insecure reason names the variable and the fix, and — like every
/// other reason here — **never quotes the value**.
///
/// This one matters more than most: the endpoint it is rejecting is by
/// definition one an operator typed, and a self-hosted collector is
/// routinely fronted by an authenticated proxy that carries its key in the
/// URL. Quoting the rejected value would print that key in the boot line of
/// every tenant the new rule silences.
#[test]
fn the_insecure_endpoint_reason_never_quotes_the_endpoint() {
    const SECRET: &str = "NotARealCollectorKey";
    let reason = Silence::InsecureEndpoint.as_str();
    assert!(
        reason.contains("OPENCOMPANY_ANALYTICS_ENDPOINT"),
        "the reason must name the variable to act on: {reason}"
    );
    assert!(
        reason.contains("https") && reason.contains("loopback"),
        "the reason must name both ways out: {reason}"
    );

    let raw = format!("http://collector.internal/track?key={SECRET}");
    assert_eq!(
        resolve(
            Deployment::HostedTenant,
            &configured(&[(ENDPOINT_ENV, raw.as_str())])
        ),
        Decision::Silent(Silence::InsecureEndpoint)
    );
    let printed = format!("{:?} {}", Silence::InsecureEndpoint, reason);
    assert!(
        !printed
            .to_ascii_lowercase()
            .contains(&SECRET.to_ascii_lowercase()),
        "the reason leaked the endpoint credential: {printed}"
    );
    // The self-check: the needle really is findable in the unredacted
    // value, or the guard above is vacuous.
    assert!(
        raw.to_ascii_lowercase()
            .contains(&SECRET.to_ascii_lowercase()),
        "the needle must be findable before redaction: {raw}"
    );
}

/// **Shape is judged before transport security**, so the two reasons stay
/// distinguishable and each sends an operator to the edit it names.
///
/// `http://collector.internal:99999/track` is both unparseable *and* plain
/// http; it must be reported as unusable, because there is no host to judge
/// until it parses and "this will not parse" is the more actionable half.
#[test]
fn an_unparseable_cleartext_endpoint_is_unusable_rather_than_insecure() {
    for both in [
        "http://collector.internal:99999/track",
        "http://collector internal/track",
        "http://[::1/track",
    ] {
        assert_eq!(
            resolve(
                Deployment::HostedTenant,
                &configured(&[(ENDPOINT_ENV, both)])
            ),
            Decision::Silent(Silence::UnusableEndpoint),
            "{both:?} does not parse, so the reason must be about the parse"
        );
    }
}

/// The controls that keep the group above from passing by rejecting
/// everything: the endpoints a deployment actually uses still resolve, and
/// still resolve to themselves.
#[test]
fn a_usable_endpoint_still_reports_to_exactly_itself() {
    for usable in [
        TEST_ENDPOINT,
        "http://127.0.0.1:9/track",
        "http://127.0.0.1:9",
        "https://collector.internal/track",
        "HTTPS://collector.internal/track",
        "https://collector.internal/track?key=NotARealCollectorKey",
        "https://someone:NotARealCollectorKey@collector.internal/track",
        "https://[::1]:8443/track",
        "https://collector.internal:8443/track#frag",
    ] {
        match resolve(
            Deployment::HostedTenant,
            &configured(&[(ENDPOINT_ENV, usable)]),
        ) {
            Decision::Report { endpoint, .. } => assert_eq!(endpoint, usable),
            other => panic!("{usable:?} must still report: {other:?}"),
        }
    }
}

/// The credential must not be printable by accident, because the accident is
/// a `{:?}` in a log line nobody reviewed.
///
/// **Both halves**, id included. OpenPanel's own web SDK treats a client id
/// as public, but there is no line in this tree that is better for carrying
/// it, and a type with one printable field and one redacted one is a type
/// someone eventually prints in full.
#[test]
fn neither_half_of_the_credential_is_printable() {
    let credentials =
        ClientCredentials::new("not-a-real-client-id", "not-a-real-client-secret");
    let printed = format!("{credentials:?}");
    for half in ["not-a-real-client-id", "not-a-real-client-secret"] {
        assert!(
            !printed.contains(half),
            "the Debug impl leaked {half}: {printed}"
        );
    }

    let decision = Decision::Report {
        endpoint: TEST_ENDPOINT.to_string(),
        credentials,
    };
    let printed = format!("{decision:?}");
    for half in ["not-a-real-client-id", "not-a-real-client-secret"] {
        assert!(
            !printed.contains(half),
            "the Debug impl leaked {half} through the decision: {printed}"
        );
    }
}
