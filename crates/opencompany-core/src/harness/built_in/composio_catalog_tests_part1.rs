use super::*;

/// The headline invariant: a listing too big to render says so, says how
/// much it dropped, and names the argument that makes the next call
/// smaller. Without this the agent has no reason to change its request.
#[test]
fn an_oversized_listing_says_it_was_cut_and_how_to_narrow_it() {
    let actions = catalogue("github", 300);
    let out = render(&actions, &request("", Detail::Names, &["github"]));

    assert!(out.contains("300 available"), "{out}");
    assert!(
        out.contains("TRUNCATED"),
        "the cut must be announced: {out}"
    );
    assert!(
        out.contains("100 more matching actions were not shown"),
        "the notice must count what it dropped: {out}"
    );
    assert!(
        out.contains("`search`") && out.contains("`toolkits`") && out.contains("`limit`"),
        "the notice must name the arguments that narrow it: {out}"
    );
    assert!(
        out.contains("Do NOT repeat this call unchanged"),
        "the notice must break the retry loop explicitly: {out}"
    );
}

/// Names mode prefers a *complete* list of slugs over a partial list with
/// prose. A 150-action toolkit does not fit with descriptions (they alone
/// run past the byte budget) but fits comfortably without them — so the
/// agent gets every slug it might need, plus the pointer to `detail:
/// "schemas"` for the one it picks.
#[test]
fn names_mode_drops_descriptions_rather_than_actions() {
    let actions = catalogue("github", 150);
    let out = render(&actions, &request("", Detail::Names, &["github"]));

    assert!(out.contains("showing 150"), "{out}");
    assert!(
        !out.contains("TRUNCATED"),
        "nothing had to be dropped: {out}"
    );
    assert!(out.contains("Descriptions omitted"), "{out}");
    assert!(
        out.contains("GITHUB_ACTION_149"),
        "the last slug must be present: {out}"
    );
    assert!(
        !out.contains("Long upstream prose"),
        "the dense fallback must not carry descriptions: {out}"
    );
    assert!(out.len() < 16 * 1024, "{} bytes", out.len());
}

/// The bound is real, not advisory — and it is a whole-entry bound, so the
/// dropped count in the notice is exact rather than approximate.
#[test]
fn rendering_stays_inside_its_byte_budget_and_never_splits_an_entry() {
    // 300 schema blocks at ~500 bytes each is far past the budget.
    let actions = catalogue("gmail", 300);
    let mut request = request("", Detail::Schemas, &["gmail"]);
    request.limit = SCHEMAS_MAX_LIMIT;
    let out = render(&actions, &request);

    assert!(
        out.len() < 16 * 1024,
        "the rendered listing must stay under the harness tool-result cap: {} bytes",
        out.len()
    );
    // Every block that IS present is complete: its trailing newline pair and
    // its `parameters:` line both survived.
    let blocks = out.matches("## GMAIL_ACTION_").count();
    assert_eq!(
        out.matches("parameters: {").count(),
        blocks,
        "an entry was cut in half: {out}"
    );
    assert!(blocks >= 1, "at least one schema must be delivered: {out}");
}

/// A listing that fits is not decorated with a truncation notice — the
/// notice has to mean something.
#[test]
fn a_complete_listing_carries_no_truncation_notice() {
    let actions = catalogue("linear", 12);
    let out = render(&actions, &request("", Detail::Names, &["linear"]));
    assert!(out.contains("showing 12"), "{out}");
    assert!(!out.contains("TRUNCATED"), "{out}");
}

/// Narrowing is generic: the same words find the one action in a
/// hundred-slug catalogue for any toolkit, with no per-provider table.
#[test]
fn search_narrows_to_one_action_on_any_toolkit() {
    let mut actions = catalogue("github", 120);
    actions.push(CatalogAction {
        slug: "GITHUB_LIST_ISSUES".to_string(),
        toolkit: "github".to_string(),
        description: "List issues in a repository.".to_string(),
        parameters: Some(json!({"type": "object", "properties": {"repo": {"type": "string"}}})),
    });
    let mut other = catalogue("notion", 130);
    other.push(CatalogAction {
        slug: "NOTION_SEARCH_PAGES".to_string(),
        toolkit: "notion".to_string(),
        description: "Search pages in the workspace.".to_string(),
        parameters: Some(
            json!({"type": "object", "properties": {"query": {"type": "string"}}}),
        ),
    });

    let github = render(
        &actions,
        &request("list issues", Detail::Schemas, &["github"]),
    );
    assert!(github.contains("GITHUB_LIST_ISSUES"), "{github}");
    assert!(
        github.contains("\"repo\""),
        "the schema must be present: {github}"
    );
    assert!(
        !github.contains("TRUNCATED"),
        "one match is not a cut: {github}"
    );

    let notion = render(
        &other,
        &request("search pages", Detail::Schemas, &["notion"]),
    );
    assert!(notion.contains("NOTION_SEARCH_PAGES"), "{notion}");
    assert!(notion.contains("\"query\""), "{notion}");
}

