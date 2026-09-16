use super::*;
use crate::ports::types::Verdict;
use serde_json::json;

fn c(tool: &str) -> Consequence {
    consequence_of(tool, &json!({}))
}

// ── hosting (issue #1079) ───────────────────────────────────────────────

/// The three tools openhuman's `hosting/README.md` labels "Read-only." ask
/// the provider what exists and what it did. Asking whether a build
/// finished must not cost an operator an approval.
#[test]
fn a_hosting_read_does_not_park() {
    for tool in [
        "hosting_deployment_status",
        "hosting_list_sites",
        "hosting_analytics",
        "hosting_list_deployments",
        "hosting_domain_status",
    ] {
        let consequence = c(tool);
        assert_eq!(
            consequence.reach,
            Reach::Nothing,
            "`{tool}` only reads the provider"
        );
        assert!(
            !consequence.parks_under_auto(),
            "`{tool}` must not interrupt anybody"
        );
    }
}

/// The outward effects still park. Without this the downgrade above would
/// pass against a table that stopped gating the whole namespace.
#[test]
fn a_hosting_effect_still_parks() {
    for tool in [
        "hosting_launch_site",
        "hosting_add_domain",
        "hosting_set_env",
        "hosting_rollback",
    ] {
        let consequence = c(tool);
        assert_eq!(
            consequence.reach,
            Reach::Consequence,
            "`{tool}` changes provider state"
        );
        assert!(
            consequence.parks_under_auto(),
            "`{tool}` must still park under auto"
        );
    }
}

/// **The label inversion this fixes.** Before declaring these, the fallback's
/// `undeclared_group` matched on substrings: `hosting_launch_site` — the
/// actual public deployment — contains no `deploy`/`publish`/`post` and fell
/// through to `Other`, while `hosting_deployment_status` — a read — contains
/// `deploy` and came back `Publish`. The operator's card described the
/// risky call as nothing in particular and the harmless one as a publish.
#[test]
fn the_deployment_is_labelled_publish_and_the_status_read_is_not() {
    assert_eq!(c("hosting_launch_site").group, EffectGroup::Publish);
    assert_eq!(c("hosting_add_domain").group, EffectGroup::Publish);
    assert_ne!(
        c("hosting_deployment_status").group,
        EffectGroup::Publish,
        "a status read must not announce itself as a deployment"
    );
}

/// `hosting_set_env` is `Other`, not `Publish`, and the tool's own
/// description is why: "The site must be redeployed afterwards for a
/// build-time variable to take effect." It changes what the NEXT deployment
/// serves and does not itself deploy, so a `Publish` card would tell an
/// operator a deployment is happening when none is.
#[test]
fn setting_env_is_not_labelled_as_a_deployment() {
    let consequence = c("hosting_set_env");
    assert_eq!(consequence.group, EffectGroup::Other);
    assert_eq!(consequence.reach, Reach::Consequence);
}

/// Every hosting tool answers from the table, not from `undeclared()`.
///
/// This is the regression guard for the mechanism itself: the fallback's
/// `READ_ONLY_PREFIXES` are matched with `name.starts_with`, so a
/// `hosting_`-prefixed read can never match one and the fallback cannot
/// classify any of these correctly. If a row is dropped, the tool silently
/// returns to that fallback rather than erroring — so the coverage is
/// asserted directly.
///
/// **This list is a floor, not the coverage guard, and issue #913 is why
/// the difference matters.** A hardcoded list only fails when a row is
/// *removed*; it says nothing when the vendor pin *adds* a tool. That is
/// exactly what happened — `hosting_rollback`, `hosting_list_deployments`
/// and `hosting_domain_status` arrived in the pin, were wired onto live
/// agents by `hosting_tools`, and this test stayed green while all three
/// fell through to `undeclared()`. The exhaustive check is
/// `every_wired_hosting_tool_is_declared` in
/// [`crate::harness::built_in::hosting`], which enumerates the belt itself;
/// it lives there because it needs the `openhuman` feature, and this file
/// compiles in lanes that do not have it. Keep both: this one holds in
/// every lane, that one is exhaustive in the lane that ships.
#[test]
fn every_hosting_tool_is_declared() {
    let declared: std::collections::BTreeSet<&str> = declared_tools().collect();
    for tool in [
        "hosting_deployment_status",
        "hosting_list_sites",
        "hosting_analytics",
        "hosting_list_deployments",
        "hosting_domain_status",
        "hosting_launch_site",
        "hosting_add_domain",
        "hosting_set_env",
        "hosting_rollback",
    ] {
        assert!(
            declared.contains(tool),
            "`{tool}` fell back to `undeclared()`, where the `hosting_` prefix \
             defeats the read test — declare it in DECLARED"
        );
    }
}

/// The mechanism, pinned on a name that is *not* declared: a namespaced read
/// still cannot be seen by the prefix test.
///
/// Kept as documentation of why declaring is the fix rather than teaching
/// the fallback to split on `_`. Widening that test would extend trust to
/// tools no belt registered and no reviewer saw, and would turn a
/// fail-closed miss into a fail-open one.
#[test]
fn the_fallback_cannot_see_a_read_verb_behind_a_namespace() {
    assert_eq!(
        c("hosting_list_something_undeclared").reach,
        Reach::Consequence,
        "an undeclared namespaced read gates — inconvenient, and the safe direction"
    );
    assert_eq!(
        c("list_something_undeclared").reach,
        Reach::Nothing,
        "the same verb at the front is seen, which is what makes the namespace the problem"
    );
}

// -----------------------------------------------------------------------
// Issue #673: a host-scoped fetch grant, and the `auto` line it must not
// cross
// -----------------------------------------------------------------------

/// A `web_fetch` call carrying a real URL, as the policy layer sees it.
fn fetching(url: &str) -> serde_json::Value {
    json!({ WEB_FETCH_URL_KEY: url })
}

/// **The entire reason [`Standing::ScopedGrantable`] exists, as a rule.**
///
/// The naive fix for #673 — declaring `web_fetch` [`Standing::Grantable`] to
/// obtain a scoped grant — was tried and rejected: because
/// [`Consequence::parks_under_auto`] read `is_grantable`, it also stopped the
/// tool parking under `auto`, for every agent, with no card and therefore no
/// scope ever consulted. `the_auto_tier_line_is_pinned_tool_by_tool` catches
/// that, and this states the invariant that must hold for the *repair* not to
/// re-open the same hole from the other side.
///
/// Exhaustive over [`Reach`] rather than sampled, because the variant is
/// argument-classified and so appears nowhere in [`DECLARED`] for a table
/// walk to find.
#[test]
fn a_scoped_grantable_call_is_delegable_but_never_unattended_under_auto() {
    assert!(
        Standing::ScopedGrantable.is_grantable(),
        "the point of the variant is that an operator CAN delegate it"
    );
    assert!(
        !Standing::ScopedGrantable.runs_unattended_under_auto(),
        "and that it still parks under auto — collapsing these two answers \
         back together is exactly the bug issue #673 fixed"
    );

    for reach in [
        Reach::Nothing,
        Reach::Money,
        Reach::ExternalRead,
        Reach::Consequence,
    ] {
        let verdict = Consequence {
            group: EffectGroup::Other,
            reach,
            standing: Standing::ScopedGrantable,
        };
        assert_eq!(
            verdict.parks_under_auto(),
            reach.parks_under_supervision(),
            "a scoped-grantable tool must park under `auto` wherever it parks \
             under `supervised` — {reach:?} disagreed"
        );
    }
}

/// The declaration table, rendered and pinned (issue #2148).
///
/// `Standing` decides what an operator may hand a teammate for a week, and
/// a one-word edit from `PerCall` to `Grantable` widens that silently — the
/// diff reads as a typo-sized change and the review question it should
/// raise ("should this run unattended for a week?") never gets asked.
/// Rendering the whole table makes every such edit a reviewable line.
///
/// Deliberately the **static** table only, not `consequence_of`: the
/// argument-classified tools answer differently depending on whether the
/// curated Composio catalogue is compiled in, so a snapshot of their live
/// verdicts would pass on one CI lane and fail on the next.
///
/// Re-bless with `BLESS_TOOL_STANDING=1`, then read the diff. A blessed
/// snapshot nobody read is not a pin.
#[test]
fn the_declared_grantability_table_is_pinned() {
    let mut rows: Vec<String> = DECLARED
        .iter()
        .map(|d| {
            format!(
                "{} group={:?} reach={:?} standing={:?}",
                d.tool, d.group, d.reach, d.standing
            )
        })
        .collect();
    rows.sort_unstable();
    let rendered = format!("{}\n", rows.join("\n"));

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots/tool-standing.txt");
    if std::env::var_os("BLESS_TOOL_STANDING").is_some() {
        std::fs::write(&path, &rendered).expect("write the grantability snapshot");
        return;
    }
    let committed = std::fs::read_to_string(&path).expect("read the grantability snapshot");
    assert_eq!(
        rendered, committed,
        "the declared grantability table moved. If that is intended, re-bless with \
         BLESS_TOOL_STANDING=1 and say in the PR why the tool's standing changed — a tool \
         becoming Grantable is an operator being newly able to hand it over for a week"
    );
}

/// A scope-required approval with no scope to give is refused, not minted
/// wide (issue #2148).
///
/// Driven through the inner rule because the shipped table cannot currently
/// produce that combination — `web_fetch_consequence` answers
/// `ScopedGrantable` only where the scope was already read. That is exactly
/// why the case is worth a test: the rule has to hold for the next entry
/// that derives its scope by another route, and a rule whose failing case
/// can only be described in prose is one nobody has run.
#[test]
fn a_scope_required_approval_without_a_scope_is_refused() {
    let refused = decide_standing_mint_scope(Standing::ScopedGrantable, None, Verdict::Approve);
    let StandingMintScope::Refused(why) = refused else {
        panic!("a scope-required approval with no scope must not mint: {refused:?}");
    };
    assert!(
        why.contains("approve it once instead"),
        "the refusal has to leave the operator somewhere to go: {why}"
    );
}

/// The asymmetry, stated: an unscoped denial refuses everything, which is
/// broad but safe, and refusing to mint it would take away the operator's
/// only way to decline a tool for a period (issue #1458).
#[test]
fn a_scope_required_denial_may_be_unscoped() {
    assert_eq!(
        decide_standing_mint_scope(Standing::ScopedGrantable, None, Verdict::Deny),
        StandingMintScope::Unscoped
    );
}

#[test]
fn a_derived_scope_is_carried_whatever_the_declaration_says() {
    for standing in [
        Standing::Grantable,
        Standing::ScopedGrantable,
        Standing::PerCall,
    ] {
        for verdict in [Verdict::Approve, Verdict::Deny] {
            assert_eq!(
                decide_standing_mint_scope(standing, Some("github".to_string()), verdict),
                StandingMintScope::Scoped("github".to_string()),
                "{standing:?}/{verdict:?}: a scope that was derived is never dropped"
            );
        }
    }
}

/// No declared tool can reach the mint claiming a scope it cannot produce.
///
/// This is the walk that makes the invariant enforced rather than merely
/// true today: it fails the moment a classification answers
/// `ScopedGrantable` down a path where `standing_scope_of` returns `None`.
/// Both probes matter — the rich one reaches the argument-classified
/// branches, the empty one is what a mint sees when a call carried nothing
/// the classifier could read.
#[test]
fn no_scope_required_tool_can_mint_unscoped() {
    let rich = json!({
        WEB_FETCH_URL_KEY: "https://docs.rs/serde",
        COMPOSIO_ACTION_KEY: "GITHUB_GET_A_REPOSITORY",
    });
    for probe in [&rich, &json!({})] {
        for tool in declared_tools() {
            if consequence_of(tool, probe).standing != Standing::ScopedGrantable {
                continue;
            }
            assert!(
                !matches!(
                    standing_mint_scope(tool, probe, Verdict::Approve),
                    StandingMintScope::Unscoped
                ),
                "`{tool}` is scope-required but would mint unscoped, and an unscoped grant \
                 admits every host and every toolkit"
            );
        }
    }
}

/// The same rule walked over the declaration table, so a tool that becomes
/// scoped-grantable later is covered without editing this test.
///
/// The `seen` counter is the point: every tool here is probed with arguments
/// rich enough to reach the argument-classified branches, and a walk that
/// found no scoped-grantable verdict at all would pass while asserting
/// nothing.
#[test]
fn every_scoped_grantable_tool_in_the_table_follows_its_reach() {
    let probe = json!({
        WEB_FETCH_URL_KEY: "https://docs.rs/serde",
        COMPOSIO_ACTION_KEY: "GITHUB_GET_A_REPOSITORY",
    });
    let mut seen = 0;
    for tool in declared_tools() {
        let verdict = consequence_of(tool, &probe);
        if verdict.standing == Standing::ScopedGrantable {
            seen += 1;
            assert_eq!(
                verdict.parks_under_auto(),
                verdict.reach.parks_under_supervision(),
                "`{tool}` is scoped-grantable, so its reach alone decides whether it parks"
            );
        }
    }
    assert!(
        seen > 0,
        "the walk reached no scoped-grantable tool, so it proved nothing"
    );
}

