use super::*;

fn parse(text: &str) -> CompanyManifest {
    toml::from_str(text).expect("valid toml")
}

/// A valid 32-byte base58 address, built rather than pasted so the test
/// cannot drift from what the decoder accepts.
fn wallet_address() -> String {
    bs58::encode([9u8; 32]).into_string()
}

/// Writes a company bundle: `company.toml` plus optional `agents/` files.
fn write_bundle(company_toml: &str, agent_files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join(MANIFEST_FILE), company_toml).expect("write manifest");
    if !agent_files.is_empty() {
        let agents = dir.path().join(super::super::agent_file::AGENTS_DIR);
        std::fs::create_dir_all(&agents).expect("agents dir");
        for (name, body) in agent_files {
            std::fs::write(agents.join(name), body).expect("write agent");
        }
    }
    dir
}

/// The compatibility rule: a bare `company.toml` with `[[agent]]` entries
/// and no `agents/` directory keeps working exactly as it always has.
#[test]
fn an_inline_roster_still_parses_when_there_is_no_agents_directory() {
    let dir = write_bundle(
        "[company]\nname = \"X\"\n\n[[agent]]\nid = \"ceo\"\nrole = \"CEO\"\n",
        &[],
    );
    let manifest = CompanyManifest::from_path(dir.path()).expect("parses");
    // The global baseline is appended to every roster, so this asserts the
    // company's own teammates — the thing this test is about.
    let own: Vec<&str> = manifest
        .agents
        .iter()
        .filter(|agent| !agent.global)
        .map(|agent| agent.id.as_str())
        .collect();
    assert_eq!(own, ["ceo"]);
}

/// The bundle roster replaces the inline one — so a company that has moved
/// to `agents/*.toml` gets exactly those teammates.
#[test]
fn a_bundle_roster_supplies_the_agents() {
    let dir = write_bundle(
        "[company]\nname = \"X\"\n",
        &[
            ("ceo.toml", "role = \"CEO\"\ntier = \"orchestrator\"\n"),
            ("writer.toml", "role = \"Writer\"\n"),
        ],
    );
    let manifest = CompanyManifest::from_path(dir.path()).expect("parses");
    let ids: Vec<&str> = manifest
        .agents
        .iter()
        .filter(|a| !a.global)
        .map(|a| a.id.as_str())
        .collect();
    assert_eq!(ids, ["ceo", "writer"]);
    // The globals are appended after the company's own roster and none is
    // tagged `orchestrator`, so who orchestrates is unchanged by them.
    assert_eq!(super::super::orchestrator_id(&manifest.agents), Some("ceo"));
}

/// Declaring both forms is refused rather than resolved by precedence:
/// either precedence rule silently discards teammates somebody wrote down.
#[test]
fn declaring_both_roster_forms_is_refused_in_prosumer_language() {
    let dir = write_bundle(
        "[company]\nname = \"X\"\n\n[[agent]]\nid = \"ceo\"\nrole = \"CEO\"\n",
        &[("writer.toml", "role = \"Writer\"\n")],
    );
    let err = CompanyManifest::from_path(dir.path()).expect_err("refused");
    let problems = match err {
        OpenCompanyError::ManifestInvalid { problems, .. } => problems,
        other => panic!("expected ManifestInvalid, got {other}"),
    };
    assert_eq!(problems.len(), 1);
    // It must name both places and say what to do, not merely that something
    // is wrong: the operator has to know which half to delete.
    assert!(problems[0].contains("agents/*.toml"), "{problems:?}");
    assert!(problems[0].contains("[[agent]]"), "{problems:?}");
    assert!(problems[0].contains("company.toml"), "{problems:?}");
}

/// `opencompany check` must load the bundle roster too. It calls
/// [`discover`] itself (for the legacy-filename note) and so takes its own
/// route into loading — which is exactly how it came to validate a manifest
/// whose roster it had never read, reporting every desk member as missing
/// from the roster.
#[test]
fn run_check_accepts_a_bundle_roster() {
    let dir = write_bundle(
        "[company]\nname = \"X\"\n\n[[group_chat]]\nid = \"d\"\nname = \"D\"\nmembers = [\"ceo\"]\n",
        &[("ceo.toml", "role = \"CEO\"\n")],
    );
    assert!(
        super::super::run_check(dir.path()),
        "a bundle-roster company must validate through the check command"
    );
}

/// Cross-cutting validation still applies to a bundle roster: a
/// `delegates_to` target is checked against the desks in `company.toml`,
/// which the per-file loader cannot see on its own.
#[test]
fn a_bundle_roster_is_still_validated_against_the_rest_of_the_manifest() {
    let dir = write_bundle(
        "[company]\nname = \"X\"\n\n[[group_chat]]\nid = \"research\"\nname = \"Research\"\n",
        &[(
            "ceo.toml",
            "role = \"CEO\"\ndelegates_to = [\"marketing\"]\n",
        )],
    );
    let err = CompanyManifest::from_path(dir.path()).expect_err("refused");
    let problems = match err {
        OpenCompanyError::ManifestInvalid { problems, .. } => problems,
        other => panic!("expected ManifestInvalid, got {other}"),
    };
    assert!(
        problems.iter().any(|p| p.contains("marketing")),
        "{problems:?}"
    );
}

#[test]
fn an_unknown_agent_class_is_refused() {
    let manifest = parse(
        "[company]\nname = \"X\"\n\n[[agent]]\nid = \"critic\"\nrole = \"Critic\"\nclasses = [\"judgey\"]\n",
    );
    let problems = manifest.validate();
    assert!(
        problems
            .iter()
            .any(|p| p.contains("classes") && p.contains("judgey")),
        "{problems:?}"
    );
}

#[test]
fn the_known_agent_classes_are_accepted() {
    let manifest = parse(
        "[company]\nname = \"X\"\n\n[[agent]]\nid = \"critic\"\nrole = \"Critic\"\nclasses = [\"judge\", \"evidence\", \"directive\"]\n",
    );
    assert!(manifest.validate().is_empty(), "{:?}", manifest.validate());
}