/// An exact slug read back from the names view resolves to that one schema
/// — the second half of the two-step, and the reason `_` is a search
/// separator.
#[test]
fn an_exact_slug_pasted_into_search_returns_that_schema() {
    let mut actions = catalogue("slack", 90);
    actions.push(CatalogAction {
        slug: "SLACK_POST_MESSAGE".to_string(),
        toolkit: "slack".to_string(),
        description: "Post a message to a channel.".to_string(),
        parameters: Some(
            json!({"type": "object", "properties": {"channel": {"type": "string"}}}),
        ),
    });
    let out = render(
        &actions,
        &request("SLACK_POST_MESSAGE", Detail::Schemas, &["slack"]),
    );
    assert!(out.contains("SLACK_POST_MESSAGE"), "{out}");
    assert!(out.contains("\"channel\""), "{out}");
    assert!(out.contains("showing 1"), "{out}");
}

/// No match is reported as a fact, with the words that produced it, and
/// with an explicit instruction not to invent a slug.
#[test]
fn no_match_is_stated_plainly_rather_than_returned_empty() {
    let actions = catalogue("gmail", 40);
    let out = render(
        &actions,
        &request("quantum teleport", Detail::Names, &["gmail"]),
    );
    assert!(out.contains("nothing matched `quantum teleport`"), "{out}");
    assert!(
        out.contains("40 returned"),
        "the real total is stated: {out}"
    );
    assert!(out.contains("Do NOT guess a slug"), "{out}");
    assert!(!out.contains("TRUNCATED"), "nothing was cut: {out}");
}

/// A curated listing must not present itself as the full catalogue.
///
/// An unnarrowed BYOK browse asks Composio for featured actions only, so
/// the count that comes back is a preview. Calling it "available" is what
/// told an agent taking an inventory of GitHub that ~50 rows were
/// everything it could do (codex on tinyhumansai/opencompany#2153).
#[test]
fn a_curated_listing_says_it_is_a_preview() {
    let actions = catalogue("github", 50);
    let mut curated = request("", Detail::Names, &["github"]);
    curated.curated = true;
    let out = render(&actions, &curated);

    assert!(out.contains("featured"), "curation must be named: {out}");
    assert!(
        out.contains("not the full catalogue"),
        "the preview must say what it is not: {out}"
    );
    assert!(
        out.contains("search"),
        "the way to reach the rest must be given: {out}"
    );
    assert!(
        !out.contains("50 available"),
        "a curated count is not what is available: {out}"
    );

    // An unnarrowed listing that was NOT curated still reports plainly.
    let plain = render(&actions, &request("", Detail::Names, &["github"]));
    assert!(plain.contains("50 available"), "{plain}");
    assert!(!plain.contains("featured"), "{plain}");
}

/// A server-side filter that matches nothing says so about the *filter*.
///
/// Once `search` and `tags` travel to Composio, a narrowed query that
/// matches nothing comes back with zero rows — and the old message read
/// that as "this toolkit has no callable actions (it may not be
/// connected)". That is a lie about a connected toolkit, and the expensive
/// kind: an agent told a capability does not exist stops looking for it,
/// which is the failure this whole listing was rewritten to end (codex on
/// tinyhumansai/opencompany#2153).
#[test]
fn an_empty_server_filtered_response_does_not_blame_the_connection() {
    let mut narrowed = request("quantum teleport", Detail::Names, &["gmail"]);
    narrowed.tags = vec!["important".to_string()];
    // Zero rows back, because the server did the filtering.
    let out = render(&[], &narrowed);

    assert!(
        !out.contains("may not be connected"),
        "an empty filter result says nothing about the connection: {out}"
    );
    assert!(
        !out.contains("no callable actions"),
        "the toolkit was not shown to be empty: {out}"
    );
    assert!(
        out.contains("quantum teleport") && out.contains("important"),
        "both halves of the filter are named: {out}"
    );
    assert!(
        out.contains("not what the toolkit has"),
        "the count must be disclosed as the filter's, not the toolkit's: {out}"
    );

    // The unnarrowed empty case still points at the connection, which is
    // the one time that is the right thing to say.
    let bare = render(&[], &request("", Detail::Names, &["github"]));
    assert!(bare.contains("may not be connected"), "{bare}");
}

/// An empty catalogue is a different fact from an empty search, and points
/// at the connection rather than at the search words.
#[test]
fn an_empty_catalogue_points_at_the_connection() {
    let out = render(&[], &request("", Detail::Names, &["github"]));
    assert!(out.contains("none"), "{out}");
    assert!(out.contains("composio_list_connections"), "{out}");
}

/// A single schema larger than the whole budget is still delivered — a
/// correctly-described way of being useless is still useless.
#[test]
fn a_single_oversized_schema_is_still_delivered_whole() {
    let actions = vec![CatalogAction {
        slug: "GIANT_ACTION".to_string(),
        toolkit: "giant".to_string(),
        description: "One enormous schema.".to_string(),
        parameters: Some(json!({"type": "object", "blob": "z".repeat(MAX_RENDER_BYTES * 2)})),
    }];
    let out = render(&actions, &request("", Detail::Schemas, &["giant"]));
    assert!(out.contains("GIANT_ACTION"), "{out}");
    assert!(
        out.contains(&"z".repeat(1000)),
        "the only matching schema must survive whole"
    );
    assert!(!out.contains("TRUNCATED"), "nothing was dropped: {out}");
}