/// A fetch of a named host is grantable and scoped to that host; the same
/// call with an unreadable URL is neither.
///
/// The second half is not tidiness. A grant is minted with whatever
/// `standing_scope_of` returned, and an unscoped grant admits *everything*
/// (`StandingGrant::admits_scope`), so a URL-less call that stayed grantable
/// would let one approval mint a grant over every host on earth. The two
/// answers come from one function precisely so that cannot be represented.
#[test]
fn a_fetch_is_grantable_only_when_its_host_can_be_read() {
    let verdict = consequence_of(WEB_FETCH, &fetching("https://docs.rs/serde/latest"));
    assert_eq!(verdict.standing, Standing::ScopedGrantable);
    assert_eq!(
        standing_scope_of(WEB_FETCH, &fetching("https://docs.rs/serde/latest")),
        Some("https://docs.rs".to_string())
    );

    for unreadable in [
        json!({}),                         // no url at all
        fetching("not-a-url"),             // no scheme
        fetching("file:///etc/passwd"),    // not http(s)
        fetching("ftp://example.com/x"),   // not http(s)
        fetching("https://"),              // no host
        fetching("https://:8080/"),        // a port naming no host
        fetching("https://exa mple.com/"), // outside the host alphabet
    ] {
        let verdict = consequence_of(WEB_FETCH, &unreadable);
        assert_eq!(
            verdict.standing,
            Standing::PerCall,
            "an unreadable URL must not be grantable: {unreadable}"
        );
        assert_eq!(
            verdict.reach,
            Reach::Consequence,
            "an unreadable URL must stay gated, not free: {unreadable}"
        );
        assert!(verdict.reach.parks_under_supervision(), "{unreadable}");
        assert!(verdict.parks_under_auto(), "{unreadable}");
        assert_eq!(
            standing_scope_of(WEB_FETCH, &unreadable),
            None,
            "{unreadable}"
        );
    }
}

#[test]
fn read_shaped_http_is_free_but_readonly_and_spend_stay_closed() {
    for method in ["GET", "get", "HEAD", "OPTIONS"] {
        let verdict = consequence_of(
            "http_request",
            &json!({ "method": method, "url": "https://api.github.com/repos/o/r" }),
        );
        assert_eq!(verdict.reach, Reach::ExternalRead, "{method}");
        assert_eq!(verdict.standing, Standing::ScopedGrantable, "{method}");
        assert!(!verdict.reach.parks_under_supervision(), "{method}");
        assert!(!verdict.parks_under_auto(), "{method}");
        assert!(verdict.reach.denied_under_readonly(), "{method}");
        assert!(!verdict.reach.costs_money(), "{method}");
    }
}

/// A read-shaped method whose URL is a runtime expression — `=item.endpoint`
/// is tinyflows' own unresolved-value prefix — names no host yet. Free
/// classification is earned by a destination the operator can see; an
/// upstream node choosing the destination at run time is the case the
/// workflow gate's own
/// `an_unresolved_url_gates_and_says_the_destination_is_not_known_yet` pins.
#[test]
fn a_runtime_resolved_get_target_stays_gated() {
    for method in ["GET", "HEAD", "OPTIONS"] {
        let verdict = consequence_of(
            "http_request",
            &json!({ "method": method, "url": "=item.endpoint" }),
        );
        assert_eq!(verdict.reach, Reach::Consequence, "{method}");
        assert_eq!(verdict.standing, Standing::PerCall, "{method}");
        assert!(verdict.reach.parks_under_supervision(), "{method}");
        assert!(verdict.parks_under_auto(), "{method}");
    }
}

#[test]
fn mutating_and_unknown_http_methods_stay_per_call() {
    for args in [
        json!({ "method": "POST", "url": "https://api.example.com/items" }),
        json!({ "method": "DELETE", "url": "https://api.example.com/items/1" }),
        json!({ "method": "BREW", "url": "https://api.example.com/coffee" }),
        json!({ "method": 7, "url": "https://api.example.com/items" }),
    ] {
        let verdict = consequence_of("http_request", &args);
        assert_eq!(verdict.reach, Reach::Consequence, "{args}");
        assert_eq!(verdict.standing, Standing::PerCall, "{args}");
        assert!(verdict.reach.parks_under_supervision(), "{args}");
        assert!(verdict.parks_under_auto(), "{args}");
        assert!(!verdict.reach.costs_money(), "{args}");
    }
}

/// An authored `http_request` node with only a `url` omits `method`
/// entirely, and both the execution path
/// ([`crate::workflows::caps::http::to_tool_args`] forwards the omission,
/// then `HttpRequestTool` defaults it to GET) and the approvals-card path
/// ([`crate::workflows::gate::http_target`]) treat that omission as GET.
/// The classifier must agree, or a plain "fetch this URL" node parks under
/// `supervised`/`auto` while it actually runs a free read.
#[test]
fn an_omitted_http_method_defaults_to_get_and_is_free() {
    let args = json!({ "url": "https://api.example.com/items" });
    let verdict = consequence_of("http_request", &args);
    assert_eq!(verdict.reach, Reach::ExternalRead, "{args}");
    assert_eq!(verdict.standing, Standing::ScopedGrantable, "{args}");
    assert!(!verdict.reach.parks_under_supervision(), "{args}");
    assert!(!verdict.parks_under_auto(), "{args}");
}

/// A `GET` with a `body` is not a read: [`to_tool_args`] forwards the body
/// verbatim and `HttpRequestTool::execute_request` attaches it to the
/// outgoing request regardless of method, so this is an outbound data
/// transmission wearing a read-shaped method (PR #1989 review 3905098660).
/// Proven red pre-fix: before this change `http_request_consequence`
/// looked at `method` alone, so this call classified `ExternalRead` /
/// `ScopedGrantable` and ran unattended under `supervised`/`auto`.
#[test]
fn a_get_carrying_a_body_stays_gated() {
    let args = json!({
        "method": "GET",
        "url": "https://api.example.com/items",
        "body": "exfiltrated-secret",
    });
    let verdict = consequence_of("http_request", &args);
    assert_eq!(verdict.reach, Reach::Consequence, "{args}");
    assert_eq!(verdict.standing, Standing::PerCall, "{args}");
    assert!(verdict.reach.parks_under_supervision(), "{args}");
    assert!(verdict.parks_under_auto(), "{args}");
}

/// A `GET` carrying an `X-HTTP-Method-Override` header is not a read
/// either: many server frameworks treat that header as the real verb, so
/// a "free" GET can trigger a server-side mutation the operator never saw
/// a card for (PR #1989 review 3905098660). Proven red pre-fix for the
/// same reason as the body case above — the pre-fix classifier never
/// looked at `headers`.
#[test]
fn a_get_carrying_a_method_override_header_stays_gated() {
    let args = json!({
        "method": "GET",
        "url": "https://api.example.com/items/1",
        "headers": { "X-HTTP-Method-Override": "DELETE" },
    });
    let verdict = consequence_of("http_request", &args);
    assert_eq!(verdict.reach, Reach::Consequence, "{args}");
    assert_eq!(verdict.standing, Standing::PerCall, "{args}");
    assert!(verdict.reach.parks_under_supervision(), "{args}");
    assert!(verdict.parks_under_auto(), "{args}");
}

/// A read-shaped call with only allowlisted headers must stay free —
/// otherwise the fix would over-correct into gating every authenticated
/// read.
#[test]
fn a_get_with_only_safe_headers_stays_free() {
    let args = json!({
        "method": "GET",
        "url": "https://api.example.com/items",
        "headers": {
            "Accept": "application/json",
            "Authorization": "Bearer token",
            "If-None-Match": "\"abc123\"",
        },
    });
    let verdict = consequence_of("http_request", &args);
    assert_eq!(verdict.reach, Reach::ExternalRead, "{args}");
    assert_eq!(verdict.standing, Standing::ScopedGrantable, "{args}");
}

#[test]
fn web_fetch_is_an_external_read() {
    let args = fetching("https://docs.rs/serde");
    let verdict = consequence_of(WEB_FETCH, &args);
    assert_eq!(verdict.reach, Reach::ExternalRead);
    assert_eq!(verdict.standing, Standing::ScopedGrantable);
    assert!(!verdict.reach.parks_under_supervision());
    assert!(!verdict.parks_under_auto());
    assert!(verdict.reach.denied_under_readonly());
    assert!(!verdict.reach.costs_money());
}

/// `curl` shares `web_fetch`'s URL-reading shape but, unlike it, always
/// streams its response to a file under the workspace `downloads/` dir
/// (`CurlTool::execute`) — a write on every successful call. A readable
/// URL must not downgrade it to `web_fetch`'s `Reach::ExternalRead`: that
/// would let a workspace write skip the `supervised` park.
#[test]
fn curl_stays_consequence_gated_even_with_a_readable_url() {
    let args = fetching("https://docs.rs/serde");
    let verdict = consequence_of("curl", &args);
    assert_eq!(verdict.reach, Reach::Consequence);
    assert_eq!(verdict.standing, Standing::PerCall);
    assert!(verdict.reach.parks_under_supervision());
    assert!(verdict.parks_under_auto());
    assert!(verdict.reach.denied_under_readonly());
    assert!(!verdict.reach.costs_money());
    assert_eq!(standing_scope_of("curl", &args), None);
}

/// **The userinfo trap.** `https://docs.rs@evil.example/` fetches
/// `evil.example` — everything before the last `@` is credentials. A reader
/// that took the authority left-to-right would hand this call the `docs.rs`
/// scope and let any URL claim any grant, so it is asserted rather than
/// trusted to the shape of the code.
#[test]
fn credentials_in_a_url_cannot_claim_another_hosts_scope() {
    assert_eq!(
        standing_scope_of(WEB_FETCH, &fetching("https://docs.rs@evil.example/x")),
        Some("https://evil.example".to_string())
    );
    // Two `@` — the host is still what follows the LAST one.
    assert_eq!(
        standing_scope_of(WEB_FETCH, &fetching("https://a@b@evil.example/x")),
        Some("https://evil.example".to_string())
    );
    // A backslash is a path separator per WHATWG, so it terminates the
    // authority exactly as `/` does. Without this split the URL would read
    // `docs.rs` as the host and let `evil.example` satisfy a grant minted
    // for `docs.rs` — the userinfo trap re-opened through a delimiter this
    // split never handled.
    assert_eq!(
        standing_scope_of(WEB_FETCH, &fetching("https://evil.example\\@docs.rs/x")),
        Some("https://evil.example".to_string())
    );
}

/// The host key is exact. Neither a suffix nor a subdomain of a granted host
/// resolves to that host's scope, because both are hosts the operator never
/// read on the card.
#[test]
fn the_host_key_admits_neither_a_suffix_nor_a_subdomain() {
    let granted = standing_scope_of(WEB_FETCH, &fetching("https://docs.rs/")).unwrap();
    for impostor in [
        "https://evil-docs.rs/", // suffix match would admit this
        "https://evil.docs.rs/", // subdomain match would admit this
        "https://docs.rs.evil/", // prefix match would admit this
        "http://docs.rs/",       // the cleartext twin
        "https://docs.rs:8443/", // a different service on the same host
    ] {
        assert_ne!(
            standing_scope_of(WEB_FETCH, &fetching(impostor)),
            Some(granted.clone()),
            "`{impostor}` must not resolve to the scope granted for docs.rs"
        );
    }
}

/// A host is case-insensitive and `:443` is what `https` means, so these
/// spellings must produce one scope — otherwise a grant an operator approved
/// stops matching the very next call and the feature reads as broken.
#[test]
fn one_host_in_two_spellings_is_one_scope() {
    // The concrete scope is asserted first, not just the spellings against
    // each other: a regression that returned `None` for every spelling would
    // otherwise satisfy this test vacuously.
    let canonical = standing_scope_of(WEB_FETCH, &fetching("https://docs.rs/serde"));
    assert_eq!(canonical.as_deref(), Some("https://docs.rs"));
    for spelling in [
        "HTTPS://Docs.RS/Serde",
        "https://docs.rs:443/serde",
        "https://user:pw@docs.rs/serde",
    ] {
        assert_eq!(
            standing_scope_of(WEB_FETCH, &fetching(spelling)),
            canonical,
            "`{spelling}` names the same service and must share its scope"
        );
    }
}

/// **The bypass class this key must never re-open.**
///
/// Found in review of this change. The key was originally derived by reading
/// the URL string here — splitting the authority on `/`, `?` and `#`, then
/// taking whatever followed the last `@`. But `\` is *also* a path separator
/// in an http(s) URL, so `https://evil.com\@docs.rs/` is fetched from
/// `evil.com` while that reader minted a grant for `docs.rs`: an operator
/// approving "fetch from docs.rs" would have authorised `evil.com`. Tab,
/// newline and CR are stripped before parsing and were a second family of
/// the same bug.
///
/// The repair was to stop hand-parsing and derive the key from [`url::Url`],
/// the parser `reqwest` uses to perform the fetch — so there is no second
/// reader left to disagree with. This test is what keeps that true: it
/// consults `url` **independently** for the host each URL really resolves to,
/// and asserts the scope names that host. It is therefore not a tautology
/// restating the implementation — it is a cross-check that fails the moment
/// anyone reintroduces a bespoke reader, however carefully written.
///
/// Both directions are in the table on purpose. A key naming a host the fetch
/// will *not* reach lets a grant be spent elsewhere; a key naming a host the
/// operator did not see on the card is the same confusion pointed the other
/// way. Neither is acceptable.
#[test]
fn the_scope_names_the_host_the_fetching_client_will_actually_use() {
    for (raw, really_fetches) in [
        // The reported case: `\` terminates the authority, so everything
        // after it — including the `@` — is path.
        (r"https://evil.com\@docs.rs/", "evil.com"),
        // The same trick pointed the other way.
        (r"https://docs.rs\@evil.com/", "docs.rs"),
        // Mixed separators, both orders.
        (r"https://docs.rs\/@evil.com/", "docs.rs"),
        (r"https://docs.rs/\@evil.com", "docs.rs"),
        // Stripped-whitespace family: removed before parsing, so the `@`
        // that survives is a real userinfo delimiter.
        ("https://docs.rs\t@evil.com/", "evil.com"),
        ("https://docs.rs\n@evil.com/", "evil.com"),
        ("https://evil.com@\tdocs.rs/", "docs.rs"),
        // Stripping plus a backslash, together.
        ("https://\revil.com\\@docs.rs/", "evil.com"),
        // The plain userinfo case that was already defended.
        ("https://docs.rs@evil.example/x", "evil.example"),
    ] {
        // The fetching client's own answer, consulted here rather than
        // assumed — if a `url` upgrade ever changes it, this fails loudly
        // instead of the fixture quietly going stale.
        let client_host = url::Url::parse(raw)
            .unwrap_or_else(|e| panic!("fixture must parse: {raw:?}: {e}"))
            .host_str()
            .unwrap_or_else(|| panic!("fixture must name a host: {raw:?}"))
            .to_string();
        assert_eq!(
            client_host, really_fetches,
            "fixture drift: {raw:?} no longer resolves where this table says"
        );

        assert_eq!(
            standing_scope_of(WEB_FETCH, &fetching(raw)).as_deref(),
            Some(format!("https://{really_fetches}").as_str()),
            "the grant scope for {raw:?} must name the host the fetch reaches"
        );
    }
}

