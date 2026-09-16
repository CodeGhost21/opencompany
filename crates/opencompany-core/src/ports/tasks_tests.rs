use super::*;

// ── the card headline's shape invariant ──────────────────────────────────

/// Every constructor bounds its result, whatever it was handed. This is the
/// property a `String` field could not have: a producer that shoved a
/// paragraph in used to get a paragraph back.
#[test]
fn no_constructor_can_produce_an_unbounded_title() {
    let paragraph = "hey can you take a look at the pricing page, I think the tiers are \
                     confusing and we should probably reword the middle one because \
                     nobody I have shown it to can tell me what it is actually for";
    for title in [
        TaskTitle::authored(paragraph),
        TaskTitle::system(paragraph),
        TaskTitle::truncated(paragraph),
        TaskTitle::summarised(paragraph).expect("a paragraph still names something"),
    ] {
        assert!(
            title.as_str().chars().count() <= TASK_TITLE_MAX_CHARS,
            "{title}"
        );
        assert!(!title.as_str().contains('\n'));
    }
}

/// The cap counts characters, not bytes, and budgets the ellipsis inside
/// itself. A byte slice here would panic mid-codepoint; an unbudgeted
/// ellipsis would return one character more than the type advertises.
#[test]
fn the_cap_is_utf8_safe_and_includes_its_own_ellipsis() {
    let long = "價".repeat(TASK_TITLE_MAX_CHARS + 40);
    let title = TaskTitle::truncated(&long);
    assert_eq!(title.as_str().chars().count(), TASK_TITLE_MAX_CHARS);
    assert!(title.as_str().ends_with('…'));

    let exact = "a".repeat(TASK_TITLE_MAX_CHARS);
    assert_eq!(TaskTitle::truncated(&exact).as_str(), exact);
}

/// A title never breaks mid-word — the truncator prefers the last whole one.
#[test]
fn a_shortened_title_stops_at_a_word() {
    let title = TaskTitle::truncated(
        "Reword the middle pricing tier and also the top one and the bottom one              and everything else on that page",
    );
    assert!(title.as_str().ends_with('…'));
    assert!(!title.as_str().contains("  "));
}

/// Whitespace-only and empty are the same answer: nothing. A caller that
/// gets this back is expected to refuse rather than open a blank card.
#[test]
fn nothing_in_is_nothing_out() {
    for text in ["", "   ", "\n\t\n", "  \n  \n "] {
        assert!(TaskTitle::authored(text).is_empty(), "{text:?}");
        assert!(TaskTitle::truncated(text).is_empty(), "{text:?}");
        assert!(TaskTitle::summarised(text).is_none(), "{text:?}");
    }
}

/// A request that is already a good title is not degraded by passing
/// through — the commonest input on the board, and the easiest to break.
#[test]
fn an_already_good_title_survives_unchanged() {
    for good in [
        "Fix the login redirect",
        "Draft the Q3 board update",
        "Ship v2 (phase 1)",
        "Why is the pricing page slow?",
    ] {
        assert_eq!(TaskTitle::authored(good).as_str(), good, "{good}");
    }
}

/// A one-word ask is a one-word title, not padding and not an ellipsis.
#[test]
fn a_one_word_request_is_a_one_word_title() {
    assert_eq!(TaskTitle::truncated("ship").as_str(), "ship");
}

/// Casing is never touched. Upper-casing the first character reads well on
/// a sentence and corrupts every name that starts with a deliberately
/// lower-case token — which is most tool names, some brands, and every
/// title derived from a file.
#[test]
fn a_deliberately_lower_case_name_is_not_restyled() {
    for name in [
        "iPhone sync is broken",
        "notes.md",
        "npm audit is failing",
        "kubectl context keeps resetting",
        "eBay listing export",
    ] {
        assert_eq!(TaskTitle::system(name).as_str(), name, "{name}");
        assert_eq!(TaskTitle::authored(name).as_str(), name, "{name}");
    }
}

/// A multi-paragraph brief is reduced to its first line, so the detail
/// cannot ride into the headline — the note is where it belongs.
#[test]
fn a_multi_paragraph_brief_keeps_only_its_first_line() {
    let title = TaskTitle::summarised(
        "Reword the middle pricing tier\n\nBackground: three customers have \
         asked what it means.\n\nDeadline: Friday.",
    )
    .expect("a brief names something");
    assert_eq!(title.as_str(), "Reword the middle pricing tier");
}

/// The wrappers and preambles a model reaches for come off, including when
/// they are nested the other way round.
#[test]
fn model_decoration_is_stripped_rather_than_trusted() {
    for decorated in [
        "\"Reword the middle pricing tier\"",
        "**Reword the middle pricing tier**",
        "`Reword the middle pricing tier`",
        "Title: Reword the middle pricing tier",
        "Task: \"Reword the middle pricing tier\"",
        "\"Title: Reword the middle pricing tier\"",
        // Decoration nested three deep. Each layer hides the next from a
        // single-pass stripper, which is why the pass runs to a fixed point.
        "Task: \"Reword the middle pricing tier\".",
        "**Title: Reword the middle pricing tier.**",
        "\"**Reword the middle pricing tier**\"",
        "# Reword the middle pricing tier",
        "### Reword the middle pricing tier",
        "Reword the middle pricing tier.",
        "_Reword the middle pricing tier_",
        "“Reword the middle pricing tier”",
    ] {
        assert_eq!(
            TaskTitle::summarised(decorated).expect(decorated).as_str(),
            "Reword the middle pricing tier",
            "{decorated}"
        );
    }
}