/// Argument parsing: the defaults are the cheap ones, an over-large `limit`
/// is clamped rather than rejected, and an unknown `detail` degrades to the
/// names view instead of dumping 200 schemas.
#[test]
fn arguments_default_and_clamp_rather_than_failing() {
    let names = ListRequest::parse(&json!({}), vec!["github".into()]);
    assert_eq!(names.detail, Detail::Names);
    assert_eq!(names.limit, NAMES_DEFAULT_LIMIT);
    assert!(names.search.is_empty());

    let schemas = ListRequest::parse(&json!({"detail": "schemas"}), Vec::new());
    assert_eq!(schemas.limit, SCHEMAS_DEFAULT_LIMIT);

    let clamped = ListRequest::parse(&json!({"detail": "schemas", "limit": 9999}), Vec::new());
    assert_eq!(clamped.limit, SCHEMAS_MAX_LIMIT);

    let typo = ListRequest::parse(&json!({"detail": "everything"}), Vec::new());
    assert_eq!(
        typo.detail,
        Detail::Names,
        "an unknown detail must not dump schemas"
    );

    let terms = ListRequest::parse(&json!({"search": "  List   Issues "}), Vec::new());
    assert_eq!(terms.search, vec!["list".to_string(), "issues".to_string()]);
}

/// The advertised argument names and the ones the parser reads are the same
/// literals — a filter the model is told about but the tool ignores would
/// be the same bug wearing a different hat.
#[test]
fn the_advertised_schema_matches_the_arguments_the_parser_reads() {
    let schema = list_tools_parameters_schema();
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .expect("properties object");
    for key in ["toolkits", "search", "detail", "limit"] {
        assert!(properties.contains_key(key), "`{key}` is not advertised");
    }
    let description = list_tools_description();
    assert!(description.contains("search"), "{description}");
    assert!(description.contains("schemas"), "{description}");
    assert!(
        description.contains("truncated"),
        "the model must be told results can be cut: {description}"
    );

    let toolkits = list_toolkits_parameters_schema();
    let properties = toolkits
        .get("properties")
        .and_then(Value::as_object)
        .expect("properties object");
    for key in ["search", "limit"] {
        assert!(properties.contains_key(key), "`{key}` is not advertised");
    }
    assert!(list_toolkits_description().contains("search"));
}

// ── the root cause: a message cap applied to a body ────────────────

/// The regression guard for #410's actual root cause.
///
/// A tool body must never be sized by
/// [`SCRUB_MAX_BYTES`](crate::harness::mcp_probe::SCRUB_MAX_BYTES). That
/// constant is a 300-byte cap on a one-line operator message, and routing a
/// successful Composio result through it is what turned a 260-action
/// catalogue into "the first action and half of its schema, ending in `…`".
/// This pins the two halves apart: redaction never shortens, the body bound
/// is orders of magnitude larger than the message bound, and a bounded body
/// says so.
#[test]
fn a_tool_body_is_bounded_as_a_body_not_as_a_message() {
    use crate::harness::mcp_probe::{SCRUB_MAX_BYTES, redact, scrub};

    let body = format!("secret-token {}", "payload ".repeat(4_000));
    let secrets = vec!["secret-token".to_string()];

    // Redaction still redacts — that half was never the bug.
    let redacted = redact(&body, &secrets);
    assert!(
        !redacted.contains("secret-token"),
        "the token survived redaction"
    );
    assert!(redacted.contains("•••"));
    // …and it does NOT shorten. The old path lost 99% of the payload here.
    assert!(
        redacted.len() > SCRUB_MAX_BYTES * 10,
        "redact must not apply the message cap: {} bytes",
        redacted.len()
    );
    assert!(
        scrub(&body, &secrets).len() <= SCRUB_MAX_BYTES + 3,
        "scrub keeps the message cap for the messages it was built for"
    );

    // The body bound is a body bound, and it announces itself.
    const {
        assert!(
            MAX_BODY_BYTES > SCRUB_MAX_BYTES * 20,
            "a body budget sized like a message budget is the bug"
        )
    };
    let bounded = bound_body(redacted.clone(), "`GITHUB_LIST_ISSUES` output");
    assert!(
        bounded.contains("TRUNCATED"),
        "an oversized body must say so"
    );
    assert!(
        bounded.contains("bytes longer than this"),
        "the notice must quantify what was lost: {bounded}"
    );
    assert!(bounded.contains("`GITHUB_LIST_ISSUES` output"), "{bounded}");

    // A body that fits is returned untouched — no decoration, no marker.
    let small = "a short provider response".to_string();
    assert_eq!(bound_body(small.clone(), "output"), small);
}

// ── the toolkit listing ────────────────────────────────────────────

fn toolkit_catalogue(count: usize) -> Vec<CatalogToolkit> {
    (0..count)
        .map(|i| CatalogToolkit {
            slug: format!("toolkit{i:03}"),
            name: format!("Toolkit {i}"),
            description: "An integration with a long upstream description. ".repeat(4),
            connected: Some(i % 3 == 0),
        })
        .collect()
}

/// The toolkit catalogue is the same bug one level up, so it gets the same
/// self-describing cut.
#[test]
fn an_oversized_toolkit_listing_says_it_was_cut_and_how_to_narrow_it() {
    let toolkits = toolkit_catalogue(400);
    let out = render_toolkits(&toolkits, &ToolkitListRequest::parse(&json!({})));
    assert!(out.contains("400 available"), "{out}");
    assert!(out.contains("TRUNCATED"), "{out}");
    assert!(
        out.contains("more matching toolkits were not shown"),
        "{out}"
    );
    assert!(
        out.contains("`search`") || out.contains("\"search\""),
        "{out}"
    );
    assert!(
        out.len() < 16 * 1024,
        "the listing must stay under the harness cap: {} bytes",
        out.len()
    );
}

