use super::*;

const LOADABLE_WORKFLOW: &str = r#"
    id = "valid"
    name = "Valid"
    [[node]]
    id = "start"
    kind = "trigger"
    name = "Start"
    [[node]]
    id = "done"
    kind = "output"
    name = "Done"
    [[edge]]
    from = "start"
    to = "done"
"#;

/// Issue #1862 prerequisite: a workflow TOML with no `owner_desk` key —
/// every graph saved before this field existed — parses to `None` rather
/// than failing. `LOADABLE_WORKFLOW` has no `owner_desk` line by
/// construction, so this is the back-compat case.
#[test]
fn a_workflow_toml_with_no_owner_desk_parses_to_none() {
    let file = parse_workflow(LOADABLE_WORKFLOW).expect("parses");
    assert_eq!(file.owner_desk, None);
}

/// **Regression, issue #1882 review ("preserve padded stale owners").**
/// A stored `owner_desk` carrying surrounding whitespace parses to the
/// TRIMMED value, and a blank one to `None` — the same "blank means
/// absent" rule [`RawWorkflow::normalize_owner_desk`] already applies at
/// every write boundary.
///
/// RED-FIRST: this load path used to carry `raw.owner_desk` through
/// verbatim, so the stored side of the unchanged-owner comparison in
/// `workflow_create::validate_draft_against_record` was padded while the
/// draft side had been trimmed on the way in. The two never compared
/// equal, which defeated the grandfathering: an unrelated edit could be
/// refused over a stale desk, or silently re-resolved onto a different
/// desk that had since taken the same display name.
#[test]
fn a_padded_owner_desk_parses_trimmed_and_a_blank_one_parses_absent() {
    let padded = LOADABLE_WORKFLOW.replace(
        "id = \"valid\"",
        "id = \"valid\"\n        owner_desk = \"  engineering  \"",
    );
    let file = parse_workflow(&padded).expect("parses");
    assert_eq!(file.owner_desk.as_deref(), Some("engineering"));

    let blank = LOADABLE_WORKFLOW.replace(
        "id = \"valid\"",
        "id = \"valid\"\n        owner_desk = \"   \"",
    );
    let file = parse_workflow(&blank).expect("parses");
    assert_eq!(
        file.owner_desk, None,
        "blank is absent, not `Some(\"   \")`"
    );
}

/// A workflow TOML that DOES carry `owner_desk` round-trips it — the
/// lenient load path (`validate(&raw, false)`) carries the value through
/// unvalidated; desk existence is checked only at author time.
#[test]
fn a_workflow_toml_with_owner_desk_parses_it_through() {
    let with_owner = LOADABLE_WORKFLOW.replacen(
        "id = \"valid\"",
        "id = \"valid\"\n        owner_desk = \"engineering\"",
        1,
    );
    let file = parse_workflow(&with_owner).expect("parses");
    assert_eq!(file.owner_desk.as_deref(), Some("engineering"));
}

/// The filename is the id-based lookup key. A different id inside the body
/// must never leak into a list as a row that `GET /workflows/{id}` cannot
/// open, while a valid sibling continues to load.
#[test]
fn load_company_workflows_skips_filename_id_mismatches() {
    let dir = tempfile::tempdir().unwrap();
    let workflows = dir.path().join("workflows");
    std::fs::create_dir_all(&workflows).unwrap();
    std::fs::write(workflows.join("valid.toml"), LOADABLE_WORKFLOW).unwrap();
    std::fs::write(
        workflows.join("wrong-stem.toml"),
        LOADABLE_WORKFLOW.replace("id = \"valid\"", "id = \"different\""),
    )
    .unwrap();

    let loaded =
        load_company_workflows(dir.path(), &["wrong-stem".to_string(), "valid".to_string()])
            .expect("a mismatched body is skipped rather than failing the load");

    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, "valid");
}

/// The directory scan shares the loader's stem/id choke point, so a bad
/// file is absent from the picker rather than listed under its embedded id.
#[test]
fn list_source_workflows_never_lists_a_mismatched_file() {
    let dir = tempfile::tempdir().unwrap();
    let workflows = dir.path().join("workflows");
    std::fs::create_dir_all(&workflows).unwrap();
    std::fs::write(
        workflows.join("wrong-stem.toml"),
        LOADABLE_WORKFLOW.replace("id = \"valid\"", "id = \"different\""),
    )
    .unwrap();

    assert!(list_source_workflows(Some(dir.path())).is_empty());
}

#[test]
fn render_workflow_round_trips_through_parse_workflow() {
    let raw = RawWorkflow {
        id: "wf".to_string(),
        name: "WF".to_string(),
        description: Some("A test graph.".to_string()),
        owner_desk: None,
        nodes: vec![
            RawNode {
                id: "start".to_string(),
                kind: "trigger".to_string(),
                name: "Start".to_string(),
                summary: None,
                agent: None,
                schedule: None,
                config: None,
                on_error: None,
                retry: None,
                requires_approval: None,
                repeatable: None,
                destination: None,
                postcondition: None,
                verify: None,
            },
            RawNode {
                id: "worker".to_string(),
                kind: "agent".to_string(),
                name: "Worker".to_string(),
                summary: Some("Does the thing.".to_string()),
                agent: Some("ceo".to_string()),
                schedule: None,
                config: None,
                on_error: None,
                retry: None,
                requires_approval: None,
                repeatable: None,
                destination: None,
                postcondition: None,
                verify: None,
            },
        ],
        edges: vec![RawEdge {
            from: "start".to_string(),
            to: "worker".to_string(),
            label: Some("ok".to_string()),
        }],
    };
    let toml_src = render_workflow(&raw).expect("renders");
    let file = parse_workflow(&toml_src).expect("re-parses the rendered graph");
    assert_eq!(file.id, "wf");
    assert_eq!(file.nodes.len(), 2);
    assert_eq!(file.edges.len(), 1);
    let worker = file.nodes.iter().find(|n| n.id == "worker").unwrap();
    assert_eq!(worker.agent.as_deref(), Some("ceo"));
    assert_eq!(file.edges[0].label.as_deref(), Some("ok"));
}

/// Issue #1866: a declared `postcondition` survives the render -> re-parse
/// round trip [`WorkflowSpecProjection`]/the console draft path relies on,
/// and — because [`WorkflowPostconditionDef`] is table-valued —
/// `toml::to_string` does not choke on field order the way it would if a
/// scalar field followed it (the reason `postcondition` sits beside
/// `retry`, before `destination`, on [`RawNode`]).
#[test]
fn render_workflow_round_trip_preserves_postcondition() {
    let raw = RawWorkflow {
        id: "wf".to_string(),
        name: "WF".to_string(),
        description: None,
        owner_desk: None,
        nodes: vec![
            RawNode {
                id: "start".to_string(),
                kind: "trigger".to_string(),
                name: "Start".to_string(),
                summary: None,
                agent: None,
                schedule: None,
                config: None,
                on_error: None,
                retry: None,
                requires_approval: None,
                repeatable: None,
                destination: None,
                postcondition: None,
                verify: None,
            },
            RawNode {
                id: "worker".to_string(),
                kind: "agent".to_string(),
                name: "Worker".to_string(),
                summary: None,
                agent: Some("ceo".to_string()),
                schedule: None,
                config: None,
                on_error: None,
                retry: None,
                requires_approval: None,
                repeatable: None,
                destination: None,
                postcondition: Some(WorkflowPostconditionDef {
                    require: "field_present".to_string(),
                    field: Some("json.items".to_string()),
                }),
                verify: None,
            },
        ],
        edges: vec![RawEdge {
            from: "start".to_string(),
            to: "worker".to_string(),
            label: None,
        }],
    };
    let toml_src = render_workflow(&raw).expect("renders, even with a table field present");
    let file = parse_workflow(&toml_src).expect("re-parses the rendered graph");
    let worker = file.nodes.iter().find(|n| n.id == "worker").unwrap();
    let postcondition = worker
        .postcondition
        .as_ref()
        .expect("the postcondition survived the round trip");
    assert_eq!(postcondition.require, "field_present");
    // Codex #3893851369 on #1937: the bare `items` this test used to
    // assert here validates but can never resolve at runtime (see
    // `postcondition_field_with_a_bare_structured_root_is_rejected`) —
    // the documented `json.items` form is the only one `parse_workflow`
    // now accepts.
    assert_eq!(postcondition.field.as_deref(), Some("json.items"));
}

/// Issue #1866: `postcondition` is operator-only policy, exactly like
/// `retry` and `requires_approval` beside it — the agent authoring schema
/// cannot express it, so [`project_workflow_spec`] must list it in
/// [`WorkflowSpecProjection::unexpressible`] rather than silently
/// dropping it on an agent-driven full-replacement edit.
#[cfg(feature = "openhuman")]
#[test]
fn project_workflow_spec_lists_postcondition_as_unexpressible() {
    let raw = RawWorkflow {
        id: "wf".to_string(),
        name: "WF".to_string(),
        description: None,
        owner_desk: None,
        nodes: vec![RawNode {
            id: "worker".to_string(),
            kind: "agent".to_string(),
            name: "Worker".to_string(),
            summary: None,
            agent: Some("ceo".to_string()),
            schedule: None,
            config: None,
            on_error: None,
            retry: None,
            requires_approval: None,
            repeatable: None,
            destination: None,
            postcondition: Some(WorkflowPostconditionDef {
                require: "non_empty".to_string(),
                field: None,
            }),
            verify: None,
        }],
        edges: Vec::new(),
    };
    let projection = project_workflow_spec(&raw);
    assert_eq!(projection.unexpressible.len(), 1);
    let (node_id, fields) = &projection.unexpressible[0];
    assert_eq!(node_id, "worker");
    assert!(
        fields.iter().any(|(name, _)| *name == "postcondition"),
        "postcondition must be named in the unexpressible residue: {fields:?}"
    );
    assert!(
        projection.unexpressible_summary().contains("postcondition"),
        "{}",
        projection.unexpressible_summary()
    );
    // And it does not leak into the agent-facing spec itself.
    let worker_spec = projection.spec["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == "worker")
        .unwrap();
    assert!(worker_spec.get("postcondition").is_none());
}

/// A rendered graph that fails structural validation (no trigger) surfaces
/// the same prosumer-language problem `parse_workflow` gives a hand-authored
/// file — the create endpoint relies on this to turn a bad graph into a 4xx.
#[test]
fn render_workflow_of_an_invalid_graph_fails_reparse() {
    let raw = RawWorkflow {
        id: "wf".to_string(),
        name: "WF".to_string(),
        description: None,
        owner_desk: None,
        nodes: vec![RawNode {
            id: "only".to_string(),
            kind: "output".to_string(),
            name: "Only".to_string(),
            summary: None,
            agent: None,
            schedule: None,
            config: None,
            on_error: None,
            retry: None,
            requires_approval: None,
            repeatable: None,
            destination: None,
            postcondition: None,
            verify: None,
        }],
        edges: vec![],
    };
    let toml_src = render_workflow(&raw).expect("renders even though invalid");
    let err = parse_workflow(&toml_src).unwrap_err();
    assert!(err.to_string().contains("trigger"), "{err}");
}

const CAMPAIGN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../companies/marketing_agency/workflows/campaign_pipeline.toml"
));