/// The `auto` line, named tool by tool and taken from the whole table
/// rather than a sample (issue #560).
///
/// [`Consequence::parks_under_auto`] is easy to check as a predicate; what
/// an operator actually feels is *which tools* stopped asking. And since
/// #560, [`Standing::Grantable`] decides two things at once — may be
/// delegated to one teammate, **and** runs unattended for everyone under
/// `auto` — so an edit loosening one tool for a delegation reason moves it
/// across this line as a side effect.
///
/// This walks [`declared_tools`], so a tool joining or leaving the
/// unattended set fails here and has to be named deliberately. The
/// predicate test alone would not notice.
#[test]
fn the_auto_tier_line_is_pinned_tool_by_tool() {
    // The whole of what `auto` changes: parks for an operator under
    // `supervised`, runs unattended under `auto`. Every entry is the
    // agent's own sandbox or this company's own memory — nothing here
    // leaves the building or spends money.
    const MOVED_BY_AUTO: &[&str] = &[
        "apply_patch",
        "csv_export",
        "edit",
        "file_write",
        "memory_store",
        // Issue #903, and the one entry that is not the agent's private
        // sandbox: it writes into the company's shared workspace. Declared
        // deliberately. A publish still reaches no counterparty and no
        // address, and the artifact chain versions it, so the company can
        // undo it alone — the two properties every other name here has.
        // What it buys is that a finished deliverable reaches the operator
        // without a per-file decision, which is the whole point of `auto`.
        "publish_artifact",
    ];

    let crossers = |args: &serde_json::Value| {
        let mut moved: Vec<&str> = declared_tools()
            .filter(|tool| {
                let verdict = consequence_of(tool, args);
                verdict.reach.parks_under_supervision() && !verdict.parks_under_auto()
            })
            .collect();
        moved.sort_unstable();
        moved
    };

    assert_eq!(
        crossers(&json!({})),
        MOVED_BY_AUTO,
        "a tool crossed the `auto` line. If that is intended, say so here — \
         `Standing::Grantable` now also means 'runs unattended for every agent \
         while the company sits in auto', which is wider than the standing \
         grant the field is named for"
    );

    // The same walk with arguments (issue #673). Two tools are classified
    // from their arguments rather than their name, so the empty-args walk
    // above cannot see the verdict they actually produce in service — a
    // `web_fetch` reading a real URL is the grantable shape, and the bare
    // name is not. Without this the line would be pinned only for the tools
    // whose classification the walk happens to be able to reach, and a
    // `web_fetch` loosened to `Standing::Grantable` would cross this line
    // unobserved.
    assert_eq!(
        crossers(&json!({
            WEB_FETCH_URL_KEY: "https://docs.rs/serde",
            COMPOSIO_ACTION_KEY: "GITHUB_GET_A_REPOSITORY",
        })),
        MOVED_BY_AUTO,
        "a tool crossed the `auto` line once its arguments were read"
    );

    // The other direction, spelled out: the tools an operator would be
    // most alarmed to find running unattended still park.
    for tool in [
        "shell",
        "http_request",
        "git_operations",
        "workspace_write",
        "workspace_delete",
        "workspace_rename",
        "media_generate_image",
        "media_generate_video",
        "mcp_call_tool",
        "run_workflow",
        // Issue #661 (M7): removing a workflow takes its whole revision
        // history with it, so there is nothing to restore afterwards. Its
        // read and update siblings deliberately do NOT park (see `DECLARED`)
        // — naming the one that does is how that split stays a decision.
        "delete_workflow",
        "some_tool_nobody_declared",
    ] {
        assert!(
            c(tool).parks_under_auto(),
            "`{tool}` leaves the company, spends money, or cannot be seen into — \
             it must still park under auto"
        );
    }

    // And the boundary `auto` deliberately does not draw: a billed read is
    // not a park. `web_search` runs under `supervised` because openhuman
    // resolves a `RequireApproval` inline — a parked search never happens —
    // and `auto` must not be stricter than the tier it replaces. The daily
    // cap is what holds spend.
    assert!(!c("web_search").parks_under_auto());
    assert!(c("web_search").reach.costs_money());

    // The other boundary `auto` deliberately does not draw (issue #903):
    // handing a finished file to the operator. `publish_artifact` changes
    // state, so it keeps `Reach::Consequence` and still parks under
    // `supervised` — but it reaches no counterparty and no address, writes
    // only into the company's own workspace and artifact chain, and is
    // versioned, so it is reversible by the company alone. Parking it under
    // `auto` made every deliverable wait on a human: one 9-node pipeline
    // run generated 15 of these.
    assert!(
        !c("publish_artifact").parks_under_auto(),
        "handing a file to the operator does not leave the company"
    );
    assert!(
        c("publish_artifact").reach.parks_under_supervision(),
        "a supervised desk must still see a publish before it lands"
    );
    assert!(
        !c("publish_artifact").reach.costs_money(),
        "a publish is not a spend, so the daily cap must not bill for it"
    );
}

/// The argument-classified half of the same line: a Composio read runs
/// unattended under `auto`, a send does not — and the cautious fallback
/// keeps an unclassified action on the parking side.
#[test]
#[cfg(feature = "openhuman")]
fn the_auto_line_reads_composio_arguments_not_the_tool_name() {
    let auto = |slug: &str| {
        consequence_of(COMPOSIO_EXECUTE, &json!({ "tool": slug })).parks_under_auto()
    };
    assert!(!auto("GITHUB_LIST_PULL_REQUESTS"), "a catalogue read runs");
    assert!(auto("GMAIL_SEND_EMAIL"), "a send still parks");
    assert!(
        auto("GITHUB_INVENT_A_NEW_VERB"),
        "an action nobody has classified is a send, in this tier too"
    );
}

#[test]
fn the_table_names_each_tool_once() {
    let mut seen: Vec<&str> = DECLARED.iter().map(|d| d.tool).collect();
    let before = seen.len();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(before, seen.len(), "a tool is declared twice: {seen:?}");
    for entry in DECLARED {
        assert_eq!(
            entry.tool,
            entry.tool.to_ascii_lowercase(),
            "declarations are matched lowercased, so `{}` could never be found",
            entry.tool
        );
        assert_ne!(
            entry.tool, COMPOSIO_EXECUTE,
            "`composio_execute` is classified from its arguments, not the table"
        );
    }
}

/// Issue #444's headline: the three broadest capabilities in the system
/// were grantable for up to a week because their names carry no
/// consequence word. They are named tools now, and named tools are
/// classified by what they reach.
#[test]
fn arbitrary_code_addresses_and_operator_guidance_are_never_grantable() {
    for tool in [
        "shell",
        "http_request",
        "curl",
        "web_fetch",
        "workspace_create",
        "workspace_write",
        "workspace_delete",
        "workspace_rename",
        "git_operations",
        "run_workflow",
        "mcp_call_tool",
        "mcp_registry_tool_call",
    ] {
        assert_eq!(
            c(tool).standing,
            Standing::PerCall,
            "`{tool}` can reach further than a standing grant can honestly describe"
        );
    }
}

/// The other half of #444: a tool nobody has classified must not inherit
/// the longest permission available just by landing in the residual bucket.
#[test]
fn an_undeclared_tool_is_never_grantable() {
    assert_eq!(c("some_tool_nobody_declared").standing, Standing::PerCall);
    // Including one that reads — not grantable is about standing, not about
    // whether it parks.
    let read = c("list_something_undeclared");
    assert_eq!(read.reach, Reach::Nothing);
    assert_eq!(read.standing, Standing::PerCall);
}

/// Issue #441: the consequence of a Composio call is a property of the
/// action, not of the one tool name every action arrives under.
///
/// Gated on the harness feature because the read verdict comes from the
/// vendored provider catalogue, which is only linked in there — the
/// default build's cautious fallback is pinned separately by
/// [`without_the_catalogue_every_composio_action_is_a_send`].
#[test]
#[cfg(feature = "openhuman")]
fn a_composio_read_is_grantable_and_a_send_is_not() {
    let read = consequence_of(
        COMPOSIO_EXECUTE,
        &json!({ "tool": "GITHUB_LIST_PULL_REQUESTS" }),
    );
    assert_eq!(read.group, EffectGroup::Other);
    assert_eq!(read.standing, Standing::Grantable);
    // …and since issue #559 it does not park: it reaches GitHub, so
    // `readonly` still denies it, but it changes nothing and costs nothing,
    // so `supervised` runs it.
    assert_eq!(read.reach, Reach::ExternalRead);

    let send = consequence_of(COMPOSIO_EXECUTE, &json!({ "tool": "GMAIL_SEND_EMAIL" }));
    assert_eq!(send.group, EffectGroup::Send);
    assert_eq!(send.standing, Standing::PerCall);
    assert_eq!(send.reach, Reach::Consequence);
}

/// The cautious direction, four ways: an action whose slug says nothing
/// this module recognises, a missing slug, a slug of the wrong type, and
/// arguments with no slug at all.
///
/// Narrowed by issue #1818, and the narrowing is the point. `..._LIST_...`
/// left this list because a slug that names a read verb is no longer
/// "unclassifiable" — see
/// [`a_drifted_read_runs_instead_of_parking_as_spend`]. What stayed is
/// every shape that offers no evidence either way, and for those the answer
/// is the same one it always was.
#[test]
fn an_unrecognised_composio_action_is_a_send() {
    for args in [
        json!({ "tool": "GITHUB_INVENT_A_NEW_VERB" }),
        json!({ "tool": "NOTAREALTOOLKIT_DO_SOMETHING" }),
        json!({ "tool": "" }),
        json!({ "tool": 7 }),
        json!({ "arguments": { "owner": "acme" } }),
        json!({}),
    ] {
        let verdict = consequence_of(COMPOSIO_EXECUTE, &args);
        assert_eq!(
            verdict.group,
            EffectGroup::Send,
            "an unclassifiable action must read as a send: {args}"
        );
        assert_eq!(verdict.standing, Standing::PerCall, "{args}");
    }
}

/// **Issue #1818, the headline.** A Composio *read* whose slug the curated
/// catalogue cannot place no longer parks under a card that says it leaves
/// the company or spends money.
///
/// `GITHUB_ISSUES_LIST_FOR_REPO` is the live evidence from the issue: the
/// same GitHub operation as the curated `GITHUB_LIST_REPOSITORY_ISSUES`,
/// under Composio's `operationId`-derived spelling. Before this it was a
/// `Send + Consequence + PerCall` — parked, labelled as spend, and
/// un-grantable, so the desk could not even be unblocked by consenting once.
///
/// The two halves that must both hold: the reach is the one a read
/// deserves, and the group never says spend.
#[test]
#[cfg(feature = "openhuman")]
fn a_drifted_read_runs_instead_of_parking_as_spend() {
    for slug in [
        // Every slug here is absent from the curated catalogue — checked,
        // not assumed: `a_drifted_read_is_a_miss_and_its_curated_twin_is_not`
        // pins the first one as an `UncuratedAction`, and a slug that
        // quietly gained a curated entry would make this test pass for the
        // wrong reason.
        "GITHUB_ISSUES_LIST_FOR_REPO",
        "GITHUB_GET_ISSUE",
        "GITHUB_LIST_ISSUES",
        "SLACK_SEARCH_MESSAGES",
        "NOTION_SEARCH_PAGES",
    ] {
        assert!(
            matches!(
                composio_catalog_lookup(slug),
                CatalogLookup::UncuratedAction { .. } | CatalogLookup::UnknownToolkit { .. }
            ),
            "`{slug}` is curated now, so it no longer exercises the fallback — \
             pick another uncurated read"
        );
        let verdict = consequence_of(COMPOSIO_EXECUTE, &json!({ "tool": slug }));
        assert_eq!(
            verdict.reach,
            Reach::ExternalRead,
            "`{slug}` names a read verb; it must not be priced as a send"
        );
        assert_eq!(
            verdict.group,
            EffectGroup::Other,
            "`{slug}` is a read — the card must not say spend"
        );
        assert!(
            !verdict.parks_under_auto(),
            "`{slug}` must not stall an auto desk"
        );
        assert!(
            !verdict.reach.parks_under_supervision(),
            "`{slug}` must not interrupt a supervised operator either"
        );
        // The tier that still says no, and should: `readonly` promises the
        // desk reaches into nobody's account, drifted slug or not.
        assert!(verdict.reach.denied_under_readonly(), "{slug}");
    }
}

/// The other half of #1818: an inferred read runs, but it can never be
/// minted into a standing grant.
///
/// A curated read is `Grantable` because a person classified it. A verb is
/// evidence, not a classification, and a standing grant outlives the call
/// it was cut from — so the guess gets the narrow reading, which expires
/// with the turn. `PerCall` costs nothing here precisely because
/// `ExternalRead` does not park: there is no approval to save.
#[test]
#[cfg(feature = "openhuman")]
fn an_inferred_read_is_never_grantable() {
    let inferred = consequence_of(
        COMPOSIO_EXECUTE,
        &json!({ "tool": "GITHUB_ISSUES_LIST_FOR_REPO" }),
    );
    assert_eq!(inferred.standing, Standing::PerCall);
    let curated = consequence_of(
        COMPOSIO_EXECUTE,
        &json!({ "tool": "GITHUB_LIST_REPOSITORY_ISSUES" }),
    );
    assert_eq!(
        curated.standing,
        Standing::Grantable,
        "the curated twin keeps the grant a person's classification earned"
    );
    assert_eq!(
        inferred.reach, curated.reach,
        "they differ on standing and on nothing else — that is the whole distinction"
    );
    assert_eq!(inferred.group, curated.group);
}