/// Narrowing and the connected marker, which is what an agent actually
/// needs before it reaches for `composio_authorize`.
#[test]
fn toolkit_search_narrows_and_marks_connection_state() {
    let mut toolkits = toolkit_catalogue(50);
    toolkits.push(CatalogToolkit {
        slug: "googlecalendar".to_string(),
        name: "Google Calendar".to_string(),
        description: "Read and write calendar events.".to_string(),
        connected: Some(true),
    });
    let out = render_toolkits(
        &toolkits,
        &ToolkitListRequest::parse(&json!({"search": "calendar"})),
    );
    assert!(
        out.contains("googlecalendar (Google Calendar) [connected]"),
        "{out}"
    );
    assert!(out.contains("showing 1"), "{out}");
    assert!(!out.contains("TRUNCATED"), "{out}");
}

/// A company with no integrations at all is a different fact from a search
/// that matched nothing, and both are stated rather than returned empty.
#[test]
fn an_empty_toolkit_catalogue_and_an_empty_search_read_differently() {
    let none = render_toolkits(&[], &ToolkitListRequest::parse(&json!({})));
    assert!(none.contains("none available"), "{none}");

    let no_match = render_toolkits(
        &toolkit_catalogue(5),
        &ToolkitListRequest::parse(&json!({"search": "zzz"})),
    );
    assert!(no_match.contains("0 matching `zzz`"), "{no_match}");
    assert!(
        no_match.contains("Do NOT guess a toolkit slug"),
        "{no_match}"
    );
}

// -----------------------------------------------------------------------
// The capability-grounding + Composio-first routing brief (issue #1759)
// -----------------------------------------------------------------------

/// The brief must name the concrete Composio tools an agent holds and the
/// two-step it reasons in, or it re-creates the unmentioned-tool failure the
/// sandbox brief exists to stop, one surface over.
#[test]
fn the_composio_brief_names_the_tools_and_the_two_step() {
    let brief = composio_brief(&["github".to_string()], &[]);
    for tool in [
        "composio_list_toolkits",
        "composio_list_connections",
        "composio_list_tools",
        "composio_execute",
        "composio_authorize",
    ] {
        assert!(brief.contains(tool), "brief never names `{tool}`: {brief}");
    }
}

/// The routing rule is the whole point: GitHub / connected SaaS go through
/// Composio, and the raw web tools are named as the wrong door (they answer
/// 401/403 unauthenticated). The observed failure — `api.github.com` via
/// `http_request` — must be called out by name.
#[test]
fn the_composio_brief_routes_provider_apis_through_composio_not_the_web_tools() {
    let brief = composio_brief(&["github".to_string()], &[]);
    for web_tool in ["http_request", "curl", "web_fetch"] {
        assert!(
            brief.contains(web_tool),
            "the brief must warn off `{web_tool}`: {brief}"
        );
    }
    assert!(brief.contains("api.github.com"), "{brief}");
    assert!(brief.contains("401") || brief.contains("403"), "{brief}");
}

/// The grounding half: the agent is told not to promise an action it has no
/// tool for, with the exact browser overreach the issue observed named.
#[test]
fn the_composio_brief_forbids_promising_actions_it_has_no_tool_for() {
    let brief = composio_brief(&["github".to_string()], &[]);
    let lower = brief.to_lowercase();
    assert!(lower.contains("no browser"), "{brief}");
    assert!(lower.contains("do not promise"), "{brief}");
}

/// PR #1780 review (round 5): `composio_brief` is rendered whenever the
/// Composio tools are wired, with no view of whether the agent was
/// SEPARATELY granted an MCP browser tool (Browserbase and friends —
/// `build_agent`'s MCP bridge is wholly independent of the Composio grant,
/// see `build.rs`). The old wording claimed "you have no browser" as an
/// unconditional fact, which is false on that combination and could make
/// the agent refuse a browser action it actually holds a tool for. The
/// claim must be qualified on "unless separately granted a browser tool"
/// (or equivalent), not stated as an absolute.
#[test]
fn the_composio_brief_does_not_unconditionally_deny_holding_a_browser_tool() {
    let brief = composio_brief(&["github".to_string()], &[]);
    let lower = brief.to_lowercase();
    assert!(
        lower.contains("unless you were separately granted a browser tool")
            || lower.contains("unless separately granted a browser tool"),
        "the no-browser claim must be qualified, not absolute, since MCP can grant one \
         independently of Composio: {brief}"
    );
}

/// A non-empty allowlist names exactly those toolkits, lowercased, so the
/// agent is grounded in what THIS company connected rather than a generic
/// list.
#[test]
fn the_composio_brief_names_the_connected_toolkits_lowercased() {
    let brief = composio_brief(&["GitHub".to_string(), " Gmail ".to_string()], &[]);
    assert!(
        brief.contains("Connected toolkits for this company: github, gmail"),
        "{brief}"
    );
}

/// PR #1780 review (round 6): a non-empty allowlist that excludes GitHub
/// (e.g. `["slack"]`) must not tell the agent it can reach GitHub through
/// Composio — the live tools enforce the allowlist and would reject the
/// authorization/execution, routing a GitHub task toward a capability the
/// agent does not hold.
#[test]
fn the_composio_brief_does_not_advertise_github_outside_a_restricting_allowlist() {
    let brief = composio_brief(&["slack".to_string()], &[]);
    assert!(
        !brief.to_lowercase().contains("github"),
        "an allowlist that excludes GitHub must not name it as reachable: {brief}"
    );
    assert!(
        brief.contains("Connected toolkits for this company: slack"),
        "{brief}"
    );
    // The routing rule and grounding still hold generically.
    assert!(brief.contains("composio_execute"), "{brief}");
    assert!(brief.contains("http_request"), "{brief}");
}