#[test]
fn parses_the_shipped_campaign_pipeline() {
    let workflow = parse_workflow(CAMPAIGN).expect("campaign pipeline is valid");
    assert_eq!(workflow.id, "campaign_pipeline");
    assert_eq!(workflow.name, "Campaign pipeline");
    assert_eq!(workflow.nodes.len(), 8);
    assert_eq!(workflow.edges.len(), 8);
    let strategist = workflow
        .nodes
        .iter()
        .find(|n| n.id == "strategist")
        .unwrap();
    assert_eq!(strategist.kind, WorkflowNodeKind::Agent);
    assert_eq!(strategist.agent.as_deref(), Some("brand_strategist"));
    let brief = workflow.nodes.iter().find(|n| n.id == "brief").unwrap();
    assert_eq!(brief.kind, WorkflowNodeKind::Trigger);
}

#[test]
fn edge_referencing_missing_node_is_rejected() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[edge]]
        from = "start"
        to = "ghost"
    "#;
    let err = parse_workflow(src).unwrap_err();
    let message = err.to_string();
    assert!(message.contains("ghost"), "{message}");
    assert!(message.contains("not a node"), "{message}");
}

#[test]
fn missing_trigger_is_rejected() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "only"
        kind = "output"
        name = "Only"
    "#;
    let err = parse_workflow(src).unwrap_err();
    assert!(err.to_string().contains("trigger"), "{err}");
}

#[test]
fn empty_workflow_has_no_trigger() {
    let src = r#"
        id = "wf"
        name = "WF"
    "#;
    let err = parse_workflow(src).unwrap_err();
    assert!(err.to_string().contains("trigger"), "{err}");
}

#[test]
fn duplicate_node_ids_and_self_loops_are_rejected() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "a"
        kind = "trigger"
        name = "A"
        [[node]]
        id = "a"
        kind = "output"
        name = "A2"
        [[edge]]
        from = "a"
        to = "a"
    "#;
    let err = parse_workflow(src).unwrap_err();
    let message = err.to_string();
    assert!(message.contains("more than once"), "{message}");
    assert!(message.contains("itself"), "{message}");
}

#[test]
fn unknown_kind_and_stray_agent_are_rejected() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "weird"
        kind = "teleport"
        name = "Weird"
        [[node]]
        id = "gate"
        kind = "condition"
        name = "Gate"
        agent = "someone"
    "#;
    let err = parse_workflow(src).unwrap_err();
    let message = err.to_string();
    assert!(message.contains("unknown `kind`"), "{message}");
    assert!(
        message.contains("only `agent` nodes name a teammate"),
        "{message}"
    );
}

#[test]
fn unknown_top_level_keys_are_tolerated() {
    let src = r#"
        id = "wf"
        name = "WF"
        canvas_zoom = 1.5
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        extra = "ignored"
    "#;
    assert!(parse_workflow(src).is_ok());
}

// --- Per-node config / error / retry policy (P1) -----------------------

#[test]
fn node_config_parses_including_nested_tables() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "call"
        kind = "tool_call"
        name = "Export"
        [node.config]
        slug = "csv_export"
        [node.config.args]
        filename = "out.csv"
        data = "[]"
        [[edge]]
        from = "start"
        to = "call"
    "#;
    let file = parse_workflow(src).expect("config parses");
    let call = file.nodes.iter().find(|n| n.id == "call").unwrap();
    let config = call.config.as_ref().expect("config present");
    assert_eq!(config["slug"], "csv_export");
    // Nested table survives the TOML → JSON conversion.
    assert_eq!(config["args"]["filename"], "out.csv");
}

#[test]
fn non_finite_config_number_is_rejected_instead_of_dropped() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [node.config]
        threshold = nan
    "#;
    let err = parse_workflow(src).expect_err("non-JSON config must fail");
    assert!(err.to_string().contains("config"), "{err}");
}

#[test]
fn typed_error_retry_and_approval_fields_parse() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "call"
        kind = "tool_call"
        name = "Export"
        on_error = "continue"
        requires_approval = true
        [node.config]
        slug = "csv_export"
        [node.retry]
        max_attempts = 3
        backoff_ms = 100
        backoff = "exponential"
        [[edge]]
        from = "start"
        to = "call"
    "#;
    let file = parse_workflow(src).expect("parses");
    let call = file.nodes.iter().find(|n| n.id == "call").unwrap();
    assert_eq!(call.on_error.as_deref(), Some("continue"));
    assert_eq!(call.requires_approval, Some(true));
    let retry = call.retry.as_ref().expect("retry present");
    assert_eq!(retry.max_attempts, Some(3));
    assert_eq!(retry.backoff_ms, Some(100));
    assert_eq!(retry.backoff.as_deref(), Some("exponential"));
}

#[test]
fn bad_on_error_is_rejected() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        on_error = "explode"
    "#;
    let err = parse_workflow(src).unwrap_err();
    let message = err.to_string();
    assert!(message.contains("unknown `on_error`"), "{message}");
}

#[test]
fn retry_max_attempts_zero_is_rejected() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [node.retry]
        max_attempts = 0
    "#;
    let err = parse_workflow(src).unwrap_err();
    assert!(err.to_string().contains("at least 1"), "{err}");
}

#[test]
fn bad_retry_backoff_is_rejected() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [node.retry]
        backoff = "linear"
    "#;
    let err = parse_workflow(src).unwrap_err();
    assert!(err.to_string().contains("unknown `retry.backoff`"), "{err}");
}

#[test]
fn reserved_config_keys_are_rejected() {
    // `on_error` inside `config` (not as a first-class field) is a footgun.
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [node.config]
        on_error = "route"
    "#;
    let err = parse_workflow(src).unwrap_err();
    assert!(err.to_string().contains("inside `config`"), "{err}");
}

#[test]
fn postcondition_inside_config_is_rejected() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "worker"
        kind = "agent"
        name = "Worker"
        agent = "ceo"
        [node.config]
        postcondition = "non_empty"
        [[edge]]
        from = "start"
        to = "worker"
    "#;
    let err = parse_workflow(src).unwrap_err();
    assert!(err.to_string().contains("inside `config`"), "{err}");
}

#[test]
fn postcondition_valid_on_an_agent_node_parses() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "worker"
        kind = "agent"
        name = "Worker"
        agent = "ceo"
        [node.postcondition]
        require = "field_present"
        field = "json.items"
        [[edge]]
        from = "start"
        to = "worker"
    "#;
    let file = parse_workflow(src).expect("a postcondition on an agent node is valid");
    let worker = file.nodes.iter().find(|n| n.id == "worker").unwrap();
    let postcondition = worker.postcondition.as_ref().expect("postcondition set");
    assert_eq!(postcondition.require, "field_present");
    assert_eq!(postcondition.field.as_deref(), Some("json.items"));
}

#[test]
fn postcondition_on_a_non_agent_node_is_rejected() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [node.postcondition]
        require = "non_empty"
    "#;
    let err = parse_workflow(src).unwrap_err();
    assert!(
        err.to_string()
            .contains("only `agent` nodes carry a postcondition"),
        "{err}"
    );
}

#[test]
fn semantic_verify_parses_only_on_agent_nodes() {
    let valid = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "worker"
        kind = "agent"
        name = "Worker"
        agent = "ceo"
        [node.verify]
        criteria = "Name one recommendation and its evidence."
        [[edge]]
        from = "start"
        to = "worker"
    "#;
    let file = parse_workflow(valid).expect("semantic verification is valid on an agent");
    assert_eq!(
        file.nodes[1]
            .verify
            .as_ref()
            .and_then(|verify| verify.criteria.as_deref()),
        Some("Name one recommendation and its evidence.")
    );

    let invalid = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [node.verify]
    "#;
    let err = parse_workflow(invalid).unwrap_err();
    assert!(
        err.to_string()
            .contains("only `agent` nodes carry semantic verification"),
        "{err}"
    );
}

#[test]
fn postcondition_with_an_unknown_require_is_rejected() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "worker"
        kind = "agent"
        name = "Worker"
        agent = "ceo"
        [node.postcondition]
        require = "smells_right"
        [[edge]]
        from = "start"
        to = "worker"
    "#;
    let err = parse_workflow(src).unwrap_err();
    assert!(
        err.to_string()
            .contains("unknown `postcondition.require` `smells_right`"),
        "{err}"
    );
}

#[test]
fn postcondition_field_present_without_a_field_is_rejected() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "worker"
        kind = "agent"
        name = "Worker"
        agent = "ceo"
        [node.postcondition]
        require = "field_present"
        [[edge]]
        from = "start"
        to = "worker"
    "#;
    let err = parse_workflow(src).unwrap_err();
    assert!(err.to_string().contains("but no `field`"), "{err}");
}

/// Codex #3893851369 on #1937: a bare structured field like `field =
/// "items"` validates today but can NEVER resolve at runtime —
/// `evaluate_postcondition` checks `field` against the `{ text,
/// agent_ref, json }` envelope `run_turn` builds, and a bare `items`
/// root is not one of those three keys, so `resolve_path` always returns
/// `None` regardless of what the agent replies. Refused at author time
/// instead of shipping a gate that can never pass. Before this fix this
/// assertion is RED: `parse_workflow` returns `Ok`, so `.unwrap_err()`
/// panics with "called `Result::unwrap_err()` on an `Ok` value".
#[test]
fn postcondition_field_with_a_bare_structured_root_is_rejected() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "worker"
        kind = "agent"
        name = "Worker"
        agent = "ceo"
        [node.postcondition]
        require = "field_present"
        field = "items"
        [[edge]]
        from = "start"
        to = "worker"
    "#;
    let err = parse_workflow(src).unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("never lands at runtime") && message.contains("json.items"),
        "{message}"
    );
}