/// Punctuation that is part of the name stays. Only sentence-ending
/// decoration is stripped, or `Ship v2 (phase 1)` loses its bracket.
#[test]
fn punctuation_inside_a_name_is_content_not_decoration() {
    assert_eq!(
        TaskTitle::summarised("Ship v2 (phase 1)")
            .expect("a title")
            .as_str(),
        "Ship v2 (phase 1)"
    );
    assert_eq!(
        TaskTitle::summarised("Why is checkout slow?")
            .expect("a title")
            .as_str(),
        "Why is checkout slow?"
    );
}

/// A reply that is nothing but decoration names nothing, so the caller
/// falls back rather than putting punctuation on the board.
#[test]
fn decoration_with_no_name_in_it_is_no_title() {
    for junk in [
        "\"\"",
        "**",
        "...",
        "Title:",
        "``",
        "#",
        // Odd counts and unpaired marks: these do not peel to nothing, they
        // peel to ONE punctuation character, which a length check passes.
        "\"\"\"",
        "*",
        "-",
        "—",
        "?!",
        "'",
        "\"\"\"\"\"",
        "   \"\"\"   ",
    ] {
        assert!(TaskTitle::summarised(junk).is_none(), "{junk:?}");
    }
}

/// A person's own title is kept whatever it is made of. The junk test that
/// rejects an unusable *model reply* must never reach these constructors:
/// somebody who names a card `🚀` means it, and blanking it persists a card
/// with no headline — worse than the punctuation title the test prevents.
#[test]
fn a_symbol_only_title_a_person_chose_is_kept() {
    for chosen in ["🚀", "✅", "---", "???", "42", "#1"] {
        assert_eq!(TaskTitle::authored(chosen).as_str(), chosen, "{chosen}");
        assert_eq!(TaskTitle::system(chosen).as_str(), chosen, "{chosen}");
        assert!(!TaskTitle::truncated(chosen).is_empty(), "{chosen}");
    }
    // …and the same text from a model is still refused, because that is a
    // guess at a name rather than somebody's choice of one.
    assert!(TaskTitle::summarised("---").is_none());
    assert!(TaskTitle::summarised("🚀").is_none());
}

/// Non-Latin scripts are neither mangled nor case-folded — the pass is
/// character-wise, and upper-casing is a no-op where a script has no case.
#[test]
fn a_non_english_title_is_left_intact() {
    for text in [
        "価格ページの中段プランを書き直す",
        "Переписать средний тариф",
        "إعادة صياغة الفئة الوسطى",
    ] {
        assert_eq!(
            TaskTitle::summarised(text).expect(text).as_str(),
            text,
            "{text}"
        );
    }
}

/// A stored board loads back exactly as it was written, raw-message titles
/// and all. Normalising on read would silently rewrite durable records, and
/// re-summarising would make the board unstable between refreshes.
#[test]
fn a_title_stored_by_an_older_build_round_trips_verbatim() {
    let legacy = "hey can you take a look at the pricing page, I think the tiers are…";
    let json = serde_json::to_string(&legacy).expect("serialises");
    let loaded: TaskTitle = serde_json::from_str(&json).expect("deserialises");
    assert_eq!(loaded.as_str(), legacy);
    assert_eq!(serde_json::to_string(&loaded).expect("re-serialises"), json);
}

/// The type is transparent on the wire, so no stored board needs migrating
/// and no console field changes shape.
#[test]
fn a_title_is_a_bare_string_on_the_wire() {
    let json = serde_json::to_string(&TaskTitle::authored("Ship it")).expect("serialises");
    assert_eq!(json, "\"Ship it\"");
}

/// No titler wired is the offline company and the default build: the card
/// is named exactly as it was before any of this existed.
#[tokio::test]
async fn without_a_titler_a_card_is_named_by_shortening_the_request() {
    let request = "hey can you take a look at the pricing page, I think the tiers are \
                   confusing and we should probably reword the middle one";
    assert_eq!(
        mint_task_title(request, None, None).await,
        TaskTitle::truncated(request)
    );
}

/// A titler that cannot answer — unreachable, too slow, unreadable — leaves
/// the card named, never unnamed and never failed.
#[tokio::test]
async fn a_titler_that_declines_falls_back_rather_than_failing() {
    struct Silent;

    #[async_trait]
    impl TitleSummariser for Silent {
        async fn title(&self, _request: &str) -> Option<TaskTitle> {
            None
        }
    }

    let request = "reword the middle pricing tier please";
    assert_eq!(
        mint_task_title(request, None, Some(&Silent)).await,
        TaskTitle::truncated(request)
    );
    assert!(
        !mint_task_title(request, None, Some(&Silent))
            .await
            .is_empty()
    );
}