/// Open mode (an empty allowlist) must NOT invent a provider the company may
/// not have connected — it points the agent at `composio_list_connections`
/// to discover the real set instead. This is requirement #3: never advertise
/// a toolkit that is not known-connected.
#[test]
fn the_composio_brief_open_mode_points_at_discovery_without_naming_a_provider() {
    let brief = composio_brief(&[], &[]);
    assert!(
        brief.contains("not fixed here"),
        "open mode must defer to discovery: {brief}"
    );
    assert!(
        !brief.contains("Connected toolkits for this company:"),
        "open mode must not claim a specific connected set: {brief}"
    );
    // The routing rule and grounding still hold with no allowlist.
    assert!(brief.contains("composio_execute"), "{brief}");
    assert!(brief.contains("http_request"), "{brief}");
}

/// PR #1780 review (round 7): an empty allowlist means the CLIENT applies
/// no restriction — the backend's own server-enforced allowlist still
/// decides what is actually connected (see `TenantComposio::toolkits`'s
/// doc comment). It is not proof that GitHub, specifically, is connected.
/// Naming GitHub in the "GitHub and other SaaS" heading before
/// `composio_list_connections` has run promises a capability the agent may
/// not hold — the same class of bug requirement #3 (the discovery-pointer
/// test above) already guards against for the "Connected toolkits for
/// this company:" line.
#[test]
fn the_composio_brief_open_mode_does_not_name_github_before_discovery() {
    let brief = composio_brief(&[], &[]);
    assert!(
        !brief.contains("Connected integrations (GitHub and other SaaS)")
            && !brief.contains("You reach GitHub"),
        "open mode must not claim GitHub is reachable before discovery: {brief}"
    );
    // The routing rule and grounding still hold generically.
    assert!(brief.contains("composio_execute"), "{brief}");
    assert!(brief.contains("http_request"), "{brief}");
    // ...and the guardrail keeps its concrete example. Naming
    // `api.github.com` as something NOT to hand-roll is the opposite of
    // claiming GitHub is reachable, and the turn test
    // `the_composio_routing_brief_reaches_the_model_system_prompt`
    // asserts the model actually sees it. An earlier revision of this
    // test forbade the substring "github" outright, which took the
    // example down with the claim and broke that guarantee.
    assert!(brief.contains("api.github.com"), "{brief}");
}

/// A native capability the agent already holds a built-in tool for is named
/// as such, and told to be used directly rather than routed through Composio.
#[test]
fn the_composio_brief_names_native_capabilities_as_built_in_not_composio() {
    let brief = composio_brief(&["github".to_string()], &["search"]);
    assert!(
        brief.contains("built-in tools of your own: search"),
        "the native capability must be named: {brief}"
    );
    assert!(
        brief.contains("do not route them through Composio"),
        "the precedence must tell the agent to use the built-in tool directly: {brief}"
    );
}

/// Empty native caps render no precedence line, leaving the brief as it was
/// before native-first routing — and the S2 raw-HTTP deflection warning is
/// untouched either way.
#[test]
fn the_composio_brief_with_no_native_caps_is_unchanged_and_keeps_the_deflection_warning() {
    let brief = composio_brief(&["github".to_string()], &[]);
    assert!(
        !brief.contains("built-in tools of your own"),
        "no native caps must add no precedence line: {brief}"
    );
    // The S2 raw-HTTP deflection warning is verbatim regardless.
    assert!(brief.contains("http_request"), "{brief}");
    assert!(brief.contains("api.github.com"), "{brief}");
    assert!(brief.contains("401") || brief.contains("403"), "{brief}");
}

/// The two levers coexist on one brief: a connected toolkit is still routed
/// through Composio while a native capability is called out as built-in.
#[test]
fn the_composio_brief_routes_composio_toolkits_and_native_caps_separately() {
    let brief = composio_brief(&["gmail".to_string()], &["search"]);
    assert!(
        brief.contains("Connected toolkits for this company: gmail"),
        "the connected toolkit is still routed through Composio: {brief}"
    );
    assert!(
        brief.contains("built-in tools of your own: search"),
        "the native capability is called out as built-in: {brief}"
    );
}

// -----------------------------------------------------------------------
// The http_request deflection guardrail (issue #1759, slice S2)
// -----------------------------------------------------------------------

/// The headline case the guardrail exists for: a raw call to
/// `api.github.com` when `github` is connected is deflected, and the refusal
/// names the Composio route the agent should have taken.
#[test]
fn a_connected_provider_host_is_deflected_with_the_composio_route() {
    let connected = vec!["github".to_string()];
    let reason = web_call_deflection(&connected, "https://api.github.com/repos/o/r/issues")
        .expect("a connected provider host must be deflected");
    assert!(reason.contains("api.github.com"), "{reason}");
    assert!(reason.contains("composio_execute"), "{reason}");
    assert!(reason.contains("composio_list_tools"), "{reason}");
    assert!(
        reason.contains("401") || reason.contains("403"),
        "the refusal must explain the unauthenticated failure: {reason}"
    );
}