/// The fallback's own table, stated rather than sampled through
/// `consequence_of` (issue #1818).
///
/// Both directions matter and they are not symmetric: a `true` here runs
/// unattended, a `false` is the pre-#1818 park an operator can still
/// approve. So the read side is checked for the shapes that must run, and
/// the send side for every shape that must not — including the two the
/// rules exist for, whole-segment matching and first-verb-wins.
#[test]
fn the_verb_fallback_asks_for_evidence_of_a_read() {
    for slug in [
        "GITHUB_ISSUES_LIST_FOR_REPO",
        "GITHUB_LIST_REPOSITORY_ISSUES",
        "GITHUB_GET_A_PULL_REQUEST",
        "GMAIL_FETCH_EMAILS",
        "SLACK_SEARCH_MESSAGES",
        "NOTION_QUERY_DATABASE",
        "linear_list_issues",
        // Whole-segment matching: the mutating verb is a prefix of the
        // object, not the verb. A `contains` rule would send both.
        "GITHUB_LIST_STARGAZERS",
        "GMAIL_LIST_DRAFTS",
    ] {
        assert!(
            composio_slug_reads_by_verb(slug),
            "`{slug}` names a read verb and nothing that mutates"
        );
    }
    for slug in [
        "GMAIL_SEND_EMAIL",
        "GITHUB_CREATE_AN_ISSUE",
        "GITHUB_CREATE_OR_UPDATE_FILE_CONTENTS",
        "STRIPE_CREATE_A_CHARGE",
        "TWITTER_POST_TWEET",
        "GOOGLECALENDAR_QUICK_ADD",
        // No verb either list knows: absence of evidence is a send here,
        // which is the whole difference from upstream's `classify_unknown`.
        "GITHUB_INVENT_A_NEW_VERB",
        "NOTAREALTOOLKIT_DO_SOMETHING",
        "",
        "_",
        // The compound shapes a first-verb-wins rule would let through: a
        // read verb opens each one and a mutation follows it.
        "GITHUB_GET_AND_UPDATE_ISSUE",
        "GMAIL_FIND_OR_CREATE_CONTACT",
        "GMAIL_GET_AND_DELETE_THREAD",
        "GITHUB_LIST_AND_REMOVE_LABELS",
        // The same shape without the conjunction, which is why the object
        // slot gets no exemption. This one is real: a curated **write**.
        "GOOGLESHEETS_FIND_REPLACE",
        "TELEGRAM_ANSWER_CALLBACK_QUERY",
        // Curated, so the fallback never sees it — but if it did, the
        // noun `DRAFT` is indistinguishable from an elided second verb and
        // the rule takes the over-gating side.
        "GMAIL_GET_DRAFT",
    ] {
        assert!(
            !composio_slug_reads_by_verb(slug),
            "`{slug}` is not evidence of a read"
        );
    }
}

/// Same verdict, different reasons — and the reasons are now separable
/// (issue #470). A slug the catalogue cannot place is a legitimate call to
/// an unclassified action; an argument shape with no readable slug is a
/// caller bug that the send verdict would otherwise hide, which is exactly
/// how the `tool_slug` fixtures passed for as long as they did.
#[test]
fn a_missing_action_key_is_distinguishable_from_an_unknown_action() {
    assert_eq!(
        composio_action_slug(&json!({ "tool": "NOTAREALTOOLKIT_LIST_THINGS" })),
        Ok("NOTAREALTOOLKIT_LIST_THINGS"),
        "an uncatalogued slug is still a slug — the catalogue, not this reader, \
         is what declines it"
    );
    for (args, expected) in [
        (
            json!({ "tool_slug": "GMAIL_SEND_EMAIL" }),
            ActionKeyMiss::KeyAbsent,
        ),
        (
            json!({ "arguments": { "owner": "acme" } }),
            ActionKeyMiss::KeyAbsent,
        ),
        (json!({}), ActionKeyMiss::KeyAbsent),
        (json!({ "tool": 7 }), ActionKeyMiss::NotAString),
        (json!({ "tool": null }), ActionKeyMiss::NotAString),
        (json!({ "tool": "" }), ActionKeyMiss::Empty),
        (json!({ "tool": "   " }), ActionKeyMiss::Empty),
        (json!("GMAIL_SEND_EMAIL"), ActionKeyMiss::NotAnObject),
        (json!(null), ActionKeyMiss::NotAnObject),
    ] {
        assert_eq!(composio_action_slug(&args), Err(expected), "{args}");
        // …and the verdict is unchanged by any of it: the log line is the
        // only thing that differs, so this can never loosen a decision.
        assert_eq!(
            consequence_of(COMPOSIO_EXECUTE, &args).group,
            EffectGroup::Send,
            "{args}"
        );
    }
}

/// A grant scope is read through the same reader, so a call whose slug the
/// classifier could not find cannot resolve a toolkit either — `None`, and
/// a scoped grant refuses to admit `None`.
#[test]
fn an_unreadable_action_key_resolves_no_grant_scope() {
    for args in [
        json!({ "tool_slug": "GITHUB_LIST_PULL_REQUESTS" }),
        json!({ "tool": "" }),
        json!({ "tool": 7 }),
        json!({}),
    ] {
        assert_eq!(standing_scope_of(COMPOSIO_EXECUTE, &args), None, "{args}");
    }
}

/// The seam, pinned from the other side. Without the harness feature the
/// curated catalogue is not linked in, and the mint path still has to
/// answer the grantability question — so it answers it the cautious way,
/// for a read as much as for a send. A default build can only ever see a
/// `composio_execute` effect replayed from a journal line an openhuman
/// build wrote, so refusing the standing scope there costs an operator one
/// approve-once and never a wrong grant.
#[test]
#[cfg(not(feature = "openhuman"))]
fn without_the_catalogue_every_composio_action_is_a_send() {
    // Including one whose verb the #1818 fallback would happily call a
    // read. The fallback is deliberately not consulted here: a build that
    // cannot place `GITHUB_LIST_PULL_REQUESTS` has not earned the right to
    // infer anything, and `CatalogueAbsent` is the arm that says so.
    for slug in [
        "GITHUB_LIST_PULL_REQUESTS",
        "GITHUB_ISSUES_LIST_FOR_REPO",
        "GMAIL_SEND_EMAIL",
    ] {
        assert_eq!(
            composio_catalog_lookup(slug),
            CatalogLookup::CatalogueAbsent,
            "`{slug}` cannot be looked up in a build with no catalogue, and the \
             record must say that rather than blaming the slug (issue #1818)"
        );
        let verdict = consequence_of(COMPOSIO_EXECUTE, &json!({ "tool": slug }));
        assert_eq!(verdict.group, EffectGroup::Send, "{slug}");
        assert_eq!(verdict.standing, Standing::PerCall, "{slug}");
    }
}

/// The seam named from the other side (issue #1818): with the catalogue
/// linked in, no lookup may ever answer "there is no catalogue".
///
/// `CatalogueAbsent` is a fact about the binary. If it could also arise
/// from a slug, the operator-facing warning it triggers — *every* Composio
/// action over-gates in this build — would be a lie told once per stale
/// slug, and the deployment bug it exists to surface would be unfindable.
#[test]
#[cfg(feature = "openhuman")]
fn a_catalogued_build_never_reports_the_catalogue_absent() {
    for slug in [
        "GITHUB_LIST_PULL_REQUESTS",
        "GITHUB_ISSUES_LIST_FOR_REPO",
        "GMAIL_SEND_EMAIL",
        "NOTAREALTOOLKIT_DO_SOMETHING",
        "noUnderscore",
    ] {
        assert_ne!(
            composio_catalog_lookup(slug),
            CatalogLookup::CatalogueAbsent,
            "`{slug}` was looked up against a catalogue that is present"
        );
    }
}

/// Deliberately pinned: upstream's own `classify_unknown` would call
/// `GITHUB_INVENT_A_NEW_VERB` a read (its fallback arm returns `Read` when
/// no write verb matches). We do not use it, and this is the test that says
/// so — if somebody swaps the lookup for the heuristic to "cover more
/// slugs", the unknown-is-a-send guarantee goes with it.
#[test]
#[cfg(feature = "openhuman")]
fn we_do_not_fall_back_to_the_upstream_read_default() {
    use tinymemory_api::composio::scopes::{ToolScope, classify_unknown};
    assert_eq!(
        classify_unknown("GITHUB_INVENT_A_NEW_VERB"),
        ToolScope::Read,
        "upstream's fallback still defaults to read; if this changes the \
         comment above is stale, not the behaviour"
    );
    assert!(!composio_catalog_lookup("GITHUB_INVENT_A_NEW_VERB").is_read());
    // …and issue #1818's fallback did not quietly become that heuristic
    // either. It asks for a read verb; upstream asks only for the absence
    // of a write one, and this slug is the case that separates them.
    assert!(!composio_slug_reads_by_verb("GITHUB_INVENT_A_NEW_VERB"));
    assert_eq!(
        consequence_of(
            COMPOSIO_EXECUTE,
            &json!({ "tool": "GITHUB_INVENT_A_NEW_VERB" })
        )
        .group,
        EffectGroup::Send
    );
}

/// **The safety property of the #1818 fallback, over the whole catalogue.**
///
/// The fallback only ever fires on slugs the catalogue *cannot* place, so
/// there is no direct corpus of them to test against. The curated catalogue
/// is the next best thing and it is a strong one: ~680 actions a person
/// hand-classified as `Read` / `Write` / `Admin`. Running the verb rule
/// over them measures exactly what it would do on the uncurated slugs of
/// the same shape.
///
/// The two directions are **not** symmetric, so they are asserted
/// differently:
///
/// * A `Write` or `Admin` the rule calls a read would run unattended.
///   That is the bug this test exists to prevent, and it is asserted at
///   zero. It found two real vocabulary gaps when it was written —
///   `TELEGRAM_ANSWER_CALLBACK_QUERY` (a write, whose `QUERY` is a noun)
///   and `GOOGLESHEETS_FIND_REPLACE` (a write, a find-and-replace with the
///   conjunction elided) — which is why `ANSWER` and `REPLACE` are in
///   `MUTATES`.
/// * A `Read` the rule calls a send merely parks, which is the pre-#1818
///   behaviour. So that side gets a floor rather than a zero: the point is
///   to notice a rule that has stopped rescuing anything, not to chase the
///   last slug.
#[test]
#[cfg(feature = "openhuman")]
fn the_fallback_never_calls_a_curated_write_a_read() {
    use tinymemory_api::composio::catalogs::catalog_for_toolkit;
    use tinymemory_api::composio::scopes::{ToolScope, agent_ready_toolkits};

    let entries: Vec<_> = agent_ready_toolkits()
        .into_iter()
        .filter_map(catalog_for_toolkit)
        .flatten()
        .collect();
    assert!(
        entries.len() > 400,
        "the vendored catalogue should be hundreds of actions, found {} — this test \
         is only worth anything if it walks a real corpus",
        entries.len()
    );

    let leaked: Vec<&str> = entries
        .iter()
        .filter(|entry| entry.scope != ToolScope::Read)
        .filter(|entry| composio_slug_reads_by_verb(entry.slug))
        .map(|entry| entry.slug)
        .collect();
    assert!(
        leaked.is_empty(),
        "the verb fallback would run these curated writes unattended: {leaked:?}. \
         Each one names a verb `MUTATES` is missing — add it there rather than \
         narrowing the rule."
    );

    // The other direction: a floor, because over-gating is only a park.
    let reads: Vec<&str> = entries
        .iter()
        .filter(|entry| entry.scope == ToolScope::Read)
        .map(|entry| entry.slug)
        .collect();
    let rescued = reads
        .iter()
        .filter(|slug| composio_slug_reads_by_verb(slug))
        .count();
    assert!(
        rescued * 100 >= reads.len() * 85,
        "the verb rule recognises only {rescued} of {} curated reads. It has stopped \
         rescuing the drifted reads #1818 is about — a read verb was dropped, or a \
         `MUTATES` entry is matching a noun.",
        reads.len()
    );
}

/// **Issue #754.** The catalogue miss the whole issue is about, pinned from
/// both sides of the pair it was reported with.
///
/// `GITHUB_ISSUES_LIST_FOR_REPO` and `GITHUB_LIST_REPOSITORY_ISSUES` are the
/// same GitHub operation under two naming conventions — Composio's live
/// `operationId`-derived slug and the curated descriptive one. The second is
/// classified as a read and runs; the first misses the catalogue and parks.
///
/// The miss is still a miss after issue #1818 — that is what this test is
/// for, and it is why the two layers are separate functions. #1818 changed
/// what the classifier *does* with a miss; it must not change what the
/// catalogue *reports*, or the drift signal #754 exists for would be
/// silently switched off by the fix that made drift survivable.
#[test]
#[cfg(feature = "openhuman")]
fn a_drifted_read_is_a_miss_and_its_curated_twin_is_not() {
    assert_eq!(
        composio_catalog_lookup("GITHUB_LIST_REPOSITORY_ISSUES"),
        CatalogLookup::Curated { read: true },
        "the curated spelling is a read"
    );
    assert_eq!(
        composio_catalog_lookup("GITHUB_ISSUES_LIST_FOR_REPO"),
        CatalogLookup::UncuratedAction {
            toolkit: "github".to_string()
        },
        "the live spelling of the same operation is a catalogue MISS, and \
         naming it as such is the whole of #754 — the curated name is still \
         the thing to fix even though #1818 stopped the miss from parking"
    );
}

/// A curated **write** is not a miss, and telling them apart is what keeps
/// the signal readable (issue #754).
///
/// Both classify as a send, so a boolean cannot separate them — which is
/// exactly why the drift was invisible. If every send were reported as a
/// catalogue miss, `GMAIL_SEND_EMAIL` would drown the handful of slugs that
/// have actually drifted.
#[test]
#[cfg(feature = "openhuman")]
fn a_curated_write_is_not_reported_as_drift() {
    assert_eq!(
        composio_catalog_lookup("GMAIL_SEND_EMAIL"),
        CatalogLookup::Curated { read: false },
        "a curated send is the gate working, not the catalogue rotting"
    );
}