/// A desk `tools` ceiling is optional and absent by default, so every
/// manifest written before desks could scope tools keeps its meaning.
#[test]
fn a_desk_tool_ceiling_defaults_to_empty() {
    let manifest = parse(
        "[company]\nname = \"X\"\n\n[[agent]]\nid = \"ceo\"\nrole = \"CEO\"\n\n[[group_chat]]\nid = \"d\"\nname = \"D\"\nmembers = [\"ceo\"]\n",
    );
    assert!(manifest.group_chats[0].tools.is_empty());
    assert!(manifest.validate().is_empty());
}

/// A manifest naming no `[users].mode` signs people in by email, exactly as
/// every manifest did before the key existed.
#[test]
fn users_mode_defaults_to_email() {
    let manifest = parse("[company]\nname = \"X\"\n");
    assert_eq!(manifest.users.mode, "email");
    assert!(manifest.validate().is_empty());
}

#[test]
fn an_unknown_users_mode_is_named_in_prosumer_language() {
    let manifest = parse("[company]\nname = \"X\"\n[users]\nmode = \"walet\"\n");
    let problems = manifest.validate();
    assert!(
        problems
            .iter()
            .any(|p| p.contains("`[users].mode`") && p.contains("walet")),
        "{problems:?}"
    );
}

/// The interesting failure is not a malformed value but a **silently
/// unread** one: each mode reads exactly one bootstrap list, and filling in
/// the other is an operator who believes they granted access and has not.
#[test]
fn a_bootstrap_list_the_mode_never_reads_is_a_problem() {
    let manifest = parse(&format!(
        "[company]\nname = \"X\"\n[users]\nmode = \"email\"\nwallets = [\"{}\"]\n",
        wallet_address()
    ));
    let problems = manifest.validate();
    assert!(
        problems.iter().any(|p| p.contains("`[users].wallets`")),
        "{problems:?}"
    );

    let manifest =
        parse("[company]\nname = \"X\"\n[users]\nmode = \"wallet\"\nadmins = [\"a@b.com\"]\n");
    assert!(
        manifest
            .validate()
            .iter()
            .any(|p| p.contains("`[users].admins`")),
        "{:?}",
        manifest.validate()
    );
}

/// `none` reads neither list, because it admits nobody but the person at the
/// machine and has no way to add a second.
#[test]
fn none_mode_reads_no_bootstrap_list_at_all() {
    let manifest =
        parse("[company]\nname = \"X\"\n[users]\nmode = \"none\"\nadmins = [\"a@b.com\"]\n");
    let problems = manifest.validate();
    assert!(
        problems.iter().any(|p| p.contains("no sign-in")),
        "{problems:?}"
    );

    // Naming no list is the correct `none` manifest, and validates clean.
    let manifest = parse("[company]\nname = \"X\"\n[users]\nmode = \"none\"\n");
    assert!(manifest.validate().is_empty(), "{:?}", manifest.validate());
}

/// A wallet that cannot be decoded can never verify a signature, so it is
/// caught by `opencompany check` rather than by a person who cannot sign in.
#[test]
fn a_malformed_bootstrap_wallet_is_rejected() {
    let manifest =
        parse("[company]\nname = \"X\"\n[users]\nmode = \"wallet\"\nwallets = [\"0OIl\"]\n");
    let problems = manifest.validate();
    assert!(
        problems.iter().any(|p| p.contains("`[users].wallets`")),
        "{problems:?}"
    );

    let manifest = parse(&format!(
        "[company]\nname = \"X\"\n[users]\nmode = \"wallet\"\nwallets = [\"{}\"]\n",
        wallet_address()
    ));
    assert!(manifest.validate().is_empty(), "{:?}", manifest.validate());
}

/// `normalize_email` only lowercases and trims, so an `[users].admins`
/// entry with no `@` can still be a normalized key — including one that
/// collides with the `local:owner` scheme `LoginIdentity::parse` reserves
/// for the `none`-mode owner. Caught here, before a bootstrapped user is
/// ever stored under that exact key.
#[test]
fn a_bootstrap_admin_that_is_not_an_email_address_is_rejected() {
    let manifest = parse(
        "[company]\nname = \"X\"\n[users]\nmode = \"email\"\nadmins = [\"Local:Owner\"]\n",
    );
    let problems = manifest.validate();
    assert!(
        problems.iter().any(|p| p.contains("`[users].admins`")),
        "{problems:?}"
    );

    let manifest = parse(
        "[company]\nname = \"X\"\n[users]\nmode = \"email\"\nadmins = [\"ada@example.com\"]\n",
    );
    assert!(manifest.validate().is_empty(), "{:?}", manifest.validate());
}

#[test]
fn bare_agents_toml_is_valid() {
    let manifest = parse(
        r#"
        [company]
        name = "Agentic Marketing Agency"
        output = "Campaigns across every channel"
        human_role = "Campaign review and sign-off"

        [[agent]]
        id = "copywriter"
        role = "Copywriter"
        description = "Write ads."
        "#,
    );
    assert!(manifest.validate().is_empty(), "{:?}", manifest.validate());
}

#[test]
fn defaults_are_prosumer_safe() {
    let manifest = parse("[company]\nname = \"Solo\"\n");
    assert_eq!(manifest.brain.mode, "hosted");
    assert_eq!(manifest.tools.provider, "openhuman");
    assert_eq!(manifest.policy.mode, "supervised");
    assert!(!manifest.place.discoverable);
    // Issue #684: this asserted the three-string default verbatim, which is
    // how the defect survived — the list's *contents* were pinned and its
    // *effect* never was, so a list that matched nothing passed. It is
    // empty now, and what makes the defaults prosumer-safe is the
    // `supervised` mode asserted above: `evaluate_supervised` parks every
    // Spend / Sign / Publish effect on its own.
    assert!(
        manifest.policy.always_approve.is_empty(),
        "the default always-approve list is empty on purpose — see \
         DEFAULT_ALWAYS_APPROVE"
    );
}