/// Requirement #2: the SAME host passes through untouched when its toolkit is
/// NOT connected — the company may legitimately hit a public endpoint of a
/// provider it has not wired.
#[test]
fn the_same_host_passes_through_when_its_toolkit_is_not_connected() {
    // Some other toolkit is connected, but not github.
    let connected = vec!["slack".to_string()];
    assert!(
        web_call_deflection(&connected, "https://api.github.com/repos/o/r").is_none(),
        "an unconnected provider host must pass through"
    );
    // And with nothing connected at all.
    assert!(
        web_call_deflection(&[], "https://api.github.com/repos/o/r").is_none(),
        "no connected toolkits means no deflection"
    );
}

/// A non-provider host is never deflected, whatever is connected.
#[test]
fn a_non_provider_host_always_passes_through() {
    let connected = vec!["github".to_string(), "gmail".to_string()];
    assert!(web_call_deflection(&connected, "https://example.com/data.json").is_none());
    assert!(web_call_deflection(&connected, "https://raw.githubusercontent.com/x").is_none());
}

/// Sub-domains of a connected provider's API host are caught; the connected
/// list is normalised (trim + lowercase) the same way [`composio_brief`]
/// normalises it, so a manifest `"GitHub"` still matches.
#[test]
fn subdomains_are_caught_and_the_connected_list_is_normalised() {
    let connected = vec![" GitHub ".to_string()];
    assert!(
        web_call_deflection(&connected, "https://uploads.api.github.com/x").is_some(),
        "a sub-domain of the API host must be deflected"
    );
}

/// PR #1780 review (round 12): `https://api.github.com./repos/o/r` is a
/// valid absolute-FQDN spelling — the trailing dot is the DNS root label
/// and resolves to the exact same host — but `Url::host_str` keeps that
/// dot verbatim, so before this fix neither the equality nor the
/// subdomain-suffix arm of `host_is` matched it and the request bypassed
/// deflection entirely.
#[test]
fn a_trailing_dot_fqdn_still_matches_the_provider_host() {
    let connected = vec!["github".to_string()];
    assert!(
        web_call_deflection(&connected, "https://api.github.com./repos/o/r").is_some(),
        "the root-label trailing dot must not defeat the host match"
    );
}

/// A URL that does not parse to a host is not a provider call — it passes
/// through rather than panicking or denying.
#[test]
fn an_unparseable_url_passes_through() {
    let connected = vec!["github".to_string()];
    assert!(web_call_deflection(&connected, "not a url").is_none());
    assert!(web_call_deflection(&connected, "file:///etc/hosts").is_none());
}

/// PR #1780 review: Slack's Web API is served from `slack.com/api/*`, but
/// `slack.com` also hosts Slack's public marketing/help pages. A
/// `web_fetch` of one of those pages needs no Composio connection and has
/// no equivalent Composio action, so it must pass through untouched even
/// when `slack` is connected — only the `/api/` path is deflected.
#[test]
fn slack_deflection_is_scoped_to_the_api_path_not_the_whole_domain() {
    let connected = vec!["slack".to_string()];
    assert!(
        web_call_deflection(&connected, "https://slack.com/api/chat.postMessage").is_some(),
        "a real Slack Web API call must still be deflected"
    );
    assert!(
        web_call_deflection(&connected, "https://slack.com/help/some-public-article").is_none(),
        "a public slack.com page outside /api/ must pass through"
    );
    assert!(
        web_call_deflection(&connected, "https://slack.com/").is_none(),
        "the bare domain root must pass through"
    );
}

/// Same shape as Slack: Discord's REST API is served from
/// `discord.com/api/*`, but `discord.com` also hosts the main web client
/// and public invite/marketing pages.
#[test]
fn discord_deflection_is_scoped_to_the_api_path_not_the_whole_domain() {
    let connected = vec!["discord".to_string()];
    assert!(
        web_call_deflection(&connected, "https://discord.com/api/v10/users/@me").is_some(),
        "a real Discord API call must still be deflected"
    );
    assert!(
        web_call_deflection(&connected, "https://discord.com/invite/somepublicserver")
            .is_none(),
        "a public discord.com page outside /api/ must pass through"
    );
}

/// `www.googleapis.com` is Google's shared legacy gateway for many APIs,
/// not just Drive — unlike Slack/Discord this is one host fronting several
/// unrelated *products*, not a product mixing API and public-page traffic.
/// Before the fix this entry had no path prefix, so a `googledrive`
/// connection deflected `www.googleapis.com/youtube/v3/...` to the Drive
/// toolkit even though Drive cannot serve it (PR #1780 review).
///
/// PR #1780 review (round 8): like the `/upload/drive/` sibling prefix
/// above, batching several Drive calls into one request goes to
/// `www.googleapis.com/batch/drive/v3` — a third sibling prefix under the
/// same shared gateway host, not a deeper path under `/drive/`. Before
/// this fix `googledrive`'s prefix set had no `/batch/drive/` entry, so a
/// batch request passed straight through instead of being deflected.
#[test]
fn drive_deflection_on_the_legacy_host_is_scoped_to_drive_paths() {
    let connected = vec!["googledrive".to_string()];
    assert!(
        web_call_deflection(&connected, "https://www.googleapis.com/drive/v3/files").is_some(),
        "a real Drive call on the legacy gateway host must still be deflected"
    );
    assert!(
        web_call_deflection(
            &connected,
            "https://www.googleapis.com/youtube/v3/search?q=rust"
        )
        .is_none(),
        "an unrelated Google API sharing the legacy gateway host must pass through"
    );
    assert!(
        web_call_deflection(&connected, "https://drive.googleapis.com/drive/v3/files")
            .is_some(),
        "the dedicated Drive host stays deflected unscoped"
    );
    assert!(
        web_call_deflection(
            &connected,
            "https://www.googleapis.com/upload/drive/v3/files"
        )
        .is_some(),
        "the resumable/media upload route on the legacy gateway host must also be deflected"
    );
    assert!(
        web_call_deflection(&connected, "https://www.googleapis.com/batch/drive/v3").is_some(),
        "the batch endpoint on the legacy gateway host must also be deflected, the same as \
         Drive's sibling /upload/drive/ prefix"
    );
}