/// A slug whose toolkit has no curated surface is a *different* miss from a
/// slug its toolkit has never heard of, and the record says which.
#[test]
#[cfg(feature = "openhuman")]
fn an_unrecognised_toolkit_is_its_own_kind_of_miss() {
    assert!(matches!(
        composio_catalog_lookup("NOTAREALTOOLKIT_LIST_THINGS"),
        CatalogLookup::UnknownToolkit { .. }
    ));
}

/// Issue #443: the agent persona instructs every agent to call these rather
/// than answer a capability question from memory. They read local
/// registration state and reach nothing.
#[test]
fn listing_mcp_servers_and_tools_never_parks_but_calling_through_one_does() {
    for tool in [
        "mcp_list_servers",
        "mcp_list_tools",
        "mcp_registry_list_tools",
    ] {
        assert_eq!(c(tool).reach, Reach::Nothing, "`{tool}` reads local state");
    }
    for tool in ["mcp_call_tool", "mcp_registry_tool_call"] {
        assert!(
            c(tool).reach.parks_under_supervision(),
            "`{tool}` can perform any effect the remote server advertises"
        );
    }
}

/// The sibling defects the same sweep turned up: four pure reads of the
/// agent's own workspace that parked because the read-only-prefix rule
/// keys on the *start* of a name and none of them begins with one.
///
/// `read_workspace_state` was in this list until issue #459 showed it is
/// not a read at all — see
/// [`reading_workspace_state_is_classified_with_shell_because_it_runs_git`].
#[test]
fn a_workspace_read_never_parks_whatever_its_name_begins_with() {
    for tool in [
        "file_read",
        "glob",
        "grep",
        "image_info",
        "list",
        "memory_recall",
        "workspace_list",
        "workspace_read",
        "workspace_search",
        "media_list_models",
        "composio_list_toolkits",
        "composio_list_connections",
        "composio_list_tools",
    ] {
        assert_eq!(c(tool).reach, Reach::Nothing, "`{tool}` is a read");
    }
}

/// Issue #459: `read_workspace_state` is not the read its name promises.
/// It runs `git` in `{root}/{company}/{agent}/workspace`, and git reads
/// `.git/config` from that directory — a file the agent's own `file_write`
/// can author, and one whose keys can name a command to run. Until the
/// vendored `run_git` refuses untrusted repository config, it is
/// classified with `shell`.
///
/// The standing assertion is the one that matters most: `file_write` is
/// grantable, so if this were grantable too, the pair could be handed over
/// together for a week and the hole would be open for the length of the
/// grant with nobody watching.
#[test]
fn reading_workspace_state_is_classified_with_shell_because_it_runs_git() {
    let verdict = c("read_workspace_state");
    assert_eq!(verdict.reach, Reach::Consequence);
    assert!(
        verdict.reach.parks_under_supervision(),
        "running git under config the agent wrote must reach an operator"
    );
    assert!(
        verdict.reach.denied_under_readonly(),
        "`readonly` promises nothing runs; a config key can name a command"
    );
    assert_eq!(
        verdict.standing,
        Standing::PerCall,
        "a standing grant here would reopen the hole for its whole duration"
    );
    assert_eq!(
        verdict.reach,
        c("shell").reach,
        "it is the `shell` shape and should stay pinned to `shell`'s verdict"
    );
}

/// The feature keeps its point: the tools an agent uses to actually do work
/// in its own sandbox stay grantable, so an operator handing over a stretch
/// of autonomy is still handing over something useful.
#[test]
fn the_agents_own_workspace_writes_stay_grantable() {
    for tool in [
        "file_write",
        "edit",
        "apply_patch",
        "csv_export",
        "memory_store",
    ] {
        let verdict = c(tool);
        assert_eq!(verdict.standing, Standing::Grantable, "`{tool}`");
        // They mutate, so `readonly` must still deny and `supervised` must
        // still park the first call.
        assert!(verdict.reach.parks_under_supervision(), "`{tool}`");
    }
}

/// Issue #559, every acceptance criterion in one place — the four verdicts
/// `ExternalRead` has to give, and the one it must not.
///
/// The three predicates are the whole of the behaviour: `Reach` is never
/// matched exhaustively outside this module, so adding a variant changes
/// nothing anywhere until one of these answers differently.
#[test]
#[cfg(feature = "openhuman")]
fn a_composio_read_runs_under_supervision_and_is_still_denied_under_readonly() {
    use crate::policy::test_support::{COMPOSIO_READ_SLUG, composio_read_args};

    let read = consequence_of(COMPOSIO_EXECUTE, &composio_read_args());
    assert_eq!(
        read.reach,
        Reach::ExternalRead,
        "`{COMPOSIO_READ_SLUG}` is tagged `Read` in the vendored catalogue"
    );

    // 1. Under `supervised` it runs, instead of costing the operator a card.
    assert!(
        !read.reach.parks_under_supervision(),
        "reading a mailbox must not interrupt a person"
    );
    // 2. Under `readonly` it is still denied: that tier's contract is that
    //    nothing outside the company is reached at all.
    assert!(read.reach.denied_under_readonly());
    // 3. And it is NOT spend. This is the criterion that rules out reusing
    //    `Reach::Money`, whose `costs_money()` feeds the daily cap — every
    //    page of every mailbox would have counted against it.
    assert!(
        !read.reach.costs_money(),
        "a read is not billed; folding it into `Money` would bill it"
    );
    assert_ne!(read.group, EffectGroup::Spend);

    // 4. The standing answer is unchanged — it stops mattering for
    //    `supervised` now that nothing parks there, but still governs
    //    `readonly` and any tier added later.
    assert_eq!(read.standing, Standing::Grantable);
    assert_eq!(read.group, EffectGroup::Other);
}

/// The other half of #559: only the **read** branch moved.
///
/// The `send` binding is shared by the missing-key path and the non-read
/// path, so these hold structurally — but nothing stops a later edit from
/// touching that shared binding, which is the whole reason to assert them.
#[test]
fn a_composio_send_and_every_unclassifiable_call_still_park() {
    use crate::policy::test_support::{composio_send_args, composio_unclassified_args};

    let cases: [(&str, serde_json::Value); 6] = [
        ("a catalogued send", composio_send_args()),
        ("an uncatalogued action", composio_unclassified_args()),
        (
            "an unrecognised slug in a real toolkit",
            json!({ "tool": "GITHUB_INVENT_A_NEW_VERB" }),
        ),
        ("a non-string `tool`", json!({ "tool": 7 })),
        ("an empty slug", json!({ "tool": "" })),
        ("a missing `tool` key", json!({ "arguments": { "q": "x" } })),
    ];

    for (what, args) in cases {
        let verdict = consequence_of(COMPOSIO_EXECUTE, &args);
        assert_eq!(verdict.reach, Reach::Consequence, "{what}: {args}");
        assert!(
            verdict.reach.parks_under_supervision(),
            "{what} must still park: {args}"
        );
        assert!(verdict.reach.denied_under_readonly(), "{what}: {args}");
        assert_eq!(verdict.group, EffectGroup::Send, "{what}: {args}");
        assert_eq!(verdict.standing, Standing::PerCall, "{what}: {args}");
    }
}

/// `ExternalRead` must not leak into the spend cap.
#[test]
fn external_reads_never_claim_the_spend_bucket() {
    let args = json!({
        "url": "https://api.github.com/repos/o/r",
        "method": "GET",
        COMPOSIO_ACTION_KEY: "GITHUB_GET_A_REPOSITORY",
    });
    let mut seen = 0;
    for tool in declared_tools() {
        let verdict = consequence_of(tool, &args);
        if verdict.reach == Reach::ExternalRead {
            seen += 1;
            assert!(!verdict.reach.costs_money(), "`{tool}` is not spend");
            assert_ne!(verdict.group, EffectGroup::Spend, "`{tool}` is not spend");
        }
    }
    assert!(seen > 0, "the walk reached no external read");
}

#[test]
fn a_metered_read_is_allowed_under_supervision_and_denied_under_readonly() {
    let search = c("web_search");
    assert_eq!(search.reach, Reach::Money);
    assert!(!search.reach.parks_under_supervision());
    assert!(search.reach.denied_under_readonly());
    assert!(search.reach.costs_money());
    assert_eq!(search.group, EffectGroup::Spend);
    assert_eq!(search.standing, Standing::PerCall);
}

#[test]
fn declared_tools_covers_the_table_and_the_argument_classified_tool() {
    let all: Vec<&str> = declared_tools().collect();
    assert!(all.contains(&COMPOSIO_EXECUTE));
    assert!(all.contains(&"shell"));
    // `composio_execute` is the one roster entry with no `DECLARED` row;
    // the other four shadow theirs and are counted once.
    assert_eq!(all.len(), DECLARED.len() + 1);
}

/// The two mechanisms **partition** the names the gate knows: every name
/// [`declared_tools`] yields is answered either from its arguments or from
/// the table, never neither and never ambiguously (issue #877).
///
/// This is the criterion #877 states in as many words — *"the coverage test
/// keeps saying which tools answer from arguments and which from the
/// table"*. The roster is the authority on which side a tool is on, because
/// it is the same list [`consequence_of`] dispatches through: a tool cannot
/// be graded by argument without appearing here, and appearing here is what
/// puts it on the argument side of this assertion.
#[test]
fn the_roster_and_the_table_partition_the_known_tool_names() {
    let known: std::collections::BTreeSet<&str> = declared_tools().collect();
    let graded: std::collections::BTreeSet<&str> =
        ARGUMENT_GRADED.iter().map(|(tool, _)| *tool).collect();
    let tabled: std::collections::BTreeSet<&str> = DECLARED
        .iter()
        .map(|d| d.tool)
        .filter(|tool| !graded.contains(tool))
        .collect();

    assert!(
        graded.is_disjoint(&tabled),
        "a name cannot be answered by both mechanisms — the roster shadows \
         the table, so a shadowed row is not on the table side"
    );
    let union: std::collections::BTreeSet<&str> = graded.union(&tabled).copied().collect();
    assert_eq!(
        union, known,
        "every known tool name must sit on exactly one side of the \
         partition; if this fails, a mechanism has grown a name \
         `declared_tools` cannot see"
    );

    // And the sides say what they are, so the failure message above is
    // actionable rather than a set difference.
    for tool in &graded {
        assert!(
            argument_grader(tool).is_some(),
            "`{tool}` is on the roster but `consequence_of` would not \
             dispatch it"
        );
    }
    for tool in &tabled {
        assert!(
            argument_grader(tool).is_none(),
            "`{tool}` answers from the table but a classifier claims it too"
        );
    }
}

/// A classifier added to the roster is enumerated by [`declared_tools`]
/// **without** anyone remembering to add it there too.
///
/// This is the regression the old shape could not guard: [`declared_tools`]
/// used to `chain(once(COMPOSIO_EXECUTE))`, naming the single exception by
/// hand, so a fifth argument-graded tool with no [`DECLARED`] row would
/// have been dispatched and yet invisible to every test that walks
/// [`declared_tools`] — #877's "quietly join the coarse side". Driving the
/// derivation with a synthetic roster is the only way to assert it without
/// shipping a fake tool.
#[test]
fn a_roster_entry_with_no_table_row_is_still_enumerated() {
    const SYNTHETIC: &[(&str, Grader)] = &[("not_a_real_tool", shell_consequence)];
    let names: Vec<&str> = tool_names(DECLARED, SYNTHETIC).collect();
    assert!(
        names.contains(&"not_a_real_tool"),
        "a roster entry with no `DECLARED` row must still be enumerated"
    );
    assert_eq!(
        names.len(),
        DECLARED.len() + 1,
        "and exactly once — the row-less entry is appended, nothing else moves"
    );
}

/// A roster entry that shadows a [`DECLARED`] row is counted **once**.
///
/// The union is what makes the partition above meaningful: a concatenation
/// would double-count the roster entries that keep fallback rows, and every
/// caller that walks [`declared_tools`] as a set — `always_approve`,
/// `judgement`, the harness roster — would silently do redundant work over
/// duplicated names.
#[test]
fn a_roster_entry_that_shadows_a_table_row_is_enumerated_once() {
    let names: Vec<&str> = declared_tools().collect();
    for tool in ["shell", WEB_FETCH, "http_request", GIT_OPERATIONS] {
        assert_eq!(
            names.iter().filter(|name| **name == tool).count(),
            1,
            "`{tool}` holds both a roster entry and a `DECLARED` row and \
             must be enumerated once"
        );
    }
}

/// Every roster name is lower-case and appears once.
///
/// [`consequence_of`] lower-cases the incoming tool name before asking
/// [`argument_grader`], so a mixed-case entry would be an entry that never
/// fires — a classifier silently replaced by its table row, which is the
/// fail-open shape this whole cluster of issues exists to prevent. A
/// duplicate name would be a second classifier the first one shadows.
#[test]
fn the_roster_is_lower_case_and_has_no_duplicates() {
    let mut seen = std::collections::BTreeSet::new();
    for (tool, _) in ARGUMENT_GRADED {
        assert_eq!(
            *tool,
            tool.to_ascii_lowercase(),
            "`{tool}` is matched against a lower-cased name and would never fire"
        );
        assert!(seen.insert(*tool), "`{tool}` appears twice on the roster");
    }
}

/// The declaration is matched case-insensitively, the way every other arm
/// of the gate reads a tool name.
#[test]
fn lookup_ignores_case() {
    assert_eq!(c("SHELL").standing, Standing::PerCall);
    assert_eq!(c("Workspace_Read").reach, Reach::Nothing);
    // `composio_execute` is matched by the same lowercasing pass, so an
    // upper-cased tool name still reaches the argument classifier rather
    // than falling through to the undeclared heuristics.
    assert_eq!(
        consequence_of("COMPOSIO_EXECUTE", &json!({ "tool": "GMAIL_SEND_EMAIL" })).group,
        EffectGroup::Send
    );
    #[cfg(feature = "openhuman")]
    assert_eq!(
        consequence_of(
            "COMPOSIO_EXECUTE",
            &json!({ "tool": "github_list_branches" })
        )
        .standing,
        Standing::Grantable,
        "the curated lookup is case-insensitive on the slug too"
    );
}