#[test]
fn workflows_run_cap_defaults_when_omitted() {
    // Issue #401: an absent `[workflows].max_in_flight_runs` takes the
    // generous default and never trips validation.
    let manifest = parse("[company]\nname = \"X\"\n");
    assert_eq!(
        manifest.workflows.max_in_flight_runs,
        crate::company::types::DEFAULT_MAX_IN_FLIGHT_RUNS
    );
    assert!(manifest.validate().is_empty(), "{:?}", manifest.validate());
}

#[test]
fn workflows_run_cap_parses_explicit_value() {
    let manifest = parse("[company]\nname = \"X\"\n[workflows]\nmax_in_flight_runs = 3\n");
    assert_eq!(manifest.workflows.max_in_flight_runs, 3);
    assert!(manifest.validate().is_empty(), "{:?}", manifest.validate());
}

#[test]
fn workflows_run_cap_of_zero_is_rejected() {
    // Issue #401: `0` would refuse every run, so it is a validation error
    // named in prosumer language rather than a silently wedged company.
    let manifest = parse("[company]\nname = \"X\"\n[workflows]\nmax_in_flight_runs = 0\n");
    let problems = manifest.validate();
    assert!(
        problems
            .iter()
            .any(|p| p.contains("`[workflows].max_in_flight_runs`")
                && p.contains("at least 1")),
        "{problems:?}"
    );
}

#[test]
fn valid_plan_section_passes() {
    let manifest = parse(
        "[company]\nname = \"X\"\n[plan]\nname = \"starter\"\nperiod = \"monthly\"\n[plan.token_budgets]\nweb = 500000\n",
    );
    assert!(manifest.validate().is_empty(), "{:?}", manifest.validate());
}

#[test]
fn absent_plan_is_valid() {
    // No `[plan]` → gating off; the default section must not trip validation.
    let manifest = parse("[company]\nname = \"X\"\n");
    assert!(manifest.validate().is_empty(), "{:?}", manifest.validate());
}

#[test]
fn rejects_unknown_plan_name_in_prosumer_language() {
    let manifest = parse("[company]\nname = \"X\"\n[plan]\nname = \"enterprise\"\n");
    let problems = manifest.validate();
    assert!(
        problems.iter().any(|p| p.contains("`[plan].name`")
            && p.contains("free, starter, pro, unlimited")
            && p.contains("enterprise")),
        "{problems:?}"
    );
}

#[test]
fn rejects_bad_plan_period() {
    let manifest =
        parse("[company]\nname = \"X\"\n[plan]\nname = \"free\"\nperiod = \"hourly\"\n");
    let problems = manifest.validate();
    assert!(
        problems
            .iter()
            .any(|p| p.contains("`[plan].period`") && p.contains("hourly")),
        "{problems:?}"
    );
}

#[test]
fn rejects_non_gateable_budget_namespace() {
    let manifest = parse(
        "[company]\nname = \"X\"\n[plan]\nname = \"pro\"\n[plan.token_budgets]\ntelepathy = 100\n",
    );
    let problems = manifest.validate();
    assert!(
        problems
            .iter()
            .any(|p| p.contains("telepathy") && p.contains("token_budgets")),
        "{problems:?}"
    );
}

/// Each tier is accepted by name from a `company.toml` (issue #560).
///
/// This is the test for the trap that adding `auto` set. The validator keeps
/// its own list of modes (`POLICY_MODES`) and runs *before*
/// `PolicyMode::parse` ever sees the string, so a tier added to the enum and
/// the parser but not to that list is rejected at load with "must be one of
/// …" — unreachable from the only place anybody sets it, while every test in
/// `harness::policy` still passes because they all construct a `Policy`
/// directly and never cross this boundary.
///
/// The mode words are **literals** on purpose. Deriving them from
/// `POLICY_MODES` — the first version of this test — passes vacuously when a
/// mode is missing from that list, because the missing case simply stops
/// being generated. Revert-and-check caught it; the literal cannot be
/// removed by the edit it is meant to detect.
///
/// `harness::policy` holds the matching direction: that `POLICY_MODES` and
/// the enum agree, so a tier cannot be added here and nowhere else.
#[test]
fn every_tier_is_accepted_by_name_from_a_company_toml() {
    for mode in ["readonly", "supervised", "auto", "full"] {
        let manifest = parse(&format!(
            "[company]\nname = \"X\"\n[policy]\nmode = \"{mode}\"\n"
        ));
        let problems = manifest.validate();
        assert!(
            problems.is_empty(),
            "`[policy].mode = \"{mode}\"` is a tier the runtime knows but the manifest \
             validator rejects — unreachable from a company.toml: {problems:?}"
        );
    }
}

/// An `access = "record"` grant to a built-in ledger whose `writers`
/// excludes this agent must not silently disagree — it is a manifest
/// error, not a refusal the agent discovers at call time.
#[test]
fn a_record_grant_disagreeing_with_a_builtins_writers_is_rejected() {
    let agents = vec![toml::from_str::<crate::company::Agent>(
        "id = \"intern\"\nrole = \"Intern\"\nledgers = [{ name = \"risks\", access = \"record\" }]\n",
    )
    .unwrap()];
    let risks = crate::ledger::parse(
        &serde_json::json!({
            "slug": "risks",
            "title": "Risks",
            "fields": [
                { "name": "id", "role": "id" },
                { "name": "risk", "role": "title" },
                { "name": "status", "role": "status" }
            ],
            "statuses": [{ "name": "open" }, { "name": "closed", "closed": true }],
            "writers": ["cfo"]
        }),
        true,
    )
    .unwrap();

    let problems = ledger_grant_problems(&agents, &[risks]);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("agent `intern`"), "{}", problems[0]);
    assert!(problems[0].contains("`risks`"), "{}", problems[0]);
    assert!(problems[0].contains("writers"), "{}", problems[0]);
}