/// PR #1780 review: `api.atlassian.com` only covers the OAuth-3LO gateway.
/// The Jira REST API an agent plausibly curls by hand lives on the
/// tenant's own domain, `<site>.atlassian.net/rest/api/...` — before the
/// fix that host was not in the table at all, so this request passed
/// straight through with no credential instead of being deflected to
/// Composio. The same tenant host also serves the ordinary Jira web UI,
/// so the match must stay scoped to `/rest/api/`.
#[test]
fn jira_deflection_covers_the_tenant_specific_atlassian_net_host() {
    let connected = vec!["jira".to_string()];
    assert!(
        web_call_deflection(
            &connected,
            "https://my-company.atlassian.net/rest/api/3/issue/PROJ-1"
        )
        .is_some(),
        "a tenant's Jira Cloud REST API call must be deflected"
    );
    assert!(
        web_call_deflection(
            &connected,
            "https://my-company.atlassian.net/jira/software/projects/PROJ/boards/1"
        )
        .is_none(),
        "the tenant's public Jira web UI outside /rest/api/ must pass through"
    );
    assert!(
        web_call_deflection(
            &connected,
            "https://api.atlassian.com/ex/jira/some-cloud-id/rest/api/3/issue/PROJ-1"
        )
        .is_some(),
        "a Jira call through the OAuth-3LO gateway stays deflected"
    );
    assert!(
        web_call_deflection(
            &connected,
            "https://api.atlassian.com/ex/confluence/some-cloud-id/rest/api/content"
        )
        .is_none(),
        "another Atlassian product on the shared gateway must pass through — \
         a jira connection is not a Confluence one"
    );
    assert!(
        web_call_deflection(
            &connected,
            "https://my-company.atlassian.net/rest/agile/1.0/board"
        )
        .is_some(),
        "Jira Software's Agile REST family on the tenant host must also be deflected"
    );
}

/// PR #1780 review (finding 6): `www.googleapis.com` is the same shared
/// legacy gateway for Calendar as it is for Drive. Before the fix
/// `googlecalendar` only recognised `calendar.googleapis.com`, so the
/// standard REST URL most examples and agents actually curl,
/// `www.googleapis.com/calendar/v3/...`, passed straight through with no
/// credential instead of being deflected to Composio.
///
/// PR #1780 review (round 9): like Drive's `/batch/drive/` sibling prefix
/// (round 8), Calendar's batch endpoint on the shared gateway,
/// `www.googleapis.com/batch/calendar/v3`, is a sibling of `/calendar/`,
/// not a deeper path under it. Before this fix `googlecalendar`'s prefix
/// set had no `/batch/calendar/` entry, so a batch request passed
/// straight through instead of being deflected.
#[test]
fn calendar_deflection_on_the_legacy_host_is_scoped_to_calendar_paths() {
    let connected = vec!["googlecalendar".to_string()];
    assert!(
        web_call_deflection(
            &connected,
            "https://www.googleapis.com/calendar/v3/calendars/primary/events"
        )
        .is_some(),
        "a real Calendar call on the legacy gateway host must be deflected"
    );
    assert!(
        web_call_deflection(
            &connected,
            "https://www.googleapis.com/youtube/v3/search?q=rust"
        )
        .is_none(),
        "an unrelated Google API sharing the legacy gateway host must pass through"
    );
    assert!(
        web_call_deflection(
            &connected,
            "https://calendar.googleapis.com/calendar/v3/calendars/primary/events"
        )
        .is_some(),
        "the dedicated Calendar host stays deflected unscoped"
    );
    assert!(
        web_call_deflection(&connected, "https://www.googleapis.com/batch/calendar/v3")
            .is_some(),
        "the batch endpoint on the legacy gateway host must also be deflected, the same as \
         Drive's sibling /batch/drive/ prefix"
    );
}