/// The two halves of #457's scoping, exercised **together and directly**
/// (issue #610).
///
/// [`standing_scope_of`] mints the scope and
/// [`StandingGrant::admits_scope`] spends it, and since #559 no tier routes
/// a Composio read through both — see the retention note on
/// `standing_scope_of`. Each half is pinned on its own elsewhere, and each
/// of those tests spells the toolkit as its own `"github"` literal. Two
/// literals in two files are not an agreement: change what
/// `standing_scope_of` returns and both suites can be made green
/// separately while the pairing they describe is broken, with no live
/// caller left to notice.
///
/// So nothing here is written down. Every scope comes out of
/// `standing_scope_of` and goes straight into a grant or into
/// `admits_scope`, which makes this a test of whether the two functions
/// still agree rather than of what either one says.
#[test]
#[cfg(feature = "openhuman")]
fn the_minted_scope_is_the_scope_a_grant_admits() {
    use crate::runtime::grants::{GrantId, StandingGrant};

    let scope_of = |slug: &str| standing_scope_of(COMPOSIO_EXECUTE, &json!({ "tool": slug }));

    let minted = scope_of("GITHUB_LIST_BRANCHES");
    assert!(
        minted.is_some(),
        "a catalogued action must resolve a toolkit, or this test proves nothing"
    );
    // Minted the way the cycle mints one: from the parked effect's payload.
    let grant = StandingGrant {
        id: GrantId::new("g610"),
        agent: "ops".to_string(),
        workflow: None,
        tool: COMPOSIO_EXECUTE.to_string(),
        verdict: crate::ports::types::Verdict::Approve,
        granted_by: crate::ports::types::Actor {
            kind: crate::ports::types::ActorKind::User,
            id: "user-1".to_string(),
        },
        approval_id: crate::ports::types::ApprovalId::new("a610"),
        at_millis: 1_000,
        expires_at_millis: u64::MAX,
        origin_thread: None,
        origin_parent: None,
        origin_task: None,
        scope: minted,
    };

    // A second read from the provider the operator named. Scoped by
    // toolkit and not by slug, so a *different* GitHub action passes.
    assert!(
        grant.admits_scope(scope_of("GITHUB_LIST_PULL_REQUESTS").as_deref()),
        "the operator consented to a provider, not to one action slug"
    );
    // Another provider's read: every other dimension matches and the scope
    // is the one thing that says no.
    assert!(
        !grant.admits_scope(scope_of("GMAIL_FETCH_EMAILS").as_deref()),
        "'read from GitHub' is not consent to read the company's mail"
    );
    // An action the catalogue cannot place resolves to `None`, and a scoped
    // grant refuses `None` rather than guessing permissively.
    assert_eq!(
        scope_of("NOT_A_REAL_TOOLKIT_DO_SOMETHING"),
        None,
        "an unplaceable action must not resolve a toolkit"
    );
    assert!(
        !grant.admits_scope(scope_of("NOT_A_REAL_TOOLKIT_DO_SOMETHING").as_deref()),
        "unknown is a send here too"
    );
    // And the unscoped grant a pre-#457 journal line replays into still
    // admits whatever its `(agent, tool)` pair already admitted.
    let unscoped = StandingGrant {
        scope: None,
        ..grant
    };
    assert!(
        unscoped.admits_scope(scope_of("GMAIL_FETCH_EMAILS").as_deref()),
        "an unscoped grant must keep behaving as it did before scopes existed"
    );
}

/// Issue #457: a standing grant on `composio_execute` has to record *which
/// provider*, because the card said "read from GitHub" and the tool name
/// says nothing at all.
#[test]
#[cfg(feature = "openhuman")]
fn a_composio_call_is_scoped_to_its_toolkit() {
    assert_eq!(
        standing_scope_of(COMPOSIO_EXECUTE, &json!({ "tool": "GITHUB_LIST_BRANCHES" })),
        Some("github".to_string())
    );
    // A different action in the same toolkit is the same scope — the
    // operator agreed to a provider, not to one slug.
    assert_eq!(
        standing_scope_of(
            COMPOSIO_EXECUTE,
            &json!({ "tool": "GITHUB_LIST_PULL_REQUESTS" })
        ),
        Some("github".to_string())
    );
    // A different provider is a different scope, which is the whole point.
    assert_eq!(
        standing_scope_of(COMPOSIO_EXECUTE, &json!({ "tool": "GMAIL_FETCH_EMAILS" })),
        Some("gmail".to_string())
    );
    // The tool name is matched the same lowercasing way the rest of the
    // gate matches it.
    assert_eq!(
        standing_scope_of(
            "COMPOSIO_EXECUTE",
            &json!({ "tool": "github_list_branches" })
        ),
        Some("github".to_string())
    );
}

/// Nothing the catalogue can place is `None`, and `None` is what a scoped
/// grant refuses — so an unplaceable slug can never ride somebody else's
/// permission.
///
/// Unchanged by issue #1818, and worth saying why: the verb fallback moved
/// `..._LIST_...` off the parking path, but it did not give it a scope. A
/// grant is minted against a *toolkit*, and a toolkit the catalogue has
/// never heard of is still not one an operator consented to.
#[test]
fn an_unplaceable_composio_call_has_no_scope() {
    for args in [
        json!({ "tool": "NOTAREALTOOLKIT_LIST_THINGS" }),
        json!({ "tool": "" }),
        json!({ "tool": 7 }),
        json!({ "arguments": { "owner": "acme" } }),
        json!({}),
    ] {
        assert_eq!(
            standing_scope_of(COMPOSIO_EXECUTE, &args),
            None,
            "nothing to narrow to: {args}"
        );
    }
}

/// Every other tool has no scope, and must not grow one by accident: the
/// name of `file_write` already is the whole of what it can do.
#[test]
fn a_tool_whose_name_says_everything_has_no_scope() {
    for tool in [
        "file_write",
        "memory_forget",
        "memory_store",
        "shell",
        "workspace_write",
        "workspace_create",
    ] {
        assert_eq!(
            standing_scope_of(tool, &json!({ "tool": "GITHUB_LIST_BRANCHES" })),
            None,
            "`{tool}` is not a Composio call whatever its arguments say"
        );
    }
}

/// The other side of the seam. A default build cannot mint a Composio
/// standing grant at all (see
/// `without_the_catalogue_every_composio_action_is_a_send`), so answering
/// "no scope" here widens nothing — there is no scoped grant to widen.
#[test]
#[cfg(not(feature = "openhuman"))]
fn without_the_catalogue_nothing_carries_a_scope() {
    for slug in ["GITHUB_LIST_BRANCHES", "GMAIL_SEND_EMAIL"] {
        assert_eq!(
            standing_scope_of(COMPOSIO_EXECUTE, &json!({ "tool": slug })),
            None,
            "{slug}"
        );
    }
}

/// The literals above and the constants the tools themselves return are two
/// copies of the same string. This is the test that keeps them one.
#[test]
#[cfg(feature = "openhuman")]
fn the_declared_names_are_the_names_the_tools_return() {
    use crate::harness::{orchestrator, publish, search, workflow_admin, workspace_tools};
    for name in [
        workflow_admin::READ_WORKFLOW_TOOL,
        workflow_admin::UPDATE_WORKFLOW_TOOL,
        workflow_admin::DELETE_WORKFLOW_TOOL,
        orchestrator::QUERY_COMPANY_TOOL,
        orchestrator::SPAWN_TASK_TOOL,
        orchestrator::DELEGATE_TO_DESK_TOOL,
        orchestrator::DELEGATE_TO_TEAMMATE_TOOL,
        orchestrator::ADD_AGENT_TOOL,
        orchestrator::CREATE_WORKFLOW_TOOL,
        orchestrator::ASSIGN_TASK_TOOL,
        orchestrator::REVIEW_TASK_TOOL,
        orchestrator::RUN_WORKFLOW_TOOL,
        publish::PUBLISH_ARTIFACT_TOOL,
        search::WEB_SEARCH_TOOL,
        workspace_tools::WORKSPACE_LIST_TOOL,
        workspace_tools::WORKSPACE_READ_TOOL,
        workspace_tools::WORKSPACE_SEARCH_TOOL,
        workspace_tools::WORKSPACE_CREATE_TOOL,
        workspace_tools::WORKSPACE_WRITE_TOOL,
        workspace_tools::WORKSPACE_RENAME_TOOL,
        workspace_tools::WORKSPACE_DELETE_TOOL,
        crate::harness::composio_catalog::LIST_TOOLS_TOOL,
        crate::harness::composio_catalog::LIST_TOOLKITS_TOOL,
    ] {
        assert!(
            DECLARED.iter().any(|d| d.tool == name),
            "`{name}` is a live tool constant with no declaration"
        );
    }
}

/// Where the console keeps the words an operator reads instead of a tool
/// name. Named once so both the parser and every failure message point at
/// the same file.
const LANGUAGE_TS: &str = "frontend/src/lib/language.ts";

/// One object literal in [`LANGUAGE_TS`], read as `key -> sentence`.
///
/// A line parser rather than the two alternatives, and the reasons are the
/// same ones that make this a `cargo test` at all:
///
/// * a **checked-in generated manifest** of the declared set would give the
///   contract two failure sites and a window between the declaration commit
///   and the regenerate commit where nothing is wrong;
/// * a **CI grep** would have no local signal for the Rust contributor who
///   adds the next `Reach::Consequence` line — and that is who introduced
///   all three instances of this defect (#372, #551 → #671, now #701).
///
/// It is deliberately literal about the shape it accepts: an object literal
/// opened by `const <NAME>` on a line ending in `{` and closed by a `};`
/// line. Anything else panics rather than returning a short list, because a
/// parser that silently reads nothing turns this test into a green light
/// for the exact regression it exists to catch — see the vacuity guards in
/// [`every_consequence_tool_has_a_console_label`].
///
/// It returns pairs rather than keys (issue #743) because the distinctness
/// half needs the sentences, and one parse feeding both halves is the point:
/// this test exists because a hand-maintained restatement of the declared
/// set drifts from it, and a second parser over the same file would be that
/// same mistake one level down.
///
/// Values are taken literally — the text between the first `:` and the
/// trailing comma, unquoted. Every entry in both tables is a plain string
/// literal on one line today; a template literal or a concatenation would
/// arrive here as its own source text and, being unequal to any other
/// entry, would pass the distinctness check without asserting anything about
/// what an operator reads. That is the one shape to reject rather than
/// tolerate, and the `>=` floors below are what would catch a table that
/// reshaped into it wholesale.
fn label_pairs(source: &str, decl: &str) -> Vec<(String, String)> {
    let mut lines = source.lines();
    let opened = lines.any(|line| {
        let line = line.trim_start();
        line.starts_with(&format!("const {decl}")) && line.ends_with('{')
    });
    assert!(
        opened,
        "no `const {decl} … {{` line in {LANGUAGE_TS}. If the table was \
         renamed or reshaped, update this parser — do not delete the test"
    );

    let mut pairs = Vec::new();
    for line in lines {
        let line = line.trim();
        if line == "};" {
            return pairs;
        }
        if line.is_empty() || line.starts_with("//") || line.starts_with('*') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        pairs.push((
            key.trim().trim_matches('"').to_string(),
            value
                .trim()
                .trim_end_matches(',')
                .trim()
                .trim_matches('"')
                .to_string(),
        ));
    }
    panic!("`const {decl}` in {LANGUAGE_TS} is never closed by a `}};` line");
}