/// Companion GREEN: the documented `json.` prefix from the same field
/// name parses fine — the rejection above targets the missing prefix,
/// not the field name `items` itself.
#[test]
fn postcondition_field_with_the_json_prefix_still_parses() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "worker"
        kind = "agent"
        name = "Worker"
        agent = "ceo"
        [node.postcondition]
        require = "field_present"
        field = "json.items"
        [[edge]]
        from = "start"
        to = "worker"
    "#;
    let file = parse_workflow(src).expect("the documented `json.` prefix is accepted");
    let worker = file.nodes.iter().find(|n| n.id == "worker").unwrap();
    assert_eq!(
        worker
            .postcondition
            .as_ref()
            .and_then(|p| p.field.as_deref()),
        Some("json.items")
    );
}

/// Codex #3894162768 on #1937 — direct extension of the bare-root check
/// above: `text` and `agent_ref` are always plain strings in the
/// emitted output, so a dotted descendant like `text.foo` or
/// `agent_ref.id` can never resolve at runtime (`resolve_path` indexes a
/// `Value::String` with `.get("foo")`, which is always `None` — never a
/// panic, never a value). Before this fix this assertion is RED:
/// `parse_workflow` returns `Ok`, so `.unwrap_err()` panics.
#[test]
fn postcondition_field_dotted_into_text_or_agent_ref_is_rejected() {
    for field in ["text.foo", "agent_ref.id"] {
        let src = format!(
            r#"
            id = "wf"
            name = "WF"
            [[node]]
            id = "start"
            kind = "trigger"
            name = "Start"
            [[node]]
            id = "worker"
            kind = "agent"
            name = "Worker"
            agent = "ceo"
            [node.postcondition]
            require = "field_present"
            field = "{field}"
            [[edge]]
            from = "start"
            to = "worker"
        "#
        );
        let err = parse_workflow(&src)
            .expect_err(&format!("field `{field}` dots into a scalar root"));
        assert!(
            err.to_string().contains("always a plain string"),
            "field `{field}`: {err}"
        );
    }
}

/// Companion GREEN: the exact, childless roots `text` and `agent_ref`
/// stay valid on their own — the rejection above targets a dotted
/// DESCENDANT, not the roots themselves (already proven fine by
/// `postcondition_field_that_merely_resembles_a_reserved_key_still_parses`'s
/// bare `"text"` case; pinned again here alongside `agent_ref` for
/// symmetry with the failing test above).
#[test]
fn postcondition_field_of_exactly_text_or_agent_ref_still_parses() {
    for field in ["text", "agent_ref"] {
        let src = format!(
            r#"
            id = "wf"
            name = "WF"
            [[node]]
            id = "start"
            kind = "trigger"
            name = "Start"
            [[node]]
            id = "worker"
            kind = "agent"
            name = "Worker"
            agent = "ceo"
            [node.postcondition]
            require = "field_present"
            field = "{field}"
            [[edge]]
            from = "start"
            to = "worker"
        "#
        );
        parse_workflow(&src)
            .unwrap_or_else(|err| panic!("field `{field}` on its own is valid: {err}"));
    }
}

/// Codex #3894277296 on #1937 — the fourth structurally-impossible-gate
/// finding: `non_empty_list` only ever accepts a `Value::Array`, but
/// `text`/`agent_ref` are real, exact, childless roots (they pass every
/// check above), and BOTH are unconditionally strings — no reply the
/// agent could ever give makes `resolve_path(output, "text")` or
/// `resolve_path(output, "agent_ref")` come back an array. Before this
/// fix this assertion is RED: `parse_workflow` returns `Ok`, so
/// `.unwrap_err()` panics.
#[test]
fn postcondition_non_empty_list_on_text_or_agent_ref_is_rejected() {
    for field in ["text", "agent_ref"] {
        let src = format!(
            r#"
            id = "wf"
            name = "WF"
            [[node]]
            id = "start"
            kind = "trigger"
            name = "Start"
            [[node]]
            id = "worker"
            kind = "agent"
            name = "Worker"
            agent = "ceo"
            [node.postcondition]
            require = "non_empty_list"
            field = "{field}"
            [[edge]]
            from = "start"
            to = "worker"
        "#
        );
        let err = parse_workflow(&src).expect_err(&format!(
            "non_empty_list on `{field}` can never see an array"
        ));
        let message = err.to_string();
        assert!(
            message.contains("can never pass") && message.contains(field),
            "field `{field}`: {message}"
        );
    }
}

/// Companion GREEN, both halves of the rule: `non_empty_list` still
/// parses fine against `json` content (the root this predicate CAN be
/// satisfied through), and `field_present` still parses fine against
/// `text`/`agent_ref` (already pinned by
/// `postcondition_field_of_exactly_text_or_agent_ref_still_parses`
/// above — restated here as the other half of the same intersection
/// rule: `field_present` accepts any non-null kind, so it never
/// conflicts with a root's fixed kind, only `non_empty_list` does).
#[test]
fn postcondition_non_empty_list_on_json_content_still_parses() {
    for field in ["json", "json.items"] {
        let src = format!(
            r#"
            id = "wf"
            name = "WF"
            [[node]]
            id = "start"
            kind = "trigger"
            name = "Start"
            [[node]]
            id = "worker"
            kind = "agent"
            name = "Worker"
            agent = "ceo"
            [node.postcondition]
            require = "non_empty_list"
            field = "{field}"
            [[edge]]
            from = "start"
            to = "worker"
        "#
        );
        parse_workflow(&src)
            .unwrap_or_else(|err| panic!("field `{field}` is `json` content: {err}"));
    }
}

/// Codex #3893619015 on #1937: `text`/`agent_ref` are the two top-level
/// keys the emitted output always carries (`run_turn` inserts the raw
/// reply string / real roster id, then merges the parsed reply's own
/// fields in with `or_insert` — base wins on any collision, so
/// `delivery.rs::report_text` keeps finding prose in the overwhelming
/// majority of nodes whose reply isn't structured at all). A `field`
/// drilling into the parsed reply's OWN `json.text`/`json.agent_ref` key
/// can never be validated consistently with what a downstream binding
/// reads — the gate would check whatever shape the model chose to put
/// under that key, but the emitted value stays the raw string / real
/// roster id regardless. Refused at author time rather than left as a
/// silent runtime divergence a workflow could actually ship with.
#[test]
fn postcondition_field_into_reserved_json_key_is_rejected() {
    for field in ["json.text", "json.agent_ref", "json.text.nested"] {
        let src = format!(
            r#"
            id = "wf"
            name = "WF"
            [[node]]
            id = "start"
            kind = "trigger"
            name = "Start"
            [[node]]
            id = "worker"
            kind = "agent"
            name = "Worker"
            agent = "ceo"
            [node.postcondition]
            require = "field_present"
            field = "{field}"
            [[edge]]
            from = "start"
            to = "worker"
        "#
        );
        let err = parse_workflow(&src)
            .expect_err(&format!("field `{field}` collides with a reserved key"));
        assert!(
            err.to_string().contains("reserved"),
            "field `{field}`: {err}"
        );
    }
}

/// Companion GREEN: a `field` that does NOT collide with either reserved
/// key — including one that merely starts with `text`/`agent_ref` as a
/// substring, or names the outer envelope's own `text` (not
/// `json.text`) — still parses. The rejection is exactly the two
/// reserved dotted paths, not a blanket ban on the words `text` or
/// `agent_ref` anywhere in a `field`.
#[test]
fn postcondition_field_that_merely_resembles_a_reserved_key_still_parses() {
    for field in [
        "json.items",
        "json.text_summary",
        "json.agent_reference",
        "text",
    ] {
        let src = format!(
            r#"
            id = "wf"
            name = "WF"
            [[node]]
            id = "start"
            kind = "trigger"
            name = "Start"
            [[node]]
            id = "worker"
            kind = "agent"
            name = "Worker"
            agent = "ceo"
            [node.postcondition]
            require = "field_present"
            field = "{field}"
            [[edge]]
            from = "start"
            to = "worker"
        "#
        );
        parse_workflow(&src).unwrap_or_else(|err| {
            panic!("field `{field}` does not collide with a reserved key: {err}")
        });
    }
}

#[test]
fn config_agent_ref_on_agent_node_is_rejected() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "worker"
        kind = "agent"
        name = "Worker"
        agent = "ceo"
        [node.config]
        agent_ref = "impostor"
        [[edge]]
        from = "start"
        to = "worker"
    "#;
    let err = parse_workflow(src).unwrap_err();
    assert!(err.to_string().contains("agent_ref"), "{err}");
}

#[test]
fn route_without_error_edge_is_rejected() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "call"
        kind = "tool_call"
        name = "Call"
        on_error = "route"
        [[edge]]
        from = "start"
        to = "call"
    "#;
    let err = parse_workflow(src).unwrap_err();
    assert!(
        err.to_string().contains("no outgoing edge labeled `error`"),
        "{err}"
    );
}

#[test]
fn error_edge_without_route_is_rejected() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "call"
        kind = "tool_call"
        name = "Call"
        [[node]]
        id = "recover"
        kind = "output"
        name = "Recover"
        [[edge]]
        from = "start"
        to = "call"
        [[edge]]
        from = "call"
        to = "recover"
        label = "error"
    "#;
    let err = parse_workflow(src).unwrap_err();
    assert!(err.to_string().contains("only a routing node"), "{err}");
}

#[test]
fn route_with_matching_error_edge_is_valid() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "call"
        kind = "tool_call"
        name = "Call"
        on_error = "route"
        [node.config]
        slug = "csv_export"
        [[node]]
        id = "recover"
        kind = "output"
        name = "Recover"
        [[edge]]
        from = "start"
        to = "call"
        [[edge]]
        from = "call"
        to = "recover"
        label = "error"
    "#;
    assert!(parse_workflow(src).is_ok());
}

// --- P2: the six new node kinds ----------------------------------------