/// Same shape as Calendar and Drive: `gmail.googleapis.com` is the
/// dedicated host, but `www.googleapis.com/gmail/v1/...` is the same
/// shared legacy gateway an agent plausibly curls by hand. Before the fix
/// the table only had the dedicated host, so this call passed through
/// unscoped.
///
/// PR #1780 review: like Drive's `/upload/drive/` sibling prefix
/// (finding 7), Gmail's legacy-gateway media/resumable upload route for
/// sending or importing a message with an attachment is
/// `/upload/gmail/v1/users/me/messages/send`, not `/gmail/v1/...` — a
/// sibling path prefix under the same host, not a deeper path under
/// `/gmail/`. Before the fix `gmail`'s prefix set only had `/gmail/`, so
/// this raw unauthenticated request passed straight through instead of
/// being deflected to Composio.
///
/// PR #1780 review (round 10): like Drive's and Calendar's `/batch/`
/// sibling prefixes (rounds 8 and 9), batching several Gmail calls into
/// one request goes to `www.googleapis.com/batch/gmail/v1` — another
/// sibling of `/gmail/`, not a deeper path under it. Before this fix
/// Gmail's prefix set had no `/batch/gmail/` entry, so a batch request
/// passed straight through instead of being deflected.
#[test]
fn gmail_deflection_on_the_legacy_host_is_scoped_to_gmail_paths() {
    let connected = vec!["gmail".to_string()];
    assert!(
        web_call_deflection(
            &connected,
            "https://www.googleapis.com/gmail/v1/users/me/messages"
        )
        .is_some(),
        "a real Gmail call on the legacy gateway host must be deflected"
    );
    assert!(
        web_call_deflection(&connected, "https://www.googleapis.com/drive/v3/files").is_none(),
        "an unrelated Google API sharing the legacy gateway host must pass through"
    );
    assert!(
        web_call_deflection(
            &connected,
            "https://gmail.googleapis.com/gmail/v1/users/me/messages"
        )
        .is_some(),
        "the dedicated Gmail host stays deflected unscoped"
    );
    assert!(
        web_call_deflection(
            &connected,
            "https://www.googleapis.com/upload/gmail/v1/users/me/messages/send"
        )
        .is_some(),
        "the legacy media/resumable upload route on the shared gateway host must also be \
         deflected, the same as Drive's sibling /upload/drive/ prefix"
    );
    assert!(
        web_call_deflection(&connected, "https://www.googleapis.com/batch/gmail/v1").is_some(),
        "the batch endpoint on the legacy gateway host must also be deflected, the same as \
         Drive's sibling /batch/drive/ and Calendar's sibling /batch/calendar/ prefixes"
    );
}

/// PR #1780 review (round 5): `api.github.com` is GitHub's dedicated REST
/// host, but release-asset uploads are served from a SIBLING host,
/// `uploads.github.com` — not a sub-domain of `api.github.com`, so
/// `host_is` never caught it before this host was added to the table. An
/// agent uploading a release asset by hand hit this host with no
/// credential and passed straight through instead of being deflected.
#[test]
fn github_upload_host_is_deflected_alongside_the_api_host() {
    let connected = vec!["github".to_string()];
    assert!(
        web_call_deflection(
            &connected,
            "https://uploads.github.com/repos/o/r/releases/1/assets?name=out.zip"
        )
        .is_some(),
        "the release-asset upload host must be deflected alongside api.github.com"
    );
    assert!(
        web_call_deflection(&connected, "https://api.github.com/repos/o/r/issues").is_some(),
        "the dedicated API host stays deflected"
    );
}

/// PR #1780 review (round 6): `api.stripe.com` is Stripe's general REST
/// host, but file uploads (dispute evidence, identity documents, ...) are
/// served from a SIBLING host, `files.stripe.com` — not a subdomain of
/// `api.stripe.com`, so `host_is` never caught it before this host was
/// added to the table, and a raw file-upload request passed through
/// unauthenticated instead of being deflected.
#[test]
fn stripe_file_upload_host_is_deflected_alongside_the_api_host() {
    let connected = vec!["stripe".to_string()];
    assert!(
        web_call_deflection(&connected, "https://files.stripe.com/v1/files").is_some(),
        "the file-upload host must be deflected alongside api.stripe.com"
    );
    assert!(
        web_call_deflection(&connected, "https://api.stripe.com/v1/charges").is_some(),
        "the dedicated API host stays deflected"
    );
}

/// PR #1780 review (round 11): `dropbox` is a first-class toolkit in the
/// operator console's connection catalogue
/// (`frontend/src/lib/connections.ts`), but `toolkit_api_hosts` had no
/// match arm for it and fell through to the `_ => &[]` default, so a
/// company that connected Dropbox got no deflection at all — a raw call
/// to either of Dropbox's two API hosts (the RPC host and the
/// content-transfer host used for upload/download) passed straight
/// through unauthenticated instead of being deflected to Composio.
#[test]
fn dropbox_is_deflected_across_its_api_and_content_hosts() {
    let connected = vec!["dropbox".to_string()];
    assert!(
        web_call_deflection(&connected, "https://api.dropboxapi.com/2/files/list_folder")
            .is_some(),
        "the RPC API host must be deflected"
    );
    assert!(
        web_call_deflection(&connected, "https://content.dropboxapi.com/2/files/upload")
            .is_some(),
        "the content-transfer host (upload/download) must also be deflected"
    );
}

/// PR #1780 review (round 13): same gap as Dropbox (round 11) —
/// `twitter` (surfaced as "X" in the console) and `linkedin` are
/// first-class toolkits in `frontend/src/lib/connections.ts`, but
/// neither had a match arm here, so both fell through to the `_ => &[]`
/// default and got no deflection at all.
#[test]
fn x_and_linkedin_are_deflected_across_their_api_hosts() {
    let connected = vec!["twitter".to_string(), "linkedin".to_string()];
    assert!(
        web_call_deflection(&connected, "https://api.twitter.com/2/tweets").is_some(),
        "the legacy twitter.com API host must be deflected"
    );
    assert!(
        web_call_deflection(&connected, "https://api.x.com/2/tweets").is_some(),
        "the current x.com API host must also be deflected"
    );
    assert!(
        web_call_deflection(&connected, "https://api.linkedin.com/rest/posts").is_some(),
        "the LinkedIn API host must be deflected"
    );
}
}