/// The fix itself, at the seam: a rambling ask is named after the **work**,
/// and the headline is no longer the message wearing an ellipsis.
#[tokio::test]
async fn a_rambling_ask_is_named_after_the_work() {
    struct Names(&'static str);

    #[async_trait]
    impl TitleSummariser for Names {
        async fn title(&self, _request: &str) -> Option<TaskTitle> {
            TaskTitle::summarised(self.0)
        }
    }

    let request = "hey can you take a look at the pricing page, I think the tiers are \
                   confusing and we should probably reword the middle one";
    let title = mint_task_title(
        request,
        None,
        Some(&Names("Reword the middle pricing tier")),
    )
    .await;

    assert_eq!(title.as_str(), "Reword the middle pricing tier");
    // The property that actually broke: the headline is not the message.
    assert!(
        !request.starts_with(title.as_str().trim_end_matches('…')),
        "the title is still an excerpt of the request: {title}"
    );
    assert!(!title.as_str().ends_with('…'));
}

/// Pins the **Rust** list's ids and their order against a literal, so a
/// reorder or a rename is a deliberate two-place edit rather than a
/// side effect.
///
/// It does **not** protect against drift from the console's mirror in
/// `frontend/src/lib/tasks-sample.ts` — a Rust test cannot see the TS list,
/// so a column added on one side and not the other keeps this green.
/// Closing that gap means generating one list from the other (a build step
/// this crate does not have, across a separate npm build), so for now the
/// mirror is maintained by hand and the two lists are reviewed together.
#[test]
fn columns_are_ordered_and_unique() {
    assert_eq!(
        BOARD_COLUMNS,
        [
            "todo",
            "planning",
            "in_progress",
            "paused",
            "in_review",
            "done"
        ]
    );
    let mut sorted = BOARD_COLUMNS.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        BOARD_COLUMNS.len(),
        "column ids must be unique"
    );
}

#[test]
fn is_board_column_accepts_only_board_columns() {
    for column in BOARD_COLUMNS {
        assert!(is_board_column(column), "{column} is a board column");
    }
    // Issue #301: To-do is the one not-started column, Planning is new, and
    // the old Backlog pool is gone — a client still writing it gets a 400
    // rather than a card the board cannot render.
    assert!(is_board_column(COLUMN_TODO));
    assert!(is_board_column(COLUMN_PLANNING));
    assert!(!is_board_column(LEGACY_COLUMN_BACKLOG));
    // Near-misses a typo'd client might send.
    assert!(!is_board_column("to_do"));
    assert!(!is_board_column("To-do"));
    assert!(!is_board_column("inprogress"));
    assert!(!is_board_column(""));
}

/// Issue #352: every board column has a human label, and an unknown id
/// prints itself rather than a guess. The labels are the mirror of the
/// console's `TASK_COLUMNS`; asserting them against literals is what makes a
/// rename a deliberate two-place edit.
#[test]
fn every_column_has_a_human_label() {
    assert_eq!(column_label(COLUMN_TODO), "To-do");
    assert_eq!(column_label(COLUMN_PLANNING), "Planning");
    assert_eq!(column_label(COLUMN_IN_PROGRESS), "In progress");
    assert_eq!(column_label(COLUMN_PAUSED), "Paused");
    assert_eq!(column_label(COLUMN_IN_REVIEW), "In review");
    assert_eq!(column_label(COLUMN_DONE), "Done");
    for column in BOARD_COLUMNS {
        let label = column_label(column);
        assert!(!label.is_empty());
        assert!(
            !label.contains('_'),
            "{column} still reads as a wire word: {label}"
        );
    }
    assert_eq!(column_label("something_new"), "something_new");
}

/// Issue #337: the whole automatic edge, pinned status by status.
///
/// Every row is asserted against a literal rather than derived, because the
/// table *is* the decision — a mapping that computed itself from some other
/// property could drift without this failing.
#[test]
fn a_settled_run_lands_its_card_by_the_table() {
    assert_eq!(
        column_for_settled_run(RunStatus::Succeeded),
        Some(COLUMN_IN_REVIEW)
    );
    // Issue #465: a run parked on an approval stopped short of a result, so
    // it parks the card rather than presenting it as reviewable work.
    assert_eq!(
        column_for_settled_run(RunStatus::WaitingApproval),
        Some(COLUMN_PAUSED)
    );
    assert_eq!(
        column_for_settled_run(RunStatus::Paused),
        Some(COLUMN_PAUSED)
    );
    assert_eq!(column_for_settled_run(RunStatus::Failed), Some(COLUMN_TODO));
    assert_eq!(
        column_for_settled_run(RunStatus::Cancelled),
        Some(COLUMN_TODO)
    );
    // Issue #1809: a by-design decline returns the card to To-do, same as a
    // failure or cancel — the reason is on the note and the card becomes a
    // one-off, never a stuck column of its own.
    assert_eq!(
        column_for_settled_run(RunStatus::Declined),
        Some(COLUMN_TODO)
    );
    // Not settled — an in-flight attempt has no landing to write.
    assert_eq!(column_for_settled_run(RunStatus::Pending), None);
    assert_eq!(column_for_settled_run(RunStatus::Running), None);
}