/// Each new node kind parses to its enum variant, and `WORKFLOW_NODE_KINDS`
/// advertises all twelve.
#[test]
fn new_node_kinds_parse() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "sw"
        kind = "switch"
        name = "Switch"
        [node.config]
        field = "=item.kind"
        [[node]]
        id = "mg"
        kind = "merge"
        name = "Merge"
        [[node]]
        id = "so"
        kind = "split_out"
        name = "Split"
        [[node]]
        id = "tf"
        kind = "transform"
        name = "Transform"
        [[node]]
        id = "op"
        kind = "output_parser"
        name = "Parse"
        [[edge]]
        from = "start"
        to = "sw"
        [[edge]]
        from = "sw"
        to = "mg"
        [[edge]]
        from = "mg"
        to = "so"
        [[edge]]
        from = "so"
        to = "tf"
        [[edge]]
        from = "tf"
        to = "op"
    "#;
    let file = parse_workflow(src).expect("new kinds parse");
    let kind = |id: &str| file.nodes.iter().find(|n| n.id == id).unwrap().kind;
    assert_eq!(kind("sw"), WorkflowNodeKind::Switch);
    assert_eq!(kind("mg"), WorkflowNodeKind::Merge);
    assert_eq!(kind("so"), WorkflowNodeKind::SplitOut);
    assert_eq!(kind("tf"), WorkflowNodeKind::Transform);
    assert_eq!(kind("op"), WorkflowNodeKind::OutputParser);
    assert_eq!(WORKFLOW_NODE_KINDS.len(), 12);
    assert!(WORKFLOW_NODE_KINDS.contains(&"sub_workflow"));
}

/// A `sub_workflow` node with a non-empty `workflow_id` string is valid.
#[test]
fn sub_workflow_with_workflow_id_is_valid() {
    let src = r#"
        id = "parent"
        name = "Parent"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "child"
        kind = "sub_workflow"
        name = "Child"
        [node.config]
        workflow_id = "greet"
        [[edge]]
        from = "start"
        to = "child"
    "#;
    let file = parse_workflow(src).expect("sub_workflow parses");
    let child = file.nodes.iter().find(|n| n.id == "child").unwrap();
    assert_eq!(child.kind, WorkflowNodeKind::SubWorkflow);
    assert_eq!(child.config.as_ref().unwrap()["workflow_id"], "greet");
}

/// A `sub_workflow` node with no `config` is rejected — it names nothing to run.
#[test]
fn sub_workflow_without_config_is_rejected() {
    let src = r#"
        id = "parent"
        name = "Parent"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "child"
        kind = "sub_workflow"
        name = "Child"
    "#;
    let err = parse_workflow(src).unwrap_err();
    assert!(err.to_string().contains("workflow_id"), "{err}");
}

/// A `sub_workflow` node with an empty `workflow_id` is rejected.
#[test]
fn sub_workflow_with_empty_workflow_id_is_rejected() {
    let src = r#"
        id = "parent"
        name = "Parent"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "child"
        kind = "sub_workflow"
        name = "Child"
        [node.config]
        workflow_id = ""
    "#;
    let err = parse_workflow(src).unwrap_err();
    assert!(err.to_string().contains("empty `workflow_id`"), "{err}");
}

/// A `sub_workflow` node naming its own workflow id is a static self-reference.
#[test]
fn sub_workflow_self_reference_is_rejected() {
    let src = r#"
        id = "loopy"
        name = "Loopy"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "child"
        kind = "sub_workflow"
        name = "Child"
        [node.config]
        workflow_id = "loopy"
        [[edge]]
        from = "start"
        to = "child"
    "#;
    let err = parse_workflow(src).unwrap_err();
    assert!(err.to_string().contains("its own workflow id"), "{err}");
}

/// An inline `workflow` child graph is reserved — a sub_workflow must
/// reference a saved workflow by id so the child passes OpenCompany validation.
#[test]
fn sub_workflow_inline_child_graph_is_rejected() {
    let src = r#"
        id = "parent"
        name = "Parent"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "child"
        kind = "sub_workflow"
        name = "Child"
        [node.config]
        workflow_id = "greet"
        [node.config.workflow]
        id = "inlined"
    "#;
    let err = parse_workflow(src).unwrap_err();
    assert!(err.to_string().contains("inline `workflow`"), "{err}");
}

/// On a `switch`, an `error`-labeled edge is a legitimate case name — it must
/// NOT trip the `error`-label ⇔ `on_error = "route"` coupling check.
#[test]
fn switch_error_label_is_a_case_not_a_route() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "sw"
        kind = "switch"
        name = "Switch"
        [node.config]
        field = "=item.kind"
        [[node]]
        id = "err_case"
        kind = "output"
        name = "Error case"
        [[node]]
        id = "ok_case"
        kind = "output"
        name = "OK case"
        [[edge]]
        from = "start"
        to = "sw"
        [[edge]]
        from = "sw"
        to = "err_case"
        label = "error"
        [[edge]]
        from = "sw"
        to = "ok_case"
        label = "ok"
    "#;
    assert!(
        parse_workflow(src).is_ok(),
        "an error-labeled switch case must be valid without on_error = route"
    );
}

// --- Per-kind required config (issue #661) ------------------------------

/// A `condition` node with no `config.field` still LOADS on the lenient
/// read path (issue #682: pre-#661 saved graphs must keep loading), but the
/// STRICT author-time pass reports it — without a field the engine tests the
/// whole item and the branch is silently meaningless. This is the regression
/// guard: a field-less-condition graph parses via `parse_workflow` yet is
/// rejected by `validate(_, true)`.
#[test]
fn condition_without_field_loads_leniently_but_strict_rejects() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "gate"
        kind = "condition"
        name = "Gate"
        [[node]]
        id = "yes_out"
        kind = "output"
        name = "Yes"
        [[node]]
        id = "no_out"
        kind = "output"
        name = "No"
        [[edge]]
        from = "start"
        to = "gate"
        [[edge]]
        from = "gate"
        to = "yes_out"
        label = "yes"
        [[edge]]
        from = "gate"
        to = "no_out"
        label = "no"
    "#;
    // Lenient load path accepts it — a graph persisted before #661 still loads.
    assert!(parse_workflow(src).is_ok());
    // Strict author-time pass reports the missing field.
    let raw: RawWorkflow = toml::from_str(src).expect("the fixture is valid TOML");
    let problems = validate(&raw, true).join("\n");
    assert!(problems.contains("config.field"), "{problems}");
}

/// A `condition` branch labeled anything but `yes`/`no` loads leniently
/// (issue #682) but is rejected by the STRICT author-time pass — an
/// off-vocabulary label silently maps onto the `true` port.
#[test]
fn condition_branch_with_non_yes_no_label_loads_leniently_but_strict_rejects() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "gate"
        kind = "condition"
        name = "Gate"
        [node.config]
        field = "=item.ok"
        [[node]]
        id = "a"
        kind = "output"
        name = "A"
        [[node]]
        id = "b"
        kind = "output"
        name = "B"
        [[edge]]
        from = "start"
        to = "gate"
        [[edge]]
        from = "gate"
        to = "a"
        label = "pass"
        [[edge]]
        from = "gate"
        to = "b"
        label = "no"
    "#;
    // Lenient load path accepts the off-vocabulary label.
    assert!(parse_workflow(src).is_ok());
    // Strict author-time pass reports it.
    let raw: RawWorkflow = toml::from_str(src).expect("the fixture is valid TOML");
    let problems = validate(&raw, true).join("\n");
    assert!(problems.contains("labeled `yes` or `no`"), "{problems}");
}

/// A well-formed `condition` — a `field` plus `yes`/`no` branches — parses.
#[test]
fn condition_with_field_and_yes_no_labels_is_valid() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "gate"
        kind = "condition"
        name = "Gate"
        [node.config]
        field = "=item.approved"
        [[node]]
        id = "a"
        kind = "output"
        name = "A"
        [[node]]
        id = "b"
        kind = "output"
        name = "B"
        [[edge]]
        from = "start"
        to = "gate"
        [[edge]]
        from = "gate"
        to = "a"
        label = "yes"
        [[edge]]
        from = "gate"
        to = "b"
        label = "no"
    "#;
    assert!(parse_workflow(src).is_ok());
}

/// An `http_request` node missing `config.method` / `config.url` loads
/// leniently (issue #682) but the STRICT pass reports BOTH missing keys.
#[test]
fn http_request_without_method_or_url_loads_leniently_but_strict_rejects() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "fetch"
        kind = "http_request"
        name = "Fetch"
        [[edge]]
        from = "start"
        to = "fetch"
    "#;
    // Lenient load path accepts it.
    assert!(parse_workflow(src).is_ok());
    // Strict author-time pass reports both missing config keys at once.
    let raw: RawWorkflow = toml::from_str(src).expect("the fixture is valid TOML");
    let message = validate(&raw, true).join("\n");
    assert!(message.contains("config.method"), "{message}");
    assert!(message.contains("config.url"), "{message}");
}

/// A `switch` node with neither `field` nor `expression` loads leniently
/// (issue #682) but the STRICT pass reports the missing discriminant.
#[test]
fn switch_without_discriminant_loads_leniently_but_strict_rejects() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "sw"
        kind = "switch"
        name = "Switch"
        [[node]]
        id = "case_a"
        kind = "output"
        name = "A"
        [[edge]]
        from = "start"
        to = "sw"
        [[edge]]
        from = "sw"
        to = "case_a"
        label = "a"
    "#;
    // Lenient load path accepts it.
    assert!(parse_workflow(src).is_ok());
    // Strict author-time pass reports the missing discriminant.
    let raw: RawWorkflow = toml::from_str(src).expect("the fixture is valid TOML");
    let problems = validate(&raw, true).join("\n");
    assert!(problems.contains("discriminant"), "{problems}");
}

/// Strict author-time parity (issue #661/#682): a `tool_call` with no `slug`
/// loads leniently now, but the STRICT pass reports it — the same shape the
/// console-draft path rejects — so `translate` never has to fall back to the
/// node id as a placeholder slug once a graph reaches an author surface.
#[test]
fn tool_call_without_slug_loads_leniently_but_strict_rejects() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "call"
        kind = "tool_call"
        name = "Call"
        [[edge]]
        from = "start"
        to = "call"
    "#;
    // Lenient load path accepts it.
    assert!(parse_workflow(src).is_ok());
    // Strict author-time pass reports the missing slug.
    let raw: RawWorkflow = toml::from_str(src).expect("the fixture is valid TOML");
    let problems = validate(&raw, true).join("\n");
    assert!(problems.contains("config.slug"), "{problems}");
}

// --- G15: inescapable cycles + reachability (issue #540) ----------------