/// A `read` grant never conflicts with `writers` — only `record` implies
/// write access, so only `record` is checked.
#[test]
fn a_read_grant_never_conflicts_with_writers() {
    let agents = vec![toml::from_str::<crate::company::Agent>(
        "id = \"intern\"\nrole = \"Intern\"\nledgers = [{ name = \"risks\", access = \"read\" }]\n",
    )
    .unwrap()];
    let risks = crate::ledger::parse(
        &serde_json::json!({
            "slug": "risks",
            "title": "Risks",
            "fields": [
                { "name": "id", "role": "id" },
                { "name": "risk", "role": "title" },
                { "name": "status", "role": "status" }
            ],
            "statuses": [{ "name": "open" }, { "name": "closed", "closed": true }],
            "writers": ["cfo"]
        }),
        true,
    )
    .unwrap();

    assert!(ledger_grant_problems(&agents, &[risks]).is_empty());
}

/// A `delegates_to` entry must name a real desk (issue #176).
///
/// The failure this catches is silent at runtime rather than loud: a member
/// whose allowlist resolves to nothing still carries `delegate_to_desk`, and
/// every call it makes is refused as off-allowlist. The manifest is where
/// that is visible.
#[test]
fn rejects_a_delegates_to_entry_that_is_not_a_desk() {
    let manifest = parse(
        "[company]\nname = \"X\"\n\
         [[agent]]\nid = \"lead\"\nrole = \"Lead\"\ndelegates_to = [\"writer\"]\n\
         [[agent]]\nid = \"writer\"\nrole = \"Writer\"\n\
         [[group_chat]]\nid = \"content\"\nname = \"Content desk\"\nmembers = [\"writer\"]\n",
    );
    let problems = manifest.validate();
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("agent `lead`"), "{}", problems[0]);
    assert!(problems[0].contains("`writer`"), "{}", problems[0]);
    // The most common mistake is naming the teammate instead of the desk,
    // so the message has to say which vocabulary the field takes.
    assert!(problems[0].contains("teammate ids"), "{}", problems[0]);
}

/// Desk **ids**, desk **names**, and the `"*"` wildcard all resolve; an
/// empty entry is called out separately from an unknown one.
#[test]
fn accepts_desk_ids_names_and_the_wildcard_in_delegates_to() {
    let ok = parse(
        "[company]\nname = \"X\"\n\
         [[agent]]\nid = \"lead\"\nrole = \"Lead\"\ndelegates_to = [\"content\", \"Legal desk\", \"*\"]\n\
         [[group_chat]]\nid = \"content\"\nname = \"Content desk\"\n\
         [[group_chat]]\nid = \"legal\"\nname = \"Legal desk\"\n",
    );
    assert!(ok.validate().is_empty(), "{:?}", ok.validate());

    let blank = parse(
        "[company]\nname = \"X\"\n\
         [[agent]]\nid = \"lead\"\nrole = \"Lead\"\ndelegates_to = [\"  \"]\n\
         [[group_chat]]\nid = \"content\"\nname = \"Content desk\"\n",
    );
    let problems = blank.validate();
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("empty entry"), "{}", problems[0]);
}

/// The depth knob is bounded on both sides (issue #176): `0` would mean
/// "delegation off" wearing a depth's clothes, and past the ceiling the
/// per-level fan-out cap compounds into a runaway.
#[test]
fn rejects_a_delegation_depth_outside_its_bounds() {
    for depth in ["0", "5"] {
        let manifest = parse(&format!(
            "[company]\nname = \"X\"\n[tools]\nmax_delegation_depth = {depth}\n"
        ));
        let problems = manifest.validate();
        assert_eq!(problems.len(), 1, "depth {depth}: {problems:?}");
        assert!(
            problems[0].contains("`[tools].max_delegation_depth`"),
            "{}",
            problems[0]
        );
        assert!(problems[0].contains("between 1 and 4"), "{}", problems[0]);
    }
    for depth in ["1", "2", "3", "4"] {
        let manifest = parse(&format!(
            "[company]\nname = \"X\"\n[tools]\nmax_delegation_depth = {depth}\n"
        ));
        assert!(
            manifest.validate().is_empty(),
            "depth {depth} must be accepted: {:?}",
            manifest.validate()
        );
    }
    // Absent is always fine and means the default.
    let bare = parse("[company]\nname = \"X\"\n");
    assert_eq!(bare.tools.max_delegation_depth, None);
    assert!(bare.validate().is_empty());
}

/// An existing manifest that names no `delegates_to` parses to the empty
/// allowlist, which is what keeps #176 a no-op for every company that did
/// not ask for it.
#[test]
fn delegates_to_defaults_to_empty() {
    let manifest = parse("[company]\nname = \"X\"\n[[agent]]\nid = \"a\"\nrole = \"A\"\n");
    assert!(manifest.agents[0].delegates_to.is_empty());
}

#[test]
fn rejects_bad_policy_mode_in_prosumer_language() {
    let manifest = parse("[company]\nname = \"X\"\n[policy]\nmode = \"supervized\"\n");
    let problems = manifest.validate();
    assert_eq!(problems.len(), 1);
    assert!(problems[0].contains("`[policy].mode`"));
    assert!(problems[0].contains("readonly, supervised, auto, full"));
    assert!(problems[0].contains("supervized"));
}

#[test]
fn rejects_non_snake_case_and_duplicate_ids() {
    let manifest = parse(
        r#"
        [company]
        name = "X"
        [[agent]]
        id = "BadId"
        role = "A"
        [[agent]]
        id = "dup"
        role = "B"
        [[agent]]
        id = "dup"
        role = "C"
        "#,
    );
    let problems = manifest.validate();
    assert!(problems.iter().any(|p| p.contains("snake_case")));
    assert!(problems.iter().any(|p| p.contains("more than once")));
}