/// The operator decision of 2026-08-05, pinned as its own test because it
/// supersedes the `Succeeded → Done` row epic #183 §4 originally wrote.
///
/// **Done is reached only by a person.** Nothing in the automatic edge may
/// write [`COLUMN_DONE`]; the only route there is
/// `review_landing_column(Approve)`. If a future change makes a settle land
/// in Done, it fails here first.
#[test]
fn the_automatic_edge_never_writes_done() {
    assert_eq!(
        column_for_settled_run(RunStatus::Succeeded),
        Some(COLUMN_IN_REVIEW),
        "a clean success stops for a person; Done is not automatic"
    );
    for status in [
        RunStatus::Pending,
        RunStatus::Running,
        RunStatus::WaitingApproval,
        RunStatus::Paused,
        RunStatus::Succeeded,
        RunStatus::Failed,
        RunStatus::Cancelled,
        RunStatus::Declined,
    ] {
        assert_ne!(
            column_for_settled_run(status),
            Some(COLUMN_DONE),
            "{status} must not auto-advance a card to Done"
        );
    }
}

/// **Issue #465, stated as the rule rather than the value.** Only a run that
/// produced a result may land in the column a review verdict consumes.
///
/// The teeth: `review_landing_column(Approve)` turns [`COLUMN_IN_REVIEW`]
/// into [`COLUMN_DONE`] in one gesture, and it is the only route to Done. A
/// run that stopped at an unauthorised call has produced nothing to accept —
/// in the reported case, nothing at all — so leaving it reviewable put
/// unstarted work one click from finished. #337 removed the automatic route
/// to that state; this removes the manual one.
///
/// Written as a loop over "did this run produce a result" rather than as an
/// equality on `WaitingApproval`, so a future status that also stops short
/// has to answer the same question instead of inheriting a column.
#[test]
fn only_a_run_that_produced_a_result_lands_where_review_can_approve_it() {
    for status in [
        RunStatus::WaitingApproval,
        RunStatus::Paused,
        RunStatus::Failed,
        RunStatus::Cancelled,
        RunStatus::Declined,
    ] {
        assert_ne!(
            column_for_settled_run(status),
            Some(COLUMN_IN_REVIEW),
            "{status} produced nothing to review, so it must not present as \
             reviewable work a verdict could approve straight to Done"
        );
    }
    // The converse, so this cannot be satisfied by emptying the column: a
    // run that *did* produce a result still reaches the reviewer.
    assert_eq!(
        column_for_settled_run(RunStatus::Succeeded),
        Some(COLUMN_IN_REVIEW)
    );
}

/// Whatever the table says must be a column the board actually renders —
/// otherwise a settle writes a card straight off the board, which is the
/// silent disappearance [`BOARD_COLUMNS`] exists to prevent.
#[test]
fn every_landing_is_a_real_board_column() {
    for status in [
        RunStatus::Pending,
        RunStatus::Running,
        RunStatus::WaitingApproval,
        RunStatus::Paused,
        RunStatus::Succeeded,
        RunStatus::Failed,
        RunStatus::Cancelled,
        RunStatus::Declined,
    ] {
        if let Some(column) = column_for_settled_run(status) {
            assert!(is_board_column(column), "{status} lands in '{column}'");
        }
    }
}

/// Issue #335: a long paste is truncated on a character boundary, never on
/// a byte one — a message of multi-byte text must not panic the write path
/// or persist a split codepoint. Anything within the cap passes through
/// untouched.
#[test]
fn cap_discussion_is_codepoint_safe() {
    let long: String = "é".repeat(MAX_DISCUSSION_CHARS + 50);
    let capped = cap_discussion(&long);
    assert_eq!(capped.chars().count(), MAX_DISCUSSION_CHARS);
    // A clean multiple of 2 (é is 2 bytes) — no half-written character.
    assert_eq!(capped.len(), MAX_DISCUSSION_CHARS * 2);

    assert_eq!(cap_discussion("looks good to me"), "looks good to me");
    assert_eq!(cap_discussion(""), "");
}

// --- The plan brief (issue #337) ----------------------------------------

fn prereq(kind: PrereqKind, status: PrereqStatus) -> Prerequisite {
    Prerequisite {
        kind,
        name: "github".to_string(),
        status,
        note: "the work opens a pull request".to_string(),
    }
}

fn plan_with(prerequisites: Vec<Prerequisite>) -> TaskPlan {
    TaskPlan {
        description: "Open a PR that adds the changelog entry".to_string(),
        steps: vec![PlanStep {
            title: "Draft the entry".to_string(),
            detail: "Write it against the released version".to_string(),
            estimated_cost_usd: Some(0.02),
            estimated_minutes: Some(5),
        }],
        prerequisites,
        risks: vec!["the release may not be tagged yet".to_string()],
        verification: "the PR exists and CI is green".to_string(),
        scope: "the changelog only; no code changes".to_string(),
        proposed_assignee: Some("maya".to_string()),
        assignee_candidates: Vec::new(),
        planned_at_millis: 42,
    }
}