/// A bare two-node cycle with no branch to leave it is a trap: once the run
/// reaches `a` it loops `a → b → a` forever. Rejected, naming both nodes.
/// (Negative control for the cycle check: delete the SCC-exit check and this
/// graph — which has no condition/switch at all — would wrongly pass.)
#[test]
fn multi_node_cycle_with_no_exit_is_rejected() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "a"
        kind = "agent"
        name = "A"
        agent = "ceo"
        [[node]]
        id = "b"
        kind = "agent"
        name = "B"
        agent = "ceo"
        [[edge]]
        from = "start"
        to = "a"
        [[edge]]
        from = "a"
        to = "b"
        [[edge]]
        from = "b"
        to = "a"
    "#;
    let err = parse_workflow(src).unwrap_err();
    let message = err.to_string();
    assert!(message.contains("form a loop"), "{message}");
    assert!(message.contains("`a`"), "{message}");
    assert!(message.contains("`b`"), "{message}");
}

/// A miniature of the shipped `game_build_pipeline` shape: a `condition`
/// guards the loop, with a `yes` branch that leaves it and a `no` branch
/// that loops back. That is a legal bounded retry — it must parse clean.
#[test]
fn condition_guarded_loop_is_valid() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "work"
        kind = "agent"
        name = "Work"
        agent = "ceo"
        [[node]]
        id = "gate"
        kind = "condition"
        name = "Good enough?"
        [node.config]
        field = "=item.good_enough"
        [[node]]
        id = "done"
        kind = "output"
        name = "Ship"
        [[edge]]
        from = "start"
        to = "work"
        [[edge]]
        from = "work"
        to = "gate"
        [[edge]]
        from = "gate"
        to = "done"
        label = "yes"
        [[edge]]
        from = "gate"
        to = "work"
        label = "no"
    "#;
    assert!(
        parse_workflow(src).is_ok(),
        "a condition-guarded retry loop must stay valid"
    );
}

/// The shipped guarded-retry preset itself must stay valid — its loop
/// (`gameplay → assets → balance → qa → gate → gameplay`) is escapable
/// because `gate` is a `condition` whose `yes` branch leaves the loop.
#[test]
fn the_shipped_guarded_loop_preset_is_valid() {
    const GAME: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../companies/game_studio/workflows/game_build_pipeline.toml"
    ));
    parse_workflow(GAME).expect("the game-studio guarded loop is valid");
}

/// A cycle that DOES contain a `condition`, but whose only edges all stay
/// inside the loop, is still inescapable — a branch that never leaves the SCC
/// buys nothing. (Negative control: the exit test, not merely "contains a
/// condition", is what this asserts.)
#[test]
fn inescapable_cycle_containing_condition_with_no_exit_is_rejected() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "a"
        kind = "agent"
        name = "A"
        agent = "ceo"
        [[node]]
        id = "gate"
        kind = "condition"
        name = "Gate"
        [[edge]]
        from = "start"
        to = "a"
        [[edge]]
        from = "a"
        to = "gate"
        [[edge]]
        from = "gate"
        to = "a"
        label = "again"
    "#;
    let err = parse_workflow(src).unwrap_err();
    assert!(err.to_string().contains("form a loop"), "{err}");
}

/// A node no edge ever reaches from the trigger would never execute. It is
/// rejected, naming the orphan. (Negative control for reachability.)
#[test]
fn unreachable_node_is_rejected() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "reached"
        kind = "output"
        name = "Reached"
        [[node]]
        id = "orphan"
        kind = "output"
        name = "Orphan"
        [[edge]]
        from = "start"
        to = "reached"
    "#;
    let err = parse_workflow(src).unwrap_err();
    let message = err.to_string();
    assert!(message.contains("cannot be reached"), "{message}");
    assert!(message.contains("`orphan`"), "{message}");
    // The reached node must NOT be named — only the genuine orphan.
    assert!(!message.contains("`reached`"), "{message}");
}

/// With no trigger at all, the reachability check stays silent: the
/// "needs at least one trigger" problem is the real one, and flagging every
/// node as unreachable on top of it would just be noise.
#[test]
fn no_trigger_skips_reachability_noise() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "only"
        kind = "output"
        name = "Only"
        [[node]]
        id = "other"
        kind = "output"
        name = "Other"
    "#;
    let err = parse_workflow(src).unwrap_err();
    let message = err.to_string();
    assert!(message.contains("trigger"), "{message}");
    assert!(
        !message.contains("cannot be reached"),
        "reachability must be skipped with no trigger: {message}"
    );
}

/// A trigger with an EMPTY id already fails id-validation, and it seeds no
/// entry into the reachability BFS. The gate keys off `trigger_ids` (usable
/// entries), not the raw trigger count, so the run's one valid node is NOT
/// piled with a bogus "cannot be reached" on top of the real "missing an
/// `id`" problem. (Regression, #540.)
#[test]
fn empty_id_trigger_skips_reachability_noise() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = ""
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "work"
        kind = "output"
        name = "Work"
    "#;
    let err = parse_workflow(src).unwrap_err();
    let message = err.to_string();
    assert!(message.contains("missing an `id`"), "{message}");
    assert!(
        !message.contains("cannot be reached"),
        "an id-less trigger must not spawn reachability noise: {message}"
    );
}

#[test]
fn legacy_files_without_new_fields_parse_unchanged() {
    // A graph authored before the P1 fields existed: every new field is None.
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
    "#;
    let file = parse_workflow(src).expect("legacy parses");
    let start = &file.nodes[0];
    assert!(start.config.is_none());
    assert!(start.on_error.is_none());
    assert!(start.retry.is_none());
    assert!(start.requires_approval.is_none());
    assert!(start.destination.is_none());
}

// --- Output destination (issue #170) ------------------------------------

/// A graph with one `output` node carrying `destination` of `kind`, plus an
/// optional `target` line.
fn with_destination(kind: &str, target: Option<&str>) -> String {
    let target_line = target
        .map(|t| format!("target = \"{t}\"\n"))
        .unwrap_or_default();
    format!(
        r#"
id = "wf"
name = "WF"
[[node]]
id = "start"
kind = "trigger"
name = "Start"
[[node]]
id = "done"
kind = "output"
name = "Report back"
[node.destination]
kind = "{kind}"
{target_line}
[[edge]]
from = "start"
to = "done"
"#
    )
}

/// Each of the three destination kinds parses onto the first-class field
/// with its target contract intact.
#[test]
fn output_destinations_parse_for_every_kind() {
    let owner = parse_workflow(&with_destination("owner", None)).expect("owner parses");
    let dest = owner.nodes[1].destination.as_ref().expect("present");
    assert_eq!(dest.kind, "owner");
    assert_eq!(dest.target, None);

    let email = parse_workflow(&with_destination("email", Some("ada@example.com")))
        .expect("email parses");
    let dest = email.nodes[1].destination.as_ref().expect("present");
    assert_eq!(dest.kind, "email");
    assert_eq!(dest.target.as_deref(), Some("ada@example.com"));

    let channel =
        parse_workflow(&with_destination("channel", Some("operator"))).expect("channel parses");
    let dest = channel.nodes[1].destination.as_ref().expect("present");
    assert_eq!(dest.kind, "channel");
    assert_eq!(dest.target.as_deref(), Some("operator"));
}

/// The reachability predicate mirrors delivery's per-kind outcome: `owner`
/// always lands (the durable operator channel is its guaranteed fallback,
/// issue #1757), `email` needs a mailbox (issue #1046), `channel` needs a
/// wired, non-operator target, anything else never lands.
#[test]
fn destination_reachability_matches_delivery() {
    let owner = WorkflowDestinationDef {
        kind: "owner".to_string(),
        target: None,
    };
    // Owner lands with a mailbox (emails admins) AND without one (durable
    // operator channel) — issue #1757.
    assert!(destination_is_reachable(&owner, true, &[]));
    assert!(destination_is_reachable(&owner, false, &[]));

    let email = WorkflowDestinationDef {
        kind: "email".to_string(),
        target: Some("ada@example.com".to_string()),
    };
    assert!(destination_is_reachable(&email, true, &[]));
    assert!(!destination_is_reachable(&email, false, &[]));

    let eng = vec!["engineering".to_string()];
    let channel = WorkflowDestinationDef {
        kind: "channel".to_string(),
        target: Some("engineering".to_string()),
    };
    assert!(destination_is_reachable(&channel, false, &eng));
    // An unwired channel never lands, mailbox or not.
    let unwired = WorkflowDestinationDef {
        kind: "channel".to_string(),
        target: Some("marketing".to_string()),
    };
    assert!(!destination_is_reachable(&unwired, true, &eng));
    // `operator` IS reachable now (issue #1757): it is a durable channel the
    // company always wires, so `deliverable_channel_ids` lists it.
    let operator = WorkflowDestinationDef {
        kind: "channel".to_string(),
        target: Some(crate::runtime::channel::OPERATOR_CHANNEL.to_string()),
    };
    assert!(destination_is_reachable(
        &operator,
        true,
        &[crate::runtime::channel::OPERATOR_CHANNEL.to_string()],
    ));
    // But only when it is in the wired set — an empty runtime still can't.
    assert!(!destination_is_reachable(&operator, true, &[]));
}

/// The none-vs-any line the arm gate rides on. A graph with one unreachable
/// output (a channel to an unwired desk) and one reachable channel output is
/// deliverable — a partially-deliverable schedule still arms; only a graph
/// where **nothing** can land is refused.
///
/// Uses two `channel` outputs rather than owner+channel because `owner` now
/// always lands (issue #1757), so it can no longer play the unreachable half.
#[test]
fn mixed_graph_is_deliverable_when_any_output_lands() {
    let src = r#"
id = "wf"
name = "WF"
[[node]]
id = "start"
kind = "trigger"
name = "Start"
schedule = "0 9 * * *"
[[node]]
id = "to_unwired"
kind = "output"
name = "Unwired"
[node.destination]
kind = "channel"
target = "marketing"
[[node]]
id = "to_channel"
kind = "output"
name = "Channel"
[node.destination]
kind = "channel"
target = "engineering"
[[edge]]
from = "start"
to = "to_unwired"
[[edge]]
from = "start"
to = "to_channel"
"#;
    let file = parse_workflow(src).expect("parses");
    assert!(file.has_output_destination());
    let eng = vec!["engineering".to_string()];
    // The `marketing` output is dead (unwired), but the `engineering`
    // channel output lands.
    assert!(file.has_deliverable_output(false, &eng));
    // Wire nothing: now neither channel lands, so nothing does.
    assert!(!file.has_deliverable_output(false, &[]));
}