/// Every tool that reaches an operator resolves to a sentence, not to
/// "Use one of its tools" (issue #701).
///
/// The console's `approvalAction` resolves `EFFECT_LABELS` → `TOOL_LABELS`
/// → a generic fallback, so a gated tool in neither table asks an operator
/// to consent to "use one of its tools" — the #372 defect. It has now
/// recurred three times, and every time the commit that caused it was a
/// Rust one adding a `Reach::Consequence` declaration with no reason to
/// open the frontend at all. So the coupling belongs here, next to
/// [`DECLARED`], where that contributor's `cargo test` reports it.
///
/// Scoped to the whole [`Reach::Consequence`] class rather than to the
/// per-call subset. The grantable ones (`file_write`, `edit`,
/// `apply_patch`, `csv_export`) park exactly the same way, and they are
/// *also* the ones issue #374's Standing-permissions list renders through
/// `toolAction` with no payload block to disambiguate them. A test scoped
/// to `Standing::PerCall` would ship blind to four instances of the class
/// it exists to kill.
#[test]
fn every_consequence_tool_has_a_console_label() {
    let source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../frontend/src/lib/language.ts"
    ));
    // Read as pairs, not keys: the distinctness half below needs the
    // sentences. `label_keys` is the same parse, so the two halves cannot
    // disagree about what the tables hold.
    let effect_labels = label_pairs(source, "EFFECT_LABELS");
    let tool_labels = label_pairs(source, "TOOL_LABELS");
    let effects: Vec<&str> = effect_labels.iter().map(|(k, _)| k.as_str()).collect();
    let tools: Vec<&str> = tool_labels.iter().map(|(k, _)| k.as_str()).collect();

    // Vacuity guards. A parse that quietly returned nothing would report
    // every gated tool as unlabelled — noisy, and therefore self-correcting.
    // A parse that quietly returned *the wrong block* would report none of
    // them, which is the failure that matters: the test would pass forever
    // while the console regressed. These anchors are entries with no reason
    // to move, so a reformat of `language.ts` breaks here loudly instead.
    for anchor in ["payment.send", "workflow.approve"] {
        assert!(
            effects.contains(&anchor),
            "parsed EFFECT_LABELS from {LANGUAGE_TS} without `{anchor}` — \
             the parser is reading the wrong block, not the table shrinking"
        );
    }
    for anchor in ["shell", "workspace_create"] {
        assert!(
            tools.contains(&anchor),
            "parsed TOOL_LABELS from {LANGUAGE_TS} without `{anchor}` — \
             the parser is reading the wrong block, not the table shrinking"
        );
    }
    assert!(
        effects.len() >= 15 && tools.len() >= 10,
        "parsed only {} EFFECT_LABELS and {} TOOL_LABELS keys from \
         {LANGUAGE_TS}; both tables are larger than that, so the parser is \
         stopping early",
        effects.len(),
        tools.len()
    );

    let gated: Vec<&str> = declared_tools()
        .filter(|tool| c(tool).reach.parks_under_supervision())
        .collect();

    // The walk's own vacuity guard (issue #743). A distinctness check over
    // an empty or truncated set passes having asserted nothing, which is
    // precisely the fail-open shape the guards above exist to refuse — and
    // the shape that made #706's reproduction wrong by half.
    //
    // A floor rather than an exact count, matching the `>=` idiom above: the
    // gated set is 25 today and grows whenever a `Reach::Consequence` line
    // is declared, so pinning it exactly would fail every such commit for
    // being correct. What must never happen is the walk *shrinking* toward
    // the four hardcoded names this widened.
    assert!(
        gated.len() >= 20,
        "only {} tools were selected as gated; the declaration table holds \
         far more `Reach::Consequence` entries than that, so the walk is \
         selecting almost nothing and everything below it is vacuous",
        gated.len()
    );

    let mut unlabelled: Vec<&str> = gated
        .iter()
        .copied()
        .filter(|tool| !effects.iter().any(|k| k == tool) && !tools.iter().any(|k| k == tool))
        .collect();
    unlabelled.sort_unstable();
    assert!(
        unlabelled.is_empty(),
        "{unlabelled:?} park for an operator but have no entry in either \
         label map in {LANGUAGE_TS}, so their approval card reads \"Use one \
         of its tools\". Add each to TOOL_LABELS — a gated tool's label \
         only ever appears above the payload block, which is what \
         EFFECT_LABELS entries do not assume and why its \
         EFFECT_DONE_LABELS mirror would demand a past-tense twin these \
         kinds never reach"
    );

    // ...and no two of them read the same sentence (issue #743).
    //
    // Checked after the unlabelled walk on purpose: an unlabelled pair would
    // collide here too, on the fallback, and reporting that as "these two
    // read alike" would name the symptom while the assertion above names the
    // cause. Ordering the two is what keeps one failure message honest.
    //
    // Resolved through the console's own rung order — `EFFECT_LABELS` then
    // `TOOL_LABELS` — because that is what `toolAction` does, and a tool
    // present in both resolves to the effect sentence. Comparing the tables
    // separately would miss exactly the collision that ordering creates.
    let sentence = |tool: &str| -> &str {
        effect_labels
            .iter()
            .chain(tool_labels.iter())
            .find(|(key, _)| key == tool)
            .map(|(_, value)| value.as_str())
            .expect("every gated tool is labelled — asserted directly above")
    };

    let mut by_sentence: std::collections::BTreeMap<&str, Vec<&str>> = Default::default();
    for tool in &gated {
        by_sentence.entry(sentence(tool)).or_default().push(tool);
    }
    let collisions: Vec<String> = by_sentence
        .iter()
        .filter(|(_, sharing)| sharing.len() > 1)
        .map(|(reads, sharing)| format!("{sharing:?} all read {reads:?}"))
        .collect();
    assert!(
        collisions.is_empty(),
        "two gated tools render the same sentence: {}. The Standing \
         permissions list (#374) puts no payload block under a row, so two \
         rows reading alike are two permissions an operator cannot choose \
         between — and on an approval card the payload only disambiguates \
         them if they happen to carry different arguments. Give each its own \
         words in {LANGUAGE_TS}",
        collisions.join("; ")
    );
}

// -----------------------------------------------------------------------
// Issue #875: `shell`, classified by the command it was handed
// -----------------------------------------------------------------------

// Gated to match its callers. Every test below that grades a shell command
// is `#[cfg(feature = "openhuman")]`, so without the feature they compile
// away and this helper is left with none — `dead_code` under the default
// lane's `-D warnings`, which is what turned the `Rust` job red.
#[cfg(feature = "openhuman")]
fn shell(command: &str) -> Consequence {
    consequence_of(SHELL, &json!({ SHELL_COMMAND_KEY: command }))
}

/// The complaint this issue is about: an agent looking at its own workspace
/// paid an approval per command. These are the exact shapes an operator was
/// approving on staging.
#[test]
#[cfg(feature = "openhuman")]
fn a_read_of_the_agents_own_workspace_runs_unattended() {
    for command in [
        "grep -l -i \"resets\\|forgot\" session_raw/*.jsonl",
        "grep -c -i plus session_raw/*.jsonl",
        "find . -maxdepth 4 -type d",
        "cat notes.md",
        "ls -la",
        "wc -l src/main.rs",
    ] {
        let c = shell(command);
        assert_eq!(
            c.reach,
            Reach::Nothing,
            "`{command}` reads and changes nothing"
        );
        assert!(
            !c.reach.parks_under_supervision(),
            "`{command}` must not park under any acting tier"
        );
    }
}

/// Bases omitted from the vendor's name-only allowlist may run unattended
/// only when their actual argv excludes every writing form we admit around.
#[test]
#[cfg(feature = "openhuman")]
fn argv_sensitive_workspace_reads_run_without_admitting_writes() {
    for command in [
        "sed -n '860,915p' src/policy/consequence.rs",
        "sed 's/old/new/g' notes.txt",
        "sort names.txt",
        "sort -r names.txt",
        "awk '{ print $1 }' data.txt",
        "awk -F, '{ print $2 }' data.csv",
    ] {
        assert_eq!(
            shell(command).reach,
            Reach::Nothing,
            "`{command}` has a provably read-only argv"
        );
    }

    for command in [
        "sed -i 's/old/new/g' notes.txt",
        "sed -ni.bak 's/old/new/g' notes.txt",
        "sed --in-place=.bak 's/old/new/g' notes.txt",
        "sed -f transform.sed notes.txt",
        "sort -o sorted.txt names.txt",
        "sort -ruooutput.txt names.txt",
        "sort --output=sorted.txt names.txt",
        "sort --compress-program=gzip names.txt",
        "awk '{ print $1 > \"out.txt\" }' data.txt",
        "awk '{ print $1 }' data.txt > out.txt",
        "awk '{ print $1 | \"tee out.txt\" }' data.txt",
        "awk 'BEGIN { system(\"touch out.txt\") }' data.txt",
        "awk -f report.awk data.txt",
    ] {
        assert_eq!(
            shell(command).reach,
            Reach::Consequence,
            "`{command}` can write or execute and must park"
        );
    }
}

/// The argv exceptions remain behind #876's lexical containment boundary.
#[test]
#[cfg(feature = "openhuman")]
fn argv_sensitive_reads_still_refuse_escapes_and_ambiguous_shell_syntax() {
    for command in [
        "sed -n '1p' /etc/passwd",
        "sort /etc/passwd",
        "awk '{ print $1 }' /etc/passwd",
        "sed -n '1p' ~/.ssh/config",
        "sort ~/secrets.txt",
        "awk '{ print $1 }' ~/.secrets",
        "sed -n '1p' ../secret.txt",
        "sort data/../../secret.txt",
        "awk '{ print $1 }' ../secret.txt",
        "sed -n \"$(cat program.sed)\" notes.txt",
        "sort \"$(cat filenames.txt)\"",
        "awk \"$(cat program.awk)\" data.txt",
        "sed -n '1p notes.txt",
        "sort $SORT_FLAGS names.txt",
        "awk '{ print $1 }' data.txt; rm data.txt",
    ] {
        assert_eq!(
            shell(command).reach,
            Reach::Consequence,
            "`{command}` is outside the lexical boundary or has ambiguous argv"
        );
    }
}

/// A command the vendored classifier grades `Read` — because it grades by
/// command name, never by path — must still park when its arguments name a
/// location outside the agent's own directory. Falsified against the
/// pre-fix behaviour: before `shell_command_reaches_outside_cwd` existed,
/// `shell_command_is_read` alone was sufficient and every one of these
/// downgraded to `Reach::Nothing` — `cat`/`ls`/`grep`/`readlink` are all in
/// the vendored `READ_ONLY_BASES` regardless of what they are pointed at.
#[test]
#[cfg(feature = "openhuman")]
fn a_read_that_reaches_outside_the_workspace_still_parks() {
    for command in [
        "cat /etc/passwd",
        "cat ~/.ssh/id_rsa",
        "ls /root",
        "grep -r secret /etc",
        "readlink ~",
        "cat ../../secrets.env",
        "head --lines=5 /var/log/auth.log",
        "cat notes/../../../etc/passwd",
    ] {
        let c = shell(command);
        assert_eq!(
            c.reach,
            Reach::Consequence,
            "`{command}` reaches outside the workspace and must still park"
        );
        assert!(
            c.parks_under_auto(),
            "`{command}` must still park under auto"
        );
    }
}

/// The lexical backstop alone, independent of the classifier — pins the
/// exact set of shapes it does and does not flag. No `openhuman` feature
/// needed: this is pure string logic with no classifier dependency.
#[test]
fn shell_command_reaches_outside_cwd_flags_the_realistic_escapes() {
    for command in [
        "cat /etc/passwd",
        "ls ~/.ssh",
        "cat ../secret.env",
        "cat notes/../../etc/passwd",
        "head --lines=5 /var/log/auth.log",
        "cat \"/etc/passwd\"",
    ] {
        assert!(
            shell_command_reaches_outside_cwd(command),
            "`{command}` should be flagged as reaching outside the cwd"
        );
    }

    for command in [
        "cat notes.md",
        "grep -l foo session_raw/*.jsonl",
        "find . -maxdepth 4 -type d",
        "ls -la",
        "wc -l src/main.rs",
    ] {
        assert!(
            !shell_command_reaches_outside_cwd(command),
            "`{command}` stays inside the cwd and should not be flagged"
        );
    }
}

/// Everything that is not provably a read keeps exactly the verdict it had
/// before this issue: it parks, and it can hold no standing grant.
#[test]
#[cfg(feature = "openhuman")]
fn anything_that_acts_still_parks() {
    for command in [
        "rm -rf /",
        "curl https://example.com",
        "npm install -g something",
        "echo hi > file.txt",
        "git push origin main",
        "chmod 777 /etc/passwd",
    ] {
        let c = shell(command);
        assert_eq!(c.reach, Reach::Consequence, "`{command}` acts");
        assert_eq!(
            c.standing,
            Standing::PerCall,
            "`{command}` may hold no standing grant"
        );
        assert!(
            c.parks_under_auto(),
            "`{command}` must still park under auto"
        );
    }
}

// ── git_operations, graded by its `operation` (issue #877) ─────────────

fn git(operation: &str) -> Consequence {
    consequence_of(GIT_OPERATIONS, &json!({ GIT_OPERATION_KEY: operation }))
}

/// Orienting in your own workspace should not cost an operator anything.
#[test]
fn a_git_read_operation_does_not_park() {
    for operation in GIT_READ_ONLY_OPERATIONS {
        let c = git(operation);
        assert_eq!(
            c.reach,
            Reach::Nothing,
            "`git {operation}` only reads the repository"
        );
        assert!(
            !c.parks_under_auto(),
            "`git {operation}` must not interrupt anybody"
        );
    }
}

/// The writes upstream names still park. Without this the downgrade above
/// would pass against a build that stopped gating everything.
#[test]
fn a_git_write_operation_still_parks() {
    for operation in ["commit", "add", "checkout", "stash", "reset", "revert"] {
        let c = git(operation);
        assert_eq!(c.reach, Reach::Consequence, "`git {operation}` acts");
        assert!(
            c.parks_under_auto(),
            "`git {operation}` must still park under auto"
        );
    }
}

/// **The fail-closed requirement.** An operation this classifier does not
/// recognise must still ask.
///
/// The first six are real git subcommands in **neither** upstream list —
/// `requires_write_access` does not name them and `is_read_only` does not
/// either — so they are genuinely unclassified rather than merely absent
/// from a list somebody forgot to extend. `push` is the one that matters
/// most: it reaches a configured remote, which is an address this layer
/// never sees. The last two are a typo and an invented name, which is what
/// a model produces on a bad day.
///
/// This passing is the whole safety argument for the downgrade: membership
/// is affirmative, so the failure mode of an unknown operation is an extra
/// approval, never a silent act.
#[test]
fn an_unrecognised_git_operation_still_parks() {
    for operation in [
        "push",
        "pull",
        "fetch",
        "merge",
        "rebase",
        "clone",
        "stauts",
        "frobnicate",
    ] {
        let c = git(operation);
        assert_eq!(
            c.reach,
            Reach::Consequence,
            "`git {operation}` is not provably a read, so it must ask"
        );
        assert!(
            c.parks_under_auto(),
            "`git {operation}` must park under auto"
        );
    }
}

/// An argument that cannot be read gates. The tool's schema marks
/// `operation` required, so each of these is a call that could not have run
/// — guessing at one would be inventing a verdict for a call that never
/// happened.
#[test]
fn a_git_call_with_no_readable_operation_parks() {
    for args in [
        json!({}),
        json!({ GIT_OPERATION_KEY: null }),
        json!({ GIT_OPERATION_KEY: 7 }),
        json!({ GIT_OPERATION_KEY: ["status"] }),
        json!({ "op": "status" }),
    ] {
        let c = consequence_of(GIT_OPERATIONS, &args);
        assert_eq!(
            c.reach,
            Reach::Consequence,
            "unreadable args must park: {args}"
        );
    }
}

/// Case matters, matching upstream's `matches!`. `STATUS` is not `status`,
/// and a classifier that normalised case here would be answering a question
/// upstream does not ask.
#[test]
fn git_operation_matching_is_case_sensitive() {
    for operation in ["STATUS", "Status", "LOG"] {
        assert_eq!(
            git(operation).reach,
            Reach::Consequence,
            "`{operation}` is not the operation upstream classifies"
        );
    }
}