/// Only `missing` blocks. `needsApproval` and `unknown` ride on the brief
/// as warnings — an approval-gated tool is asked about at the moment it is
/// used, and an inventory the host could not reach is an admission, not a
/// verdict in either direction.
#[test]
fn only_a_missing_prerequisite_blocks_the_dispatch() {
    assert!(PrereqStatus::Missing.blocks());
    assert!(!PrereqStatus::Satisfied.blocks());
    assert!(!PrereqStatus::NeedsApproval.blocks());
    assert!(!PrereqStatus::Unknown.blocks());

    let clear = plan_with(vec![
        prereq(PrereqKind::Connection, PrereqStatus::Satisfied),
        prereq(PrereqKind::Permission, PrereqStatus::NeedsApproval),
        prereq(PrereqKind::Composio, PrereqStatus::Unknown),
    ]);
    assert!(clear.is_dispatchable());
    assert!(clear.blockers().is_empty());

    let blocked = plan_with(vec![
        prereq(PrereqKind::Connection, PrereqStatus::Satisfied),
        prereq(PrereqKind::Mcp, PrereqStatus::Missing),
    ]);
    assert!(!blocked.is_dispatchable());
    assert_eq!(blocked.blockers().len(), 1);
    assert_eq!(blocked.blockers()[0].kind, PrereqKind::Mcp);

    // A plan claiming nothing is dispatchable — "needs nothing" is a
    // legitimate answer, not a suspicious one.
    assert!(plan_with(Vec::new()).is_dispatchable());
}

/// A kind this host cannot check must not fail the parse and must not read
/// as satisfied. It deserializes to `Other`, which the verifier stamps
/// `unknown` — the model gets to be wrong without costing us the plan.
#[test]
fn an_unknown_prerequisite_kind_parses_as_other() {
    let raw = r#"{"kind":"quantum_flux","name":"x","status":"unknown","note":"n"}"#;
    let parsed: Prerequisite = serde_json::from_str(raw).expect("an odd kind still parses");
    assert_eq!(parsed.kind, PrereqKind::Other);
    assert_eq!(parsed.status, PrereqStatus::Unknown);

    // Every known kind still round-trips to its own variant.
    for (wire, kind) in [
        ("connection", PrereqKind::Connection),
        ("composio", PrereqKind::Composio),
        ("mcp", PrereqKind::Mcp),
        ("credential", PrereqKind::Credential),
        ("file", PrereqKind::File),
        ("permission", PrereqKind::Permission),
        ("assignee", PrereqKind::Assignee),
    ] {
        let raw = format!(r#"{{"kind":"{wire}","name":"x","status":"missing","note":"n"}}"#);
        let parsed: Prerequisite = serde_json::from_str(&raw).expect("known kind");
        assert_eq!(parsed.kind, kind);
        assert_eq!(parsed.kind.as_str(), wire);
    }
}

/// The additive-wire contract: a card persisted before #337 loads with no
/// plan, and a card that has never been planned serializes byte-identically
/// to the pre-#337 shape. This is what makes "no migration on any of the
/// three backends" true rather than hoped for.
#[test]
fn the_plan_field_is_additive_on_the_wire() {
    let legacy = r#"{
        "id": "t-1",
        "title": "Unplanned work",
        "column": "todo",
        "priority": "medium",
        "assignee": "maya",
        "updatedAtMillis": 7
    }"#;
    let card: TaskRecord = serde_json::from_str(legacy).expect("a pre-#337 card parses");
    assert!(card.plan.is_none());

    // Matched on the key, not the substring: the fixture's title contains
    // the word "unplanned", and a looser check passes for the wrong reason.
    let round_tripped = serde_json::to_string(&card).unwrap();
    assert!(
        !round_tripped.contains("\"plan\":"),
        "an unplanned card must not grow a key: {round_tripped}"
    );

    // And a planned card round-trips its whole brief, verdicts included.
    let planned = TaskRecord {
        plan: Some(plan_with(vec![prereq(
            PrereqKind::Connection,
            PrereqStatus::Missing,
        )])),
        ..card
    };
    let json = serde_json::to_string(&planned).unwrap();
    assert!(json.contains("\"status\":\"missing\""), "{json}");
    assert!(json.contains("\"estimatedCostUsd\":0.02"), "{json}");
    let back: TaskRecord = serde_json::from_str(&json).expect("round trip");
    assert_eq!(back, planned);
}

/// Issue #301's whole migration, at the seam every persistence backend
/// shares. sqlite and mongodb store a card as a `task_json` string and the
/// fs bundle as a JSON array, so all three — plus export/import — parse
/// through this one `Deserialize`. A stored `backlog` card therefore heals
/// on read instead of failing `is_board_column` and vanishing from the
/// board.
#[test]
fn a_stored_backlog_card_reads_back_in_todo() {
    // A raw blob in exactly the shape a pre-#301 build persisted.
    let legacy = r#"{
        "id": "t-1",
        "title": "Bounced work",
        "note": "[operator] cancelled while in flight",
        "column": "backlog",
        "priority": "medium",
        "assignee": "maya",
        "updatedAtMillis": 7
    }"#;
    let migrated: TaskRecord = serde_json::from_str(legacy).expect("legacy card parses");
    assert_eq!(migrated.column, COLUMN_TODO);
    assert!(
        is_board_column(&migrated.column),
        "a migrated card must render on the board"
    );
    // The reason the collapse is lossless: the note survives untouched, so
    // "bounced back" is still readable on the card.
    assert_eq!(
        migrated.note.as_deref(),
        Some("[operator] cancelled while in flight")
    );

    // The next upsert persists the new literal — nothing re-writes it back.
    let round_tripped = serde_json::to_string(&migrated).unwrap();
    assert!(
        round_tripped.contains("\"column\":\"todo\""),
        "{round_tripped}"
    );

    // Migration is exactly one mapping; every live column is passed through
    // untouched, so a future column cannot be silently rewritten.
    for column in BOARD_COLUMNS {
        assert_eq!(migrate_column(column.to_string()), column);
    }
}