/// Issue #1757: `operator` is reserved for the built-in, read-only
/// Operator system channel. A manifest desk claiming it would be
/// indistinguishable from the system channel in the desk list, and every
/// message sent there would be refused by `chat_and_emit`'s read-only
/// guard (`src/server/operator.rs`), which does not know or care where a
/// `chat_id == OPERATOR_CHANNEL` came from.
#[test]
fn rejects_a_group_chat_claiming_the_reserved_operator_id() {
    let manifest = parse(
        "[company]\nname = \"X\"\n\
         [[agent]]\nid = \"ceo\"\nrole = \"CEO\"\n\
         [[group_chat]]\nid = \"operator\"\nname = \"Operator\"\nmembers = [\"ceo\"]\n",
    );
    let problems = manifest.validate();
    assert!(
        problems
            .iter()
            .any(|p| p.contains("reserved") && p.contains("operator")),
        "{problems:?}"
    );
}

/// Issue #1781 review (Codex P2): the id check alone is not enough —
/// `server::operator::resolve_desk` matches a desk by id *or*
/// case-insensitive name, so a desk at a harmless id but named "Operator"
/// shadows the system channel exactly as thoroughly as claiming the
/// literal id would: `GET {scope}/chat/history?desk=operator` (the
/// console's pinned read-only row) resolves to this desk instead of the
/// system feed, and its own writable transcript displays through the
/// identity the console assumes is read-only.
#[test]
fn rejects_a_group_chat_named_operator_even_with_a_harmless_id() {
    let manifest = parse(
        "[company]\nname = \"X\"\n\
         [[agent]]\nid = \"ceo\"\nrole = \"CEO\"\n\
         [[group_chat]]\nid = \"ops\"\nname = \"Operator\"\nmembers = [\"ceo\"]\n",
    );
    let problems = manifest.validate();
    assert!(
        problems
            .iter()
            .any(|p| p.contains("reserved") && p.contains("Operator")),
        "{problems:?}"
    );
}

/// Case-insensitive, matching `resolve_desk`'s own fold — "operator" and
/// "OPERATOR" alias the same collision as "Operator" does.
#[test]
fn the_operator_name_reservation_folds_case() {
    let manifest = parse(
        "[company]\nname = \"X\"\n\
         [[agent]]\nid = \"ceo\"\nrole = \"CEO\"\n\
         [[group_chat]]\nid = \"ops\"\nname = \"operator\"\nmembers = [\"ceo\"]\n",
    );
    let problems = manifest.validate();
    assert!(
        problems.iter().any(|p| p.contains("reserved")),
        "{problems:?}"
    );
}

/// PR #1781 review follow-up: the id/name reservation above only blocks
/// the literal `OPERATOR_CHANNEL` name ("Operator"), but a grandfathered
/// collision diverts the durable feed to
/// `OPERATOR_CHANNEL_COLLISION_FALLBACK` ("operator-feed") instead —
/// `server::operator::resolve_desk` resolves a `?desk=` selector against
/// `chat.name.eq_ignore_ascii_case(desk)` with no distinction between the
/// two addresses. A desk named `operator-feed` therefore still passes
/// this validation, survives `from_path_for_reload`, and then shadows
/// the fallback address exactly as thoroughly as a desk literally named
/// "Operator" would shadow the primary one: `GET
/// {scope}/chat/history?desk=operator-feed`, the request the console's
/// pinned Operator row makes once diverted, resolves to this desk instead
/// of the collision-fallback feed.
#[test]
fn the_operator_feed_fallback_name_is_also_reserved() {
    let manifest = parse(
        "[company]\nname = \"X\"\n\
         [[agent]]\nid = \"ceo\"\nrole = \"CEO\"\n\
         [[group_chat]]\nid = \"ops\"\nname = \"operator-feed\"\nmembers = [\"ceo\"]\n",
    );
    let problems = manifest.validate();
    assert!(
        problems
            .iter()
            .any(|p| p.contains("reserved") && p.contains("operator-feed")),
        "{problems:?}"
    );
}

/// Follow-up to the group-chat guard above: `RESERVED_AGENT_IDS` already
/// stops a console-minted teammate from taking `system`
/// (`mint_agent_id`), but a manifest agent's id is read straight from the
/// TOML and this loop never consulted the same list — so a manifest could
/// still declare `id = "system"` and collide with the runtime's own
/// author id (`SYSTEM_AUTHOR`, issue #966): `senderOf` reads `agent_id`
/// by value and would render every subsequent system notice as that
/// teammate.
#[test]
fn rejects_a_manifest_agent_claiming_a_reserved_id() {
    let manifest = parse(
        "[company]\nname = \"X\"\n\
         [[agent]]\nid = \"system\"\nrole = \"whatever\"\n",
    );
    let problems = manifest.validate();
    assert!(
        problems
            .iter()
            .any(|p| p.contains("reserved") && p.contains("system")),
        "{problems:?}"
    );
}

/// The same guard covers every entry in `RESERVED_AGENT_IDS`, not just
/// `system` — `operator`, `agents`, and `desks` are equally live manifest
/// agent ids until this check runs.
///
/// Lowercased before use: the reserved-id arm compares
/// `eq_ignore_ascii_case` on purpose (`RESERVED_AGENT_IDS`'s own doc),
/// because one entry — `DEFAULT_DESK`, `"General"` — is a prosumer display
/// string, not a slug. Every manifest agent id must already be snake_case
/// (checked one arm above this one), so submitting `"General"` verbatim
/// never reaches the reserved-id arm at all — it is rejected first, and
/// correctly, as an invalid id format. Lowercasing exercises the guard
/// through the one shape a manifest id can actually take, for every
/// reserved value including that one.
#[test]
fn rejects_every_reserved_id_as_a_manifest_agent_id() {
    for reserved in crate::ports::types::RESERVED_AGENT_IDS {
        let candidate = reserved.to_ascii_lowercase();
        let manifest = parse(&format!(
            "[company]\nname = \"X\"\n[[agent]]\nid = \"{candidate}\"\nrole = \"whatever\"\n"
        ));
        let problems = manifest.validate();
        assert!(
            problems
                .iter()
                .any(|p| p.contains("reserved") && p.to_ascii_lowercase().contains(&candidate)),
            "id {candidate:?} (reserved: {reserved:?}) should have been rejected: {problems:?}"
        );
    }
}