#[test]
fn unknown_destination_kind_is_rejected() {
    let err = parse_workflow(&with_destination("carrier_pigeon", Some("x"))).unwrap_err();
    let message = err.to_string();
    assert!(message.contains("unknown `destination.kind`"), "{message}");
    // The message names what IS supported, not just what isn't.
    assert!(message.contains("owner"), "{message}");
}

/// An `email` destination MUST name an address. This is the validation half
/// of the security boundary: a workflow cannot mail "somebody" — the
/// recipient is pinned in the graph, where a reviewer can see it.
#[test]
fn email_destination_without_an_address_is_rejected() {
    let err = parse_workflow(&with_destination("email", Some("ada"))).unwrap_err();
    assert!(err.to_string().contains("not an email address"), "{err}");
    let err = parse_workflow(&with_destination("email", None)).unwrap_err();
    assert!(err.to_string().contains("not an email address"), "{err}");
}

#[test]
fn channel_destination_without_a_target_is_rejected() {
    let err = parse_workflow(&with_destination("channel", None)).unwrap_err();
    assert!(err.to_string().contains("no `target`"), "{err}");
}

/// `owner` resolves server-side, so a target on it is a mistake worth
/// naming — otherwise an author writes an address there and quietly gets
/// the admins instead.
#[test]
fn owner_destination_with_a_target_is_rejected() {
    let err = parse_workflow(&with_destination("owner", Some("ada@example.com"))).unwrap_err();
    assert!(err.to_string().contains("`owner` destination"), "{err}");
}

/// `repeatable` is a statement about a call, so a node that makes none has
/// no answer to give — and an inert declaration an author believes is a
/// guard is worse than no field at all (issue #850).
#[test]
fn repeatable_on_a_node_that_makes_no_call_is_rejected() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "worker"
        kind = "agent"
        name = "Worker"
        agent = "ceo"
        repeatable = false
        [[edge]]
        from = "start"
        to = "worker"
    "#;
    let err = parse_workflow(src).unwrap_err();
    assert!(
        err.to_string()
            .contains("only `tool_call` and `http_request` nodes make a call"),
        "{err}"
    );
}

/// The two kinds that do make a call accept it.
#[test]
fn repeatable_is_accepted_on_a_tool_call() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "publish"
        kind = "tool_call"
        name = "Publish"
        repeatable = false
        [node.config]
        slug = "shell"
        [[edge]]
        from = "start"
        to = "publish"
    "#;
    let wf = parse_workflow(src).expect("valid");
    let node = wf.nodes.iter().find(|n| n.id == "publish").expect("node");
    assert_eq!(node.repeatable, Some(false));
}

/// `repeatable` inside `config` would ride into the engine graph as an
/// inert key and guard nothing — reject it like the other reserved keys.
#[test]
fn repeatable_inside_config_is_rejected() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "publish"
        kind = "tool_call"
        name = "Publish"
        [node.config]
        slug = "shell"
        repeatable = false
        [[edge]]
        from = "start"
        to = "publish"
    "#;
    let err = parse_workflow(src).unwrap_err();
    assert!(
        err.to_string()
            .contains("puts `repeatable` inside `config`"),
        "{err}"
    );
}

/// Only `output` nodes report back, so a `destination` anywhere else is a
/// silent no-op waiting to happen.
#[test]
fn destination_on_a_non_output_node_is_rejected() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "worker"
        kind = "agent"
        name = "Worker"
        agent = "ceo"
        [node.destination]
        kind = "owner"
    "#;
    let err = parse_workflow(src).unwrap_err();
    assert!(
        err.to_string()
            .contains("only `output` nodes route a report"),
        "{err}"
    );
}

/// `destination` inside `config` would ride into the engine graph as an
/// inert key and deliver nothing — reject it like the other reserved keys.
#[test]
fn destination_inside_config_is_rejected() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "done"
        kind = "output"
        name = "Done"
        [node.config]
        destination = "owner"
        [[edge]]
        from = "start"
        to = "done"
    "#;
    let err = parse_workflow(src).unwrap_err();
    let message = err.to_string();
    assert!(message.contains("destination"), "{message}");
    assert!(message.contains("inside `config`"), "{message}");
}

/// A destination-bearing graph renders back to TOML and re-parses to the
/// same model — the create route's persist path depends on this.
#[test]
fn destination_round_trips_through_render_and_parse() {
    let raw = RawWorkflow {
        id: "wf".to_string(),
        name: "WF".to_string(),
        description: None,
        owner_desk: None,
        nodes: vec![
            RawNode {
                id: "start".to_string(),
                kind: "trigger".to_string(),
                name: "Start".to_string(),
                summary: None,
                agent: None,
                schedule: None,
                config: None,
                on_error: None,
                retry: None,
                requires_approval: None,
                repeatable: None,
                destination: None,
                postcondition: None,
                verify: None,
            },
            RawNode {
                id: "done".to_string(),
                kind: "output".to_string(),
                name: "Report".to_string(),
                summary: None,
                agent: None,
                schedule: None,
                config: None,
                on_error: None,
                retry: None,
                requires_approval: None,
                repeatable: None,
                destination: Some(WorkflowDestinationDef {
                    kind: "email".to_string(),
                    target: Some("ada@example.com".to_string()),
                }),
                postcondition: None,
                verify: None,
            },
        ],
        edges: vec![RawEdge {
            from: "start".to_string(),
            to: "done".to_string(),
            label: None,
        }],
    };
    let toml_src = render_workflow(&raw).expect("renders");
    let file = parse_workflow(&toml_src).expect("re-parses");
    let dest = file.nodes[1].destination.as_ref().expect("present");
    assert_eq!(dest.kind, "email");
    assert_eq!(dest.target.as_deref(), Some("ada@example.com"));
}

/// A legacy graph (no `destination` anywhere) renders byte-identically to
/// what it rendered before the field existed — `skip_serializing_if` is what
/// keeps an unchanged file from churning on every re-save.
#[test]
fn a_graph_without_a_destination_renders_no_destination_key() {
    let raw = RawWorkflow {
        id: "wf".to_string(),
        name: "WF".to_string(),
        description: None,
        owner_desk: None,
        nodes: vec![RawNode {
            id: "start".to_string(),
            kind: "trigger".to_string(),
            name: "Start".to_string(),
            summary: None,
            agent: None,
            schedule: None,
            config: None,
            on_error: None,
            retry: None,
            requires_approval: None,
            repeatable: None,
            destination: None,
            postcondition: None,
            verify: None,
        }],
        edges: Vec::new(),
    };
    let toml_src = render_workflow(&raw).expect("renders");
    assert!(!toml_src.contains("destination"), "{toml_src}");
}

// --- trigger schedule (issue #169) --------------------------------------

/// A trigger's `schedule` survives the render → parse round trip the create
/// endpoint runs, and lands on the parsed node.
#[test]
fn trigger_schedule_round_trips_render_and_parse() {
    let raw = RawWorkflow {
        id: "wf".to_string(),
        name: "WF".to_string(),
        description: None,
        owner_desk: None,
        nodes: vec![
            RawNode {
                id: "start".to_string(),
                kind: "trigger".to_string(),
                name: "Start".to_string(),
                summary: None,
                agent: None,
                schedule: Some("0 * * * *".to_string()),
                config: None,
                on_error: None,
                retry: None,
                requires_approval: None,
                repeatable: None,
                destination: None,
                postcondition: None,
                verify: None,
            },
            RawNode {
                id: "done".to_string(),
                kind: "output".to_string(),
                name: "Done".to_string(),
                summary: None,
                agent: None,
                schedule: None,
                config: None,
                on_error: None,
                retry: None,
                requires_approval: None,
                repeatable: None,
                destination: None,
                postcondition: None,
                verify: None,
            },
        ],
        edges: vec![RawEdge {
            from: "start".to_string(),
            to: "done".to_string(),
            label: None,
        }],
    };
    let toml_src = render_workflow(&raw).expect("renders");
    let file = parse_workflow(&toml_src).expect("re-parses");
    let start = file.nodes.iter().find(|n| n.id == "start").unwrap();
    assert_eq!(start.schedule.as_deref(), Some("0 * * * *"));
    let done = file.nodes.iter().find(|n| n.id == "done").unwrap();
    assert!(done.schedule.is_none());
}

/// A trigger schedule parses from hand-authored TOML too, including the
/// named-weekday dialect the manifest `[[schedule]]` crons already accept.
#[test]
fn trigger_schedule_parses_from_toml() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        schedule = "0 9 * * MON"
    "#;
    let file = parse_workflow(src).expect("parses");
    assert_eq!(file.nodes[0].schedule.as_deref(), Some("0 9 * * MON"));
}

/// `schedule` says when the *workflow* starts, so it is trigger-only.
#[test]
fn schedule_on_a_non_trigger_node_is_rejected() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [[node]]
        id = "worker"
        kind = "agent"
        name = "Worker"
        agent = "ceo"
        schedule = "0 * * * *"
    "#;
    let err = parse_workflow(src).unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("only `trigger` nodes carry a schedule"),
        "{message}"
    );
}

/// A malformed cron is rejected at validation with the parser's own
/// message, so it can never be persisted as an expression that never fires.
#[test]
fn invalid_trigger_schedule_cron_is_rejected() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        schedule = "every hour"
    "#;
    let err = parse_workflow(src).unwrap_err();
    let message = err.to_string();
    assert!(message.contains("not a valid cron"), "{message}");
    assert!(message.contains("needs 5 fields"), "{message}");
    assert!(message.contains("UTC"), "{message}");

    // An out-of-range field is caught by the same parser.
    let out_of_range = src.replace("every hour", "0 99 * * *");
    let err = parse_workflow(&out_of_range).unwrap_err();
    assert!(err.to_string().contains("not a valid cron"), "{err}");
}

/// Two scheduled triggers would double-run the workflow, and honoring only
/// the first would silently drop a schedule the operator saved — so the
/// graph is rejected, naming both offenders.
#[test]
fn two_scheduled_triggers_are_rejected() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "nightly"
        kind = "trigger"
        name = "Nightly"
        schedule = "0 2 * * *"
        [[node]]
        id = "hourly"
        kind = "trigger"
        name = "Hourly"
        schedule = "0 * * * *"
    "#;
    let err = parse_workflow(src).unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("at most one scheduled trigger"),
        "{message}"
    );
    assert!(message.contains("`nightly`"), "{message}");
    assert!(message.contains("`hourly`"), "{message}");
}