fn plain_card() -> TaskRecord {
    TaskRecord {
        id: "t-1".to_string(),
        title: TaskTitle::authored("Draft the spec"),
        note: None,
        column: COLUMN_IN_REVIEW.to_string(),
        priority: "medium".to_string(),
        assignee: "maya".to_string(),
        updated_at_millis: 7,
        origin: None,
        parent_task_id: None,
        output: None,
        // #339's baseline fixture stays baseline: it exists to prove the
        // output stamp round-trips against a card carrying nothing else.
        plan: None,
        planning_attempts: Vec::new(),
        deliverable: TaskDeliverable::Once,
        workflow_proposal: None,
        origin_run_id: None,
        origin_workflow_id: None,
        origin_message_seq: None,
        bounced: None,
    }
}

/// Issue #1890 step 5: the origin is one value and **the same two keys**.
///
/// The whole claim of the type change is that no stored board migrates, so
/// this pins the bytes rather than the shape: a card raised in a thread
/// serializes to `originChatId` + `originParent` exactly as it did when
/// those were two loose fields, and a board card writes neither key.
#[test]
fn an_origin_serializes_to_the_two_keys_it_always_did() {
    let mut card = plain_card();
    card.origin = TaskOrigin::new(Some("engineering".to_string()), Some(EventSeq::new(41)));
    let json = serde_json::to_string(&card).expect("serializes");
    assert!(json.contains(r#""originChatId":"engineering""#), "{json}");
    assert!(json.contains(r#""originParent":41"#), "{json}");
    assert!(
        !json.contains(r#""origin":"#),
        "the value is flattened away"
    );
    assert_eq!(
        serde_json::from_str::<TaskRecord>(&json).expect("round trip"),
        card
    );

    // A channel-level card writes the desk and skips the thread, and a
    // board card writes neither — so an existing card's stored bytes are
    // unchanged rather than merely equivalent.
    card.origin = TaskOrigin::new(Some("engineering".to_string()), None);
    let channel = serde_json::to_string(&card).expect("serializes");
    assert!(
        channel.contains(r#""originChatId":"engineering""#),
        "{channel}"
    );
    assert!(!channel.contains("originParent"), "{channel}");

    card.origin = None;
    let board = serde_json::to_string(&card).expect("serializes");
    assert!(!board.contains("originChatId"), "{board}");
    assert!(!board.contains("originParent"), "{board}");
}

/// A stored card carrying the **drifted pair** loads with no origin at all.
///
/// A thread root beside no desk names no conversation. It was reachable
/// while these were two independent fields — #1890 B stamped the parent
/// from the raising message's own `parent` and D then changed what an
/// unparented message means — and a card in that state settled its marker
/// somewhere its thread could not see. `TaskOrigin` cannot represent it, so
/// the orphan is dropped on read instead of being carried forward.
#[test]
fn a_thread_root_without_a_desk_is_not_a_conversation() {
    let drifted = r#"{
        "id": "t-1",
        "title": "Draft the spec",
        "column": "in_review",
        "priority": "medium",
        "assignee": "maya",
        "updatedAtMillis": 7,
        "originParent": 41
    }"#;
    let card: TaskRecord = serde_json::from_str(drifted).expect("drifted card parses");
    assert!(card.origin.is_none(), "a parent alone is not an origin");
    assert_eq!(card.origin_chat_id(), None);
    assert_eq!(card.origin_parent(), None);
    assert!(
        !serde_json::to_string(&card)
            .expect("serializes")
            .contains("originParent"),
        "the orphan is dropped on read, not carried forward"
    );
}

/// Issue #661 (M5): the run reference round-trips as camelCase, and — the
/// part that matters — a card written before it existed still loads.
///
/// The legacy half is the load-bearing one. All three backends persist a card
/// as an opaque JSON blob, so `#[serde(default)]` is the entire migration:
/// a board written yesterday must deserialize today with both ids `None`,
/// not fail to load. And a card that carries no run reference must serialize
/// **without the keys at all**, so every existing card's stored bytes are
/// unchanged rather than merely equivalent.
#[test]
fn a_run_reference_round_trips_and_is_absent_on_a_legacy_card() {
    let mut card = plain_card();
    card.origin_run_id = Some("run-9".to_string());
    card.origin_workflow_id = Some("digest".to_string());

    let json = serde_json::to_string(&card).expect("serialize");
    assert!(json.contains("\"originRunId\":\"run-9\""), "{json}");
    assert!(json.contains("\"originWorkflowId\":\"digest\""), "{json}");
    let back: TaskRecord = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, card);

    // A card with no run reference keeps the pre-#661 wire shape exactly.
    let plain = plain_card();
    let json = serde_json::to_string(&plain).expect("serialize");
    assert!(!json.contains("originRunId"), "{json}");
    assert!(!json.contains("originWorkflowId"), "{json}");

    // And a payload written before the fields existed still loads.
    let legacy = serde_json::json!({
        "id": "t-legacy",
        "title": "Older card",
        "column": "todo",
        "priority": "medium",
        "assignee": "",
        "updatedAtMillis": 3
    });
    let loaded: TaskRecord = serde_json::from_value(legacy).expect("a pre-#661 card loads");
    assert_eq!(loaded.origin_run_id, None);
    assert_eq!(loaded.origin_workflow_id, None);
}

/// Issue #339: the whole stamp round-trips as camelCase, and — the part
/// that matters — a card written before it existed still loads, with
/// `None`. All three backends persist a card as an opaque JSON blob, so a
/// `#[serde(default)]` field is the entire migration.
#[test]
fn an_output_stamp_round_trips_and_is_absent_on_a_legacy_card() {
    let mut card = plain_card();
    card.output = Some(TaskOutput {
        source: TaskOutputSource::Run {
            run_id: "run-2".to_string(),
            attempt: Some(2),
        },
        at_millis: 99,
        artifacts: vec![TaskOutputArtifact {
            artifact_id: "a-1".to_string(),
            version: 3,
            title: "Launch spec".to_string(),
            kind: ArtifactKind::Markdown,
        }],
        workflows: vec![TaskOutputWorkflow {
            workflow_id: "digest".to_string(),
            run_id: Some("wf-run-1".to_string()),
            action: TaskOutputAction::Ran,
        }],
    });

    let json = serde_json::to_string(&card).expect("serialize");
    assert!(json.contains(r#""runId":"run-2""#), "{json}");
    assert!(json.contains(r#""artifactId":"a-1""#), "{json}");
    assert!(json.contains(r#""action":"ran""#), "{json}");
    assert_eq!(card, serde_json::from_str(&json).expect("round trip"));

    // Exactly the blob a pre-#339 build persisted: no `output` key at all.
    let legacy = r#"{
        "id": "t-1",
        "title": "Draft the spec",
        "column": "done",
        "priority": "medium",
        "assignee": "maya",
        "updatedAtMillis": 7
    }"#;
    let loaded: TaskRecord = serde_json::from_str(legacy).expect("a legacy card parses");
    assert_eq!(
        loaded.output, None,
        "a card that never recorded an attempt must not be given a synthesized one"
    );
    // And an unstamped card carries no empty scaffolding on the wire.
    let round_tripped = serde_json::to_string(&loaded).expect("serialize");
    assert!(!round_tripped.contains("output"), "{round_tripped}");
}

/// The trace case, pinned as its own test because it is the acceptance
/// criterion most easily lost: *"every card has a link including tasks that
/// produced no file."* An output with neither artifacts nor workflows is a
/// complete, valid stamp — the attempt is the deliverable — so `runId` must
/// survive a round trip on its own, with both lists omitted entirely.
#[test]
fn a_task_that_produced_no_file_still_carries_a_link() {
    let mut card = plain_card();
    card.output = Some(TaskOutput {
        source: TaskOutputSource::Run {
            run_id: "run-1".to_string(),
            attempt: Some(1),
        },
        at_millis: 5,
        artifacts: Vec::new(),
        workflows: Vec::new(),
    });

    let json = serde_json::to_string(&card).expect("serialize");
    assert!(json.contains(r#""runId":"run-1""#), "{json}");
    assert!(!json.contains("artifacts"), "{json}");
    assert!(!json.contains("workflows"), "{json}");

    let back: TaskRecord = serde_json::from_str(&json).expect("round trip");
    let output = back.output.expect("the stamp survives with no deliverable");
    assert_eq!(output.source.run_id(), Some("run-1"));
    assert!(output.artifacts.is_empty());
    assert!(output.workflows.is_empty());
}

/// The attempt ordinal is a label, not an identity: a stamp written when
/// the run row could not be read still addresses its attempt.
#[test]
fn an_unknown_attempt_ordinal_still_leaves_an_addressable_link() {
    let output = TaskOutput {
        source: TaskOutputSource::Run {
            run_id: "run-9".to_string(),
            attempt: None,
        },
        at_millis: 5,
        artifacts: Vec::new(),
        workflows: Vec::new(),
    };
    let json = serde_json::to_string(&output).expect("serialize");
    assert!(!json.contains("attempt"), "{json}");
    let back: TaskOutput = serde_json::from_str(&json).expect("round trip");
    assert_eq!(back.source.run_id(), Some("run-9"));
    assert_eq!(back.source.attempt(), None);
}

// --- issue #806: an output whose producer is not a run --------------------

/// **The migration guarantee.** Every card written before `TaskOutputSource`
/// existed carries a bare `runId` + `attempt` pair, and there is no dual-read
/// anywhere — so if this stops deserializing into `Run`, every stamped card
/// in every store silently loses its link.
///
/// The literal here is deliberately hand-written rather than produced by
/// serializing the current type: a test that round-trips today's shape would
/// still pass if both halves drifted together, which is exactly the failure
/// it is supposed to catch.
#[test]
fn a_stamp_written_before_the_source_union_still_reads_as_a_run() {
    let stored = r#"{"runId":"run-7","attempt":2,"atMillis":1234}"#;
    let output: TaskOutput =
        serde_json::from_str(stored).expect("a pre-#806 stamp must still load");
    assert_eq!(output.source.run_id(), Some("run-7"));
    assert_eq!(output.source.attempt(), Some(2));
    assert_eq!(output.at_millis, 1234);
}

/// And the other direction: a `Run` source must still *write* those exact
/// keys, so a card stamped by this build is readable by anything that has
/// not been updated — and by the console's `"runId" in output` discriminator.
#[test]
fn a_run_source_serializes_to_the_keys_it_always_did() {
    let output = TaskOutput {
        source: TaskOutputSource::Run {
            run_id: "run-7".to_string(),
            attempt: Some(2),
        },
        at_millis: 1234,
        artifacts: Vec::new(),
        workflows: Vec::new(),
    };
    let json = serde_json::to_string(&output).expect("serialize");
    assert!(json.contains(r#""runId":"run-7""#), "{json}");
    assert!(json.contains(r#""attempt":2"#), "{json}");
    assert!(
        !json.contains("chatId") && !json.contains("source"),
        "the union must be flattened, not nested or tagged: {json}"
    );
}

/// A chat turn's stamp carries the conversation and **no** run — the whole
/// point of #806. `run_id()` answering `None` is the truthful answer to a
/// question about runs, and is what stops a reader labelling a conversation
/// as an attempt.
#[test]
fn a_chat_turn_stamp_carries_a_conversation_and_no_run() {
    let output = TaskOutput {
        source: TaskOutputSource::ChatTurn {
            chat_id: "chat-3".to_string(),
        },
        at_millis: 9,
        artifacts: Vec::new(),
        workflows: vec![TaskOutputWorkflow {
            workflow_id: "wf-1".to_string(),
            run_id: None,
            action: TaskOutputAction::Created,
        }],
    };
    let json = serde_json::to_string(&output).expect("serialize");
    assert!(json.contains(r#""chatId":"chat-3""#), "{json}");
    assert!(
        !json.contains("runId\":\"chat"),
        "a chat turn must never be written as a run: {json}"
    );

    let back: TaskOutput = serde_json::from_str(&json).expect("round trip");
    assert_eq!(back.source.run_id(), None);
    assert_eq!(back.source.attempt(), None);
    assert_eq!(
        back.source,
        TaskOutputSource::ChatTurn {
            chat_id: "chat-3".to_string()
        }
    );
    assert_eq!(
        back.workflows.len(),
        1,
        "the deliverable it produced is what makes the link worth having"
    );
}

// --- The plan → workflow bridge (issue #580) -----------------------------

/// The deliverable enum's wire words are pinned — `once`/`workflow` are the
/// values the REST boundary validates and the operator's toggle sends. A
/// rename here is a deliberate edit, not an accident.
#[test]
fn the_deliverable_wire_words_are_stable() {
    assert_eq!(TaskDeliverable::default(), TaskDeliverable::Once);
    assert_eq!(TaskDeliverable::Once.as_str(), "once");
    assert_eq!(TaskDeliverable::Workflow.as_str(), "workflow");
    assert!(TaskDeliverable::Once.is_once());
    assert!(!TaskDeliverable::Workflow.is_once());
    // Serde uses the same lowercase words the operator's payload carries.
    assert_eq!(
        serde_json::to_string(&TaskDeliverable::Workflow).unwrap(),
        "\"workflow\""
    );
    let back: TaskDeliverable = serde_json::from_str("\"once\"").unwrap();
    assert_eq!(back, TaskDeliverable::Once);
}

/// The whole additive-wire contract for #580: a card written before it loads
/// as a one-off with no proposal, a one-off card stays byte-identical to a
/// pre-#580 card (no `deliverable` key), and a workflow card with a proposal
/// round-trips intact. This is what makes "no migration on any backend" true.
#[test]
fn the_deliverable_and_proposal_fields_are_additive_on_the_wire() {
    // A pre-#580 blob: no `deliverable`, no `workflowProposal`.
    let legacy = r#"{
        "id": "t-1",
        "title": "Unbridged work",
        "column": "todo",
        "priority": "medium",
        "assignee": "maya",
        "updatedAtMillis": 7
    }"#;
    let card: TaskRecord = serde_json::from_str(legacy).expect("a pre-#580 card parses");
    assert_eq!(card.deliverable, TaskDeliverable::Once);
    assert!(card.workflow_proposal.is_none());

    // A once card grows neither key — byte-identical to the pre-#580 shape.
    let round_tripped = serde_json::to_string(&card).unwrap();
    assert!(
        !round_tripped.contains("deliverable"),
        "a once card must not grow a deliverable key: {round_tripped}"
    );
    assert!(
        !round_tripped.contains("workflowProposal"),
        "an unproposed card must not grow a proposal key: {round_tripped}"
    );

    // A workflow card carrying a proposal round-trips whole.
    let proposed = TaskRecord {
        planning_attempts: Vec::new(),
        deliverable: TaskDeliverable::Workflow,
        workflow_proposal: Some(TaskWorkflowProposal {
            summary: "Email the weekly digest every Monday".to_string(),
            ops: serde_json::json!({
                "id": "weekly-digest",
                "name": "Weekly digest",
                "nodes": [{ "id": "t", "kind": "trigger" }],
                "edges": []
            }),
            generated_at_millis: 99,
            run_id: "run-7".to_string(),
        }),
        column: COLUMN_IN_REVIEW.to_string(),
        ..card
    };
    let json = serde_json::to_string(&proposed).unwrap();
    assert!(json.contains("\"deliverable\":\"workflow\""), "{json}");
    assert!(json.contains("\"workflowProposal\":"), "{json}");
    assert!(json.contains("\"runId\":\"run-7\""), "{json}");
    let back: TaskRecord = serde_json::from_str(&json).expect("round trip");
    assert_eq!(back, proposed);
}