/// Issue #1781 review (Codex P1): `register_company`'s `serve` boot loop
/// reloads every company directory's `company.toml` on each restart, so
/// a company whose roster already grandfathers a teammate at a
/// [`RESERVED_AGENT_IDS`](crate::ports::types::RESERVED_AGENT_IDS) id —
/// `operator`, the case the rest of this codebase's grandfather-support
/// machinery (`channel.rs`, `operator.rs`, `delivery.rs`) exists to run
/// correctly — must still be able to boot. `from_path`, the strict
/// authoring-time loader, is proven first to still refuse it (unchanged
/// behavior, pinning the pre-fix failure this regresses against);
/// `from_path_for_reload` must accept the identical manifest.
#[test]
fn from_path_for_reload_grandfathers_a_manifest_agent_at_a_reserved_id() {
    let dir = write_bundle(
        "[company]\nname = \"Acme\"\n\n[[agent]]\nid = \"operator\"\nrole = \"Chief of Staff\"\n",
        &[],
    );

    let strict = CompanyManifest::from_path(dir.path());
    assert!(
        strict.is_err(),
        "sanity check: the strict authoring loader must still refuse this manifest, \
         or this test is not exercising the rule it claims to"
    );

    let reloaded = CompanyManifest::from_path_for_reload(dir.path())
        .expect("a company that already grandfathers an `operator` teammate must reboot");
    assert!(
        reloaded.agents.iter().any(|a| a.id == "operator"),
        "the grandfathered agent itself must still be loaded, not merely tolerated: {:?}",
        reloaded.agents
    );
}

/// The reload loader still enforces every other manifest rule — it
/// grandfathers exactly the reserved-agent-id collision, not validation
/// as a whole, so a company directory hand-edited into a genuinely
/// invalid shape (here, a duplicate agent id) must still refuse to boot.
#[test]
fn from_path_for_reload_still_refuses_an_unrelated_validation_problem() {
    let dir = write_bundle(
        "[company]\nname = \"Acme\"\n\n\
         [[agent]]\nid = \"writer\"\nrole = \"Writer\"\n\n\
         [[agent]]\nid = \"writer\"\nrole = \"Also Writer\"\n",
        &[],
    );

    let err = CompanyManifest::from_path_for_reload(dir.path())
        .expect_err("a duplicate agent id must still be refused on reload");
    assert!(
        format!("{err}").contains("more than once"),
        "unexpected error: {err}"
    );
}

/// Issue #1781 review (Codex P1): the `operator` group-chat id/name
/// reservation (`rejects_a_group_chat_claiming_the_reserved_operator_id`
/// above) is the desk-side twin of the agent-id reservation
/// `from_path_for_reload_grandfathers_a_manifest_agent_at_a_reserved_id`
/// covers — both postdate real companies, since `operator` only became a
/// reserved system channel with issue #1757. The agent-id arm was gated
/// on `enforce_reserved_agent_ids`; this arm was not, so a company whose
/// desk list already declared `id = "operator"` before the reservation
/// shipped could reboot as an agent-only grandfather case but never as a
/// desk one — `register_company`'s `serve` boot loop would refuse it on
/// every restart. `from_path` is proven first to still refuse it
/// (pinning the pre-fix failure this regresses against);
/// `from_path_for_reload` must accept the identical manifest and keep
/// the desk itself loaded.
#[test]
fn from_path_for_reload_grandfathers_a_group_chat_at_the_reserved_operator_id() {
    let dir = write_bundle(
        "[company]\nname = \"Acme\"\n\n\
         [[agent]]\nid = \"ceo\"\nrole = \"CEO\"\n\n\
         [[group_chat]]\nid = \"operator\"\nname = \"Legacy Ops\"\nmembers = [\"ceo\"]\n",
        &[],
    );

    let strict = CompanyManifest::from_path(dir.path());
    assert!(
        strict.is_err(),
        "sanity check: the strict authoring loader must still refuse this manifest, \
         or this test is not exercising the rule it claims to"
    );

    let reloaded = CompanyManifest::from_path_for_reload(dir.path())
        .expect("a company that already has a desk at the `operator` id must reboot");
    assert!(
        reloaded.group_chats.iter().any(|c| c.id == "operator"),
        "the grandfathered desk itself must still be loaded, not merely tolerated: {:?}",
        reloaded.group_chats
    );
}

/// The name-collision twin of the test above: a desk at a harmless id but
/// named "Operator" shadows the system channel exactly as thoroughly
/// (`server::operator::resolve_desk` matches by id *or* case-insensitive
/// name — see `rejects_a_group_chat_named_operator_even_with_a_harmless_id`),
/// and was equally unconditional before this fix.
#[test]
fn from_path_for_reload_grandfathers_a_group_chat_named_operator() {
    let dir = write_bundle(
        "[company]\nname = \"Acme\"\n\n\
         [[agent]]\nid = \"ceo\"\nrole = \"CEO\"\n\n\
         [[group_chat]]\nid = \"legacy_ops\"\nname = \"Operator\"\nmembers = [\"ceo\"]\n",
        &[],
    );

    let strict = CompanyManifest::from_path(dir.path());
    assert!(
        strict.is_err(),
        "sanity check: the strict authoring loader must still refuse this manifest, \
         or this test is not exercising the rule it claims to"
    );

    let reloaded = CompanyManifest::from_path_for_reload(dir.path())
        .expect("a company that already has a desk named \"Operator\" must reboot");
    assert!(
        reloaded.group_chats.iter().any(|c| c.id == "legacy_ops"),
        "the grandfathered desk itself must still be loaded, not merely tolerated: {:?}",
        reloaded.group_chats
    );
}

