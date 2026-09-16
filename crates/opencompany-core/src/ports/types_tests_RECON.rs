use super::*;
use crate::ports::workflow_runner::DeliveryStatus;

#[test]
fn chat_outputs_reject_metadata_for_the_wrong_kind() {
    let workspace = r#"{"kind":"workspace-node","targetId":"node-1","title":"Draft"}"#;
    let artifact = r#"{"kind":"artifact","targetId":"artifact-1","title":"Brief","taskId":"task-1","version":2}"#;
    assert!(serde_json::from_str::<ChatOutput>(workspace).is_ok());
    assert!(serde_json::from_str::<ChatOutput>(artifact).is_ok());

    for invalid in [
        r#"{"kind":"workspace-node","targetId":"node-1","title":"Draft","taskId":"task-1"}"#,
        r#"{"kind":"workspace-node","targetId":"node-1","title":"Draft","version":2}"#,
        r#"{"kind":"artifact","targetId":"artifact-1","title":"Brief"}"#,
        r#"{"kind":"artifact","targetId":"artifact-1","title":"Brief","taskId":"task-1"}"#,
        r#"{"kind":"artifact","targetId":"artifact-1","title":"Brief","version":2}"#,
    ] {
        assert!(
            serde_json::from_str::<ChatOutput>(invalid).is_err(),
            "{invalid}"
        );
    }
}

/// The answers must survive the **blob**, not merely the record.
///
/// `CompanyRecord` gained a `setup` field and the fs store round-tripped it
/// for free, because it serialises the whole record. SQLite and MongoDB do
/// not: they rebuild a record field by field from `OverlayBlob`, so anything
/// missing there is dropped silently on the way back out — losing exactly the
/// answers Phase 2 builds workflows from, on exactly the backends a hosted
/// tenant runs, and nowhere else. `--all-features` compilation is what
/// surfaced it; this is what keeps it surfaced.
#[test]
fn the_setup_answers_survive_the_overlay_blob() {
    let answers = crate::company::setup::SetupAnswers {
        industry: "E-commerce — homeware".into(),
        team_hint: "someone on dispatch".into(),
        automate: "meta ads, order dispatch".into(),
    };
    let mut record = CompanyRecord {
        overlay_desk_hive: Vec::new(),
        overlay_retired_agents: Vec::new(),
        overlay_agent_edits: Vec::new(),
        id: CompanyId::new("acme"),
        manifest: toml::from_str("[company]\nname = \"Acme\"\n").expect("manifest"),
        ledger: Vec::new(),
        lifecycle: "running".to_string(),
        overlay_agents: Vec::new(),
        overlay_desk_members: Vec::new(),
        overlay_desk_order: Vec::new(),
        overlay_desks: Vec::new(),
        overlay_workflows: Vec::new(),
        overlay_budgets: Vec::new(),
        overlay_policy: None,
        overlay_tool_grants: None,
        overlay_desk_tools: Default::default(),
        disabled_workflows: Vec::new(),
        template_provenance: None,
        setup: Some(answers.clone()),
        name_confirmed: false,
        activation_completed_at: None,
        created_at_millis: None,
    };

    let json = serde_json::to_string(&OverlayBlob::from_record(&record)).expect("serialize");
    let parsed = OverlayBlob::parse(&json).expect("parse");
    assert_eq!(
        parsed.setup,
        Some(answers),
        "the answers were dropped by the blob the SQL backends rebuild from"
    );

    // A company that never went through setup carries nothing, and a row
    // written before the field existed still loads.
    record.setup = None;
    let json = serde_json::to_string(&OverlayBlob::from_record(&record)).expect("serialize");
    assert_eq!(OverlayBlob::parse(&json).expect("parse").setup, None);
    assert_eq!(
        OverlayBlob::parse("{\"agents\":[]}")
            .expect("legacy row")
            .setup,
        None
    );
}

/// Issue #1741: `SecretValue` derived `Serialize`, so
/// `serde_json::to_value` over anything holding one emitted the plaintext
/// credential. Unlike the `Debug` surface — patched five separate times on
/// the *enclosing* structs, each time after somebody noticed a live key in
/// a log line — no test anywhere caught the serialize side.
///
/// The guard lives on `SecretValue` itself, so the assertions below are
/// deliberately made through containers the type knows nothing about: a
/// struct with a plain `#[derive(Serialize)]` standing in for the next
/// config struct somebody writes, plus `Option`, `Vec`, a map value, and
/// `#[serde(flatten)]` (a genuinely different serde code path), across
/// both `to_string` and `to_value` (also different code paths in
/// `serde_json`). Regress the impl to a derive and every arm fails.
#[test]
fn secret_value_redacts_in_debug_and_serialize() {
    use std::collections::BTreeMap;

    // Obviously fake, and distinctive enough that a substring hit is a real
    // hit. Same sentinel as the four existing planted-secret tests.
    const FAKE_SECRET: &str = "NOT-A-REAL-KEY-planted-for-tests";

    // Case-**insensitive**. A leak that arrives lowercased, uppercased, or
    // case-mangled on the way out is still a leak, and an exact-case search
    // reads it as clean — which is how a sibling change shipped a
    // leak-detection test that passed a deliberate leak.
    fn leaks(rendering: &str) -> bool {
        rendering
            .to_ascii_lowercase()
            .contains(&FAKE_SECRET.to_ascii_lowercase())
    }

    // Sanity: the detector detects. Without this the whole test could be
    // vacuous and read as green.
    assert!(
        leaks(&format!("token={}", FAKE_SECRET.to_ascii_lowercase())),
        "the leak detector cannot see a lowercased sentinel; every \
         assertion below would be vacuous"
    );

    /// The next config struct somebody writes: derives `Serialize` and
    /// `Debug` with no idea a secret is in there.
    #[derive(Debug, Serialize)]
    struct UnsuspectingConfig {
        bind: String,
        token: SecretValue,
        optional: Option<SecretValue>,
        many: Vec<SecretValue>,
        by_name: BTreeMap<String, SecretValue>,
        // No map-*key* arm: `SecretValue` derives neither `Ord` nor
        // `Hash`, so it cannot occupy a key position in any std map. That
        // is worth keeping — a credential is not an identity to index by.
        #[serde(flatten)]
        nested: NestedSecrets,
    }

    /// Flattened into the outer struct, so serde uses `FlatMapSerializer`
    /// instead of the ordinary struct serializer.
    #[derive(Debug, Serialize)]
    struct NestedSecrets {
        inner: SecretValue,
    }

    let secret = SecretValue(FAKE_SECRET.to_string());
    let config = UnsuspectingConfig {
        bind: "127.0.0.1:8080".to_string(),
        token: secret.clone(),
        optional: Some(secret.clone()),
        many: vec![secret.clone(), secret.clone()],
        by_name: BTreeMap::from([("github".to_string(), secret.clone())]),
        nested: NestedSecrets {
            inner: secret.clone(),
        },
    };

    // --- Serialize, both serde_json entry points -----------------------
    let as_string = serde_json::to_string(&config).expect("serialize");
    assert!(
        !leaks(&as_string),
        "plaintext reached to_string: {as_string}"
    );

    let as_value = serde_json::to_value(&config).expect("to_value");
    let value_text = as_value.to_string();
    assert!(
        !leaks(&value_text),
        "plaintext reached to_value: {value_text}"
    );

    // The bare type, not just embedded in something.
    let bare = serde_json::to_string(&secret).expect("serialize bare");
    assert!(
        !leaks(&bare),
        "plaintext reached a bare serialization: {bare}"
    );
    assert_eq!(bare, format!("\"{SECRET_REDACTED}\""));

    // Redaction is *visible*, not a silently dropped field: an operator
    // reading a dump can tell a secret was there and was withheld.
    assert!(
        as_string.contains(SECRET_REDACTED),
        "the marker is missing, so the field vanished silently: {as_string}"
    );
    // Everything non-secret still serializes normally — the guard is
    // scoped to the secret, not to the struct.
    assert!(as_string.contains("127.0.0.1:8080"), "{as_string}");

    // --- Debug, plain and alternate ------------------------------------
    for rendering in [format!("{config:?}"), format!("{config:#?}")] {
        assert!(
            !leaks(&rendering),
            "plaintext reached a Debug rendering: {rendering}"
        );
        assert!(rendering.contains(SECRET_REDACTED), "{rendering}");
    }
    // On the type itself, so an enclosing struct's *derived* Debug is safe
    // and the container stops having to remember.
    assert_eq!(
        format!("{secret:?}"),
        format!("SecretValue({SECRET_REDACTED})")
    );

    // --- The persistence door is still open ----------------------------
    // Every secret-store backend writes `expose()` and reads back through
    // the constructor; none of them touch serde. That path must keep
    // returning the plaintext or storing a credential stops working.
    assert_eq!(secret.expose(), FAKE_SECRET);
    assert_eq!(SecretValue(secret.expose().to_string()), secret);

    // --- Deserialization keeps working ---------------------------------
    // Reading a secret *in* never leaks one, so `Deserialize` stays
    // derived: a config or stored shape may name a `SecretValue` field.
    let loaded: SecretValue =
        serde_json::from_str(&format!("\"{FAKE_SECRET}\"")).expect("deserialize");
    assert_eq!(loaded.expose(), FAKE_SECRET);

    // The asymmetry is deliberate, and asserted so nobody discovers it in
    // production: a serde round-trip yields the marker, which fails closed
    // at the point of use rather than carrying a live credential onward.
    let round_tripped: SecretValue = serde_json::from_str(&bare).expect("round-trip");
    assert_eq!(round_tripped.expose(), SECRET_REDACTED);
    assert_ne!(round_tripped, secret);
}

fn round_trip<T>(value: &T) -> T
where
    T: Serialize + for<'de> Deserialize<'de>,
{
    let json = serde_json::to_string(value).expect("serialize");
    serde_json::from_str(&json).expect("deserialize")
}