/// The at-most-one rule counts *schedules*, not triggers: a graph may still
/// have several triggers, and one of them may be scheduled.
#[test]
fn multiple_triggers_are_still_allowed_when_at_most_one_is_scheduled() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "manual"
        kind = "trigger"
        name = "Manual"
        [[node]]
        id = "webhook"
        kind = "trigger"
        name = "Webhook"
        [[node]]
        id = "nightly"
        kind = "trigger"
        name = "Nightly"
        schedule = "0 2 * * *"
    "#;
    let file = parse_workflow(src).expect("several triggers stay legal");
    assert_eq!(file.nodes.len(), 3);
    let scheduled: Vec<&str> = file
        .nodes
        .iter()
        .filter(|n| n.schedule.is_some())
        .map(|n| n.id.as_str())
        .collect();
    assert_eq!(scheduled, vec!["nightly"]);

    // And with no schedules at all, unchanged from before this rule.
    let bare = src.replace("schedule = \"0 2 * * *\"", "");
    assert!(parse_workflow(&bare).is_ok());
}

/// Two *malformed* schedules report the bad crons AND the at-most-one
/// problem together, matching the module's report-everything-at-once
/// contract.
#[test]
fn two_bad_schedules_report_every_problem_at_once() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "a"
        kind = "trigger"
        name = "A"
        schedule = "nightly"
        [[node]]
        id = "b"
        kind = "trigger"
        name = "B"
        schedule = "hourly"
    "#;
    let err = parse_workflow(src).unwrap_err();
    let message = err.to_string();
    assert!(message.contains("not a valid cron"), "{message}");
    assert!(
        message.contains("at most one scheduled trigger"),
        "{message}"
    );
}

/// `config.schedule` would be silently ignored (the first-class field wins),
/// so it is a reserved key like the other first-class node fields.
#[test]
fn config_schedule_is_rejected_as_a_reserved_key() {
    let src = r#"
        id = "wf"
        name = "WF"
        [[node]]
        id = "start"
        kind = "trigger"
        name = "Start"
        [node.config]
        schedule = "0 * * * *"
    "#;
    let err = parse_workflow(src).unwrap_err();
    let message = err.to_string();
    assert!(message.contains("`schedule` inside `config`"), "{message}");
}

/// A graph authored before `schedule` existed parses with the field unset
/// and re-renders byte-identically — the field is skipped when `None`, so
/// adding it rewrites nothing on disk.
#[test]
fn legacy_graph_without_schedule_re_renders_byte_identically() {
    let raw = RawWorkflow {
        id: "wf".to_string(),
        name: "WF".to_string(),
        description: Some("Legacy.".to_string()),
        owner_desk: None,
        nodes: vec![RawNode {
            id: "start".to_string(),
            kind: "trigger".to_string(),
            name: "Start".to_string(),
            summary: Some("Kicks off.".to_string()),
            agent: None,
            schedule: None,
            config: None,
            on_error: None,
            retry: None,
            requires_approval: None,
            repeatable: None,
            destination: None,
            postcondition: None,
            verify: None,
        }],
        edges: Vec::new(),
    };
    let first = render_workflow(&raw).expect("renders");
    assert!(
        !first.contains("schedule"),
        "an unset schedule must not be written: {first}"
    );

    let file = parse_workflow(&first).expect("parses");
    assert!(file.nodes[0].schedule.is_none());

    // Re-render the parsed graph through the same shape: byte-identical.
    let round_tripped = RawWorkflow {
        id: file.id.clone(),
        name: file.name.clone(),
        description: file.description.clone(),
        owner_desk: file.owner_desk.clone(),
        nodes: file
            .nodes
            .iter()
            .map(|n| RawNode {
                id: n.id.clone(),
                kind: n.kind.as_str().to_string(),
                name: n.name.clone(),
                summary: n.summary.clone(),
                agent: n.agent.clone(),
                schedule: n.schedule.clone(),
                config: None,
                on_error: n.on_error.clone(),
                retry: n.retry.clone(),
                requires_approval: n.requires_approval,
                repeatable: None,
                destination: n.destination.clone(),
                postcondition: None,
                verify: None,
            })
            .collect(),
        edges: Vec::new(),
    };
    assert_eq!(render_workflow(&round_tripped).expect("re-renders"), first);
}

// --- seed ∪ overlay union (issue #168) ----------------------------------

use crate::ports::types::OverlayWorkflow;

/// A minimal valid graph body with the given id and display name.
fn body(id: &str, name: &str) -> String {
    format!(
        r#"
id = "{id}"
name = "{name}"
[[node]]
id = "start"
kind = "trigger"
name = "Start"
[[node]]
id = "done"
kind = "output"
name = "Done"
[[edge]]
from = "start"
to = "done"
"#
    )
}

fn overlay(id: &str, name: &str) -> OverlayWorkflow {
    OverlayWorkflow {
        id: id.to_string(),
        toml: body(id, name),
    }
}

/// Writes a seed graph to `<dir>/workflows/<id>.toml`.
fn seed(dir: &Path, id: &str, name: &str) {
    let workflows = dir.join("workflows");
    std::fs::create_dir_all(&workflows).unwrap();
    std::fs::write(workflows.join(format!("{id}.toml")), body(id, name)).unwrap();
}

/// The hosted shape: no source directory at all, so the overlay is the only
/// source. This is the read half of the #168 fix.
#[test]
fn load_union_falls_back_to_the_overlay_with_no_source_dir() {
    let overlays = vec![overlay("hosted", "Hosted flow")];
    let file = load_workflow_union(None, &overlays, "hosted")
        .expect("loads")
        .expect("present");
    assert_eq!(file.id, "hosted");
    assert_eq!(file.name, "Hosted flow");
    assert_eq!(file.nodes.len(), 2);
}

/// A source directory that simply has no file for the id also falls through.
#[test]
fn load_union_falls_back_when_the_seed_file_is_absent() {
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), "other", "Other");
    let overlays = vec![overlay("mine", "Mine")];
    let file = load_workflow_union(Some(dir.path()), &overlays, "mine")
        .expect("loads")
        .expect("present");
    assert_eq!(file.name, "Mine");
}

/// Documented precedence: the committed seed file wins over an overlay body
/// with the same id. The overlay is shadowed, not destroyed.
#[test]
fn load_union_prefers_the_seed_file_on_an_id_collision() {
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), "dup", "From seed");
    let overlays = vec![overlay("dup", "From overlay")];
    let file = load_workflow_union(Some(dir.path()), &overlays, "dup")
        .expect("loads")
        .expect("present");
    assert_eq!(file.name, "From seed");
}

/// An id neither source has is `Ok(None)` — the caller's clean 404, not an
/// error.
#[test]
fn load_union_of_an_unknown_id_is_none() {
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), "known", "Known");
    assert!(
        load_workflow_union(Some(dir.path()), &[], "ghost")
            .expect("no error")
            .is_none()
    );
    assert!(
        load_workflow_union(None, &[], "ghost")
            .expect("no error")
            .is_none()
    );
}

/// A malformed overlay body surfaces as an error labelled with its id — the
/// same shape a malformed on-disk file gets.
#[test]
fn load_union_of_a_malformed_overlay_is_an_error() {
    let overlays = vec![OverlayWorkflow {
        id: "broken".to_string(),
        toml: "id = \"broken\"\nname = \"Broken\"\n".to_string(),
    }];
    let err = load_workflow_union(None, &overlays, "broken").unwrap_err();
    assert!(err.to_string().contains("trigger"), "{err}");
    assert!(err.to_string().contains("broken.toml"), "{err}");
}

/// The list union dedupes by id with the seed winning, and keeps a stable
/// order (seed scan first, then overlays by id).
#[test]
fn list_union_dedupes_with_source_winning() {
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), "dup", "From seed");
    seed(dir.path(), "aaa", "Seed A");
    let overlays = vec![
        overlay("zzz", "Overlay Z"),
        overlay("dup", "From overlay"),
        overlay("mmm", "Overlay M"),
    ];
    let files = list_workflows_union(Some(dir.path()), &overlays);
    let ids: Vec<&str> = files.iter().map(|f| f.id.as_str()).collect();
    assert_eq!(ids, vec!["aaa", "dup", "mmm", "zzz"]);
    let dup = files.iter().find(|f| f.id == "dup").unwrap();
    assert_eq!(dup.name, "From seed", "the seed file must win");
}

/// With no source directory, the list is exactly the overlay set.
#[test]
fn list_union_with_no_source_dir_is_the_overlay_set() {
    let overlays = vec![overlay("b", "B"), overlay("a", "A")];
    let files = list_workflows_union(None, &overlays);
    let ids: Vec<&str> = files.iter().map(|f| f.id.as_str()).collect();
    assert_eq!(ids, vec!["a", "b"]);
}

/// One malformed overlay skips only itself — the same tolerance the seed
/// scan has, so a single bad graph never empties the picker.
#[test]
fn list_union_skips_a_malformed_overlay() {
    let overlays = vec![
        overlay("good", "Good"),
        OverlayWorkflow {
            id: "bad".to_string(),
            toml: "id = \"bad\"\nname =".to_string(),
        },
    ];
    let files = list_workflows_union(None, &overlays);
    let ids: Vec<&str> = files.iter().map(|f| f.id.as_str()).collect();
    assert_eq!(ids, vec!["good"]);
}

/// A malformed overlay whose id collides with a global workflow must not
/// let the global slip into the list: `load_workflow_with_globals`
/// resolves the company's (broken) definition first and errors, so a list
/// entry backed by the global instead would be one the loader can never
/// actually return.
#[test]
fn list_with_globals_reserves_a_malformed_overlays_id_against_the_global() {
    let taken = crate::globals::workflows()[0].id.clone();
    let overlays = vec![OverlayWorkflow {
        id: taken.clone(),
        toml: "id = \"broken\"\nname =".to_string(),
    }];

    let listed = list_workflows_with_globals(None, &overlays, &[]);
    assert!(
        listed.iter().all(|f| f.id != taken),
        "the global must not appear in place of the company's malformed definition: {listed:?}"
    );

    let loaded = load_workflow_with_globals(None, &overlays, &[], &taken);
    assert!(
        loaded.is_err(),
        "the loader must surface the malformed overlay's error, not the global"
    );
}