#[test]
fn rejects_unknown_channel_and_bad_tier() {
    let manifest = parse(
        r#"
        [company]
        name = "X"
        [[agent]]
        id = "a"
        role = "A"
        tier = "genius"
        [channels.telepathy]
        enabled = true
        "#,
    );
    let problems = manifest.validate();
    assert!(problems.iter().any(|p| p.contains("telepathy")));
    assert!(
        problems
            .iter()
            .any(|p| p.contains("`tier`") && p.contains("genius"))
    );
}

#[test]
fn public_company_requires_handle() {
    let manifest = parse("[company]\nname = \"X\"\n[place]\ndiscoverable = true\n");
    let problems = manifest.validate();
    assert!(problems.iter().any(|p| p.contains("@handle")));
}

#[test]
fn rejects_bad_skill_price_and_cron() {
    let manifest = parse(
        r#"
        [company]
        name = "X"
        handle = "x"
        [place]
        discoverable = true
        skills = [{ id = "seo.audit", price_usd = "free" }]
        [[schedule]]
        cron = "every monday"
        prompt = "review"
        "#,
    );
    let problems = manifest.validate();
    assert!(problems.iter().any(|p| p.contains("price_usd")));
    assert!(problems.iter().any(|p| p.contains("5 fields")));
}

/// `parse_usd` rejects negative amounts by construction (`amount >= 0.0`),
/// but the only existing skill-price test exercises the non-numeric edge
/// (`"free"`). A negative decimal string parses fine as an `f64` and would
/// slip through a check that only asked "is this a number".
#[test]
fn rejects_a_negative_skill_price() {
    let manifest = parse(
        r#"
        [company]
        name = "X"
        handle = "x"
        [place]
        discoverable = true
        skills = [{ id = "seo.audit", price_usd = "-5.00" }]
        "#,
    );
    let problems = manifest.validate();
    assert!(
        problems
            .iter()
            .any(|p| p.contains("price_usd") && p.contains("-5.00")),
        "{problems:?}"
    );
}

#[test]
fn rejects_a_duplicate_skill_id() {
    let manifest = parse(
        r#"
        [company]
        name = "X"
        handle = "x"
        [place]
        discoverable = true
        skills = [
            { id = "seo.audit", price_usd = "0.00" },
            { id = "seo.audit", price_usd = "25.00" },
        ]
        "#,
    );
    let problems = manifest.validate();
    assert!(
        problems
            .iter()
            .any(|p| p.contains("seo.audit") && p.contains("more than once")),
        "{problems:?}"
    );
}

#[test]
fn accepts_group_chats_connections_and_workflows() {
    let manifest = parse(
        r#"
        [company]
        name = "Agentic Marketing Agency"

        [[agent]]
        id = "creative_director"
        role = "Creative Director"
        [[agent]]
        id = "copywriter"
        role = "Copywriter"

        [[group_chat]]
        id = "creative"
        name = "Creative studio"
        description = "Copy, design, and campaigns"
        members = ["creative_director", "copywriter"]

        [[connection]]
        provider = "slack"
        priority = "high"
        scopes = ["chat:write"]
        reason = "Post campaign updates"

        [workflows]
        enabled = ["campaign_pipeline"]
        "#,
    );
    assert!(manifest.validate().is_empty(), "{:?}", manifest.validate());
    assert_eq!(manifest.group_chats.len(), 1);
    assert_eq!(manifest.group_chats[0].members.len(), 2);
    assert_eq!(manifest.connections[0].provider, "slack");
    assert_eq!(manifest.workflows.enabled, vec!["campaign_pipeline"]);
}

#[test]
fn rejects_unknown_member_bad_priority_and_workflow_id() {
    let manifest = parse(
        r#"
        [company]
        name = "X"
        [[agent]]
        id = "a"
        role = "A"

        [[group_chat]]
        id = "team"
        name = "Team"
        members = ["ghost"]

        [[connection]]
        provider = "slack"
        priority = "urgent"

        [workflows]
        enabled = ["Bad-Id"]
        "#,
    );
    let problems = manifest.validate();
    assert!(
        problems
            .iter()
            .any(|p| p.contains("ghost") && p.contains("not an agent")),
        "{problems:?}"
    );
    assert!(
        problems
            .iter()
            .any(|p| p.contains("`priority`") && p.contains("urgent")),
        "{problems:?}"
    );
    assert!(
        problems
            .iter()
            .any(|p| p.contains("workflow id") && p.contains("Bad-Id")),
        "{problems:?}"
    );
}

/// A bundle laying an `mcp.json` beside its `company.toml` gets those
/// servers, and they are held to the same validator an inline entry is.
#[test]
fn a_bundle_mcp_json_reaches_the_manifest() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join(MANIFEST_FILE), "[company]\nname = \"X\"\n")
        .expect("write manifest");
    std::fs::write(
        dir.path().join("mcp.json"),
        r#"{"mcpServers": {"deepwiki": {"url": "https://mcp.deepwiki.com/mcp"}}}"#,
    )
    .expect("write mcp.json");

    let manifest = CompanyManifest::from_path(dir.path()).expect("loads");
    let names: Vec<&str> = manifest
        .mcp_servers
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(names, ["deepwiki"]);
}

/// A server declared in both forms is refused rather than resolved by
/// precedence — the roster's rule, for the roster's reason: either
/// precedence rule silently discards a declaration somebody wrote down.
#[test]
fn a_server_declared_in_both_forms_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join(MANIFEST_FILE),
        "[company]\nname = \"X\"\n[[mcp_server]]\nname = \"deepwiki\"\nendpoint = \"https://one.test/mcp\"\n",
    )
    .expect("write manifest");
    std::fs::write(
        dir.path().join("mcp.json"),
        r#"{"mcpServers": {"deepwiki": {"url": "https://two.test/mcp"}}}"#,
    )
    .expect("write mcp.json");

    let err = CompanyManifest::from_path(dir.path()).expect_err("must refuse");
    let text = err.to_string();
    assert!(text.contains("deepwiki"), "{text}");
}