/// The additive proof this repo asks of every new journal field: a message
/// carrying no mentions must serialize **byte-for-byte** as it did before
/// the field existed, so no stored record migrates and the cross-backend
/// round-trip needs no special case.
#[test]
fn a_message_with_no_mentions_serializes_as_it_did_before_the_field() {
    let event = CompanyEvent::OperatorMessage {
        text: "hello".to_string(),
        by: None,
        chat: None,
        parent: None,
        deliverable: None,
        mentions: Vec::new(),
        attachments: Vec::new(),
    };
    let json = serde_json::to_string(&event).expect("serialize");
    assert_eq!(json, r#"{"kind":"OperatorMessage","text":"hello"}"#);
}

/// The same for a reply, whose `mention_depth` is a `u8` and would
/// otherwise serialize as a literal `0` on every reply ever written.
#[test]
fn a_reply_with_no_mentions_serializes_as_it_did_before_the_fields() {
    let event = CompanyEvent::AgentReply {
        audience: Vec::new(),
        chat_id: "general".to_string(),
        agent_id: "ceo".to_string(),
        text: "hi".to_string(),
        steps: Vec::new(),
        task_id: None,
        outputs: Vec::new(),
        parent: None,
        mentions: Vec::new(),
        mention_depth: 0,
    };
    let json = serde_json::to_string(&event).expect("serialize");
    assert_eq!(
        json,
        r#"{"kind":"AgentReply","chat_id":"general","agent_id":"ceo","text":"hi"}"#
    );
}

/// And the other direction: a record written before either field existed
/// still loads, which is what `#[serde(default)]` is there for.
#[test]
fn a_message_journaled_before_mentions_existed_still_loads() {
    let stored = r#"{"kind":"OperatorMessage","text":"hello"}"#;
    let event: CompanyEvent = serde_json::from_str(stored).expect("deserialize");
    match event {
        CompanyEvent::OperatorMessage { mentions, .. } => assert!(mentions.is_empty()),
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn a_mention_round_trips_with_its_target_and_span() {
    let mention = Mention {
        target: MentionTarget::Agent {
            id: "engineer".to_string(),
        },
        text: "@engineer".to_string(),
        offset: 4,
        quiet: false,
    };
    assert_eq!(round_trip(&mention), mention);
    // `quiet` is omitted when false, so an ordinary mention stays small on
    // the wire and in the journal.
    let json = serde_json::to_string(&mention).expect("serialize");
    assert!(!json.contains("quiet"), "{json}");
}

#[test]
fn every_mention_target_round_trips() {
    for target in [
        MentionTarget::Agent {
            id: "engineer".to_string(),
        },
        MentionTarget::User {
            id: "u1".to_string(),
        },
        MentionTarget::Desk {
            id: "engineering".to_string(),
        },
        MentionTarget::Everyone,
    ] {
        assert_eq!(round_trip(&target), target);
    }
}

// ── Issue #174: cycle usage carries cost, and folds ─────────────────────

/// A cycle with nothing to report writes nothing, and any single non-zero
/// field makes it real usage — including a token-less charge.
#[test]
fn token_usage_is_zero_only_when_every_field_is() {
    assert!(TokenUsage::default().is_zero());
    for usage in [
        TokenUsage {
            input: 1,
            ..TokenUsage::default()
        },
        TokenUsage {
            output: 1,
            ..TokenUsage::default()
        },
        TokenUsage {
            cached_input: 1,
            ..TokenUsage::default()
        },
        TokenUsage {
            cost_usd: 0.0001,
            ..TokenUsage::default()
        },
    ] {
        assert!(!usage.is_zero(), "{usage:?} is real usage");
    }
}

/// Several model passes in one cycle accumulate into one total.
#[test]
fn token_usage_folds_passes_together() {
    let mut total = TokenUsage::default();
    total.fold(&TokenUsage {
        input: 100,
        output: 20,
        cached_input: 10,
        cost_usd: 0.01,
    });
    total.fold(&TokenUsage {
        input: 50,
        output: 5,
        cached_input: 0,
        cost_usd: 0.02,
    });
    assert_eq!(total.input, 150);
    assert_eq!(total.output, 25);
    assert_eq!(total.cached_input, 10);
    assert!((total.cost_usd - 0.03).abs() < 1e-9);
}

/// A bogus peer value must never wrap the meter into a huge or tiny number.
#[test]
fn token_usage_fold_saturates_instead_of_overflowing() {
    let mut total = TokenUsage {
        input: u64::MAX,
        output: u64::MAX,
        cached_input: u64::MAX,
        cost_usd: 0.0,
    };
    total.fold(&TokenUsage {
        input: 10,
        output: 10,
        cached_input: 10,
        cost_usd: 0.0,
    });
    assert_eq!(total.input, u64::MAX);
    assert_eq!(total.output, u64::MAX);
    assert_eq!(total.cached_input, u64::MAX);
}

/// The cost fields are additive on the wire: a peer that predates them still
/// decodes, and an all-zero usage still serializes them for a peer that has
/// them.
#[test]
fn token_usage_decodes_a_payload_without_the_cost_fields() {
    let legacy: TokenUsage = serde_json::from_str(r#"{"input":7,"output":3}"#).unwrap();
    assert_eq!(legacy.input, 7);
    assert_eq!(legacy.output, 3);
    assert_eq!(legacy.cached_input, 0);
    assert_eq!(legacy.cost_usd, 0.0);
    assert_eq!(round_trip(&legacy), legacy);
}

/// The `TurnStep` wire shape is camelCase with snake_case enum values:
/// `{kind, status, label, detail?, elapsedMs?}`. Locks the contract the
/// console `TurnStep` mirror in `frontend/src/api/types.ts` depends on.
#[test]
fn turn_step_wire_shape_is_camel_case_with_snake_case_enums() {
    let step = TurnStep {
        kind: TurnStepKind::ToolCall,
        status: TurnStepStatus::Error,
        label: "Searching the web".to_string(),
        detail: Some("brave · search".to_string()),
        elapsed_ms: Some(1234),
        ..TurnStep::default()
    };
    let json = serde_json::to_value(&step).unwrap();
    assert_eq!(json["kind"], "tool_call");
    assert_eq!(json["status"], "error");
    assert_eq!(json["label"], "Searching the web");
    assert_eq!(json["detail"], "brave · search");
    assert_eq!(json["elapsedMs"], 1234);
    assert_eq!(round_trip(&step), step);
}

/// A step with no detail/elapsed omits both keys, and every kind/status
/// value serializes to its documented snake_case token.
#[test]
fn turn_step_omits_absent_fields_and_covers_every_variant() {
    let bare = TurnStep {
        kind: TurnStepKind::Thinking,
        status: TurnStepStatus::Ok,
        label: "Thinking".to_string(),
        detail: None,
        elapsed_ms: None,
        ..TurnStep::default()
    };
    let json = serde_json::to_value(&bare).unwrap();
    assert_eq!(json["kind"], "thinking");
    assert_eq!(json["status"], "ok");
    assert!(json.get("detail").is_none(), "absent detail is omitted");
    assert!(json.get("elapsedMs").is_none(), "absent elapsed is omitted");

    assert_eq!(serde_json::to_value(TurnStepKind::Note).unwrap(), "note");
    assert_eq!(
        serde_json::to_value(TurnStepStatus::Running).unwrap(),
        "running"
    );
}

/// `OutboundMessage.steps` is additive: an empty timeline is omitted from
/// the wire entirely (so every prior producer round-trips byte-identically),
/// and a legacy `{channel, text}` payload still loads with an empty `steps`.
#[test]
fn outbound_message_steps_are_additive_and_omitted_when_empty() {
    let no_steps = OutboundMessage {
        message_id: None,
        task_id: None,
        outputs: Vec::new(),
        channel: "operator".to_string(),
        agent: None,
        text: "hi".to_string(),
        steps: Vec::new(),
        reply_to: None,
        mentions: Vec::new(),
    };
    let json = serde_json::to_string(&no_steps).unwrap();
    assert_eq!(json, r#"{"channel":"operator","text":"hi"}"#);

    let legacy: OutboundMessage =
        serde_json::from_str(r#"{"channel":"operator","text":"hi"}"#).unwrap();
    assert!(legacy.steps.is_empty());

    let with_steps = OutboundMessage {
        message_id: None,
        task_id: None,
        outputs: Vec::new(),
        channel: "operator".to_string(),
        agent: None,
        text: "done".to_string(),
        steps: vec![TurnStep {
            kind: TurnStepKind::Note,
            status: TurnStepStatus::Error,
            label: "MCP: brave unavailable".to_string(),
            detail: Some("server rejected the call".to_string()),
            elapsed_ms: None,
            ..TurnStep::default()
        }],
        reply_to: None,
        mentions: Vec::new(),
    };
    assert_eq!(round_trip(&with_steps), with_steps);
}

/// Issue #246: `OutboundMessage.task_id` is additive on exactly the same
/// terms as `steps` above — a bubble that opened no card must serialize
/// byte-for-byte as it did before the field existed, and a payload written
/// before it existed must still load. Without both halves every already-
/// stored response would change shape the moment this field shipped.
#[test]
fn outbound_message_task_id_is_additive_and_omitted_when_absent() {
    let no_card = OutboundMessage {
        message_id: None,
        task_id: None,
        outputs: Vec::new(),
        channel: "operator".to_string(),
        agent: None,
        text: "hi".to_string(),
        steps: Vec::new(),
        reply_to: None,
        mentions: Vec::new(),
    };
    assert_eq!(
        serde_json::to_string(&no_card).unwrap(),
        r#"{"channel":"operator","text":"hi"}"#,
        "a bubble that opened no card keeps the pre-#246 wire form"
    );

    let legacy: OutboundMessage =
        serde_json::from_str(r#"{"channel":"operator","text":"hi"}"#).unwrap();
    assert!(legacy.task_id.is_none());

    let with_card = OutboundMessage {
        message_id: None,
        task_id: Some("t-42".to_string()),
        outputs: Vec::new(),
        channel: "operator".to_string(),
        agent: None,
        text: "opened one".to_string(),
        steps: Vec::new(),
        reply_to: None,
        mentions: Vec::new(),
    };
    assert_eq!(round_trip(&with_card), with_card);
    assert!(
        serde_json::to_string(&with_card)
            .unwrap()
            .contains(r#""taskId":"t-42""#),
        "the console reads the card off a camelCase key"
    );
}

/// `AgentReply.steps` is additive the same way: a reply journaled before
/// the field existed loads with an empty timeline, and a tool-less reply
/// omits the key so its on-disk form is byte-identical to the legacy log.
#[test]
fn agent_reply_steps_are_additive_and_omitted_when_empty() {
    let legacy: CompanyEvent = serde_json::from_str(
        r#"{"kind":"AgentReply","chat_id":"main","agent_id":"ceo","text":"hi"}"#,
    )
    .expect("a pre-steps AgentReply still loads");
    match &legacy {
        CompanyEvent::AgentReply { steps, .. } => assert!(steps.is_empty()),
        other => panic!("expected AgentReply, got {other:?}"),
    }

    // A tool-less reply serializes without the `steps` key.
    let tool_less = CompanyEvent::AgentReply {
        audience: Vec::new(),
        mentions: Vec::new(),
        mention_depth: 0,
        parent: None,
        task_id: None,
        outputs: Vec::new(),
        chat_id: "main".to_string(),
        agent_id: "ceo".to_string(),
        text: "hi".to_string(),
        steps: Vec::new(),
    };
    let json = serde_json::to_value(&tool_less).unwrap();
    assert!(json.get("steps").is_none());

    // A reply with a timeline round-trips it.
    let with_steps = CompanyEvent::AgentReply {
        audience: Vec::new(),
        mentions: Vec::new(),
        mention_depth: 0,
        parent: None,
        task_id: None,
        outputs: Vec::new(),
        chat_id: "main".to_string(),
        agent_id: "ceo".to_string(),
        text: "done".to_string(),
        steps: vec![TurnStep {
            kind: TurnStepKind::ToolCall,
            status: TurnStepStatus::Ok,
            label: "Reading messages".to_string(),
            detail: None,
            elapsed_ms: Some(12),
            ..TurnStep::default()
        }],
    };
    let back: CompanyEvent =
        serde_json::from_str(&serde_json::to_string(&with_steps).unwrap()).unwrap();
    assert_eq!(back, with_steps);
}

/// #185: the `task_id` correlation key is additive in both directions —
/// an event journaled before it existed still loads, and an untagged event
/// still serializes byte-for-byte as it did before the field was added.
///
/// That second half is the migration-free guarantee: every already-persisted
/// `AgentReply` / `McpCallFailed` in every company's log must round-trip
/// unchanged, or the cross-backend export/import comparison breaks.
#[test]
fn task_id_correlation_is_additive_and_omitted_when_absent() {
    let legacy: CompanyEvent = serde_json::from_str(
        r#"{"kind":"AgentReply","chat_id":"main","agent_id":"ceo","text":"hi"}"#,
    )
    .expect("a pre-task_id AgentReply still loads");
    match &legacy {
        CompanyEvent::AgentReply { task_id, .. } => assert!(task_id.is_none()),
        other => panic!("expected AgentReply, got {other:?}"),
    }

    // An untagged reply keeps the legacy wire shape exactly.
    let untagged = CompanyEvent::AgentReply {
        audience: Vec::new(),
        mentions: Vec::new(),
        mention_depth: 0,
        parent: None,
        chat_id: "main".to_string(),
        agent_id: "ceo".to_string(),
        text: "hi".to_string(),
        steps: Vec::new(),
        task_id: None,
        outputs: Vec::new(),
    };
    assert_eq!(
        serde_json::to_string(&untagged).unwrap(),
        r#"{"kind":"AgentReply","chat_id":"main","agent_id":"ceo","text":"hi"}"#
    );

    // A dispatch-produced reply carries the key and round-trips.
    let tagged = CompanyEvent::AgentReply {
        audience: Vec::new(),
        mentions: Vec::new(),
        mention_depth: 0,
        parent: None,
        chat_id: "t-1".to_string(),
        agent_id: "ceo".to_string(),
        text: "done".to_string(),
        steps: Vec::new(),
        task_id: Some("t-1".to_string()),
        outputs: Vec::new(),
    };
    let back: CompanyEvent =
        serde_json::from_str(&serde_json::to_string(&tagged).unwrap()).unwrap();
    assert_eq!(back, tagged);

    // Same contract on the failure event.
    let legacy_mcp: CompanyEvent = serde_json::from_str(
        r#"{"kind":"McpCallFailed","server":"gh","tool":"issues","status":"credential_required","message":"needs auth"}"#,
    )
    .expect("a pre-task_id McpCallFailed still loads");
    match &legacy_mcp {
        CompanyEvent::McpCallFailed { task_id, .. } => assert!(task_id.is_none()),
        other => panic!("expected McpCallFailed, got {other:?}"),
    }
}

/// #185: the dispatch terminal round-trips, and reports where the card
/// landed so a stopped run is distinguishable from a successful one.
#[test]
fn desk_task_completed_round_trips() {
    let done = CompanyEvent::DeskTaskCompleted {
        task_id: "t-1".to_string(),
        desk: "ceo".to_string(),
        output: "shipped".to_string(),
        column: "in_review".to_string(),
        artifact_ids: Vec::new(),
        origin_chat_id: None,
        origin_parent: None,
    };
    let json = serde_json::to_string(&done).unwrap();
    assert!(json.contains(r#""kind":"DeskTaskCompleted""#));
    assert!(
        !json.contains("artifact_ids"),
        "a task that published nothing must add nothing to the log: {json}"
    );
    assert!(
        !json.contains("origin_chat_id"),
        "a board-created card names no conversation, so it must add nothing \
         to the log either: {json}"
    );
    let back: CompanyEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(back, done);
}

/// Issue #377: the terminal carries the conversation the card was raised
/// from, and a line written before the field existed still replays as
/// origin-less — which is the truth about it (nobody raised it from a chat
/// that this log records), not a default standing in for a lost id.
///
/// The legacy blob is asserted verbatim for the same reason #244's is: it
/// is exactly what is already on disk in every company's event log. If this
/// fails, the change needs a migration rather than a `#[serde(default)]`.
#[test]
fn desk_task_completed_carries_its_origin_chat_and_still_reads_the_old_shape() {
    let done = CompanyEvent::DeskTaskCompleted {
        task_id: "t-1".to_string(),
        desk: "engineer".to_string(),
        output: "shipped".to_string(),
        column: "in_review".to_string(),
        artifact_ids: Vec::new(),
        origin_chat_id: Some("engineering".to_string()),
        origin_parent: None,
    };
    let json = serde_json::to_string(&done).unwrap();
    assert!(json.contains(r#""origin_chat_id":"engineering""#), "{json}");
    assert_eq!(
        serde_json::from_str::<CompanyEvent>(&json).unwrap(),
        done,
        "the origin must survive the round trip"
    );

    // The responder and the channel are different words on purpose — this
    // is why the origin has to be carried rather than derived from `desk`.
    assert!(
        !json.contains(r#""desk":"engineering""#),
        "the responder is not the channel: {json}"
    );

    let legacy = r#"{"kind":"DeskTaskCompleted","task_id":"t-1","desk":"ceo","output":"shipped","column":"in_review"}"#;
    assert_eq!(
        serde_json::from_str::<CompanyEvent>(legacy).unwrap(),
        CompanyEvent::DeskTaskCompleted {
            task_id: "t-1".to_string(),
            desk: "ceo".to_string(),
            output: "shipped".to_string(),
            column: "in_review".to_string(),
            artifact_ids: Vec::new(),
            origin_chat_id: None,
            origin_parent: None,
        },
        "a pre-#377 journal line must replay with no origin, not fail"
    );
}

/// Issue #1890 B: the thread half of that origin round-trips, is skipped
/// when absent, and a line written before it existed replays as
/// channel-level — which is the truth about such a line, not a default
/// standing in for one.
#[test]
fn the_terminal_carries_the_thread_its_card_was_raised_in() {
    let threaded = CompanyEvent::DeskTaskCompleted {
        task_id: "t-1".to_string(),
        desk: "ceo".to_string(),
        output: "shipped".to_string(),
        column: "in_review".to_string(),
        artifact_ids: Vec::new(),
        origin_chat_id: Some("growth".to_string()),
        origin_parent: Some(EventSeq::new(41)),
    };
    let json = serde_json::to_string(&threaded).unwrap();
    assert!(json.contains(r#""origin_parent":41"#), "{json}");
    assert_eq!(
        serde_json::from_str::<CompanyEvent>(&json).unwrap(),
        threaded,
        "the root must survive the round trip"
    );

    // Skipped when absent, so an unthreaded settle is byte-identical to a
    // pre-B one and adds nothing to the log.
    let flat = CompanyEvent::DeskTaskCompleted {
        task_id: "t-1".to_string(),
        desk: "ceo".to_string(),
        output: "shipped".to_string(),
        column: "in_review".to_string(),
        artifact_ids: Vec::new(),
        origin_chat_id: Some("growth".to_string()),
        origin_parent: None,
    };
    let json = serde_json::to_string(&flat).unwrap();
    assert!(!json.contains("origin_parent"), "{json}");

    let legacy = r#"{"kind":"DeskTaskCompleted","task_id":"t-1","desk":"ceo","output":"shipped","column":"in_review","origin_chat_id":"growth"}"#;
    assert_eq!(
        serde_json::from_str::<CompanyEvent>(legacy).unwrap(),
        flat,
        "a pre-#1890-B line must replay as channel-level, not fail"
    );
}

/// Issue #244: the terminal anchor names what the run published, and a line
/// written before the field existed still replays.
///
/// The legacy blob is asserted verbatim because it is exactly what is
/// already on disk in every company's event log — if this ever fails, the
/// change needs a migration rather than a `#[serde(default)]`.
#[test]
fn desk_task_completed_carries_artifact_ids_and_still_reads_the_old_shape() {
    let done = CompanyEvent::DeskTaskCompleted {
        task_id: "t-1".to_string(),
        desk: "ceo".to_string(),
        output: "Drafted the launch spec.".to_string(),
        column: "in_review".to_string(),
        artifact_ids: vec!["art-1".to_string(), "art-2".to_string()],
        origin_chat_id: None,
        origin_parent: None,
    };
    let json = serde_json::to_string(&done).unwrap();
    assert!(
        json.contains(r#""artifact_ids":["art-1","art-2"]"#),
        "{json}"
    );
    assert_eq!(
        serde_json::from_str::<CompanyEvent>(&json).unwrap(),
        done,
        "the ids must survive the round trip"
    );

    let legacy = r#"{"kind":"DeskTaskCompleted","task_id":"t-1","desk":"ceo","output":"shipped","column":"in_review"}"#;
    assert_eq!(
        serde_json::from_str::<CompanyEvent>(legacy).unwrap(),
        CompanyEvent::DeskTaskCompleted {
            task_id: "t-1".to_string(),
            desk: "ceo".to_string(),
            output: "shipped".to_string(),
            column: "in_review".to_string(),
            artifact_ids: Vec::new(),
            origin_chat_id: None,
            origin_parent: None,
        },
        "a pre-#244 journal line must replay with no artifacts, not fail"
    );
}

/// Issue #364: a thread parent round-trips, and a message journaled before
/// threads existed still replays — as unparented, which is the truth about
/// it and not a default standing in for one.
///
/// The legacy blobs are asserted verbatim because they are exactly what is
/// already on disk in every company's log. A message that never was a thread
/// reply must serialize byte-for-byte as it always did, so export/import and
/// the cross-backend round-trip need no migration.
#[test]
fn a_thread_parent_round_trips_and_a_pre_thread_line_still_loads() {
    for legacy in [
        r#"{"kind":"OperatorMessage","text":"hi"}"#,
        r#"{"kind":"AgentReply","chat_id":"main","agent_id":"ceo","text":"hi"}"#,
    ] {
        let event: CompanyEvent = serde_json::from_str(legacy).unwrap();
        match &event {
            CompanyEvent::OperatorMessage { parent, .. }
            | CompanyEvent::AgentReply { parent, .. } => assert!(
                parent.is_none(),
                "a pre-#364 line was never a thread reply: {legacy}"
            ),
            other => panic!("unexpected variant: {other:?}"),
        }
        assert_eq!(
            serde_json::to_string(&event).unwrap(),
            legacy,
            "an unparented message must serialize exactly as it did before"
        );
    }

    let threaded = CompanyEvent::OperatorMessage {
        mentions: Vec::new(),
        parent: Some(EventSeq::new(41)),
        text: "a follow-up".into(),
        by: None,
        chat: Some("studio".into()),
        deliverable: None,
        attachments: Vec::new(),
    };
    let json = serde_json::to_string(&threaded).unwrap();
    assert!(json.contains(r#""parent":41"#), "{json}");
    assert_eq!(
        serde_json::from_str::<CompanyEvent>(&json).unwrap(),
        threaded
    );

    let answered = CompanyEvent::AgentReply {
        audience: Vec::new(),
        mentions: Vec::new(),
        mention_depth: 0,
        parent: Some(EventSeq::new(41)),
        task_id: None,
        outputs: Vec::new(),
        chat_id: "studio".into(),
        agent_id: "ceo".into(),
        text: "on it".into(),
        steps: Vec::new(),
    };
    let json = serde_json::to_string(&answered).unwrap();
    assert!(json.contains(r#""parent":41"#), "{json}");
    assert_eq!(
        serde_json::from_str::<CompanyEvent>(&json).unwrap(),
        answered
    );
}

/// Issue #364: a reaction round-trips, and an unattributed one adds no
/// `by` key — the same additive contract every optional actor here keeps.
#[test]
fn a_reaction_round_trips() {
    let anonymous = CompanyEvent::ReactionToggled {
        message_seq: EventSeq::new(4),
        emoji: "👍".into(),
        on: true,
        by: None,
    };
    let json = serde_json::to_string(&anonymous).unwrap();
    assert_eq!(
        json,
        r#"{"kind":"ReactionToggled","message_seq":4,"emoji":"👍","on":true}"#
    );
    assert_eq!(
        serde_json::from_str::<CompanyEvent>(&json).unwrap(),
        anonymous
    );

    let attributed = CompanyEvent::ReactionToggled {
        message_seq: EventSeq::new(4),
        emoji: "🎉".into(),
        on: false,
        by: Some(Actor {
            kind: ActorKind::User,
            id: "u1".into(),
        }),
    };
    let json = serde_json::to_string(&attributed).unwrap();
    assert_eq!(
        serde_json::from_str::<CompanyEvent>(&json).unwrap(),
        attributed
    );
}

/// The three intents are exactly the three wire words, and only those
/// (issue #1152).
///
/// Pinned as literals because the console and the journal both write them:
/// a rename here silently stops matching every message already on disk.
#[test]
fn a_message_intent_round_trips_through_its_wire_word() {
    for (intent, word) in [
        (MessageIntent::Chat, "chat"),
        (MessageIntent::Once, "once"),
        (MessageIntent::Workflow, "workflow"),
    ] {
        let json = serde_json::to_string(&intent).unwrap();
        assert_eq!(json, format!("\"{word}\""));
        assert_eq!(
            serde_json::from_str::<MessageIntent>(&json).unwrap(),
            intent
        );
        assert_eq!(intent.as_str(), word);
    }
    assert!(
        serde_json::from_str::<MessageIntent>(r#""build""#).is_err(),
        "the set is closed: an unknown word is a 400, not a silent default"
    );
}

/// "Just chatting" has no deliverable, and that is the whole point of the
/// type (issue #1152).
///
/// A card can never *be* "not work", so the honest mapping from a `Chat`
/// message onto the card field is "there is no card" — `None` — rather than
/// a third `TaskDeliverable` variant every stored reader would owe a branch
/// for.
#[test]
fn only_a_work_intent_maps_onto_a_card_deliverable() {
    use crate::ports::tasks::TaskDeliverable;

    assert_eq!(MessageIntent::Chat.deliverable(), None);
    assert_eq!(
        MessageIntent::Once.deliverable(),
        Some(TaskDeliverable::Once)
    );
    assert_eq!(
        MessageIntent::Workflow.deliverable(),
        Some(TaskDeliverable::Workflow)
    );
    assert!(MessageIntent::Chat.is_chat());
    assert!(!MessageIntent::Once.is_chat());
    assert!(!MessageIntent::Workflow.is_chat());
}

/// **No journaled record migrates** (issue #1152).
///
/// Retyping `OperatorMessage::deliverable` from `TaskDeliverable` to
/// [`MessageIntent`] is only safe if every value already written under that
/// key still loads, and still writes back the same bytes. Getting this wrong
/// does not fail CI — it fails on somebody's event log, on whichever of the
/// three backends they run, the next time a company boots. So the claim is a
/// test rather than a sentence in a doc comment.
///
/// The blobs are asserted verbatim in both directions: parsed to the value
/// the new type gives them, and re-serialized byte-for-byte back to what is
/// on disk.
#[test]
fn every_journaled_deliverable_value_still_loads_and_writes_back_identically() {
    for (blob, expected) in [
        (r#"{"kind":"OperatorMessage","text":"hi"}"#, None),
        (
            r#"{"kind":"OperatorMessage","text":"ship the landing page","deliverable":"once"}"#,
            Some(MessageIntent::Once),
        ),
        (
            r#"{"kind":"OperatorMessage","text":"build me a weekly report","deliverable":"workflow"}"#,
            Some(MessageIntent::Workflow),
        ),
    ] {
        let event: CompanyEvent = serde_json::from_str(blob).unwrap_or_else(|e| {
            panic!("a stored line must still load: {blob} — {e}");
        });
        match &event {
            CompanyEvent::OperatorMessage { deliverable, .. } => {
                assert_eq!(*deliverable, expected, "{blob}")
            }
            other => panic!("unexpected variant: {other:?}"),
        }
        assert_eq!(
            serde_json::to_string(&event).unwrap(),
            blob,
            "a stored line must serialize back byte-for-byte"
        );
    }
}

/// The new word travels on the same key, and only when it was chosen
/// (issue #1152).
///
/// The absent case is the compatibility half that matters most: "Do it
/// once" is not the default *because it is sent* — it is the default
/// because nothing is sent, so an unmarked message is byte-identical on the
/// wire to every message journaled before this control existed.
#[test]
fn a_chat_intent_journals_under_the_same_key_and_absence_stays_absent() {
    let chatting = CompanyEvent::OperatorMessage {
        mentions: Vec::new(),
        text: "morning all".into(),
        by: None,
        chat: None,
        parent: None,
        deliverable: Some(MessageIntent::Chat),
        attachments: Vec::new(),
    };
    assert_eq!(
        serde_json::to_string(&chatting).unwrap(),
        r#"{"kind":"OperatorMessage","text":"morning all","deliverable":"chat"}"#
    );
    assert_eq!(
        serde_json::from_str::<CompanyEvent>(
            r#"{"kind":"OperatorMessage","text":"morning all","deliverable":"chat"}"#
        )
        .unwrap(),
        chatting
    );

    let unmarked = CompanyEvent::OperatorMessage {
        mentions: Vec::new(),
        text: "morning all".into(),
        by: None,
        chat: None,
        parent: None,
        deliverable: None,
        attachments: Vec::new(),
    };
    assert_eq!(
        serde_json::to_string(&unmarked).unwrap(),
        r#"{"kind":"OperatorMessage","text":"morning all"}"#,
        "no choice must still put nothing on the wire"
    );
}

#[test]
fn an_operator_message_journaled_before_attribution_still_loads() {
    // Exactly what is already on disk in every existing company's event
    // log. If this ever fails, the change needs a migration.
    let legacy = r#"{"kind":"OperatorMessage","text":"hi"}"#;
    let event: CompanyEvent = serde_json::from_str(legacy).unwrap();
    assert_eq!(
        event,
        CompanyEvent::OperatorMessage {
            mentions: Vec::new(),
            parent: None,
            text: "hi".into(),
            by: None,
            chat: None,
            deliverable: None,
            attachments: Vec::new(),
        }
    );
}

#[test]
fn an_unattributed_message_serializes_exactly_as_it_did_before() {
    // `skip_serializing_if` keeps the old bytes. This is what lets
    // export/import and the fs/sqlite/mongo round-trip stay green without
    // touching a single stored record.
    let event = CompanyEvent::OperatorMessage {
        mentions: Vec::new(),
        parent: None,
        text: "hi".into(),
        by: None,
        chat: None,
        deliverable: None,
        attachments: Vec::new(),
    };
    assert_eq!(
        serde_json::to_string(&event).unwrap(),
        r#"{"kind":"OperatorMessage","text":"hi"}"#
    );
}

#[test]
fn an_attributed_message_round_trips_with_its_actor() {
    let event = CompanyEvent::OperatorMessage {
        mentions: Vec::new(),
        parent: None,
        text: "hi".into(),
        by: Some(Actor {
            kind: ActorKind::User,
            id: "u1".into(),
        }),
        chat: None,
        deliverable: None,
        attachments: Vec::new(),
    };
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["by"]["kind"], "user");
    assert_eq!(json["by"]["id"], "u1");
    assert_eq!(serde_json::from_value::<CompanyEvent>(json).unwrap(), event);
}

#[test]
fn actor_kind_is_still_copy() {
    // A `String`-carrying variant would have taken this away from every
    // existing holder, which is why the User id lives on `Actor` instead.
    fn assert_copy(kind: ActorKind) -> (ActorKind, ActorKind) {
        (kind, kind)
    }
    let (a, b) = assert_copy(ActorKind::User);
    assert_eq!(a, b);
}

#[test]
fn company_event_variants_round_trip_tagged() {
    let events = vec![
        CompanyEvent::OperatorMessage {
            mentions: Vec::new(),
            parent: None,
            text: "hi".into(),
            by: None,
            chat: None,
            deliverable: None,
            attachments: Vec::new(),
        },
        CompanyEvent::WebhookReceived {
            channel: "email".into(),
            body: serde_json::json!({"subject": "hello"}),
        },
        CompanyEvent::ScheduleFired {
            cron: "0 9 * * *".into(),
            prompt: "daily standup".into(),
        },
        CompanyEvent::A2aTaskReceived {
            from: "@peer".into(),
            task: serde_json::json!({"skill": "seo.audit"}),
        },
        CompanyEvent::ApprovalResolved {
            approval_id: ApprovalId::new("a1"),
            verdict: Verdict::Approve,
            by: Actor {
                kind: ActorKind::Operator,
                id: "owner".into(),
            },
        },
        CompanyEvent::FeedbackFiled {
            note: "too slow".into(),
        },
        CompanyEvent::PaymentReceived {
            amount_usd: 25.0,
            memo: "invoice #1".into(),
        },
    ];
    for event in &events {
        assert_eq!(&round_trip(event), event);
    }

    // The tag field is emitted under `kind`.
    let json = serde_json::to_value(&events[0]).unwrap();
    assert_eq!(json["kind"], "OperatorMessage");
    assert_eq!(json["text"], "hi");
}

#[test]
fn mcp_call_failed_round_trips_and_is_byte_stable() {
    let event = CompanyEvent::McpCallFailed {
        task_id: None,
        server: "browserbase".into(),
        tool: "browse".into(),
        status: "tool_call_rejected".into(),
        message: "server rejected the call".into(),
    };
    assert_eq!(round_trip(&event), event);
    // The tag is emitted under `kind`, and the field set is fixed — a byte
    // guard so a later field addition is a deliberate, tested change.
    assert_eq!(
        serde_json::to_string(&event).unwrap(),
        r#"{"kind":"McpCallFailed","server":"browserbase","tool":"browse","status":"tool_call_rejected","message":"server rejected the call"}"#
    );
}

#[test]
fn task_steered_round_trips_and_omits_empty_fields() {
    // A plain pause: no `instruction`, no `by` — both must be OMITTED from
    // the wire (skip_serializing_if), so old logs stay byte-stable.
    let pause = CompanyEvent::TaskSteered {
        task_id: "t1".into(),
        action: "pause".into(),
        instruction: None,
        by: None,
    };
    assert_eq!(round_trip(&pause), pause);
    assert_eq!(
        serde_json::to_string(&pause).unwrap(),
        r#"{"kind":"TaskSteered","task_id":"t1","action":"pause"}"#
    );

    // A redirect carries its (capped) instruction; still no actor.
    let redirect = CompanyEvent::TaskSteered {
        task_id: "t1".into(),
        action: "redirect".into(),
        instruction: Some("focus on the API".into()),
        by: None,
    };
    assert_eq!(round_trip(&redirect), redirect);
    assert_eq!(
        serde_json::to_string(&redirect).unwrap(),
        r#"{"kind":"TaskSteered","task_id":"t1","action":"redirect","instruction":"focus on the API"}"#
    );
}

/// Issue #335: an unattributed post must serialize with **no** `by` key, so
/// the variant's wire shape is the same one a machine-credentialled post
/// wrote before attribution could ever be present — and an attributed one
/// round-trips its actor.
#[test]
fn task_discussion_posted_round_trips_and_omits_an_absent_actor() {
    let anonymous = CompanyEvent::TaskDiscussionPosted {
        task_id: "t1".into(),
        text: "blocked on the API key".into(),
        by: None,
    };
    assert_eq!(round_trip(&anonymous), anonymous);
    assert_eq!(
        serde_json::to_string(&anonymous).unwrap(),
        r#"{"kind":"TaskDiscussionPosted","task_id":"t1","text":"blocked on the API key"}"#
    );

    let attributed = CompanyEvent::TaskDiscussionPosted {
        task_id: "t1".into(),
        text: "unblocked".into(),
        by: Some(Actor {
            kind: ActorKind::User,
            id: "u-7".into(),
        }),
    };
    assert_eq!(round_trip(&attributed), attributed);
}

/// Issue #358: the tombstone's wire shape, pinned because it is written
/// into `events.jsonl` and read back by a *different* instance on import.
/// The pair (post, tombstone) is what stops a withdrawn message being
/// resurrected, so a tombstone that failed to round-trip would silently
/// restore the text it was appended to remove.
#[test]
fn task_discussion_redacted_round_trips_and_omits_an_absent_actor() {
    let anonymous = CompanyEvent::TaskDiscussionRedacted {
        task_id: "t1".into(),
        seq: 42,
        by: None,
    };
    assert_eq!(round_trip(&anonymous), anonymous);
    assert_eq!(
        serde_json::to_string(&anonymous).unwrap(),
        r#"{"kind":"TaskDiscussionRedacted","task_id":"t1","seq":42}"#
    );

    let attributed = CompanyEvent::TaskDiscussionRedacted {
        task_id: "t1".into(),
        seq: 42,
        by: Some(Actor {
            kind: ActorKind::User,
            id: "u-7".into(),
        }),
    };
    assert_eq!(round_trip(&attributed), attributed);
}

#[test]
fn verdict_serializes_lowercase() {
    assert_eq!(
        serde_json::to_string(&Verdict::Approve).unwrap(),
        "\"approve\""
    );
    assert_eq!(serde_json::to_string(&Verdict::Deny).unwrap(), "\"deny\"");
    assert_eq!(
        serde_json::from_str::<Verdict>("\"approve\"").unwrap(),
        Verdict::Approve
    );
}

#[test]
fn effect_round_trips_and_accessors_read_fields() {
    let effect = Effect {
        kind: "payment.send".into(),
        group: EffectGroup::Spend,
        amount_usd: Some(42.5),
        established_thread: true,
        first_time_counterparty: false,
        payload: serde_json::json!({"to": "@vendor"}),
        agent: None,
        run_id: None,
    };
    let back = round_trip(&effect);
    assert_eq!(back, effect);
    assert_eq!(effect.kind(), "payment.send");
    assert_eq!(effect.group(), EffectGroup::Spend);
    assert_eq!(effect.amount_usd(), Some(42.5));
    assert!(effect.is_established_thread());
    assert!(!effect.is_first_time_counterparty());
}

#[test]
fn effect_disposition_round_trips() {
    for disp in [
        EffectDisposition::Executed,
        EffectDisposition::PendingApproval(ApprovalId::new("x")),
        EffectDisposition::Denied {
            reason: "over cap".into(),
        },
    ] {
        assert_eq!(round_trip(&disp), disp);
    }
}

#[test]
fn policy_decision_round_trips() {
    for dec in [
        PolicyDecision::Allow,
        PolicyDecision::RequireApproval,
        PolicyDecision::Deny,
    ] {
        assert_eq!(round_trip(&dec), dec);
    }
}

#[test]
fn event_seq_orders_numerically() {
    assert!(EventSeq::new(1) < EventSeq::new(2));
    assert_eq!(EventSeq::new(7).value(), 7);
}

#[test]
fn agent_card_round_trips_with_extended_fields() {
    let card = AgentCard {
        handle: "acme".into(),
        description: "We audit SEO.".into(),
        skills: vec!["seo.audit".into()],
        name: "Acme SEO".into(),
        actor_type: "agent".into(),
        endpoint: "https://host/a2a/acme".into(),
        supported_interfaces: vec!["a2a-jsonrpc".into()],
        capabilities: vec!["seo.audit".into()],
        tags: vec!["seo.audit".into()],
        payment_requirements: vec![CardPayment {
            skill_id: "seo.audit".into(),
            price: "25.00".into(),
            asset: "USDC".into(),
            network: "solana".into(),
        }],
    };
    assert_eq!(round_trip(&card), card);
}

fn desk_record(toml_src: &str, overlay: Vec<OverlayDeskMember>) -> CompanyRecord {
    CompanyRecord {
        overlay_desk_hive: Vec::new(),
        overlay_retired_agents: Vec::new(),
        overlay_agent_edits: Vec::new(),
        id: CompanyId::new("acme"),
        manifest: toml::from_str(toml_src).expect("parse manifest"),
        ledger: Vec::new(),
        lifecycle: "running".to_string(),
        overlay_agents: Vec::new(),
        overlay_desk_members: overlay,
        overlay_desk_order: Vec::new(),
        overlay_desks: Vec::new(),
        overlay_workflows: Vec::new(),
        overlay_budgets: Vec::new(),
        overlay_policy: None,
        overlay_tool_grants: None,
        overlay_desk_tools: Default::default(),
        disabled_workflows: Vec::new(),
        template_provenance: None,
        setup: None,
        name_confirmed: false,
        activation_completed_at: None,
        created_at_millis: None,
    }
}

/// Like [`desk_record`] but with an explicit per-desk order overlay, for the
/// desk-hierarchy tests.
fn desk_record_ordered(
    toml_src: &str,
    overlay: Vec<OverlayDeskMember>,
    order: Vec<OverlayDeskOrder>,
) -> CompanyRecord {
    let mut record = desk_record(toml_src, overlay);
    record.overlay_desk_order = order;
    record
}

#[test]
fn desk_hive_overrides_precede_manifest_and_are_replaced_or_cleared() {
    let manifest = "[company]\nname = \"Acme\"\n\
         [[group_chat]]\nid = \"studio\"\nname = \"Studio\"\nmembers = []\n\
         [group_chat.hive]\nquorum = 2\n";
    let mut record = desk_record(manifest, Vec::new());

    // The manifest wins where no edit exists, and an unknown desk falls
    // through to the default rather than borrowing another desk's table.
    assert_eq!(record.effective_desk_hive("studio").quorum, Some(2));
    assert_eq!(
        record.effective_desk_hive("unknown"),
        crate::hivemind::HiveConfig::default()
    );
    assert!(!record.desk_hive_is_installed("studio"));

    let first = crate::hivemind::HiveConfig {
        quorum: Some(1),
        ..Default::default()
    };
    record.upsert_desk_hive(DeskHiveOverride {
        desk_id: "studio".into(),
        hive: first,
    });
    assert!(record.desk_hive_is_installed("studio"));
    assert_eq!(record.effective_desk_hive("studio").quorum, Some(1));

    let replacement = crate::hivemind::HiveConfig {
        quorum: Some(3),
        ..Default::default()
    };
    record.upsert_desk_hive(DeskHiveOverride {
        desk_id: "studio".into(),
        hive: replacement,
    });
    assert_eq!(record.overlay_desk_hive.len(), 1);
    assert_eq!(record.effective_desk_hive("studio").quorum, Some(3));
    assert!(record.clear_desk_hive("studio"));
    assert!(!record.clear_desk_hive("studio"));
    assert_eq!(record.effective_desk_hive("studio").quorum, Some(2));
}

/// The effective membership is the manifest members first, then overlay
/// additions in insertion order, deduplicated — the shared rule the REST
/// list and the harness desk-lead resolver both read.
#[test]
fn effective_desk_members_unions_manifest_and_overlay_deduped() {
    let manifest = "[company]\nname = \"Acme\"\n\
         [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
         [[agent]]\nid = \"eng\"\nrole = \"Engineer\"\n\
         [[group_chat]]\nid = \"studio\"\nname = \"Studio\"\nmembers = [\"ceo\"]\n";
    let record = desk_record(
        manifest,
        vec![
            OverlayDeskMember {
                desk_id: "studio".into(),
                agent_id: "eng".into(),
            },
            // A duplicate of a manifest member is not added twice.
            OverlayDeskMember {
                desk_id: "studio".into(),
                agent_id: "ceo".into(),
            },
            // An addition for a different desk is ignored here.
            OverlayDeskMember {
                desk_id: "other".into(),
                agent_id: "eng".into(),
            },
        ],
    );
    assert_eq!(
        record.effective_desk_members("studio"),
        vec!["ceo".to_string(), "eng".to_string()]
    );
    // An unknown desk with only an overlay addition still resolves it.
    assert_eq!(
        record.effective_desk_members("other"),
        vec!["eng".to_string()]
    );
}

/// A three-member manifest desk whose order override permutes the members.
const HIERARCHY_MANIFEST: &str = "[company]\nname = \"Acme\"\n\
     [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
     [[agent]]\nid = \"eng\"\nrole = \"Engineer\"\n\
     [[agent]]\nid = \"des\"\nrole = \"Designer\"\n\
     [[group_chat]]\nid = \"studio\"\nname = \"Studio\"\nmembers = [\"ceo\", \"eng\", \"des\"]\n";

fn order(desk: &str, ids: &[&str]) -> Vec<OverlayDeskOrder> {
    vec![OverlayDeskOrder {
        desk_id: desk.into(),
        ordered: ids.iter().map(|s| s.to_string()).collect(),
    }]
}

/// A full permutation reorders the manifest members exactly as given.
#[test]
fn desk_order_reorders_manifest_members() {
    let record = desk_record_ordered(
        HIERARCHY_MANIFEST,
        Vec::new(),
        order("studio", &["des", "ceo", "eng"]),
    );
    assert_eq!(
        record.effective_desk_members("studio"),
        vec!["des".to_string(), "ceo".to_string(), "eng".to_string()]
    );
}

/// The override can promote an overlay-added member above manifest members —
/// the whole-set permutation a per-member rank could not express.
#[test]
fn desk_order_promotes_overlay_member_to_lead() {
    let record = desk_record_ordered(
        HIERARCHY_MANIFEST,
        vec![OverlayDeskMember {
            desk_id: "studio".into(),
            agent_id: "cto".into(),
        }],
        order("studio", &["cto", "ceo", "eng", "des"]),
    );
    let members = record.effective_desk_members("studio");
    assert_eq!(members[0], "cto");
    assert_eq!(
        members,
        vec![
            "cto".to_string(),
            "ceo".to_string(),
            "eng".to_string(),
            "des".to_string()
        ]
    );
}

/// An absent or empty override reproduces the base order byte-for-byte.
#[test]
fn desk_order_absent_or_empty_keeps_base_order() {
    let base = desk_record(HIERARCHY_MANIFEST, Vec::new());
    let base_members = base.effective_desk_members("studio");
    assert_eq!(base_members, vec!["ceo", "eng", "des"]);

    // An explicit empty override for the desk is a no-op too.
    let empty = desk_record_ordered(HIERARCHY_MANIFEST, Vec::new(), order("studio", &[]));
    assert_eq!(empty.effective_desk_members("studio"), base_members);
}

/// Ids in the override that are no longer desk members are ignored.
#[test]
fn desk_order_ignores_stale_ids() {
    let record = desk_record_ordered(
        HIERARCHY_MANIFEST,
        Vec::new(),
        order("studio", &["ghost", "des", "ceo", "eng"]),
    );
    // `ghost` is not a member, so it contributes nothing; the rest apply.
    assert_eq!(
        record.effective_desk_members("studio"),
        vec!["des".to_string(), "ceo".to_string(), "eng".to_string()]
    );
}

/// A subset override lists its ids first, then the unlisted members keep
/// their base relative order after.
#[test]
fn desk_order_subset_is_listed_first_then_default() {
    let record = desk_record_ordered(HIERARCHY_MANIFEST, Vec::new(), order("studio", &["des"]));
    // `des` promoted first; `ceo`, `eng` keep their base order behind it.
    assert_eq!(
        record.effective_desk_members("studio"),
        vec!["des".to_string(), "ceo".to_string(), "eng".to_string()]
    );
}

/// The persisted overlay blob round-trips the desk-order collection.
#[test]
fn overlay_blob_round_trips_desk_order() {
    let record = desk_record_ordered(
        HIERARCHY_MANIFEST,
        Vec::new(),
        order("studio", &["des", "ceo", "eng"]),
    );
    let blob = OverlayBlob::from_record(&record);
    let json = serde_json::to_string(&blob).expect("serialize blob");
    let parsed = OverlayBlob::parse(&json).expect("parse blob");
    assert_eq!(parsed.desk_order, record.overlay_desk_order);
}

/// The persisted overlay blob round-trips the `[policy]` override, and a
/// blob written before it existed still parses (issue #562).
///
/// Both halves matter. Without the first, a serialization path that dropped
/// the field would move an operator's approval gate back to the manifest on
/// the next load, silently. Without the second, every company record written
/// before this feature would fail to parse at all.
#[test]
fn overlay_blob_round_trips_the_policy_override() {
    let mut record = desk_record(POLICY_MANIFEST, Vec::new());
    record.overlay_policy = Some(policy_entry(Some("auto"), Some(vec!["payment.send"])));

    let blob = OverlayBlob::from_record(&record);
    let json = serde_json::to_string(&blob).expect("serialize blob");
    let parsed = OverlayBlob::parse(&json).expect("parse blob");
    assert_eq!(parsed.policy, record.overlay_policy);

    // A blob from before this field existed loads as "not overridden",
    // which is the pre-#562 behaviour exactly.
    let legacy = r#"{"agents":[],"desk_members":[],"budgets":[]}"#;
    let blob = OverlayBlob::parse(legacy).expect("blob without a policy key");
    assert!(
        blob.policy.is_none(),
        "an older record must load with the manifest's policy in charge"
    );

    // And so does the oldest form of all, the bare agent array.
    let bare = OverlayBlob::parse("[]").expect("legacy array");
    assert!(bare.policy.is_none());
}

/// An object-form blob written before `desk_order` existed still parses, and
/// the legacy bare-array form still parses — both with an empty order.
#[test]
fn overlay_blob_parses_without_desk_order_key() {
    // Object form missing the `desk_order` key (pre-#131 rows).
    let object = r#"{"agents":[{"id":"a","name":"A","role":"r"}],"desk_members":[{"desk_id":"d","agent_id":"a"}]}"#;
    let blob = OverlayBlob::parse(object).expect("object without desk_order");
    assert_eq!(blob.desk_members.len(), 1);
    assert!(blob.desk_order.is_empty());

    // Legacy bare `Vec<OverlayAgent>` form.
    let legacy = r#"[{"id":"a","name":"A","role":"r"}]"#;
    let blob = OverlayBlob::parse(legacy).expect("legacy array");
    assert!(blob.desk_order.is_empty());
}

/// `is_roster_agent` accepts both manifest agents and overlay teammates, and
/// rejects an unknown id — the validation the desk-add route relies on.
#[test]
fn is_roster_agent_covers_manifest_and_overlay() {
    let manifest = "[company]\nname = \"Acme\"\n[[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n";
    let mut record = desk_record(manifest, Vec::new());
    record.overlay_agents.push(OverlayAgent {
        provider: None,
        id: "nova".into(),
        name: "Nova".into(),
        role: "Growth".into(),
        description: None,
        tools: None,
        model: None,
        harness: None,
    });
    assert!(record.is_roster_agent("ceo"));
    assert!(record.is_roster_agent("nova"));
    assert!(!record.is_roster_agent("ghost"));
}

/// Issue #661 / L5 serde, updated for #1804's three-state grant: an absent
/// `tools` key deserializes to `None` (the standard grant) and a `None`
/// grant serializes with no `tools` key — so a record written before the
/// field existed round-trips unchanged. The two new states are wire-visible:
/// an explicit deny-all (`Some(vec![])`) serializes as `tools: []` (present,
/// NOT skipped), and a narrowed grant serializes its list.
#[test]
fn overlay_agent_tools_three_state_serde_round_trip() {
    // An old record with no `tools` key deserializes to `None` (standard).
    let legacy: OverlayAgent =
        serde_json::from_str(r#"{"id":"a","name":"A","role":"r"}"#).expect("legacy overlay");
    assert_eq!(legacy.tools, None);

    // A `None` grant is omitted from the serialized form — a standard-grant
    // teammate is byte-for-byte what it was before this field existed.
    let value = serde_json::to_value(&legacy).unwrap();
    assert!(
        value.get("tools").is_none(),
        "a None (standard) grant must not serialize a `tools` key: {value}"
    );

    // An explicit deny-all IS on the wire, as `tools: []` — it must NOT be
    // skipped, or it would read back as the standard grant (the inversion).
    let denied = OverlayAgent {
        provider: None,
        id: "d".into(),
        name: "D".into(),
        role: "r".into(),
        description: None,
        tools: Some(Vec::new()),
        model: None,
        harness: None,
    };
    let denied_value = serde_json::to_value(&denied).unwrap();
    assert_eq!(
        denied_value.get("tools"),
        Some(&serde_json::json!([])),
        "an explicit deny-all must serialize `tools: []`, not skip the key: {denied_value}"
    );
    let denied_round: OverlayAgent =
        serde_json::from_str(&serde_json::to_string(&denied).unwrap()).unwrap();
    assert_eq!(denied_round.tools, Some(Vec::new()));

    // A non-empty grant round-trips in order.
    let scoped = OverlayAgent {
        provider: None,
        id: "s".into(),
        name: "S".into(),
        role: "r".into(),
        description: None,
        tools: Some(vec!["docs.*".into(), "email".into()]),
        model: None,
        harness: None,
    };
    let round: OverlayAgent =
        serde_json::from_str(&serde_json::to_string(&scoped).unwrap()).unwrap();
    assert_eq!(
        round.tools,
        Some(vec!["docs.*".to_string(), "email".to_string()])
    );
}

/// A record with one manifest agent and no desks, for the minting tests.
fn mint_record() -> CompanyRecord {
    desk_record(
        "[company]\nname = \"Acme\"\n\
         [[agent]]\nid = \"backend_engineer\"\nrole = \"Backend Engineer\"\n",
        Vec::new(),
    )
}

fn add_overlay(record: &mut CompanyRecord, id: &str, name: &str) {
    record.overlay_agents.push(OverlayAgent {
        provider: None,
        id: id.into(),
        name: name.into(),
        role: "Worker".into(),
        description: None,
        tools: None,
        model: None,
        harness: None,
    });
}

/// A free slug is minted bare — the whole point of issue #686 is that the
/// common case reads as `agents/dana_designer/`.
#[test]
fn mint_agent_id_takes_the_bare_slug_when_it_is_free() {
    let record = mint_record();
    assert_eq!(record.mint_agent_id("Dana Designer"), "dana_designer");
    assert_eq!(record.mint_agent_id("Designer!!"), "designer");
    assert_eq!(record.mint_agent_id("24/7 Support"), "teammate");
}

/// The collision that matters: an overlay id equal to a **manifest** id is
/// skipped by `build_roster`, so the teammate would save and never
/// materialise. Suffixing is what keeps it reachable.
#[test]
fn mint_agent_id_suffixes_past_a_manifest_agent() {
    let record = mint_record();
    assert_eq!(
        record.mint_agent_id("Backend Engineer"),
        "backend_engineer_2"
    );
}

/// Repeated adds of one name walk `_2`, `_3`, … in order, so the ids a
/// company ends up with are a function of its roster and not of arrival
/// timing.
#[test]
fn mint_agent_id_walks_suffixes_deterministically() {
    let mut record = mint_record();
    let first = record.mint_agent_id("Designer");
    assert_eq!(first, "designer");
    add_overlay(&mut record, &first, "Designer");

    let second = record.mint_agent_id("Designer");
    assert_eq!(second, "designer_2");
    add_overlay(&mut record, &second, "Designer");

    assert_eq!(record.mint_agent_id("Designer"), "designer_3");

    // A degenerate name is not a special case — it suffixes like any other.
    add_overlay(&mut record, "teammate", "***");
    assert_eq!(record.mint_agent_id("🙂"), "teammate_2");
}

/// Case is not a difference: an overlay id typed with capitals still blocks
/// the lowercase slug, because `resolve_roster_agent_id` folds case and two
/// teammates one capital apart would be one unroutable key.
#[test]
fn mint_agent_id_treats_a_case_variant_id_as_taken() {
    let mut record = mint_record();
    add_overlay(&mut record, "Dana_Designer", "Dana Designer");
    assert_eq!(record.mint_agent_id("Dana Designer"), "dana_designer_2");
}

/// **Issue #1862 review**: an exact desk id must win over another desk's
/// display name.
///
/// Desk creation enforces id uniqueness but not name uniqueness, so
/// `{id: "ops", name: "sales"}` is a valid desk that can sit ahead of
/// `{id: "sales", …}`. A single pass whose predicate is
/// `id == key || name == key` returns whichever comes first, so asking for
/// the id `sales` answered `ops` — an ownership write silently targeting a
/// different desk than the caller named.
#[test]
fn an_exact_desk_id_beats_another_desks_display_name() {
    let manifest = "[company]\nname = \"Acme\"\n\
         [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n";
    let mut record = desk_record(manifest, Vec::new());
    // Deliberately in the order that loses under a first-match search: the
    // desk merely *named* "sales" is created first.
    record.overlay_desks.push(OverlayDesk {
        id: "ops".into(),
        name: "sales".into(),
        description: None,
        members: Vec::new(),
        responder: crate::ports::types::ResponderMode::default(),
        hive: Default::default(),
    });
    record.overlay_desks.push(OverlayDesk {
        id: "sales".into(),
        name: "Revenue".into(),
        description: None,
        members: Vec::new(),
        responder: crate::ports::types::ResponderMode::default(),
        hive: Default::default(),
    });

    assert_eq!(
        record.resolve_desk_id("sales").as_deref(),
        Some("sales"),
        "an exact id must resolve to itself, not to a desk that merely \
         carries it as a display name"
    );
    // The alias still resolves for a key no desk owns as an id.
    assert_eq!(record.resolve_desk_id("Revenue").as_deref(), Some("sales"));
    assert_eq!(record.resolve_desk_id("ops").as_deref(), Some("ops"));
}

/// A manifest desk's id also beats an overlay desk's display name — the
/// exact-id pass spans both lists, so ordering between them cannot decide
/// an ownership write either.
#[test]
fn a_manifest_desk_id_beats_an_overlay_desks_display_name() {
    let manifest = "[company]\nname = \"Acme\"\n\
         [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
         [[group_chat]]\nid = \"growth\"\nname = \"Content\"\nmembers = [\"ceo\"]\n";
    let mut record = desk_record(manifest, Vec::new());
    record.overlay_desks.push(OverlayDesk {
        id: "studio".into(),
        name: "growth".into(),
        description: None,
        members: Vec::new(),
        responder: crate::ports::types::ResponderMode::default(),
        hive: Default::default(),
    });

    assert_eq!(record.resolve_desk_id("growth").as_deref(), Some("growth"));
}

/// Desks resolve *before* teammates in `assignee::resolve`, by id and by
/// case-insensitive display name — so a minted id equal to either would be
/// unreachable, and both are stepped past.
#[test]
fn mint_agent_id_steps_past_desk_ids_and_desk_names() {
    let manifest = "[company]\nname = \"Acme\"\n\
         [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
         [[group_chat]]\nid = \"growth\"\nname = \"Content\"\nmembers = [\"ceo\"]\n";
    let mut record = desk_record(manifest, Vec::new());

    // By desk id.
    assert_eq!(record.mint_agent_id("Growth"), "growth_2");
    // By desk display name, which `resolve_desk_id` matches ignoring case.
    assert_eq!(record.mint_agent_id("content"), "content_2");

    // A desk name that is not itself slug-shaped is *not* reserved: nothing
    // routes on the slug of a desk name, only on the name as written, so
    // `content_desk` shadows no key that "Content Desk" answers to.
    record.overlay_desks.push(OverlayDesk {
        id: "design".into(),
        name: "Design Studio".into(),
        description: None,
        members: Vec::new(),
        responder: crate::ports::types::ResponderMode::default(),
        hive: Default::default(),
    });
    assert_eq!(record.mint_agent_id("Design Studio"), "design_studio");
    // …while the overlay desk's id is reserved exactly like a manifest one.
    assert_eq!(record.mint_agent_id("Design"), "design_2");
}

/// The operator channel and the workspace system roots are never handed to
/// a teammate, on an otherwise empty roster.
#[test]
fn mint_agent_id_never_returns_a_reserved_id() {
    let record = desk_record("[company]\nname = \"Acme\"\n", Vec::new());
    assert_eq!(record.mint_agent_id("Operator"), "operator_2");
    assert_eq!(record.mint_agent_id("Agents"), "agents_2");
    assert_eq!(record.mint_agent_id("desks"), "desks_2");
    assert_eq!(record.mint_agent_id("System"), "system_2");
    // Issue #1743: both spellings of the built-in `#general` channel. A
    // teammate minted onto one becomes the answer to every unaddressed
    // message on the company-wide line — `responder_for` checks roster ids
    // before falling back to the orchestrator — and the console renders
    // that line's transcript as the teammate's DM.
    assert_eq!(record.mint_agent_id("Main"), "main_2");
    assert_eq!(record.mint_agent_id("General"), "general_2");
    assert_eq!(
        RESERVED_AGENT_IDS,
        ["operator", "agents", "desks", "system", "main", "General"]
    );
}

/// Issue #966: the host's own author is not a name a teammate can be given.
///
/// `SYSTEM_AUTHOR` reaches the console's centred system pill by value —
/// `MessageView` projects an `AgentReply`'s `agent_id` straight into
/// `author`, and the console keys on the string. A teammate holding that id
/// would therefore render *as the host*, which is a worse confusion than the
/// one this issue set out to fix, and the value it replaces (`"operator"`)
/// was already reserved.
///
/// Its sibling `CONFINED_AGENT_ID` needs no entry here: `agent_slug` emits
/// only lowercase alphanumerics and underscores, so `"workflow-copilot"` is
/// unmintable by construction. `"system"` is an ordinary legal slug.
#[test]
fn mint_agent_id_never_returns_the_host_author() {
    let record = desk_record("[company]\nname = \"Acme\"\n", Vec::new());
    assert_eq!(
        agent_slug("System"),
        crate::ports::SYSTEM_AUTHOR,
        "the guard is needed precisely because this is a legal slug"
    );
    assert_ne!(
        record.mint_agent_id("System"),
        crate::ports::SYSTEM_AUTHOR,
        "a teammate must never be minted onto the id the runtime speaks under"
    );
}

/// Issue #1162: the resolve every surface that takes a teammate key runs.
/// An id resolves, an overlay teammate's **display name** resolves to the
/// id it was minted under, and a key that is nobody resolves to nothing.
#[test]
fn resolve_teammate_key_takes_an_id_or_a_display_name() {
    let manifest = "[company]\nname = \"Acme\"\n[[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n";
    let mut record = desk_record(manifest, Vec::new());
    record.overlay_agents.push(OverlayAgent {
        provider: None,
        id: "dana_designer".into(),
        name: "Dana Designer".into(),
        role: "Designer".into(),
        description: None,
        tools: None,
        model: None,
        harness: None,
    });

    assert_eq!(
        record.resolve_teammate_key("ceo"),
        TeammateResolution::Agent("ceo".into())
    );
    assert_eq!(
        record.resolve_teammate_key("dana_designer"),
        TeammateResolution::Agent("dana_designer".into())
    );
    // The case #1162 is about: the name `query_company` prints, grounding
    // to the id the delegation tools accept.
    assert_eq!(
        record.resolve_teammate_key("Dana Designer"),
        TeammateResolution::Agent("dana_designer".into())
    );
    assert_eq!(
        record.resolve_teammate_key("  dana designer  "),
        TeammateResolution::Agent("dana_designer".into())
    );
    assert_eq!(
        record.resolve_teammate_key("ghost"),
        TeammateResolution::Unknown
    );
    assert_eq!(
        record.resolve_teammate_key("   "),
        TeammateResolution::Unknown
    );
}

/// Ids win. A teammate whose **display name** is another teammate's id can
/// never intercept work meant for that id — the ordering is the guarantee
/// that makes one shared resolver safe to use everywhere.
#[test]
fn resolve_teammate_key_never_lets_a_name_shadow_an_id() {
    let manifest = "[company]\nname = \"Acme\"\n[[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n";
    let mut record = desk_record(manifest, Vec::new());
    record.overlay_agents.push(OverlayAgent {
        provider: None,
        id: "impostor".into(),
        name: "ceo".into(),
        role: "Growth".into(),
        description: None,
        tools: None,
        model: None,
        harness: None,
    });
    assert_eq!(
        record.resolve_teammate_key("ceo"),
        TeammateResolution::Agent("ceo".into())
    );
}

/// Two teammates answering to one display name is a collision the operator
/// created, and it is reported as one: every colliding id comes back, so a
/// caller can name them instead of silently taking the first.
#[test]
fn resolve_teammate_key_reports_a_name_two_teammates_answer_to() {
    let mut record = desk_record("[company]\nname = \"Acme\"\n", Vec::new());
    for id in ["dana_designer", "dana_designer_2"] {
        record.overlay_agents.push(OverlayAgent {
            provider: None,
            id: id.into(),
            name: "Dana Designer".into(),
            role: "Designer".into(),
            description: None,
            tools: None,
            model: None,
            harness: None,
        });
    }
    assert_eq!(
        record.resolve_teammate_key("dana designer"),
        TeammateResolution::Ambiguous(vec!["dana_designer".into(), "dana_designer_2".into()])
    );
    // Either id still resolves on its own — the collision is in the name.
    assert_eq!(
        record.resolve_teammate_key("dana_designer_2"),
        TeammateResolution::Agent("dana_designer_2".into())
    );
}

/// Whatever is minted is a legal roster id, suffix included — the same
/// grammar the manifest validator holds a hand-authored id to.
#[test]
fn every_minted_id_satisfies_the_manifest_id_grammar() {
    let mut record = mint_record();
    for name in [
        "Dana Designer",
        "Backend Engineer",
        "***",
        "24/7 Support",
        "Operator",
        "設計者",
    ] {
        let id = record.mint_agent_id(name);
        assert!(
            crate::company::is_snake_case(&id),
            "minted id {id:?} from {name:?} is not a legal roster id"
        );
        add_overlay(&mut record, &id, name);
    }
}

/// The persisted overlay blob reads both the current object form and the
/// legacy bare-`overlay_agents`-array form, so existing sqlite/mongo rows
/// load without a migration.
#[test]
fn overlay_blob_parses_object_and_legacy_array() {
    let object = r#"{"agents":[{"id":"a","name":"A","role":"r"}],"desk_members":[{"desk_id":"d","agent_id":"a"}]}"#;
    let blob = OverlayBlob::parse(object).expect("object");
    assert_eq!(blob.agents.len(), 1);
    assert_eq!(blob.desk_members.len(), 1);
    // Issue #85: an object written before provenance existed omits the key;
    // `#[serde(default)]` loads it as `None` (zero-migration back-compat).
    assert!(blob.provenance.is_none());

    // Legacy: overlay_json used to hold a bare Vec<OverlayAgent>.
    let legacy = r#"[{"id":"a","name":"A","role":"r"}]"#;
    let blob = OverlayBlob::parse(legacy).expect("legacy array");
    assert_eq!(blob.agents.len(), 1);
    assert!(blob.desk_members.is_empty());
    assert!(blob.provenance.is_none());

    // The empty-array default persisted by fresh schema.
    let blob = OverlayBlob::parse("[]").expect("empty array");
    assert!(blob.agents.is_empty());
    assert!(blob.desk_members.is_empty());
    assert!(blob.provenance.is_none());
    assert!(blob.desks.is_empty());

    // A pre-desk-creation object row (no `desks` key) loads with an empty
    // desk overlay — no migration needed.
    let pre_desks = r#"{"agents":[],"desk_members":[]}"#;
    let blob = OverlayBlob::parse(pre_desks).expect("pre-desks object");
    assert!(blob.desks.is_empty());
}

/// Issue #85: a record's template provenance round-trips through the
/// `OverlayBlob` the sqlite/mongodb stores persist, and a blob carrying
/// provenance re-parses with it intact.
#[test]
fn overlay_blob_carries_template_provenance() {
    let mut record = desk_record("[company]\nname = \"Acme\"\n", Vec::new());
    record.template_provenance = Some(TemplateProvenance {
        source_id: "law_firm".to_string(),
        version: Some("2.0.0".to_string()),
        path: Some("companies/law_firm".to_string()),
    });
    let json = serde_json::to_string(&OverlayBlob::from_record(&record)).expect("serialize");
    let blob = OverlayBlob::parse(&json).expect("reparse");
    assert_eq!(blob.provenance, record.template_provenance);
}

/// Issue #168: a runtime-authored workflow body round-trips through the
/// `OverlayBlob` the sqlite/mongodb stores persist as `overlay_json`. On a
/// hosted tenant this blob is the ONLY copy of the graph, so a serialization
/// gap here would silently delete the workflow.
#[test]
fn overlay_blob_round_trips_workflows() {
    let mut record = desk_record("[company]\nname = \"Acme\"\n", Vec::new());
    record.overlay_workflows.push(OverlayWorkflow {
        id: "greeter".to_string(),
        toml: "id = \"greeter\"\nname = \"Greeter\"\n".to_string(),
    });
    let json = serde_json::to_string(&OverlayBlob::from_record(&record)).expect("serialize");
    let blob = OverlayBlob::parse(&json).expect("reparse");
    assert_eq!(blob.workflows, record.overlay_workflows);

    // A row written before workflow bodies persisted (no `workflows` key)
    // loads as empty — no migration needed.
    let legacy = r#"{"agents":[],"desk_members":[]}"#;
    assert!(
        OverlayBlob::parse(legacy)
            .expect("pre-workflows object")
            .workflows
            .is_empty()
    );
    // …and so does the legacy bare-array form.
    assert!(
        OverlayBlob::parse("[]")
            .expect("legacy array")
            .workflows
            .is_empty()
    );
}

/// Issue #276: the paused-workflow ids ride the same overlay blob as the
/// graph bodies, reconstructed on load by both string-column stores
/// (`sqlite` and `mongodb` read `OverlayBlob::parse`). A round trip here
/// pins that the field is not dropped in `from_record`/`parse`.
#[test]
fn overlay_blob_round_trips_disabled_workflows() {
    let mut record = desk_record("[company]\nname = \"Acme\"\n", Vec::new());
    record.disabled_workflows.push("digest".to_string());
    let json = serde_json::to_string(&OverlayBlob::from_record(&record)).expect("serialize");
    let blob = OverlayBlob::parse(&json).expect("reparse");
    assert_eq!(blob.disabled_workflows, record.disabled_workflows);

    // A row written before the pause switch existed holds no `disabled_workflows`
    // key and loads as empty — the pre-#276 behaviour, no migration needed.
    let legacy = r#"{"agents":[],"desk_members":[]}"#;
    assert!(
        OverlayBlob::parse(legacy)
            .expect("pre-#276 object")
            .disabled_workflows
            .is_empty()
    );
    assert!(
        OverlayBlob::parse("[]")
            .expect("legacy array")
            .disabled_workflows
            .is_empty()
    );
}

/// A manifest with two teammates, one capped at $5/day and one uncapped —
/// the two starting positions every budget-override case builds on.
const BUDGET_ROSTER: &str = "[company]\nname = \"Acme\"\n\
     [[agent]]\nid = \"analyst\"\nrole = \"Analyst\"\nbudget_usd_daily = 5.0\n\
     [[agent]]\nid = \"writer\"\nrole = \"Writer\"\n";

fn budget_entry(agent_id: &str, cap: Option<f64>) -> BudgetOverride {
    BudgetOverride {
        agent_id: agent_id.to_string(),
        budget_usd_daily: cap,
        set_by: Actor {
            kind: ActorKind::User,
            id: "user-1".to_string(),
        },
        at_millis: 1_700_000_000_000,
    }
}

// ---- `[policy]` override (issue #562) --------------------------------

const POLICY_MANIFEST: &str = "[company]\nname = \"Acme\"\n\
     [[agent]]\nid = \"analyst\"\nrole = \"Analyst\"\n\
     [policy]\nmode = \"supervised\"\n\
     always_approve = [\"payment.send\", \"filing.submit\"]\n";

fn policy_entry(mode: Option<&str>, always: Option<Vec<&str>>) -> PolicyOverride {
    PolicyOverride {
        mode: mode.map(str::to_string),
        always_approve: always.map(|v| v.into_iter().map(str::to_string).collect()),
        auto_approve_under_usd: None,
        approval_ttl_hours: None,
        set_by: Actor {
            kind: ActorKind::User,
            id: "user-1".to_string(),
        },
        at_millis: 1_700_000_000_000,
    }
}

#[test]
fn explicit_no_cap_policy_override_survives_json_round_trip() {
    let mut override_ = policy_entry(None, None);
    override_.auto_approve_under_usd = Some(None);
    let encoded = serde_json::to_value(&override_).expect("serialize override");
    assert!(encoded["auto_approve_under_usd"].is_null());
    let decoded: PolicyOverride = serde_json::from_value(encoded).expect("deserialize override");
    assert_eq!(decoded.auto_approve_under_usd, Some(None));
}

/// With no override stored, `effective_policy` is the manifest verbatim —
/// the pre-#562 behaviour, and the net that says adding this field changed
/// nothing for a company that never uses it.
#[test]
fn effective_policy_falls_back_to_the_manifest() {
    let record = desk_record(POLICY_MANIFEST, Vec::new());
    let effective = record.effective_policy();
    assert_eq!(effective.mode, "supervised");
    assert_eq!(
        effective.always_approve,
        vec!["payment.send", "filing.submit"]
    );
}

/// A stored override beats the manifest. This is the "no redeploy" property
/// at its source: nothing here consults `company.toml` once a row exists.
#[test]
fn a_stored_policy_override_beats_the_manifest() {
    let mut record = desk_record(POLICY_MANIFEST, Vec::new());
    record.overlay_policy = Some(policy_entry(Some("full"), None));
    assert_eq!(record.effective_policy().mode, "full");
}

/// Version skew can leave an override written by a newer host on a build
/// that does not recognise its tier. Falling through to `supervised` would
/// loosen a `readonly` seed, so the manifest wins for that field while any
/// independently valid always-ask override remains in force.
#[test]
fn an_unknown_stored_policy_mode_cannot_loosen_the_manifest() {
    let manifest = POLICY_MANIFEST.replace("mode = \"supervised\"", "mode = \"readonly\"");
    let mut record = desk_record(&manifest, Vec::new());
    record.overlay_policy = Some(policy_entry(
        Some("future-tier"),
        Some(vec!["external.publish"]),
    ));

    let effective = record.effective_policy();
    assert_eq!(effective.mode, "readonly");
    assert_eq!(effective.always_approve, vec!["external.publish"]);
}

/// The two fields are independent: moving the tier must not silently reset
/// the always-ask list to the manifest's, nor the reverse.
///
/// This is the merge that makes the console usable — the tier control and
/// the always-ask editor are separate widgets, and each `PUT` names only
/// what it changed. If either field reset the other, using one control would
/// quietly undo the other, and the always-ask list is the operator's real
/// lever: it wins over every tier including `full`.
#[test]
fn overriding_one_policy_field_leaves_the_other_alone() {
    let mut record = desk_record(POLICY_MANIFEST, Vec::new());

    record.overlay_policy = Some(policy_entry(Some("full"), None));
    let effective = record.effective_policy();
    assert_eq!(effective.mode, "full");
    assert_eq!(
        effective.always_approve,
        vec!["payment.send", "filing.submit"],
        "moving the tier must not discard the manifest's always-ask list"
    );

    record.overlay_policy = Some(policy_entry(None, Some(vec!["external.publish"])));
    let effective = record.effective_policy();
    assert_eq!(
        effective.mode, "supervised",
        "editing the always-ask list must not move the tier"
    );
    assert_eq!(effective.always_approve, vec!["external.publish"]);
}

/// An emptied always-ask list is a real state, not a fallback.
///
/// `Some(vec![])` is an operator deliberately clearing the list; `None` is
/// "not overridden". If these collapsed, an operator clearing the list would
/// instead get the manifest's three defaults back — silently re-imposing the
/// gates they had just removed, and with no way to express what they meant.
#[test]
fn an_emptied_always_approve_list_is_not_a_fallback() {
    let mut record = desk_record(POLICY_MANIFEST, Vec::new());

    record.overlay_policy = Some(policy_entry(None, Some(vec![])));
    assert!(
        record.effective_policy().always_approve.is_empty(),
        "an explicitly emptied always-ask list must survive as empty"
    );

    record.overlay_policy = Some(policy_entry(None, None));
    assert_eq!(
        record.effective_policy().always_approve,
        vec!["payment.send", "filing.submit"],
        "an absent field must fall through to the manifest"
    );
}

/// The spend threshold and deadline are overridden independently of the
/// tier and list, including an explicit no-cap choice.
#[test]
fn spend_threshold_and_deadline_can_be_overridden_independently() {
    let manifest = "[company]\nname = \"Acme\"\n\
         [[agent]]\nid = \"analyst\"\nrole = \"Analyst\"\n\
         [policy]\nmode = \"supervised\"\nauto_approve_under_usd = 2.5\n";
    let mut record = desk_record(manifest, Vec::new());
    record.overlay_policy = Some(policy_entry(Some("full"), Some(vec![])));
    assert_eq!(record.effective_policy().auto_approve_under_usd, Some(2.5));
    assert_eq!(record.effective_policy().approval_ttl_hours, None);

    let override_ = record.overlay_policy.as_mut().unwrap();
    override_.auto_approve_under_usd = Some(None);
    override_.approval_ttl_hours = Some(72);
    let effective = record.effective_policy();
    assert_eq!(effective.auto_approve_under_usd, None);
    assert_eq!(effective.approval_ttl_hours, Some(72));
}

/// The roster a company was launched with is still the roster it runs, until
/// somebody edits it: with no override stored, every field reads straight off
/// the manifest. The regression net that says adding this layer changed
/// nothing for a company that never uses it.
#[test]
fn an_unedited_teammate_reads_straight_off_the_manifest() {
    let record = desk_record(EDIT_ROSTER, Vec::new());
    let analyst = record.effective_agent("analyst").expect("on the roster");
    assert!(matches!(analyst, std::borrow::Cow::Borrowed(_)));
    assert_eq!(analyst.role, "Analyst");
    assert_eq!(analyst.description.as_deref(), Some("Weighs evidence."));
    assert_eq!(analyst.name, None);
    assert!(record.effective_agent("nobody").is_none());
}

/// An edit wins over the blueprint, field by field — and only field by
/// field: what nobody touched keeps tracking `company.toml`, so a redeploy
/// that changes it is still felt.
#[test]
fn an_edit_wins_per_field_and_the_rest_still_tracks_the_manifest() {
    let mut record = desk_record(EDIT_ROSTER, Vec::new());
    record.upsert_agent_override(AgentOverride {
        agent_id: "analyst".to_string(),
        role: Some("Chief Vibes".to_string()),
        name: Some("Robin".to_string()),
        ..Default::default()
    });

    let analyst = record.effective_agent("analyst").expect("on the roster");
    assert_eq!(analyst.role, "Chief Vibes");
    assert_eq!(analyst.name.as_deref(), Some("Robin"));
    assert_eq!(
        analyst.description.as_deref(),
        Some("Weighs evidence."),
        "an untouched field must still come from the manifest"
    );
    assert_eq!(
        analyst.tools,
        Some(vec!["workspace.read".to_string()]),
        "and so must an untouched tool line"
    );
    // The blueprint itself is never rewritten — that is the whole point of
    // storing this as an overlay.
    assert_eq!(record.manifest.agents[0].role, "Analyst");
}

/// A stored empty description is the operator clearing it, not a teammate
/// whose instructions are the empty string. Collapsing the two would leave a
/// cleared description silently re-inheriting the blueprint's.
#[test]
fn a_cleared_description_stays_cleared() {
    let mut record = desk_record(EDIT_ROSTER, Vec::new());
    record.upsert_agent_override(AgentOverride {
        agent_id: "analyst".to_string(),
        description: Some(String::new()),
        ..Default::default()
    });
    assert_eq!(
        record.effective_agent("analyst").unwrap().description,
        None,
        "a cleared description must not fall back to the manifest's"
    );
}

/// Two patches of different fields are one override, merged — never two
/// rows, of which `effective_agent` would read whichever came first.
#[test]
fn a_second_edit_merges_rather_than_duplicating() {
    let mut record = desk_record(EDIT_ROSTER, Vec::new());
    record.upsert_agent_override(AgentOverride {
        agent_id: "analyst".to_string(),
        role: Some("Chief Vibes".to_string()),
        ..Default::default()
    });
    record.upsert_agent_override(AgentOverride {
        agent_id: "analyst".to_string(),
        // Double-option since #1804: `Some(Some(globs))` narrows.
        tools: Some(Some(vec!["composio".to_string()])),
        ..Default::default()
    });

    assert_eq!(record.overlay_agent_edits.len(), 1);
    let analyst = record.effective_agent("analyst").unwrap();
    assert_eq!(analyst.role, "Chief Vibes", "the earlier edit survives");
    assert_eq!(analyst.tools, Some(vec!["composio".to_string()]));
}

/// A removed teammate is off the roster everywhere the roster is read: the
/// effective list, the per-id lookup, and the membership predicate the desk
/// overlay validates against. Anything that still answered `true` here would
/// be a surface on which a deleted teammate is still addressable.
#[test]
fn a_retired_teammate_is_off_the_roster() {
    let mut record = desk_record(EDIT_ROSTER, Vec::new());
    assert!(record.is_roster_agent("analyst"));

    record.retire_agent("analyst");
    assert!(record.is_retired("analyst"));
    assert!(record.effective_agent("analyst").is_none());
    assert!(record.effective_agents().is_empty());
    assert!(!record.is_roster_agent("analyst"));
    // The blueprint is untouched — the tombstone is what removes it, which
    // is the only thing that survives the manifest being re-read on load.
    assert_eq!(record.manifest.agents[0].id, "analyst");
}

/// Retiring twice is one tombstone. A second entry changes nothing about the
/// roster but does move the harness's overlay fingerprint, which would drop
/// every live agent session for a delete that had already happened.
#[test]
fn retiring_a_teammate_twice_records_one_tombstone() {
    let mut record = desk_record(EDIT_ROSTER, Vec::new());
    record.retire_agent("analyst");
    record.retire_agent("analyst");
    assert_eq!(record.overlay_retired_agents, vec!["analyst".to_string()]);
}

/// A removed teammate loses its blueprint desk seat too. Left in place it
/// would still lead the desk, still take `delegate_to_desk` hand-offs and
/// still sit on the org chart — a delete that removed the card and nothing
/// else.
#[test]
fn a_retired_teammate_loses_its_desk_seat() {
    let manifest = "[company]\nname = \"Acme\"\n\
         [[agent]]\nid = \"analyst\"\nrole = \"Analyst\"\n\
         [[agent]]\nid = \"writer\"\nrole = \"Writer\"\n\
         [[group_chat]]\nid = \"studio\"\nname = \"Studio\"\n\
         members = [\"analyst\", \"writer\"]\n";
    let mut record = desk_record(manifest, Vec::new());
    assert_eq!(
        record.effective_desk_members("studio"),
        ["analyst", "writer"]
    );

    record.retire_agent("analyst");
    assert_eq!(
        record.effective_desk_members("studio"),
        ["writer"],
        "and the desk's lead moves to whoever is actually left"
    );
}

const EDIT_ROSTER: &str = "[company]\nname = \"Acme\"\n\
     [[agent]]\nid = \"analyst\"\nrole = \"Analyst\"\n\
     description = \"Weighs evidence.\"\ntools = [\"workspace.read\"]\n";

/// Issue #343: with no override stored, `effective_budget` is the manifest
/// value verbatim — the pre-#343 behaviour, and the regression net that says
/// adding this field changed nothing for a company that never uses it.
#[test]
fn effective_budget_falls_back_to_the_manifest() {
    let record = desk_record(BUDGET_ROSTER, Vec::new());
    assert_eq!(record.effective_budget("analyst"), Some(5.0));
    assert_eq!(record.effective_budget("writer"), None);
    // An id on no roster at all is uncapped rather than an error: the gate
    // reads this per dispatched agent and must not invent a cap.
    assert_eq!(record.effective_budget("nobody"), None);
}

/// A stored override wins over the manifest in both directions — raising a
/// cap and lowering one. This is the "no redeploy" property at its source:
/// nothing here consults `company.toml` once a row exists.
#[test]
fn a_stored_override_beats_the_manifest() {
    let mut record = desk_record(BUDGET_ROSTER, Vec::new());
    record
        .overlay_budgets
        .push(budget_entry("analyst", Some(50.0)));
    assert_eq!(record.effective_budget("analyst"), Some(50.0));

    record.overlay_budgets = vec![budget_entry("analyst", Some(1.0))];
    assert_eq!(record.effective_budget("analyst"), Some(1.0));
}

/// The distinction the issue calls out by name: clearing a cap and setting
/// it to zero are different states and must not collapse into each other.
///
/// `Some(0.0)` caps the teammate at nothing (it will refuse to dispatch);
/// `None` means explicitly uncapped and beats the manifest's $5. If these
/// two ever resolved the same way, an operator lifting a cap would instead
/// have silenced the teammate completely — the opposite of what they asked
/// for, and unrecoverable from the console.
#[test]
fn clearing_a_cap_is_not_the_same_as_zeroing_it() {
    let mut record = desk_record(BUDGET_ROSTER, Vec::new());

    record.overlay_budgets = vec![budget_entry("analyst", Some(0.0))];
    assert_eq!(record.effective_budget("analyst"), Some(0.0));

    record.overlay_budgets = vec![budget_entry("analyst", None)];
    assert_eq!(
        record.effective_budget("analyst"),
        None,
        "an explicitly-uncapped override must beat the manifest's cap"
    );
}

/// An **overlay** teammate has no manifest row, so before #343 it could not
/// be capped at all. A stored override caps it like anyone else — and
/// dropping that override returns it to uncapped, since there is no manifest
/// value underneath to fall back to.
#[test]
fn an_overlay_teammate_can_be_capped() {
    let mut record = desk_record(BUDGET_ROSTER, Vec::new());
    record.overlay_agents.push(OverlayAgent {
        provider: None,
        id: "shane".to_string(),
        name: "Shane".to_string(),
        role: "Growth".to_string(),
        description: None,
        tools: None,
        model: None,
        harness: None,
    });
    assert_eq!(record.effective_budget("shane"), None);

    record.overlay_budgets = vec![budget_entry("shane", Some(2.5))];
    assert_eq!(record.effective_budget("shane"), Some(2.5));

    record.overlay_budgets.clear();
    assert_eq!(record.effective_budget("shane"), None);
}

/// Issue #343: one override per teammate. `upsert_budget_override` replaces
/// the held row instead of appending a second, so the cap an admin last set
/// is the cap every surface reads.
///
/// Appending would leave the *first* row winning `budget_override`'s
/// find-first read — meaning a raise or a revocation would persist happily
/// and change nothing, the failure mode hardest to notice from the console.
#[test]
fn upserting_an_override_replaces_rather_than_appends() {
    let mut record = desk_record(BUDGET_ROSTER, Vec::new());
    record.upsert_budget_override(budget_entry("analyst", Some(50.0)));
    record.upsert_budget_override(budget_entry("writer", Some(3.0)));
    record.upsert_budget_override(budget_entry("analyst", None));

    assert_eq!(
        record.overlay_budgets.len(),
        2,
        "a second write for one teammate must replace, not accumulate: {:?}",
        record.overlay_budgets
    );
    assert_eq!(
        record.effective_budget("analyst"),
        None,
        "the latest write must win over the manifest's $5"
    );
    assert_eq!(record.effective_budget("writer"), Some(3.0));
}

/// Issue #343: duplicates are detectable, so a caller holding overrides it
/// did not write (a bundle import) can refuse them instead of silently
/// applying whichever row happens to sort first.
#[test]
fn duplicate_overrides_are_detected() {
    let mut record = desk_record(BUDGET_ROSTER, Vec::new());
    assert_eq!(record.duplicate_budget_agent_id(), None);

    record.overlay_budgets = vec![
        budget_entry("analyst", Some(9.0)),
        budget_entry("writer", None),
    ];
    assert_eq!(
        record.duplicate_budget_agent_id(),
        None,
        "distinct teammates are not a duplicate"
    );

    // Two rows for one teammate that disagree about the cap — the case where
    // guessing would either over-restrict or hand back a revoked allowance.
    record.overlay_budgets = vec![
        budget_entry("analyst", Some(9.0)),
        budget_entry("writer", None),
        budget_entry("analyst", Some(0.0)),
    ];
    assert_eq!(record.duplicate_budget_agent_id(), Some("analyst"));
}

/// Issue #343: the budget overrides round-trip through the `OverlayBlob` the
/// sqlite/mongodb stores persist, and pre-#343 rows load as "no overrides"
/// (the manifest still decides) rather than failing to parse.
#[test]
fn overlay_blob_round_trips_budgets() {
    let mut record = desk_record(BUDGET_ROSTER, Vec::new());
    record.overlay_budgets = vec![
        budget_entry("analyst", Some(9.0)),
        budget_entry("writer", None),
    ];
    let json = serde_json::to_string(&OverlayBlob::from_record(&record)).expect("serialize");
    let blob = OverlayBlob::parse(&json).expect("reparse");
    assert_eq!(blob.budgets, record.overlay_budgets);

    let legacy = r#"{"agents":[],"desk_members":[]}"#;
    assert!(
        OverlayBlob::parse(legacy)
            .expect("pre-budget object")
            .budgets
            .is_empty()
    );
    assert!(
        OverlayBlob::parse("[]")
            .expect("legacy array")
            .budgets
            .is_empty()
    );
}

// ---- per-agent persona override (issue #1530) ------------------------

/// A roster with one manifest agent carrying a blueprint `prompt` and one
/// without — the two starting positions every persona-override case builds on.
const PERSONA_ROSTER: &str = "[company]\nname = \"Acme\"\n\
     [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\nprompt = \"Blueprint persona.\"\n\
     [[agent]]\nid = \"eng\"\nrole = \"Engineer\"\n";

fn override_entry(agent_id: &str, instructions: Option<&str>) -> AgentOverride {
    AgentOverride {
        agent_id: agent_id.to_string(),
        instructions: instructions.map(str::to_string),
        ..Default::default()
    }
}

/// A stored override wins over the manifest `prompt`: this is how a
/// manifest/blueprint agent's persona is edited without rewriting
/// `company.toml`.
#[test]
fn effective_instructions_prefers_override() {
    let mut record = desk_record(PERSONA_ROSTER, Vec::new());
    record.overlay_agent_edits = vec![override_entry("ceo", Some("Be terse."))];
    assert_eq!(
        record.effective_instructions("ceo"),
        Some("Be terse.".to_string())
    );
}

/// With no override stored, the manifest `prompt` is returned verbatim — the
/// pre-#1530 behaviour, and the net that says adding the field changed
/// nothing for a company that never edits a persona.
#[test]
fn effective_instructions_falls_back_to_manifest_prompt() {
    let record = desk_record(PERSONA_ROSTER, Vec::new());
    assert_eq!(
        record.effective_instructions("ceo"),
        Some("Blueprint persona.".to_string())
    );
}

/// A bare overlay teammate (no manifest row) and a manifest agent that
/// declares no `prompt` both resolve to `None` when nothing overrides them.
#[test]
fn effective_instructions_none_for_bare_overlay_or_promptless_agent() {
    let record = desk_record(PERSONA_ROSTER, Vec::new());
    assert_eq!(record.effective_instructions("eng"), None);
    assert_eq!(record.effective_instructions("nobody"), None);
}

/// An override whose `instructions` is `None` carries nothing, so resolution
/// falls through to the blueprint — the "reset to blueprint" contract. A
/// stored empty-instructions row must never blank the persona.
#[test]
fn effective_instructions_empty_override_resets_to_blueprint() {
    let mut record = desk_record(PERSONA_ROSTER, Vec::new());
    record.overlay_agent_edits = vec![override_entry("ceo", None)];
    assert_eq!(
        record.effective_instructions("ceo"),
        Some("Blueprint persona.".to_string()),
        "an override that carries no instructions must fall through to the manifest"
    );
}

/// `upsert_agent_override` replaces the teammate's row in place rather than
/// accumulating a second one — the invariant `agent_override`'s first-match
/// read depends on.
#[test]
fn upsert_agent_override_replaces_not_appends() {
    let mut record = desk_record(PERSONA_ROSTER, Vec::new());
    record.upsert_agent_override(override_entry("ceo", Some("first")));
    record.upsert_agent_override(override_entry("ceo", Some("second")));
    assert_eq!(record.overlay_agent_edits.len(), 1);
    assert_eq!(
        record.effective_instructions("ceo"),
        Some("second".to_string())
    );
}

/// `clear_agent_override` drops the row so the blueprint applies again, and
/// is a no-op when nothing is stored.
#[test]
fn clear_agent_override_drops_the_row() {
    let mut record = desk_record(PERSONA_ROSTER, Vec::new());
    record.upsert_agent_override(override_entry("ceo", Some("custom")));
    record.clear_agent_override("ceo");
    assert!(record.overlay_agent_edits.is_empty());
    assert_eq!(
        record.effective_instructions("ceo"),
        Some("Blueprint persona.".to_string())
    );
    // No-op when absent.
    record.clear_agent_override("ceo");
    assert!(record.overlay_agent_edits.is_empty());
}

// ---- per-agent avatar override --------------------------------------

/// Nobody has chosen until somebody does: an untouched roster resolves to
/// `None`, which the console renders as the mascot it hashes from the id.
#[test]
fn effective_avatar_is_none_until_chosen() {
    let mut record = desk_record(PERSONA_ROSTER, Vec::new());
    assert_eq!(record.effective_avatar("ceo"), None);
    record.upsert_agent_override(AgentOverride {
        agent_id: "ceo".into(),
        avatar: Some("tiny:teal".into()),
        ..Default::default()
    });
    assert_eq!(record.effective_avatar("ceo"), Some("tiny:teal".into()));
}

/// An overlay teammate has no manifest row, and picks a face through the
/// same field — one override answers for both kinds of teammate.
#[test]
fn effective_avatar_answers_for_an_overlay_teammate() {
    let mut record = desk_record(PERSONA_ROSTER, Vec::new());
    record.overlay_agents.push(OverlayAgent {
        provider: None,
        id: "alex".into(),
        name: "Alex".into(),
        role: "Writer".into(),
        description: None,
        tools: None,
        model: None,
        harness: None,
    });
    record.upsert_agent_override(AgentOverride {
        agent_id: "alex".into(),
        avatar: Some("blob:01J8Z5Q9YQ".into()),
        ..Default::default()
    });
    assert_eq!(
        record.effective_avatar("alex"),
        Some("blob:01J8Z5Q9YQ".into())
    );
}

/// Resetting a face says nothing about the persona. The two clear paths
/// touch one field each, so neither can quietly undo the other's edit —
/// this is the regression the shared retain helper exists to prevent.
#[test]
fn clearing_one_override_field_leaves_the_others() {
    let mut record = desk_record(PERSONA_ROSTER, Vec::new());
    record.upsert_agent_override(AgentOverride {
        agent_id: "ceo".into(),
        instructions: Some("Be terse.".into()),
        ..Default::default()
    });
    record.upsert_agent_override(AgentOverride {
        agent_id: "ceo".into(),
        avatar: Some("tiny:rose".into()),
        ..Default::default()
    });

    record.clear_agent_avatar("ceo");
    assert_eq!(record.effective_avatar("ceo"), None);
    assert_eq!(
        record.effective_instructions("ceo"),
        Some("Be terse.".to_string()),
        "resetting a face must not reset the persona"
    );

    record.clear_agent_override("ceo");
    assert!(
        record.overlay_agent_edits.is_empty(),
        "the row goes once it carries nothing"
    );
}

/// The mirror of the above, and the sharper half: an avatar-only override
/// must survive a persona reset. Before the shared retain helper, the
/// persona path's `retain` did not know the field existed and dropped the
/// whole row — resetting a persona silently reset the face too.
#[test]
fn clearing_the_persona_keeps_an_avatar_only_override() {
    let mut record = desk_record(PERSONA_ROSTER, Vec::new());
    record.upsert_agent_override(AgentOverride {
        agent_id: "ceo".into(),
        avatar: Some("tiny:rose".into()),
        ..Default::default()
    });
    record.clear_agent_override("ceo");
    assert_eq!(record.effective_avatar("ceo"), Some("tiny:rose".into()));
}

/// Duplicates are detectable, so a caller holding overrides it did not write
/// (a bundle import) can refuse them rather than apply whichever sorts first.
#[test]
fn duplicate_override_agent_id_detects() {
    let mut record = desk_record(PERSONA_ROSTER, Vec::new());
    assert_eq!(
        AgentOverride::duplicate_agent_id(&record.overlay_agent_edits),
        None
    );
    record.overlay_agent_edits = vec![
        override_entry("ceo", Some("a")),
        override_entry("eng", Some("b")),
        override_entry("ceo", Some("c")),
    ];
    assert_eq!(
        AgentOverride::duplicate_agent_id(&record.overlay_agent_edits),
        Some("ceo")
    );
}

/// An override carrying nothing is empty — and an override carrying only
/// `avatar`, `model` or `harness` is not, so a face-only edit or a
/// model-only edit is persisted rather than dropped as a no-op.
#[test]
fn agent_override_is_empty_only_when_nothing_is_set() {
    assert!(override_entry("ceo", None).is_empty());
    assert!(!override_entry("ceo", Some("x")).is_empty());

    for (field, fill) in [
        (
            "name",
            Box::new(|e: &mut AgentOverride| e.name = Some("Ada".to_string()))
                as Box<dyn Fn(&mut AgentOverride)>,
        ),
        (
            "role",
            Box::new(|e: &mut AgentOverride| e.role = Some("CEO".to_string())),
        ),
        (
            "description",
            Box::new(|e: &mut AgentOverride| e.description = Some("desc".to_string())),
        ),
        (
            "tools",
            Box::new(|e: &mut AgentOverride| e.tools = Some(Some(vec!["docs.*".to_string()]))),
        ),
        (
            "instructions",
            Box::new(|e: &mut AgentOverride| e.instructions = Some("Be terse.".to_string())),
        ),
        (
            "avatar",
            Box::new(|e: &mut AgentOverride| e.avatar = Some("tiny:teal".to_string())),
        ),
        (
            "model",
            Box::new(|e: &mut AgentOverride| e.model = Some("gpt-5".to_string())),
        ),
        (
            "harness",
            Box::new(|e: &mut AgentOverride| e.harness = Some("laptop".to_string())),
        ),
        (
            "provider",
            Box::new(|e: &mut AgentOverride| e.provider = Some("anthropic".to_string())),
        ),
    ] {
        let mut edit = override_entry("ceo", None);
        fill(&mut edit);
        assert!(
            !edit.is_empty(),
            "{field} alone must make the override non-empty"
        );
    }

    // The stored "cleared" form (`Some("")`, keys rework slice 3a) is also
    // non-empty — it is an edit an operator made, not a no-op, and
    // `retain_nonempty_agent_edits` must keep the row so the clear itself
    // is not silently forgotten.
    let mut cleared_provider = override_entry("ceo", None);
    cleared_provider.provider = Some(String::new());
    assert!(
        !cleared_provider.is_empty(),
        "a cleared provider must still count as an edit"
    );
}

/// `upsert_agent_override` carries the provider half of the pair (keys
/// rework slice 3a) exactly like `model`, and clearing it with `Some("")`
/// reads back as `None` on the effective agent while leaving an
/// untouched field (like `name`) alone.
#[test]
fn an_override_carries_and_clears_the_provider() {
    let mut record = desk_record(PERSONA_ROSTER, Vec::new());
    record.upsert_agent_override(AgentOverride {
        agent_id: "ceo".into(),
        provider: Some("anthropic".to_string()),
        model: Some("test-model-large".to_string()),
        ..Default::default()
    });
    // Cloned rather than borrowed from `record`: `effective_manifest_agent`
    // is re-called after each further mutation below, and a borrow held
    // across those would conflict with `upsert_agent_override`'s `&mut self`.
    let manifest_agent = record
        .manifest
        .agents
        .iter()
        .find(|a| a.id == "ceo")
        .cloned()
        .expect("ceo is on the manifest");
    let effective = record.effective_manifest_agent(&manifest_agent);
    assert_eq!(effective.provider.as_deref(), Some("anthropic"));
    assert_eq!(effective.model.as_deref(), Some("test-model-large"));

    record.upsert_agent_override(AgentOverride {
        agent_id: "ceo".into(),
        provider: Some(String::new()),
        model: Some(String::new()),
        ..Default::default()
    });
    let effective = record.effective_manifest_agent(&manifest_agent);
    assert_eq!(effective.provider, None);
    assert_eq!(effective.model, None);

    // An upsert that names neither leaves the stored provider alone.
    record.upsert_agent_override(AgentOverride {
        agent_id: "ceo".into(),
        provider: Some("groq".to_string()),
        model: Some("test-model-small".to_string()),
        ..Default::default()
    });
    record.upsert_agent_override(AgentOverride {
        agent_id: "ceo".into(),
        name: Some("Robin".to_string()),
        ..Default::default()
    });
    let effective = record.effective_manifest_agent(&manifest_agent);
    assert_eq!(effective.provider.as_deref(), Some("groq"));
    assert_eq!(effective.name.as_deref(), Some("Robin"));
}

/// An `OverlayAgent`'s `provider` round-trips through JSON, and a record
/// written before the field existed deserializes to `None` and
/// re-serializes with no `provider` key — the same absent-means-unset
/// contract `harness` already has.
#[test]
fn an_overlay_agent_round_trips_its_provider() {
    let with_provider = OverlayAgent {
        provider: Some("anthropic".to_string()),
        id: "a".into(),
        name: "A".into(),
        role: "r".into(),
        description: None,
        tools: None,
        model: Some("test-model-large".to_string()),
        harness: None,
    };
    let json = serde_json::to_value(&with_provider).unwrap();
    assert_eq!(json.get("provider"), Some(&serde_json::json!("anthropic")));
    let round: OverlayAgent = serde_json::from_value(json).unwrap();
    assert_eq!(round.provider.as_deref(), Some("anthropic"));

    let legacy: OverlayAgent =
        serde_json::from_str(r#"{"id":"a","name":"A","role":"r"}"#).expect("legacy overlay");
    assert_eq!(legacy.provider, None);
    let legacy_value = serde_json::to_value(&legacy).unwrap();
    assert!(
        legacy_value.get("provider").is_none(),
        "an absent provider must not serialize a `provider` key: {legacy_value}"
    );
}

/// The persona overrides round-trip through the `OverlayBlob` the
/// sqlite/mongodb stores persist, and pre-#1530 rows load as "no overrides"
/// (the manifest still decides) rather than failing to parse.
#[test]
fn overlay_blob_round_trips_agent_overrides() {
    let mut record = desk_record(PERSONA_ROSTER, Vec::new());
    record.overlay_agent_edits = vec![override_entry("ceo", Some("Be terse."))];
    let json = serde_json::to_string(&OverlayBlob::from_record(&record)).expect("serialize");
    let blob = OverlayBlob::parse(&json).expect("reparse");
    assert_eq!(blob.agent_edits, record.overlay_agent_edits);

    // A pre-#1530 object row (no `agent_overrides` key) loads as empty.
    let legacy = r#"{"agents":[],"desk_members":[]}"#;
    assert!(
        OverlayBlob::parse(legacy)
            .expect("pre-persona object")
            .agent_edits
            .is_empty()
    );
}

/// An operator-created overlay desk resolves through the same
/// `effective_desk_members` / `resolve_desk_id` / `desk_exists` helpers the
/// manifest desks use, so the REST list and the harness desk-lead resolver
/// treat it identically. Member additions still layer on top.
#[test]
fn overlay_desk_resolves_like_a_manifest_desk() {
    let manifest = "[company]\nname = \"Acme\"\n\
         [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
         [[agent]]\nid = \"eng\"\nrole = \"Engineer\"\n";
    let mut record = desk_record(manifest, Vec::new());
    record.overlay_desks.push(OverlayDesk {
        id: "growth".into(),
        name: "Growth".into(),
        description: None,
        members: vec!["eng".into()],
        responder: crate::ports::types::ResponderMode::default(),
        hive: Default::default(),
    });
    // Resolves by id and by case-insensitive name.
    assert_eq!(record.resolve_desk_id("growth").as_deref(), Some("growth"));
    assert_eq!(record.resolve_desk_id("GROWTH").as_deref(), Some("growth"));
    assert!(record.desk_exists("growth"));
    // Founding member is the lead; a later overlay addition appends.
    assert_eq!(
        record.effective_desk_members("growth"),
        vec!["eng".to_string()]
    );
    record.overlay_desk_members.push(OverlayDeskMember {
        desk_id: "growth".into(),
        agent_id: "ceo".into(),
    });
    assert_eq!(
        record.effective_desk_members("growth"),
        vec!["eng".to_string(), "ceo".to_string()]
    );
}

/// An **overlay** desk never answers to a General spelling (issue #1743).
///
/// `POST .../desks` accepted `general`, `main` and the display name
/// `General` until that issue, so an upgraded record can be carrying one.
/// Every routing decision on the built-in `#general` channel funnels
/// through this one resolver — `desk_lead` → `responder_for` picks who
/// answers, and `mentioned_agents` picks who `@everyone` names — so a desk
/// that resolves here takes the company-wide line over: the console shows
/// `#general` while that desk's lead answers it, and a broadcast meant for
/// the whole roster reaches only that desk's members.
///
/// Keyed on the **key being asked for**, not on the desk, which is what
/// keeps this a narrowing of one question rather than a retirement: the
/// same desk still resolves under its own non-General id.
#[test]
fn an_overlay_desk_does_not_answer_to_a_general_spelling() {
    let manifest = "[company]\nname = \"Acme\"\n\
         [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
         [[agent]]\nid = \"eng\"\nrole = \"Engineer\"\n";
    let mut record = desk_record(manifest, Vec::new());
    record.overlay_desks.push(OverlayDesk {
        id: "main".into(),
        name: "Front office".into(),
        description: None,
        responder: Default::default(),
        members: vec!["eng".into()],
        hive: Default::default(),
    });
    record.overlay_desks.push(OverlayDesk {
        id: "ops".into(),
        name: "General".into(),
        description: None,
        responder: Default::default(),
        members: vec!["ceo".into()],
        hive: Default::default(),
    });

    for spelling in ["", "main", "Main", "MAIN", "general", "General"] {
        assert_eq!(
            record.resolve_desk_id(spelling),
            None,
            "an overlay desk must not answer to {spelling:?}"
        );
    }
    // Both desks still exist and still route under their own ids — this
    // narrows one question, it does not take a desk away.
    assert_eq!(record.resolve_desk_id("ops").as_deref(), Some("ops"));
    assert!(record.desk_exists("main"));
    assert_eq!(
        record.effective_desk_members("main"),
        vec!["eng".to_string()]
    );
}

/// ...and it must not be reachable by its **display name** either.
///
/// The guard narrows the key being asked for, so `{id: "main", name: "Front
/// office"}` slipped through it: `Front office` is not a General spelling,
/// the name match fired, and the resolver returned `main` — an id that
/// `GET .../desks` filters out and that every desk mutation refuses. Its
/// lead would answer, and the reply would be journaled under a thread the
/// console renders no channel for: a conversation with no way back.
///
/// An overlay desk on a General id is unaddressable by design; it must be
/// unaddressable by *every* address.
#[test]
fn an_overlay_desk_on_a_general_id_is_unreachable_by_name_too() {
    let mut record = desk_record(
        "[company]\nname = \"Acme\"\n\
         [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
         [[agent]]\nid = \"eng\"\nrole = \"Engineer\"\n",
        Vec::new(),
    );
    record.overlay_desks.push(OverlayDesk {
        id: "main".into(),
        name: "Front office".into(),
        description: None,
        responder: Default::default(),
        members: vec!["eng".into()],
        hive: Default::default(),
    });
    assert_eq!(
        record.resolve_desk_id("Front office"),
        None,
        "an overlay desk whose id shadows General must not answer to its name"
    );
    assert_eq!(
        record.resolve_desk_id("front office"),
        None,
        "nor case-folded"
    );
    assert_eq!(record.resolve_desk_id("main"), None, "nor to the id itself");
    // An ordinary overlay desk is untouched — this narrows one desk, not the rule.
    let mut ordinary = desk_record(
        "[company]\nname = \"Acme\"\n\
         [[agent]]\nid = \"eng\"\nrole = \"Engineer\"\n",
        Vec::new(),
    );
    ordinary.overlay_desks.push(OverlayDesk {
        id: "ops".into(),
        name: "Front office".into(),
        description: None,
        responder: Default::default(),
        members: vec!["eng".into()],
        hive: Default::default(),
    });
    assert_eq!(
        ordinary.resolve_desk_id("Front office").as_deref(),
        Some("ops")
    );
    assert_eq!(ordinary.resolve_desk_id("ops").as_deref(), Some("ops"));
}

/// A desk the **manifest** declares under a General spelling is the
/// blueprint's own General desk, and this host has always honoured it
/// (issue #1743). The narrowing above is about overlay desks only; the
/// manifest arm of the resolver is searched first and is untouched.
#[test]
fn a_blueprint_desk_still_owns_a_general_spelling() {
    let record = desk_record(
        "[company]\nname = \"Acme\"\n\
         [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
         [[agent]]\nid = \"eng\"\nrole = \"Engineer\"\n\
         [[group_chat]]\nid = \"main\"\nname = \"Front office\"\nmembers = [\"eng\"]\n",
        Vec::new(),
    );
    assert_eq!(record.resolve_desk_id("main").as_deref(), Some("main"));
    assert_eq!(
        record.effective_desk_members("main"),
        vec!["eng".to_string()]
    );
    // And by display name, the other spelling `resolve_desk_id` matches.
    let named = desk_record(
        "[company]\nname = \"Acme\"\n\
         [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
         [[group_chat]]\nid = \"ops\"\nname = \"General\"\nmembers = [\"ceo\"]\n",
        Vec::new(),
    );
    assert_eq!(named.resolve_desk_id("General").as_deref(), Some("ops"));
    assert_eq!(named.resolve_desk_id("general").as_deref(), Some("ops"));
}

/// The overlay blob round-trips operator-created desks through its persisted
/// JSON form, so a created desk survives a store save/load cycle.
#[test]
fn overlay_blob_round_trips_desks() {
    let with_desks = r#"{"agents":[],"desk_members":[],"desks":[{"id":"growth","name":"Growth","members":["eng"]}]}"#;
    let blob = OverlayBlob::parse(with_desks).expect("object with desks");
    assert_eq!(blob.desks.len(), 1);
    assert_eq!(blob.desks[0].id, "growth");
    assert_eq!(blob.desks[0].members, vec!["eng".to_string()]);
    // Re-serialize and re-parse — the desk survives the round trip.
    let json = serde_json::to_string(&blob).expect("serialize");
    let again = OverlayBlob::parse(&json).expect("reparse");
    assert_eq!(again.desks, blob.desks);
}

// ── Issue #228: a workflow run's outcome is journaled ───────────────────

fn delivery(node: &str, status: DeliveryStatus) -> DeliveryReport {
    DeliveryReport {
        node: node.to_string(),
        kind: "owner".to_string(),
        target: Some("ada@example.com".to_string()),
        status,
        detail: "emailed the company's admin".to_string(),
        reason: crate::ports::DeliveryReason::OwnerEmailed,
    }
}

/// The full-bodied variant survives the JSONL round trip the journal puts
/// every event through — including the delivery rows, which are the whole
/// reason the event exists.
#[test]
fn workflow_run_finished_round_trips_with_every_field() {
    let event = CompanyEvent::WorkflowRunFinished {
        workflow_id: "digest".to_string(),
        scheduled: true,
        run_id: Some("run-1".to_string()),
        deliveries: vec![
            delivery("owner_summary", DeliveryStatus::Skipped),
            delivery("also_sent", DeliveryStatus::Sent),
        ],
        pending_approvals: vec!["review".to_string()],
        error: None,
        cancelled: false,
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    };
    assert_eq!(round_trip(&event), event);
}

/// The failed-run shape round-trips too. This is the arm that today only
/// warns to host stdout, so it is the one an operator most needs read back.
#[test]
fn workflow_run_finished_round_trips_a_failed_run() {
    let event = CompanyEvent::WorkflowRunFinished {
        workflow_id: "digest".to_string(),
        scheduled: true,
        run_id: None,
        deliveries: Vec::new(),
        pending_approvals: Vec::new(),
        error: Some("agent node `worker` had no inference source".to_string()),
        cancelled: false,
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    };
    assert_eq!(round_trip(&event), event);
}

/// The additive contract, both halves.
///
/// **Forward:** a minimal line — only the two required fields, exactly what
/// a future/older writer might emit — still loads, so no persisted journal
/// needs migrating.
///
/// **Backward:** an empty run serializes to *only* those two fields. Every
/// optional/collection field is `skip_serializing_if`, which is what keeps
/// the wire form of an outcome-less run minimal rather than littered with
/// nulls and `[]`s.
#[test]
fn workflow_run_finished_omits_and_defaults_its_optional_fields() {
    let json = r#"{"kind":"WorkflowRunFinished","workflow_id":"digest","scheduled":false}"#;
    let event: CompanyEvent = serde_json::from_str(json).expect("minimal line loads");
    assert_eq!(
        event,
        CompanyEvent::WorkflowRunFinished {
            workflow_id: "digest".to_string(),
            scheduled: false,
            run_id: None,
            deliveries: Vec::new(),
            pending_approvals: Vec::new(),
            error: None,
            cancelled: false,
            notices: Vec::new(),
            board: Vec::new(),
            blocked_nodes: Vec::new(),
            approvals: Vec::new(),
        }
    );
    // …and serializing it back emits nothing extra.
    let out = serde_json::to_string(&event).expect("serialize");
    assert!(!out.contains("run_id"), "{out}");
    assert!(!out.contains("deliveries"), "{out}");
    assert!(!out.contains("pending_approvals"), "{out}");
    assert!(!out.contains("error"), "{out}");
    // Issue #383's field joins the same contract, which is what makes it
    // replay-safe: absent decodes as `false`, and a non-cancelled run's line
    // is byte-identical to what it was before the field existed.
    assert!(!out.contains("cancelled"), "{out}");
}

/// Issue #661 (M5): a run's board rows round-trip, and a line written before
/// they existed still replays.
///
/// Three claims, and the last two are what make this additive rather than a
/// migration: the rows survive the round trip in camelCase; a run that touched
/// no card serializes with **no `board` key at all**, so every already-written
/// journal line stays byte-identical; and a pre-#661 line decodes as empty
/// rather than failing to decode.
#[test]
fn workflow_run_finished_round_trips_board_rows() {
    use crate::ports::workflow_runner::WorkflowBoardAction;

    let event = CompanyEvent::WorkflowRunFinished {
        workflow_id: "digest".to_string(),
        scheduled: true,
        run_id: Some("run-1".to_string()),
        deliveries: Vec::new(),
        pending_approvals: Vec::new(),
        error: None,
        cancelled: false,
        notices: Vec::new(),
        board: vec![WorkflowRunBoardRow {
            action: WorkflowBoardAction::Spawned,
            task_id: Some("card-1".to_string()),
            title: Some("Reply to the auditor".to_string()),
            assignee: None,
        }],
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    };
    assert_eq!(round_trip(&event), event);
    let out = serde_json::to_string(&event).expect("serialize");
    assert!(out.contains("\"action\":\"spawned\""), "{out}");
    assert!(out.contains("\"taskId\":\"card-1\""), "{out}");
    // Absent rather than null on the arm that has nothing to say.
    assert!(!out.contains("assignee"), "{out}");

    // A run that touched no card is byte-unchanged from pre-#661.
    let untouched = CompanyEvent::WorkflowRunFinished {
        workflow_id: "digest".to_string(),
        scheduled: true,
        run_id: Some("run-1".to_string()),
        deliveries: Vec::new(),
        pending_approvals: Vec::new(),
        error: None,
        cancelled: false,
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    };
    let out = serde_json::to_string(&untouched).expect("serialize");
    assert!(!out.contains("board"), "{out}");

    // And a line written before the field existed replays as empty.
    let legacy = serde_json::json!({
        "kind": "WorkflowRunFinished",
        "workflow_id": "digest",
        "scheduled": true,
        "run_id": "run-1"
    });
    let loaded: CompanyEvent =
        serde_json::from_value(legacy).expect("a pre-#661 journal line replays");
    let CompanyEvent::WorkflowRunFinished { board, .. } = loaded else {
        panic!("expected a WorkflowRunFinished");
    };
    assert!(board.is_empty());
}

/// Issue #383: a cancelled run round-trips, and is distinguishable from a
/// failed one by more than the absence of an error.
///
/// The pairing is the assertion. A cancelled run carries `cancelled: true`
/// **and** `error: None` — so a reader that only ever looked at `error`
/// (every reader before #383) sees a clean finish, which is exactly why the
/// console needed a new field rather than a new error string.
#[test]
fn workflow_run_finished_round_trips_a_cancelled_run() {
    let event = CompanyEvent::WorkflowRunFinished {
        workflow_id: "digest".to_string(),
        scheduled: false,
        run_id: Some("run-1".to_string()),
        deliveries: Vec::new(),
        pending_approvals: Vec::new(),
        error: None,
        cancelled: true,
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    };
    assert_eq!(round_trip(&event), event);

    let out = serde_json::to_string(&event).expect("serialize");
    assert_eq!(
        out,
        r#"{"kind":"WorkflowRunFinished","workflow_id":"digest","scheduled":false,"run_id":"run-1","cancelled":true}"#,
        "the cancelled line pins its exact wire shape"
    );
}

/// A pre-#383 line — the overwhelming majority of every journal on disk —
/// loads as not cancelled rather than failing to decode.
#[test]
fn a_pre_383_finished_line_loads_as_not_cancelled() {
    let line = r#"{"kind":"WorkflowRunFinished","workflow_id":"digest","scheduled":true,"run_id":"run-9","error":"it broke"}"#;
    let event: CompanyEvent = serde_json::from_str(line).expect("pre-#383 line loads");
    let CompanyEvent::WorkflowRunFinished {
        cancelled, error, ..
    } = &event
    else {
        panic!("expected a WorkflowRunFinished");
    };
    assert!(!cancelled, "an old failed run must not read as cancelled");
    assert_eq!(error.as_deref(), Some("it broke"));
    // And re-serializing it stays byte-identical — the field is absent
    // going out as well as coming in.
    assert_eq!(
        serde_json::to_string(&event).expect("serialize"),
        line,
        "re-writing an old line must not add the new field"
    );
}

/// Issue #371's opening bracket round-trips through the JSONL the journal
/// puts every event through.
#[test]
fn workflow_run_started_round_trips() {
    let event = CompanyEvent::WorkflowRunStarted {
        workflow_id: "digest".to_string(),
        run_id: "run-1".to_string(),
        scheduled: true,
        started_by: Some(StartedBy::Operator),
        resume_semantic: None,
    };
    assert_eq!(round_trip(&event), event);
}

/// Every [`StartedBy`] arm round-trips, including the fielded `Agent` one —
/// the shape a parked blocker's sender resolution reads back.
#[test]
fn started_by_round_trips_all_arms() {
    for started_by in [
        StartedBy::Operator,
        StartedBy::Agent("ceo".to_string()),
        StartedBy::Schedule,
    ] {
        let event = CompanyEvent::WorkflowRunStarted {
            workflow_id: "digest".to_string(),
            run_id: "run-1".to_string(),
            scheduled: matches!(started_by, StartedBy::Schedule),
            started_by: Some(started_by.clone()),
            resume_semantic: None,
        };
        assert_eq!(
            round_trip(&event),
            event,
            "{started_by:?} did not round-trip"
        );
    }
}

/// A `WorkflowRunStarted` line written before this field existed (issue
/// #1862 prerequisite) still replays, with `started_by` reading back
/// `None` rather than failing to parse. Pinned against a hand-written
/// legacy payload rather than a round-trip, for the same reason
/// `a_pre_881_run_finished_line_still_replays` is: a round-trip can only
/// ever prove the new shape agrees with itself.
#[test]
fn a_pre_1862_run_started_line_still_replays_with_no_sender() {
    let legacy = serde_json::json!({
        "kind": "WorkflowRunStarted",
        "workflow_id": "digest",
        "run_id": "run-1",
        "scheduled": false
    });
    let event: CompanyEvent =
        serde_json::from_value(legacy).expect("a pre-#1862 journal line must still parse");
    let CompanyEvent::WorkflowRunStarted { started_by, .. } = &event else {
        panic!("expected a WorkflowRunStarted, got {event:?}");
    };
    assert_eq!(started_by, &None, "a legacy line names no sender");
}

/// Both node outcomes round-trip, including the elapsed reading — the field
/// that turns "it finished" into "it took this long", which is what tells a
/// slow run from a wedged one.
#[test]
fn workflow_node_finished_round_trips_both_statuses() {
    for status in [
        WorkflowNodeStatus::Ok,
        WorkflowNodeStatus::Error,
        // Issue #881's third arm. Pinned in the same loop rather than a
        // test of its own so a fourth reading cannot be added without
        // someone editing this list.
        WorkflowNodeStatus::Blocked,
        WorkflowNodeStatus::Declined,
    ] {
        let event = CompanyEvent::WorkflowNodeFinished {
            workflow_id: "digest".to_string(),
            run_id: "run-1".to_string(),
            node_id: "ceo".to_string(),
            status,
            elapsed_ms: 1234,
            diagnostics: Vec::new(),
            agent_run_id: None,
        };
        assert_eq!(round_trip(&event), event);
    }
}

/// A `WorkflowRunFinished` line written before #881 / #880 still replays.
///
/// **This is not a nicety.** The event is folded at boot, so a new field
/// without `#[serde(default)]` would make every pre-existing journal line
/// fail to parse — and the failure mode is a company silently losing its
/// whole run history, not a compile error. Pinned against a hand-written
/// legacy payload rather than a round-trip, because a round-trip can only
/// ever prove the new shape agrees with itself.
#[test]
fn a_pre_881_run_finished_line_still_replays() {
    let legacy = serde_json::json!({
        "kind": "WorkflowRunFinished",
        "workflow_id": "digest",
        "scheduled": true,
        "run_id": "run-1",
        "pending_approvals": ["review"],
        "cancelled": false
    });
    let event: CompanyEvent =
        serde_json::from_value(legacy).expect("a pre-#881 journal line must still parse");
    let CompanyEvent::WorkflowRunFinished {
        blocked_nodes,
        approvals,
        pending_approvals,
        ..
    } = &event
    else {
        panic!("expected a WorkflowRunFinished, got {event:?}");
    };
    assert!(blocked_nodes.is_empty());
    assert!(approvals.is_empty());
    assert_eq!(pending_approvals, &vec!["review".to_string()]);
}

/// A run that blocked on nobody serializes byte-for-byte as it did before
/// #881 / #880 — which is nearly every run, so this is what keeps the
/// journal from growing two empty arrays per line.
#[test]
fn a_run_that_blocked_on_nobody_adds_no_keys() {
    let event = CompanyEvent::WorkflowRunFinished {
        workflow_id: "digest".to_string(),
        scheduled: false,
        run_id: Some("run-1".to_string()),
        deliveries: Vec::new(),
        pending_approvals: Vec::new(),
        error: None,
        cancelled: false,
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    };
    let json = serde_json::to_value(&event).expect("serialize");
    assert!(json.get("blocked_nodes").is_none(), "{json}");
    assert!(json.get("approvals").is_none(), "{json}");
}

/// Every field on both #371 variants is required, and that is the point:
/// the correlation id is what groups a run's nodes with its outcome, so a
/// line without one would be unfoldable. Nothing is `skip_serializing_if`
/// — except `WorkflowRunStarted::started_by` (issue #1862 prerequisite),
/// which is additive and `None` here on purpose, so the wire form stays
/// self-describing for every field that predates it.
#[test]
fn workflow_progress_variants_serialize_every_field() {
    let started = serde_json::to_string(&CompanyEvent::WorkflowRunStarted {
        workflow_id: "digest".to_string(),
        run_id: "run-1".to_string(),
        scheduled: false,
        started_by: None,
        resume_semantic: None,
    })
    .expect("serialize");
    assert_eq!(
        started,
        r#"{"kind":"WorkflowRunStarted","workflow_id":"digest","run_id":"run-1","scheduled":false}"#
    );

    let node = serde_json::to_string(&CompanyEvent::WorkflowNodeFinished {
        workflow_id: "digest".to_string(),
        run_id: "run-1".to_string(),
        node_id: "ceo".to_string(),
        status: WorkflowNodeStatus::Error,
        elapsed_ms: 7,
        diagnostics: Vec::new(),
        agent_run_id: None,
    })
    .expect("serialize");
    assert_eq!(
        node,
        r#"{"kind":"WorkflowNodeFinished","workflow_id":"digest","run_id":"run-1","node_id":"ceo","status":"error","elapsed_ms":7}"#
    );
}

/// The replay guarantee #371 rests on, stated as a test: adding these two
/// variants cannot change how an already-persisted line loads. A journal
/// written before #371 contains neither `kind`, and the pre-#371 wire form
/// of the variant they sit beside still decodes byte-for-byte as it did.
#[test]
fn pre_371_journal_lines_are_unaffected_by_the_new_variants() {
    let line = r#"{"kind":"WorkflowRunFinished","workflow_id":"digest","scheduled":true,"pending_approvals":["review"]}"#;
    let event: CompanyEvent = serde_json::from_str(line).expect("pre-#371 line loads");
    assert_eq!(
        event,
        CompanyEvent::WorkflowRunFinished {
            workflow_id: "digest".to_string(),
            scheduled: true,
            run_id: None,
            deliveries: Vec::new(),
            pending_approvals: vec!["review".to_string()],
            error: None,
            cancelled: false,
            notices: Vec::new(),
            board: Vec::new(),
            blocked_nodes: Vec::new(),
            approvals: Vec::new(),
        }
    );
}

/// Issue #327: the workspace announcement survives the JSONL round trip the
/// journal puts every event through, and carries its discriminant.
#[test]
fn workspace_changed_round_trips() {
    let event = CompanyEvent::WorkspaceChanged {
        node_id: "n-1".to_string(),
        change: "updated".to_string(),
    };
    assert_eq!(round_trip(&event), event);
    assert_eq!(event.kind(), "WorkspaceChanged");
    let json = serde_json::to_string(&event).expect("serialize");
    assert!(json.contains(r#""kind":"WorkspaceChanged""#), "{json}");
    assert!(json.contains(r#""node_id":"n-1""#), "{json}");
}

/// The one variant whose retention class diverges from its sibling's, so
/// the choice is pinned rather than left to the next reader's memory.
///
/// `WorkspaceChanged` is Prunable: it is high-volume machine exhaust whose
/// whole meaning is "re-read the tree", nothing addresses it by sequence,
/// and nothing folds it at boot. `TaskCardChanged` stays Permanent because
/// a board card's lifecycle is the company's work history.
#[test]
fn a_workspace_announcement_is_prunable_though_its_board_sibling_is_not() {
    use crate::ports::events::RetentionClass;

    assert_eq!(
        CompanyEvent::WorkspaceChanged {
            node_id: "n-1".to_string(),
            change: "updated".to_string(),
        }
        .retention_class(),
        RetentionClass::Prunable
    );
    assert_eq!(
        CompanyEvent::TaskCardChanged {
            task_id: "t-1".to_string(),
            change: "opened".to_string(),
            column: Some("todo".to_string()),
        }
        .retention_class(),
        RetentionClass::Permanent
    );
}

/// Issue #529: the delivered-report event survives the JSONL round trip the
/// journal puts every event through, `target` and all — the whole reason it
/// exists is to be read back after a crash.
#[test]
fn workflow_report_delivered_round_trips() {
    let event = CompanyEvent::WorkflowReportDelivered {
        workflow_id: "digest".to_string(),
        run_id: "run-1".to_string(),
        node: "owner_summary".to_string(),
        kind: "owner".to_string(),
        target: Some("ada@example.com".to_string()),
    };
    assert_eq!(round_trip(&event), event);
}

/// Issue #529: the wire shape is pinned, and `target` is omitted entirely
/// when a destination named none — the same `skip_serializing_if` economy
/// every optional field on this enum keeps, so a channel line stays minimal.
#[test]
fn workflow_report_delivered_pins_its_wire_shape_and_omits_absent_target() {
    let with_target = serde_json::to_string(&CompanyEvent::WorkflowReportDelivered {
        workflow_id: "digest".to_string(),
        run_id: "run-1".to_string(),
        node: "owner_summary".to_string(),
        kind: "owner".to_string(),
        target: Some("ada@example.com".to_string()),
    })
    .expect("serialize");
    assert_eq!(
        with_target,
        r#"{"kind":"WorkflowReportDelivered","workflow_id":"digest","run_id":"run-1","node":"owner_summary","destination_kind":"owner","target":"ada@example.com"}"#
    );

    let no_target = serde_json::to_string(&CompanyEvent::WorkflowReportDelivered {
        workflow_id: "digest".to_string(),
        run_id: "run-1".to_string(),
        node: "notice".to_string(),
        kind: "channel".to_string(),
        target: None,
    })
    .expect("serialize");
    assert_eq!(
        no_target,
        r#"{"kind":"WorkflowReportDelivered","workflow_id":"digest","run_id":"run-1","node":"notice","destination_kind":"channel"}"#,
        "an absent target must not ride the line as a null"
    );
    // …and a line with no `target` loads back as `None` rather than failing.
    let decoded: CompanyEvent = serde_json::from_str(&no_target).expect("minimal line loads");
    assert_eq!(
        decoded,
        CompanyEvent::WorkflowReportDelivered {
            workflow_id: "digest".to_string(),
            run_id: "run-1".to_string(),
            node: "notice".to_string(),
            kind: "channel".to_string(),
            target: None,
        }
    );
}

/// Issue #259's two variants pin their wire shape the same way
/// `WorkflowCreated` does: `kind` + `workflow_id` + `name`, with `by`
/// omitted entirely when absent so the common unattributed line stays the
/// short one.
#[test]
fn workflow_updated_and_deleted_pin_their_wire_shape() {
    let updated = CompanyEvent::WorkflowUpdated {
        workflow_id: "digest".to_string(),
        name: "Daily digest".to_string(),
        by: None,
    };
    assert_eq!(
        serde_json::to_string(&updated).expect("serialize"),
        r#"{"kind":"WorkflowUpdated","workflow_id":"digest","name":"Daily digest"}"#
    );

    let deleted = CompanyEvent::WorkflowDeleted {
        workflow_id: "digest".to_string(),
        name: "Daily digest".to_string(),
        by: None,
    };
    assert_eq!(
        serde_json::to_string(&deleted).expect("serialize"),
        r#"{"kind":"WorkflowDeleted","workflow_id":"digest","name":"Daily digest"}"#
    );

    // Both round-trip.
    for event in [updated, deleted] {
        let line = serde_json::to_string(&event).expect("serialize");
        let back: CompanyEvent = serde_json::from_str(&line).expect("deserialize");
        assert_eq!(back, event);
    }
}

/// The graph body must never reach the journal — see the variant docs. A
/// reader of the shared append-only log (operator SSE, the inference
/// sidecar) has no business seeing agent prompts or destination addresses,
/// and the only way a body could leak here is someone adding a field.
#[test]
fn workflow_updated_carries_no_graph_body() {
    let line = serde_json::to_string(&CompanyEvent::WorkflowUpdated {
        workflow_id: "digest".to_string(),
        name: "Daily digest".to_string(),
        by: None,
    })
    .expect("serialize");
    assert!(!line.contains("toml"), "{line}");
    assert!(!line.contains("node"), "{line}");
    assert!(!line.contains("graph"), "{line}");
}

/// **The backcompat proof.** A journal written before this variant existed
/// still loads, line for line, and every one of those lines re-serializes
/// byte-identically — which is what "additive, no migration" actually
/// claims. Adding an enum variant cannot change how a sibling serializes,
/// but nothing else in the suite asserts it for the whole log, and a
/// regression here would corrupt an export/import round trip silently.
#[test]
fn a_journal_written_before_this_variant_still_loads_byte_identically() {
    // Verbatim lines in the pre-#228 on-disk shapes, including the pre-`by`
    // / pre-`chat` `OperatorMessage` and the pre-`steps` `AgentReply`.
    let legacy = [
        r#"{"kind":"OperatorMessage","text":"ship it"}"#,
        r#"{"kind":"AgentReply","chat_id":"general","agent_id":"ceo","text":"on it"}"#,
        r#"{"kind":"ScheduleFired","cron":"0 9 * * *","prompt":"daily"}"#,
        r#"{"kind":"WorkflowCreated","workflow_id":"digest","name":"Digest"}"#,
        r#"{"kind":"TaskDispatched","task_id":"t-1"}"#,
        r#"{"kind":"DeskTaskCompleted","task_id":"t-1","desk":"ceo","output":"done","column":"in_review"}"#,
    ];
    for line in legacy {
        let event: CompanyEvent = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("pre-#228 journal line must still load: {line} — {e}"));
        let again = serde_json::to_string(&event).expect("serialize");
        assert_eq!(again, line, "pre-#228 line must re-serialize unchanged");
    }
}

/// Issue #242: an effect remembers which task attempt produced it, and the
/// field is additive in the same way `Effect::agent` was — a journal line
/// written before it existed replays as `None` (no run correlation, the
/// pre-#242 behaviour) rather than failing to parse and taking the whole
/// approval queue down with it on replay.
#[test]
fn effect_run_id_round_trips_and_a_legacy_line_replays_as_none() {
    let mut effect = Effect {
        kind: "composio.execute".to_string(),
        group: EffectGroup::Other,
        amount_usd: None,
        established_thread: false,
        first_time_counterparty: false,
        payload: serde_json::json!({ "tool": "GMAIL_SEND_EMAIL" }),
        agent: Some("finance".to_string()),
        run_id: None,
    };
    let untagged = serde_json::to_string(&effect).expect("serialize");
    assert!(
        !untagged.contains("run_id"),
        "an untagged effect's wire form must be unchanged: {untagged}"
    );

    effect.run_id = Some("run-7".to_string());
    let tagged = serde_json::to_string(&effect).expect("serialize");
    assert!(tagged.contains(r#""run_id":"run-7""#), "{tagged}");
    assert_eq!(
        effect,
        serde_json::from_str::<Effect>(&tagged).expect("round trip")
    );

    // The pre-#242 line: same bytes, no field.
    let legacy: Effect = serde_json::from_str(&untagged).expect("legacy effect must load");
    assert_eq!(legacy.run_id, None);
    assert_eq!(
        legacy.agent.as_deref(),
        Some("finance"),
        "the earlier additive field must still be read alongside the new one"
    );
}

/// Issue #242: the run id rides the dispatch event, and it is additive in
/// both directions — a tagged dispatch round-trips it, and an untagged one
/// serializes exactly the shape a pre-#242 journal holds (asserted verbatim
/// above too, but here against the *writer* rather than the reader).
#[test]
fn task_dispatched_carries_its_run_id_without_changing_the_untagged_shape() {
    let untagged = CompanyEvent::TaskDispatched {
        task_id: "t-1".to_string(),
        run_id: None,
    };
    assert_eq!(
        serde_json::to_string(&untagged).expect("serialize"),
        r#"{"kind":"TaskDispatched","task_id":"t-1"}"#
    );

    let tagged = CompanyEvent::TaskDispatched {
        task_id: "t-1".to_string(),
        run_id: Some("run-7".to_string()),
    };
    let line = serde_json::to_string(&tagged).expect("serialize");
    assert!(line.contains(r#""run_id":"run-7""#), "{line}");
    assert_eq!(
        tagged,
        serde_json::from_str::<CompanyEvent>(&line).expect("round trip")
    );

    // A legacy line loads as an untagged dispatch rather than failing.
    let legacy: CompanyEvent =
        serde_json::from_str(r#"{"kind":"TaskDispatched","task_id":"t-1"}"#).expect("legacy");
    assert_eq!(legacy, untagged);
}

#[test]
fn legacy_agent_card_json_deserializes_with_defaults() {
    // A card written by an earlier phase carried only three fields; the new
    // `#[serde(default)]` fields must fill in without error.
    let json = r#"{"handle":"acme","description":"d","skills":["a"]}"#;
    let card: AgentCard = serde_json::from_str(json).expect("deserialize legacy card");
    assert_eq!(card.handle, "acme");
    assert!(card.name.is_empty());
    assert!(card.payment_requirements.is_empty());
    assert!(card.supported_interfaces.is_empty());
}

/// Issue #1682: an attachment round-trips on an `OperatorMessage`, and an
/// empty list serializes *away* — the additive shape that makes the field
/// zero-migration, on exactly the terms `mentions` / `deliverable` proved
/// for themselves above.
#[test]
fn operator_message_attachments_round_trip_and_skip_when_empty() {
    // Empty is absent: a message with no attachment serializes byte-for-byte
    // as it did before the field existed.
    let bare = CompanyEvent::OperatorMessage {
        text: "hi".into(),
        by: None,
        chat: None,
        parent: None,
        deliverable: None,
        mentions: Vec::new(),
        attachments: Vec::new(),
    };
    assert_eq!(
        serde_json::to_string(&bare).unwrap(),
        r#"{"kind":"OperatorMessage","text":"hi"}"#,
        "an empty attachment list must not appear on the wire"
    );

    // A carried attachment survives the round trip with every field intact.
    let carried = CompanyEvent::OperatorMessage {
        text: "see attached".into(),
        by: None,
        chat: None,
        parent: None,
        deliverable: None,
        mentions: Vec::new(),
        attachments: vec![Attachment {
            node_id: "node-1".into(),
            name: "diagram.png".into(),
            mime: "image/png".into(),
            size: 2048,
            extracted_text: None,
        }],
    };
    let json = serde_json::to_string(&carried).unwrap();
    assert!(json.contains(r#""nodeId":"node-1""#), "{json}");
    assert!(json.contains(r#""mime":"image/png""#), "{json}");
    let back: CompanyEvent = serde_json::from_str(&json).unwrap();
    match back {
        CompanyEvent::OperatorMessage { attachments, .. } => {
            assert_eq!(attachments.len(), 1);
            assert_eq!(attachments[0].name, "diagram.png");
            assert_eq!(attachments[0].size, 2048);
        }
        other => panic!("expected OperatorMessage, got {other:?}"),
    }

    // A pre-#1682 record with no `attachments` key still loads, as an empty
    // list — the `#[serde(default)]` half of the contract.
    let legacy = r#"{"kind":"OperatorMessage","text":"hi"}"#;
    match serde_json::from_str::<CompanyEvent>(legacy).unwrap() {
        CompanyEvent::OperatorMessage { attachments, .. } => assert!(attachments.is_empty()),
        other => panic!("expected OperatorMessage, got {other:?}"),
    }
}

/// Codex review finding on #1682, round 2: `extracted_text` is a later
/// addition to `Attachment` itself, so it needs the identical
/// omit-when-absent / default-on-load contract `attachments` got above —
/// a record journaled by the first round of the fix (a reference with no
/// extracted text) must still load, and a `None` must not put a stray key
/// on the wire.
#[test]
fn attachment_extracted_text_round_trips_and_skips_when_absent() {
    let no_text = Attachment {
        node_id: "node-1".into(),
        name: "photo.png".into(),
        mime: "image/png".into(),
        size: 2048,
        extracted_text: None,
    };
    let json = serde_json::to_string(&no_text).unwrap();
    assert!(
        !json.contains("extractedText"),
        "no extracted text must not appear on the wire: {json}"
    );
    assert_eq!(serde_json::from_str::<Attachment>(&json).unwrap(), no_text);

    let with_text = Attachment {
        node_id: "node-2".into(),
        name: "report.pdf".into(),
        mime: "application/pdf".into(),
        size: 4096,
        extracted_text: Some("Q3 revenue grew 12%.".to_string()),
    };
    let json = serde_json::to_string(&with_text).unwrap();
    assert!(
        json.contains(r#""extractedText":"Q3 revenue grew 12%.""#),
        "{json}"
    );
    assert_eq!(
        serde_json::from_str::<Attachment>(&json).unwrap(),
        with_text
    );

    // A round 1 record (the reference alone, no `extractedText` key) still
    // loads, defaulting to `None` — the same contract `attachments` itself
    // got when it was added onto `OperatorMessage`.
    let round_one = r#"{"nodeId":"node-3","name":"old.png","mime":"image/png","size":10}"#;
    let loaded: Attachment = serde_json::from_str(round_one).unwrap();
    assert_eq!(loaded.extracted_text, None);
}

/// Issue #1781 review (Codex P2): a grandfathered manifest teammate at the
/// literal id `operator` diverts the durable system feed to
/// `OPERATOR_CHANNEL_COLLISION_FALLBACK` (see `operator_feed_channel`
/// above). Retiring that teammate must not flip the feed back onto
/// `OPERATOR_CHANNEL` — the tombstone in `overlay_retired_agents` is
/// permanent (manifest removal always goes through `retire_agent`, never a
/// TOML rewrite), so the reports already journaled under the fallback
/// address would be orphaned from `/desks` and the retired teammate's own
/// historical DM rows (`chat_id == "operator"`) would start bleeding into
/// the "new" system feed the moment the id looked free again.
#[test]
fn operator_feed_channel_stays_diverted_after_the_collision_is_retired() {
    let manifest = "[company]\nname = \"Acme\"\n\
         [[agent]]\nid = \"operator\"\nrole = \"Chief of Staff\"\n";
    let mut record = desk_record(manifest, Vec::new());
    assert_eq!(
        record.operator_feed_channel(),
        crate::runtime::OPERATOR_CHANNEL_COLLISION_FALLBACK,
        "fixture must start in the collision state this test exercises"
    );

    record.retire_agent(crate::runtime::OPERATOR_CHANNEL);
    assert!(!record.is_roster_agent(crate::runtime::OPERATOR_CHANNEL));
    assert_eq!(
        record.operator_feed_channel(),
        crate::runtime::OPERATOR_CHANNEL_COLLISION_FALLBACK,
        "the feed address must stay stable once anything has ever held the \
         `operator` id — flipping back to OPERATOR_CHANNEL would orphan the \
         fallback's existing reports and resurface the retired teammate's \
         own DM history in the system feed"
    );
}

/// Issue #1781 review, Codex P2 follow-up: a direct, focused test of
/// `divert_operator_feed_permanently`/`is_operator_feed_diverted`
/// themselves, isolated from the HTTP route the desk- and teammate-
/// deletion regression tests exercise them through.
///
/// The specific risk this closes: `divert_operator_feed_permanently`
/// tombstones through `retire_agent`, keyed on
/// `OPERATOR_CHANNEL_COLLISION_FALLBACK` ("operator-feed") — a string
/// that fails the manifest agent-id format rule on its hyphen alone. If
/// `retire_agent` ever grew id validation (it does not today — it is a
/// bare idempotent push), that key would be silently rejected,
/// `is_operator_feed_diverted` would always read `false`, and the
/// tombstone this whole fix depends on would be a no-op with nothing
/// here to notice. Calling it on a record with **no live collision at
/// all** isolates exactly that: nothing but the divert call itself
/// explains the fallback staying live.
#[test]
fn divert_operator_feed_permanently_sticks_with_no_live_collision() {
    let manifest = "[company]\nname = \"Acme\"\n[[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n";
    let mut record = desk_record(manifest, Vec::new());
    assert_eq!(
        record.operator_feed_channel(),
        crate::runtime::OPERATOR_CHANNEL,
        "fixture must start on the literal address — nothing here collides \
         with `operator` yet"
    );
    assert!(!record.is_operator_feed_diverted());

    record.divert_operator_feed_permanently();

    assert!(
        record.is_operator_feed_diverted(),
        "the tombstone must read back as set immediately after the call"
    );
    assert_eq!(
        record.operator_feed_channel(),
        crate::runtime::OPERATOR_CHANNEL_COLLISION_FALLBACK,
        "operator_feed_channel must divert on the tombstone alone, with no \
         live desk/agent collision in the record at all — proving \
         `retire_agent` actually accepted the hyphenated fallback key \
         rather than silently rejecting it"
    );

    // Idempotent, like `retire_agent` itself: calling it again must not
    // duplicate the tombstone or otherwise change the outcome.
    record.divert_operator_feed_permanently();
    assert_eq!(
        record.overlay_retired_agents.len(),
        1,
        "a second call must not push a duplicate tombstone entry"
    );
}

/// The third grandfather case (PR #1781 review, CodeRabbit): a real
/// **desk** already owning `operator` must divert the feed exactly like
/// the roster-teammate case above, not stay on the literal id. Left on
/// `OPERATOR_CHANNEL`, the feed's id equals the desk's own id, and two
/// surfaces collide on it: `server::operator::operator_channel` hands
/// that id to the console as the pinned Operator row, appended (`
/// operatorSection`, `frontend/src/views/ChatView.tsx`) *after* the desk
/// section `buildChannels` already put the same id in — so `findChannel`,
/// which returns the first section match, resolves the pinned row to the
/// desk every time. And `send_to_channel_adapter` journals each workflow
/// report under `operator_feed_channel()`'s result, so with no divert
/// those reports land in `chat_id == "operator"` too — the desk's own
/// ordinary transcript, not a distinguishable feed.
#[test]
fn operator_feed_channel_diverts_off_a_grandfathered_desks_own_operator_line() {
    let manifest = "[company]\nname = \"Acme\"\n\
         [[group_chat]]\nid = \"operator\"\nname = \"Operator Desk\"\nmembers = []\n";
    let record = desk_record(manifest, Vec::new());
    assert!(record.desk_exists(crate::runtime::OPERATOR_CHANNEL));
    assert!(!record.is_roster_agent(crate::runtime::OPERATOR_CHANNEL));
    assert_eq!(
        record.operator_feed_channel(),
        crate::runtime::OPERATOR_CHANNEL_COLLISION_FALLBACK,
        "a desk already owning `operator` must divert the feed off that \
         same address, the same way a roster teammate holding it does — \
         otherwise the pinned Operator row and the desk share one id and \
         `findChannel` always resolves it to the desk"
    );
}

/// PR #1781 review follow-up (Codex P2, second pass): a desk grandfathered
/// at a harmless id but the display name `Operator` must divert the feed
/// exactly like the same-id case above — `desk_exists` alone (id-only)
/// missed it. `from_path_for_reload` already admits this exact shape
/// (`from_path_for_reload_grandfathers_a_group_chat_named_operator` in
/// `company::manifest`), and `server::operator::resolve_desk` matches a
/// `?desk=operator` selector by name as readily as by id, so the pinned
/// console row would resolve to this desk's own transcript instead of the
/// system feed if the divert never fired.
#[test]
fn operator_feed_channel_diverts_off_a_grandfathered_desks_own_operator_name() {
    let manifest = "[company]\nname = \"Acme\"\n\
         [[group_chat]]\nid = \"legacy_ops\"\nname = \"Operator\"\nmembers = []\n";
    let record = desk_record(manifest, Vec::new());
    assert!(
        !record.desk_exists(crate::runtime::OPERATOR_CHANNEL),
        "fixture must actually be in the id-is-free, name-collides state \
         this test exercises, or it is not distinguishing this case from \
         `operator_feed_channel_diverts_off_a_grandfathered_desks_own_operator_line`"
    );
    assert!(!record.is_roster_agent(crate::runtime::OPERATOR_CHANNEL));
    assert_eq!(
        record.operator_feed_channel(),
        crate::runtime::OPERATOR_CHANNEL_COLLISION_FALLBACK,
        "a desk named \"Operator\" must divert the feed off that address \
         even though its id is free — `resolve_desk` shadows by name too, \
         so the pinned Operator row would otherwise resolve to this \
         desk's own transcript"
    );
}

/// PR #1781 review follow-up (CodeRabbit P2): a double legacy collision —
/// one desk shadowing the primary `operator` address *and a second,
/// different* desk shadowing the collision-fallback's own display name
/// ("operator-feed") — leaves `operator_feed_channel` with nowhere safe
/// left to divert to. `316bc9229` and `16dcce235` block both names from
/// ever being (re-)created going forward, so this fixture only models a
/// manifest hand-edited outside those guards and reloaded via
/// `from_path_for_reload`, the same grandfathering the single-collision
/// cases above rely on.
///
/// `operator_feed_channel_fallback_shadowed` exists precisely so this
/// residual gap is detectable rather than silent — asserted here directly
/// since the logging it drives (`workflows::delivery::send_to_channel_adapter`)
/// has no return value to assert on.
#[test]
fn operator_feed_channel_fallback_shadowed_detects_a_double_collision() {
    let manifest = "[company]\nname = \"Acme\"\n\
         [[group_chat]]\nid = \"legacy_ops\"\nname = \"Operator\"\nmembers = []\n\
         [[group_chat]]\nid = \"ops2\"\nname = \"operator-feed\"\nmembers = []\n";
    let record = desk_record(manifest, Vec::new());
    assert_eq!(
        record.operator_feed_channel(),
        crate::runtime::OPERATOR_CHANNEL_COLLISION_FALLBACK,
        "the primary collision alone still diverts to the fallback address \
         — this fixture must reach the same divert as the single-collision \
         case above before the double-collision check means anything"
    );
    assert!(
        record.operator_feed_channel_fallback_shadowed(),
        "a second desk named \"operator-feed\" shadows the fallback the \
         same way the first desk shadows the primary — `resolve_desk` \
         would fold a `?desk=operator-feed` read onto that second desk \
         instead of the system feed, and this predicate must catch it"
    );
}

/// Sibling to the double-collision case above: a fallback-name collision
/// with **no** primary collision must not trip the predicate — the divert
/// never fires, so the fallback address was never actually depended on.
#[test]
fn operator_feed_channel_fallback_shadowed_is_false_without_a_primary_collision() {
    let manifest = "[company]\nname = \"Acme\"\n\
         [[group_chat]]\nid = \"ops2\"\nname = \"operator-feed\"\nmembers = []\n";
    let record = desk_record(manifest, Vec::new());
    assert_eq!(
        record.operator_feed_channel(),
        crate::runtime::OPERATOR_CHANNEL,
        "no primary collision exists in this fixture, so the feed must \
         stay on the literal `operator` address"
    );
    assert!(
        !record.operator_feed_channel_fallback_shadowed(),
        "the fallback address is never consulted unless the feed actually \
         diverted to it"
    );
}