/// **The oracle.** [`GIT_READ_ONLY_OPERATIONS`] is a copy of a vendored
/// list, and a copy that can drift silently is exactly what issue #877
/// warns against. This drives the vendored judgement directly, so upstream
/// reclassifying any of these fails the build here rather than quietly
/// widening what runs unattended.
///
/// It asserts the safety-relevant direction: **none of the operations this
/// crate downgrades is a write upstream**. The converse is not assertable —
/// `is_read_only` is a private inherent method — but it is also not the
/// dangerous direction: an operation upstream calls read-only that we
/// nonetheless gate costs an approval, while the reverse would run a write
/// unattended.
///
/// `SecurityPolicy::default()` is `AutonomyLevel::Supervised`, where
/// `gate_decision(Write)` is `Prompt` — so the tier half of
/// `external_effect_with_args`'s conjunction is `true` and the expression
/// reduces to `requires_write_access(operation)` alone. That is the only
/// configuration in which this hook answers the question this crate is
/// asking, which is why the gate itself must not call it (see
/// [`git_operations_consequence`]).
#[test]
#[cfg(feature = "openhuman")]
fn the_read_only_set_matches_the_vendored_classifier() {
    use openhuman_core::security::SecurityPolicy;
    use openhuman_core::tools::{GitOperationsTool, Tool};

    let policy = std::sync::Arc::new(SecurityPolicy::default());
    let tool = GitOperationsTool::new(policy, std::path::PathBuf::from("."));

    for operation in GIT_READ_ONLY_OPERATIONS {
        assert!(
            !tool.external_effect_with_args(&json!({ GIT_OPERATION_KEY: operation })),
            "upstream now treats `git {operation}` as a write — this crate is downgrading \
             something that acts. Remove it from GIT_READ_ONLY_OPERATIONS."
        );
    }

    // And the pairing that proves the oracle is live rather than vacuous: a
    // known write must come back `true` through the same call.
    assert!(
        tool.external_effect_with_args(&json!({ GIT_OPERATION_KEY: "commit" })),
        "the oracle answered `false` for a commit, so it is not testing anything"
    );
}

/// The classifier takes the maximum across segments, so a read cannot carry
/// an act through on its coat-tails. This is the property that makes
/// downgrading reads safe at all.
#[test]
#[cfg(feature = "openhuman")]
fn a_read_chained_to_an_act_is_an_act() {
    for command in [
        "grep -r foo . && rm -rf /tmp/x",
        "ls; curl https://example.com",
        "cat a.txt | tee b.txt",
        "find . -type f > listing.txt",
    ] {
        assert_eq!(
            shell(command).reach,
            Reach::Consequence,
            "`{command}` contains an act and must park"
        );
    }
}

/// The model's own label may raise the requirement and never lower it.
#[test]
#[cfg(feature = "openhuman")]
fn a_declared_category_escalates_only() {
    // A read the model calls destructive parks…
    let escalated = consequence_of(
        SHELL,
        &json!({ SHELL_COMMAND_KEY: "ls -la", SHELL_CATEGORY_KEY: "destructive" }),
    );
    assert_eq!(escalated.reach, Reach::Consequence);

    // …and an act the model calls a read does not stop parking.
    let attempted_downgrade = consequence_of(
        SHELL,
        &json!({ SHELL_COMMAND_KEY: "rm -rf /", SHELL_CATEGORY_KEY: "read" }),
    );
    assert_eq!(attempted_downgrade.reach, Reach::Consequence);
}

/// A call this cannot read is gated. The tool's schema requires `command`,
/// so every one of these is a call that could not have run — and none of
/// them is a reason to guess.
#[test]
fn an_unreadable_shell_call_is_gated() {
    for args in [
        json!({}),
        json!({ SHELL_COMMAND_KEY: 7 }),
        json!({ SHELL_COMMAND_KEY: null }),
        json!(null),
        json!("ls"),
    ] {
        let c = consequence_of(SHELL, &args);
        assert_eq!(c.reach, Reach::Consequence, "{args}");
        assert!(c.parks_under_auto(), "{args}");
    }
}

/// The name-level declaration is untouched: every reader that asks about
/// `shell` without arguments — the permissions list, the console labels,
/// the coverage test — still sees the gated answer.
#[test]
fn the_declaration_still_reads_as_gated_without_arguments() {
    assert_eq!(c(SHELL).reach, Reach::Consequence);
}

/// Without the harness feature there is no classifier, and the fallback
/// answers "act" for everything. Nothing pinned that: the gated-call test
/// above passes only malformed arguments, which return before
/// `shell_command_is_read` is ever reached, so the fallback could regress to
/// permissive and every default-feature lane would stay green. A command
/// that IS a read under the classifier is the case that separates them.
#[test]
#[cfg(not(feature = "openhuman"))]
fn a_read_command_still_parks_when_no_classifier_is_linked_in() {
    let c = consequence_of(SHELL, &json!({ SHELL_COMMAND_KEY: "ls -la" }));
    assert_eq!(c.reach, Reach::Consequence);
    assert!(c.parks_under_auto());
}

// ── MCP bridge calls, graded against a per-server read declaration (#1124) ──

/// A `mcp_call_tool` call as the policy layer sees it.
fn mcp_call(server: &str, tool: &str) -> serde_json::Value {
    json!({
        MCP_CALL_SERVER_KEY: server,
        MCP_CALL_TOOL_KEY: tool,
        "arguments": {},
    })
}

/// A `mcp_registry_tool_call` call — different argument keys, same shape.
fn registry_call(server_id: &str, tool_name: &str) -> serde_json::Value {
    json!({
        MCP_REGISTRY_SERVER_KEY: server_id,
        MCP_REGISTRY_TOOL_KEY: tool_name,
        "arguments": {},
    })
}

/// **Acceptance criterion 1, for both tools.** A call to a server-declared
/// read-only remote tool does not park under `auto`; every other combination
/// still parks.
///
/// This is the classifier's OWN test (criterion 4): reverting
/// [`mcp_call_reach`] to return its base for the declared pair — the whole of
/// the downgrade — makes the first two assertions fail, because the declared
/// read would park again.
#[test]
fn a_declared_read_only_remote_tool_does_not_park_but_everything_else_does() {
    let reads = McpReadSet::from_pairs([
        ("jira".to_string(), "get_issue".to_string()),
        ("registry-42".to_string(), "list_rows".to_string()),
    ]);

    // The declared read on each tool downgrades and stops parking.
    let call_read = mcp_call_reach(MCP_CALL_TOOL, &mcp_call("jira", "get_issue"), &reads);
    assert_eq!(call_read.reach, Reach::ExternalRead);
    assert!(
        !call_read.parks_under_auto(),
        "a server-declared read must not park under auto"
    );
    let registry_read = mcp_call_reach(
        MCP_REGISTRY_TOOL_CALL,
        &registry_call("registry-42", "list_rows"),
        &reads,
    );
    assert_eq!(registry_read.reach, Reach::ExternalRead);
    assert!(!registry_read.parks_under_auto());

    // Every other combination still parks: a write on the same declared
    // server, a read on an undeclared server, and the same declared tool name
    // on the WRONG tool of the pair (server declared, tool not).
    for (tool, args) in [
        (MCP_CALL_TOOL, mcp_call("jira", "create_issue")),
        (MCP_CALL_TOOL, mcp_call("confluence", "get_issue")),
        (MCP_CALL_TOOL, mcp_call("jira", "list_rows")),
        (
            MCP_REGISTRY_TOOL_CALL,
            registry_call("registry-42", "write_row"),
        ),
        (
            MCP_REGISTRY_TOOL_CALL,
            registry_call("registry-99", "list_rows"),
        ),
        // The keys are not interchangeable across the two tools: a
        // registry-shaped payload under `mcp_call_tool` reads no `server`.
        (MCP_CALL_TOOL, registry_call("jira", "get_issue")),
    ] {
        let verdict = mcp_call_reach(tool, &args, &reads);
        assert_eq!(
            verdict.reach,
            Reach::Consequence,
            "`{tool}` {args} is not an affirmatively-declared read and must park"
        );
        assert!(
            verdict.parks_under_auto(),
            "`{tool}` {args} must park under auto"
        );
    }
}

/// The fail-closed base: with no declaration, every bridge call parks — the
/// verdict both tools carried before this issue, and the answer for every
/// non-harness construction site whose policy sets no read declaration.
#[test]
fn with_no_declaration_every_bridge_call_gates() {
    let empty = McpReadSet::default();
    assert!(empty.is_empty());
    for (tool, args) in [
        (MCP_CALL_TOOL, mcp_call("jira", "get_issue")),
        (MCP_REGISTRY_TOOL_CALL, registry_call("r", "get_issue")),
    ] {
        let verdict = mcp_call_reach(tool, &args, &empty);
        assert_eq!(verdict.reach, Reach::Consequence);
        assert_eq!(verdict.standing, Standing::PerCall);
        assert!(verdict.parks_under_auto());
    }
}

/// A downgraded read is `ExternalRead`, not `Nothing`: it reaches a third
/// party's server with the company's credential, so a `readonly` desk still
/// denies it and it is never billed — the Composio-read precedent (#559).
#[test]
fn a_downgraded_read_is_denied_under_readonly_and_is_not_a_spend() {
    let reads = McpReadSet::from_pairs([("jira".to_string(), "get_issue".to_string())]);
    let verdict = mcp_call_reach(MCP_CALL_TOOL, &mcp_call("jira", "get_issue"), &reads);
    assert_eq!(verdict.reach, Reach::ExternalRead);
    assert!(
        verdict.reach.denied_under_readonly(),
        "a read of a counterparty's account is exactly what readonly refuses"
    );
    assert!(!verdict.reach.costs_money(), "a read is not billed");
    assert!(
        !verdict.reach.parks_under_supervision(),
        "supervised runs it — nothing changes and nothing is spent"
    );
    assert_eq!(verdict.standing, Standing::PerCall);
}

/// A call this cannot read gates, whichever key is missing or mistyped. The
/// tools' schemas mark both required, so each of these is a call that could
/// not have run — the same fail-closed rule the other argument graders keep.
#[test]
fn an_unreadable_bridge_call_gates_even_with_a_matching_declaration() {
    let reads = McpReadSet::from_pairs([
        ("jira".to_string(), "get_issue".to_string()),
        ("r".to_string(), "get_issue".to_string()),
    ]);
    let unreadable_call = [
        json!({ MCP_CALL_TOOL_KEY: "get_issue", "arguments": {} }), // no server
        json!({ MCP_CALL_SERVER_KEY: "jira", "arguments": {} }),    // no tool
        json!({ MCP_CALL_SERVER_KEY: 7, MCP_CALL_TOOL_KEY: "get_issue" }), // non-string
        json!({ MCP_CALL_SERVER_KEY: "jira", MCP_CALL_TOOL_KEY: null }),
        json!(null),
        json!("jira"),
    ];
    for args in unreadable_call {
        let verdict = mcp_call_reach(MCP_CALL_TOOL, &args, &reads);
        assert_eq!(verdict.reach, Reach::Consequence, "unreadable: {args}");
        assert!(verdict.parks_under_auto(), "unreadable: {args}");
    }
    // …and the registry twin, under its own keys.
    for args in [
        json!({ MCP_REGISTRY_TOOL_KEY: "get_issue", "arguments": {} }),
        json!({ MCP_REGISTRY_SERVER_KEY: "r", "arguments": {} }),
        json!({ MCP_REGISTRY_SERVER_KEY: "r", MCP_REGISTRY_TOOL_KEY: 7 }),
    ] {
        let verdict = mcp_call_reach(MCP_REGISTRY_TOOL_CALL, &args, &reads);
        assert_eq!(
            verdict.reach,
            Reach::Consequence,
            "unreadable registry: {args}"
        );
    }
}

/// The tool name is matched case-insensitively, the way every other arm of
/// the gate reads it — the argument keys, and the bridge-tool predicate.
#[test]
fn the_bridge_tool_name_is_matched_case_insensitively() {
    let reads = McpReadSet::from_pairs([("jira".to_string(), "get_issue".to_string())]);
    assert!(is_mcp_bridge_tool("MCP_CALL_TOOL"));
    assert!(is_mcp_bridge_tool("Mcp_Registry_Tool_Call"));
    assert!(!is_mcp_bridge_tool("mcp_list_tools"));
    let verdict = mcp_call_reach("MCP_CALL_TOOL", &mcp_call("jira", "get_issue"), &reads);
    assert_eq!(verdict.reach, Reach::ExternalRead);
}

/// The plain `consequence_of` — which the roster, the coverage test and every
/// company-blind caller read — still sees the gated verdict for both bridge
/// tools. The downgrade lives only where the declaration does, on the policy.
#[test]
fn consequence_of_reads_both_bridge_tools_as_gated() {
    for tool in [MCP_CALL_TOOL, MCP_REGISTRY_TOOL_CALL] {
        let verdict = consequence_of(tool, &mcp_call("jira", "get_issue"));
        assert_eq!(verdict.reach, Reach::Consequence, "`{tool}`");
        assert_eq!(verdict.standing, Standing::PerCall, "`{tool}`");
        assert!(verdict.parks_under_auto(), "`{tool}`");
    }
}

/// **Acceptance criterion 3.** Both bridge tools sit on the argument-graded
/// side of the partition, so the roster and the table stay disjoint and
/// `declared_tools` enumerates each exactly once. This is a direct probe of
/// the same facts `the_roster_and_the_table_partition_the_known_tool_names`
/// enforces over the whole set, named here so a reader of this issue's change
/// sees the criterion asserted.
#[test]
fn both_bridge_tools_are_argument_graded_and_enumerated_once() {
    for tool in [MCP_CALL_TOOL, MCP_REGISTRY_TOOL_CALL] {
        assert!(
            argument_grader(tool).is_some(),
            "`{tool}` must be dispatched from its arguments"
        );
        assert_eq!(
            declared_tools().filter(|name| *name == tool).count(),
            1,
            "`{tool}` holds both a roster entry and a DECLARED row and must be enumerated once"
        );
    }
}