// -----------------------------------------------------------------------
// Console drift guard (issue #260)
// -----------------------------------------------------------------------
//
// The console pre-flights the destination and schedule rules client-side so
// a wrong target is caught without a round trip. That is worth keeping — it
// is the difference between instant feedback and a save that bounces — but
// it makes one rule live in two hand-written places, free to drift. Issue
// #260 reports the drift that already happened: two different messages for
// the same rule.
//
// These tests are the coupling. Each shared fragment is asserted TWICE —
// once against this module's live `validate()` output, so a server rewording
// fails here, and once against the console source, so a console rewording
// fails here too. Neither side can be reworded alone.
//
// This is a tripwire, not a proof. The fragment only has to APPEAR in the
// console source, so a stale copy left in a comment would false-pass, and
// nothing here checks that the console's rule FIRES in the same cases the
// host's does. What it does buy is that the specific failure #260 describes
// — one side reworded, the other silently asserting the old contract — can
// no longer happen quietly. Closing the rest means option 3 from the issue:
// the host exposing the destination contract as data.

/// The console's workflow creator, read at compile time so a file move is a
/// build error naming the path rather than a silently-skipped test.
const CONSOLE_DIALOG: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../frontend/src/views/WorkflowCreateDialog.tsx"
));
const CONSOLE_DIALOG_PATH: &str = "frontend/src/views/WorkflowCreateDialog.tsx";

/// The console's workflow API module, which declares the picker's
/// destination kinds.
const CONSOLE_API: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../frontend/src/api/workflows.ts"
));
const CONSOLE_API_PATH: &str = "frontend/src/api/workflows.ts";

/// How an `email` destination with a non-address target ends, on both sides.
const EMAIL_TARGET_TAIL: &str = "is not an email address — give the recipient's full address.";
/// How a `channel` destination with no target ends, on both sides.
const CHANNEL_TARGET_TAIL: &str = "name the channel to post the report to.";

/// A graph that trips both destination target rules at once.
const BAD_DESTINATIONS: &str = r#"
    id = "wf"
    name = "WF"

    [[node]]
    id = "start"
    kind = "trigger"
    name = "Start"

    [[node]]
    id = "mailer"
    kind = "output"
    name = "Mailer"
    [node.destination]
    kind = "email"
    target = "nope"

    [[node]]
    id = "poster"
    kind = "output"
    name = "Poster"
    [node.destination]
    kind = "channel"
"#;

#[test]
fn destination_messages_match_the_console() {
    let raw: RawWorkflow = toml::from_str(BAD_DESTINATIONS).expect("the fixture is valid TOML");
    // Destination checks are unconditional, so the load-path (`false`) form
    // surfaces them exactly as `parse_workflow` does.
    let problems = validate(&raw, false).join("\n");

    for tail in [EMAIL_TARGET_TAIL, CHANNEL_TARGET_TAIL] {
        assert!(
            problems.contains(tail),
            "the host stopped saying `{tail}` — if that rewording is deliberate, \
             update this const AND the matching message in {CONSOLE_DIALOG_PATH}, \
             so an author who trips the pre-flight and an author who trips the 400 \
             are still told the same thing.\nhost said:\n{problems}"
        );
        assert!(
            CONSOLE_DIALOG.contains(tail),
            "{CONSOLE_DIALOG_PATH} no longer says `{tail}` — the console's \
             client-side pre-flight has drifted from the host's rule (issue #260). \
             Reword both sides together, or drop the pre-flight and surface the \
             host's message on the failed save."
        );
    }
}

/// The picker's destination kinds, extracted from the console's own
/// `DESTINATION_KINDS` block. A kind added on one side alone is either a
/// picker option the host rejects or one the host accepts and the author
/// can never choose.
#[test]
fn destination_kinds_match_the_console() {
    let start = CONSOLE_API.find("export const DESTINATION_KINDS").unwrap_or_else(|| {
        panic!("`DESTINATION_KINDS` is gone from {CONSOLE_API_PATH} — it is what this test reads")
    });
    // Slice from the array opener, NOT from the declaration: the type
    // annotation in between ends `WorkflowDestination["kind"];`, which
    // contains a literal `"];` and would close the block before the first
    // entry.
    let block = &CONSOLE_API[start..];
    let open = block.find("= [").unwrap_or_else(|| {
        panic!("`DESTINATION_KINDS` in {CONSOLE_API_PATH} is no longer an array literal")
    });
    let block = &block[open..];
    let end = block
        .find("];")
        .unwrap_or_else(|| panic!("`DESTINATION_KINDS` in {CONSOLE_API_PATH} has no `];`"));
    let block = &block[..end];

    // Scan for `value: "…"` entries. The type annotation on the same
    // declaration carries a bare `value:` with no string, so keying on the
    // opening quote is what keeps it out.
    let needle = "value: \"";
    let mut console = std::collections::BTreeSet::new();
    let mut rest = block;
    while let Some(at) = rest.find(needle) {
        rest = &rest[at + needle.len()..];
        let close = rest
            .find('"')
            .unwrap_or_else(|| panic!("unterminated `value:` in {CONSOLE_API_PATH}"));
        console.insert(&rest[..close]);
        rest = &rest[close..];
    }

    let host: std::collections::BTreeSet<&str> =
        WORKFLOW_DESTINATION_KINDS.iter().copied().collect();
    assert_eq!(
        console, host,
        "the console's DESTINATION_KINDS picker ({CONSOLE_API_PATH}) and the host's \
         WORKFLOW_DESTINATION_KINDS disagree — one side offers a kind the other \
         does not know (issue #260)"
    );
}

/// The console's `looksLikeCron` pre-flight counts whitespace-separated
/// fields and accepts exactly five, so it is only correct while the host's
/// parser draws the line in the same place. Relaxing the host to accept a
/// 6-field (seconds) expression without touching the console would make the
/// console reject input the host now takes — the drift direction #260 says
/// bites, because the console is the stricter side by construction.
#[test]
fn cron_arity_matches_the_console_preflight() {
    use crate::runtime::cron::CronExpr;
    assert!(CronExpr::parse("0 9 * * MON").is_ok(), "5 fields");
    assert!(CronExpr::parse("0 9 * *").is_err(), "4 fields");
    assert!(CronExpr::parse("0 0 9 * * MON").is_err(), "6 fields");
    assert!(
        CONSOLE_DIALOG.contains("function looksLikeCron"),
        "{CONSOLE_DIALOG_PATH} no longer defines `looksLikeCron` — this test \
         exists to pin the arity that helper assumes"
    );
}

// --- issue #1016: per-kind config gate (transform / split_out / http url /
//     output_parser) --------------------------------------------------------

/// Parses a bare TOML config table for a node.
fn cfg(src: &str) -> toml::Value {
    toml::from_str::<toml::Value>(src).expect("valid config table")
}

fn problems(kind: WorkflowNodeKind, config: Option<&toml::Value>) -> Vec<WorkflowProblem> {
    required_config_problems(kind, "n", "node `n`", config)
}

#[test]
fn transform_without_set_is_rejected_naming_the_field() {
    let out = problems(WorkflowNodeKind::Transform, None);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].node_id.as_deref(), Some("n"));
    assert_eq!(out[0].field.as_deref(), Some("config.set"));
}

#[test]
fn transform_with_empty_set_is_rejected() {
    let config = cfg("[set]\n");
    assert_eq!(
        problems(WorkflowNodeKind::Transform, Some(&config))[0]
            .field
            .as_deref(),
        Some("config.set")
    );
}

#[test]
fn transform_with_non_string_set_value_is_rejected() {
    let config = cfg("[set]\ncount = 3\n");
    let out = problems(WorkflowNodeKind::Transform, Some(&config));
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].field.as_deref(), Some("config.set"));
}

#[test]
fn transform_with_string_expressions_is_accepted() {
    let config = cfg("[set]\nname = \"=item.name\"\n");
    assert!(problems(WorkflowNodeKind::Transform, Some(&config)).is_empty());
}

#[test]
fn split_out_without_path_is_rejected_naming_the_field() {
    let out = problems(WorkflowNodeKind::SplitOut, None);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].field.as_deref(), Some("config.path"));
}

#[test]
fn split_out_with_path_is_accepted() {
    let config = cfg("path = \"items\"\n");
    assert!(problems(WorkflowNodeKind::SplitOut, Some(&config)).is_empty());
}

#[test]
fn http_request_url_must_be_a_real_url() {
    let good = cfg("method = \"GET\"\nurl = \"https://example.com/x\"\n");
    assert!(problems(WorkflowNodeKind::HttpRequest, Some(&good)).is_empty());

    // "" is rejected as before (regression guard) …
    let empty = cfg("method = \"GET\"\nurl = \"\"\n");
    assert_eq!(
        problems(WorkflowNodeKind::HttpRequest, Some(&empty))[0]
            .field
            .as_deref(),
        Some("config.url")
    );

    // … and now `ftp://x` and bare `garbage` are rejected too (new).
    for bad in ["ftp://x", "garbage", "https://"] {
        let config = cfg(&format!("method = \"GET\"\nurl = \"{bad}\"\n"));
        let out = problems(WorkflowNodeKind::HttpRequest, Some(&config));
        assert_eq!(out.len(), 1, "{bad}: {out:?}");
        assert_eq!(out[0].field.as_deref(), Some("config.url"), "{bad}");
    }
}

#[test]
fn output_parser_is_schema_less_by_default() {
    // A bare identity parser (no config at all) is accepted.
    assert!(problems(WorkflowNodeKind::OutputParser, None).is_empty());
    // A present but mistyped key is rejected, each naming its field.
    let config = cfg("auto_fix = \"yes\"\nconnection_ref = 5\n");
    let out = problems(WorkflowNodeKind::OutputParser, Some(&config));
    let fields: Vec<&str> = out.iter().filter_map(|p| p.field.as_deref()).collect();
    assert!(fields.contains(&"config.auto_fix"), "{fields:?}");
    assert!(fields.contains(&"config.connection_ref"), "{fields:?}");
}

#[test]
fn output_parser_with_well_typed_keys_is_accepted() {
    let config =
        cfg("auto_fix = true\nconnection_ref = \"conn\"\n[schema]\nname = \"string\"\n");
    assert!(problems(WorkflowNodeKind::OutputParser, Some(&config)).is_empty());
}

#[test]
fn merge_stays_config_free() {
    assert!(problems(WorkflowNodeKind::Merge, None).is_empty());
    let config = cfg("anything = \"goes\"\n");
    assert!(problems(WorkflowNodeKind::Merge, Some(&config)).is_empty());
}