/// A bad entry in `mcp.json` is reported against the manifest rather than
/// swallowed — the file is genuinely read, and its problems genuinely land.
#[test]
fn a_bad_bundle_server_is_reported_against_the_manifest() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join(MANIFEST_FILE), "[company]\nname = \"X\"\n")
        .expect("write manifest");
    std::fs::write(
        dir.path().join("mcp.json"),
        r#"{"mcpServers": {"local": {"command": "npx some-mcp"}}}"#,
    )
    .expect("write mcp.json");

    let err = CompanyManifest::from_path(dir.path()).expect_err("must refuse");
    let text = err.to_string();
    assert!(
        text.contains("stdio") && text.contains("mcp.json"),
        "{text}"
    );
}

#[test]
fn accepts_http_mcp_server_and_rejects_stdio() {
    let ok = parse(
        r#"
        [company]
        name = "X"
        [[mcp_server]]
        name = "notion"
        endpoint = "https://notion.example/mcp"
        "#,
    );
    assert!(ok.validate().is_empty(), "{:?}", ok.validate());

    let bad = parse(
        r#"
        [company]
        name = "X"
        [[mcp_server]]
        name = "local"
        command = "npx some-mcp"
        "#,
    );
    let problems = bad.validate();
    assert!(
        problems
            .iter()
            .any(|p| p.contains("stdio") && p.contains("hosted v1")),
        "{problems:?}"
    );
}

#[test]
fn accepts_byok_inference_and_rejects_bad_provider() {
    let ok = parse(
        r#"
        [company]
        name = "X"
        [inference]
        provider = "openrouter"
        [inference.models]
        "chat-v1" = "deepseek/deepseek-chat"
        "#,
    );
    assert!(ok.validate().is_empty(), "{:?}", ok.validate());
    assert_eq!(ok.inference.provider.as_deref(), Some("openrouter"));
    assert_eq!(
        ok.inference.models.get("chat-v1").map(String::as_str),
        Some("deepseek/deepseek-chat")
    );

    let bad = parse(
        r#"
        [company]
        name = "X"
        [inference]
        provider = "ollama"
        "#,
    );
    // Ollama needs a base_url.
    assert!(
        bad.validate()
            .iter()
            .any(|p| p.contains("base_url") && p.contains("required")),
        "{:?}",
        bad.validate()
    );
}

#[test]
fn effective_summary_lists_roster() {
    let manifest = parse(
        r#"
        [company]
        name = "Agentic Marketing Agency"
        [[agent]]
        id = "copywriter"
        role = "Copywriter"
        "#,
    );
    let summary = manifest.effective_summary();
    assert!(summary.contains("Agentic Marketing Agency"));
    assert!(summary.contains("copywriter"));
    assert!(summary.contains("Roster (1)"));
}

#[test]
fn signals_opportunity_studio_template_passes_lint() {
    // The Signals + Opportunity Engine ship as a venture-studio template,
    // not kernel code. This guards that the shipped manifest keeps passing
    // the same lint `opencompany check` runs — unique agent ids, priced +
    // described `[place].skills`, a `[policy]`, and a stated `human_role`.
    // The company *directory*, not its `company.toml`: this template's
    // roster lives in `agents/*.toml`, and loading the file alone would
    // leave `manifest.agents` empty — making every roster assertion below
    // pass by having nothing to check.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../companies/signals_opportunity_studio");
    let manifest = CompanyManifest::from_path(&path).expect("template manifest is valid");

    assert!(manifest.validate().is_empty(), "{:?}", manifest.validate());
    assert!(
        !manifest.agents.is_empty(),
        "the roster must actually load, or the assertions below check nothing"
    );
    assert!(
        manifest.company.human_role.is_some(),
        "the template must name what the human keeps"
    );
    // Unique agent ids.
    let mut ids: Vec<&str> = manifest.agents.iter().map(|a| a.id.as_str()).collect();
    ids.sort_unstable();
    let unique = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), unique, "agent ids must be unique");
    // Every advertised skill is priced and described.
    assert!(!manifest.place.skills.is_empty());
    for skill in &manifest.place.skills {
        assert!(
            parse_usd(&skill.price_usd).is_some(),
            "skill must be priced"
        );
        assert!(
            skill
                .description
                .as_deref()
                .is_some_and(|d| !d.trim().is_empty()),
            "skill `{}` must be described",
            skill.id
        );
    }
    // A supervised policy with a defined always-approve fence. Asserting
    // only `!is_empty()` is what let the template ship three entries that
    // matched nothing on its harness path (issue #684): a list's length
    // says nothing about whether it fires.
    assert_eq!(manifest.policy.mode, "supervised");
    assert!(!manifest.policy.always_approve.is_empty());
    // What was actually wrong is that none of the old entries named a tool,
    // and the template runs the openhuman harness. A shipped template must
    // demonstrate a gate that works on its own path, not merely a plausible
    // effect-kind string.
    assert!(
        crate::policy::always_approve::matches(
            &manifest.policy.always_approve,
            "publish_artifact"
        ),
        "the template's fence names no declared tool, so nothing in it can \
         park a harness tool call — the shape of issue #684"
    );
    // The weekly opportunity loop is a schedule.
    assert!(!manifest.schedules.is_empty());
}

#[test]
fn discover_prefers_company_toml() {
    let dir = std::env::temp_dir().join(format!("oc-discover-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(LEGACY_MANIFEST_FILE), "[company]\nname=\"L\"\n").unwrap();
    let located = discover(&dir).unwrap();
    assert!(located.legacy);
    std::fs::write(dir.join(MANIFEST_FILE), "[company]\nname=\"C\"\n").unwrap();
    let located = discover(&dir).unwrap();
    assert!(!located.legacy);
    std::fs::remove_dir_all(&dir).ok();
}
