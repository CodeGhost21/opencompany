//! What the agent's five tools promise, and what they refuse.

use std::sync::Arc;

use serde_json::json;

use super::*;
use crate::ledger::Registry;
use crate::ports::types::CompanyId;
use crate::store::FsOps;

fn ctx(home: &tempfile::TempDir) -> Ledgers {
    let ops = Arc::new(FsOps::new(home.path().to_path_buf()));
    Ledgers::new(CompanyId::new("acme"), ops)
}

fn tools(ctx: &Ledgers) -> Vec<Box<dyn Tool>> {
    ledger_tools(ctx.clone(), "ceo".to_string())
}

fn tool<'a>(tools: &'a [Box<dyn Tool>], name: &str) -> &'a dyn Tool {
    tools
        .iter()
        .find(|tool| tool.name() == name)
        .map(AsRef::as_ref)
        .unwrap_or_else(|| panic!("`{name}` is registered"))
}

fn risks() -> serde_json::Value {
    json!({
        "slug": "risks",
        "title": "Risks",
        "purpose": "What could go wrong.",
        "fields": [
            { "name": "id", "role": "id" },
            { "name": "risk", "role": "title" },
            { "name": "status", "role": "status" },
            { "name": "reason", "role": "prose" }
        ],
        "statuses": [
            { "name": "open" },
            { "name": "closed", "closed": true, "needs_reason": true }
        ]
    })
}

/// Five tools, whatever a company declares. The count must not grow per tenant:
/// the schema is built once, and a surface whose shape changes per company is
/// one no prompt can describe.
#[tokio::test]
async fn the_surface_is_five_tools_and_stays_five() {
    let home = tempfile::tempdir().unwrap();
    let ctx = ctx(&home);
    assert_eq!(tools(&ctx).len(), 5);
    ledgers::define(&ctx, &risks()).await.expect("declared");
    let built = tools(&ctx);
    let names: Vec<&str> = built.iter().map(|tool| tool.name()).collect();
    assert_eq!(names, LEDGER_TOOL_NAMES);
}

/// The one thing an agent may never do. Its absence is the enforcement — a tool
/// that is not registered cannot be reached by a model that guesses well.
#[tokio::test]
async fn there_is_no_delete_tool_and_no_retire_tool() {
    let home = tempfile::tempdir().unwrap();
    let built = tools(&ctx(&home));
    let names: Vec<&str> = built.iter().map(|tool| tool.name()).collect();
    for forbidden in [
        "delete_entry",
        "retire_ledger",
        "delete_ledger",
        "purge_ledger",
    ] {
        assert!(
            !names.contains(&forbidden),
            "`{forbidden}` must not be reachable from a turn"
        );
    }
}

#[tokio::test]
async fn listing_names_every_ledger_with_its_statuses() {
    let home = tempfile::tempdir().unwrap();
    let ctx = ctx(&home);
    let tools = tools(&ctx);
    let result = tool(&tools, LIST_LEDGERS_TOOL)
        .execute(json!({}))
        .await
        .unwrap();
    let text = format!("{result:?}");
    for slug in ["tasks", "goals", "decisions"] {
        assert!(text.contains(slug), "`{slug}` missing: {text}");
    }
    assert!(text.contains("in_progress"), "statuses are listed: {text}");
}

/// The discovery path a model actually follows: guess, and learn the real names
/// from the failure, in one turn, without having thought to list them first.
#[tokio::test]
async fn an_unknown_slug_answers_with_the_real_ones() {
    let home = tempfile::tempdir().unwrap();
    let ctx = ctx(&home);
    let tools = tools(&ctx);
    let result = tool(&tools, READ_LEDGER_TOOL)
        .execute(json!({ "ledger": "objectives" }))
        .await
        .unwrap();
    let text = format!("{result:?}");
    assert!(text.contains("objectives"), "{text}");
    assert!(text.contains("goals"), "{text}");
}

#[tokio::test]
async fn an_agent_declares_records_and_closes() {
    let home = tempfile::tempdir().unwrap();
    let ctx = ctx(&home);
    let tools = tools(&ctx);

    tool(&tools, DEFINE_LEDGER_TOOL)
        .execute(risks())
        .await
        .unwrap();

    tool(&tools, RECORD_ENTRY_TOOL)
        .execute(json!({
            "ledger": "risks",
            "id": "vendor-slip",
            "fields": { "risk": "the vendor misses the date", "status": "open" }
        }))
        .await
        .unwrap();

    let read = tool(&tools, READ_LEDGER_TOOL)
        .execute(json!({ "ledger": "risks" }))
        .await
        .unwrap();
    assert!(format!("{read:?}").contains("vendor-slip"));

    tool(&tools, CLOSE_ENTRY_TOOL)
        .execute(json!({
            "ledger": "risks",
            "id": "vendor-slip",
            "status": "closed",
            "reason": "they delivered on the 4th"
        }))
        .await
        .unwrap();

    // Closed, and still there with its reason. A closed row is an archive
    // entry, not a deletion.
    let read = tool(&tools, READ_LEDGER_TOOL)
        .execute(json!({ "ledger": "risks", "status": "closed" }))
        .await
        .unwrap();
    let text = format!("{read:?}");
    assert!(text.contains("vendor-slip"), "{text}");
    assert!(text.contains("delivered on the 4th"), "{text}");
}

/// The refusal has to say what is missing, or the turn spends itself guessing.
#[tokio::test]
async fn closing_without_a_reason_says_what_is_missing() {
    let home = tempfile::tempdir().unwrap();
    let ctx = ctx(&home);
    let tools = tools(&ctx);
    tool(&tools, DEFINE_LEDGER_TOOL)
        .execute(risks())
        .await
        .unwrap();
    let result = tool(&tools, RECORD_ENTRY_TOOL)
        .execute(json!({
            "ledger": "risks",
            "id": "r1",
            "fields": { "status": "closed" }
        }))
        .await
        .unwrap();
    assert!(format!("{result:?}").contains("reason"));
}

/// A JSON null clears a field — the one thing a merge cannot otherwise express.
#[tokio::test]
async fn a_null_field_clears_it() {
    let home = tempfile::tempdir().unwrap();
    let ctx = ctx(&home);
    let tools = tools(&ctx);
    tool(&tools, DEFINE_LEDGER_TOOL)
        .execute(risks())
        .await
        .unwrap();
    tool(&tools, RECORD_ENTRY_TOOL)
        .execute(json!({ "ledger": "risks", "id": "r1", "fields": { "risk": "a", "reason": "b" } }))
        .await
        .unwrap();
    tool(&tools, RECORD_ENTRY_TOOL)
        .execute(json!({ "ledger": "risks", "id": "r1", "fields": { "reason": null } }))
        .await
        .unwrap();
    let read = tool(&tools, READ_LEDGER_TOOL)
        .execute(json!({ "ledger": "risks", "entry": "r1" }))
        .await
        .unwrap();
    let text = format!("{read:?}");
    assert!(text.contains("risk: a"), "{text}");
    assert!(!text.contains("reason: b"), "{text}");
}

/// The board is readable through the surface and refuses a write, naming what
/// does write it — a refusal that sent the caller to `record_entry` would refuse
/// them a second time.
#[tokio::test]
async fn the_board_reads_here_and_refuses_a_write_with_a_usable_remedy() {
    let home = tempfile::tempdir().unwrap();
    let ctx = ctx(&home);
    let tools = tools(&ctx);
    tool(&tools, READ_LEDGER_TOOL)
        .execute(json!({ "ledger": "tasks" }))
        .await
        .unwrap();
    let result = tool(&tools, RECORD_ENTRY_TOOL)
        .execute(json!({ "ledger": "tasks", "id": "t1", "fields": { "title": "x" } }))
        .await
        .unwrap();
    let text = format!("{result:?}");
    assert!(text.contains("spawn_task"), "{text}");
    assert!(!text.contains("Recorded"), "{text}");
}

/// A short list that reads as complete is worse than a long one: the reader
/// concludes there is nothing more and re-proposes what was cut.
#[tokio::test]
async fn a_bounded_read_says_it_is_bounded() {
    let home = tempfile::tempdir().unwrap();
    let ctx = ctx(&home);
    let tools = tools(&ctx);
    tool(&tools, DEFINE_LEDGER_TOOL)
        .execute(risks())
        .await
        .unwrap();
    for n in 0..40 {
        tool(&tools, RECORD_ENTRY_TOOL)
            .execute(json!({
                "ledger": "risks",
                "id": format!("r{n}"),
                "fields": { "risk": "a", "status": "open" }
            }))
            .await
            .unwrap();
    }
    let read = tool(&tools, READ_LEDGER_TOOL)
        .execute(json!({ "ledger": "risks", "limit": 5 }))
        .await
        .unwrap();
    let text = format!("{read:?}");
    assert!(text.contains("5 of 40 shown"), "{text}");
    assert!(text.contains("not gone"), "{text}");
}

/// Every write is a consequence the policy gate can park; the two reads are
/// free. A read behind an approval would make an agent guess rather than look.
#[test]
fn reads_are_free_and_writes_are_consequences() {
    let home = tempfile::tempdir().unwrap();
    let tools = tools(&ctx(&home));
    for (name, level) in [
        (LIST_LEDGERS_TOOL, PermissionLevel::ReadOnly),
        (READ_LEDGER_TOOL, PermissionLevel::ReadOnly),
        (RECORD_ENTRY_TOOL, PermissionLevel::Write),
        (CLOSE_ENTRY_TOOL, PermissionLevel::Write),
        (DEFINE_LEDGER_TOOL, PermissionLevel::Write),
    ] {
        assert_eq!(
            tool(&tools, name).permission_level(),
            level,
            "`{name}` is at the wrong permission level"
        );
    }
}

/// The catalogue, not a pointer to one: a tool granted, unmentioned and never
/// called is the observed failure mode.
#[test]
fn the_brief_names_every_ledger_and_says_deletion_is_not_available() {
    let brief = ledger_brief(&Registry::build([]));
    for slug in ["tasks", "goals", "decisions"] {
        assert!(brief.contains(slug), "`{slug}` missing: {brief}");
    }
    assert!(brief.contains("read_ledger"), "{brief}");
    assert!(brief.contains("record_entry"), "{brief}");
    assert!(brief.contains("cannot delete"), "{brief}");
    // The board's line has to say it is not written here.
    assert!(brief.contains("read-only here"), "{brief}");
}

/// Every tool's schema must parse as a schema and require what it needs, or a
/// model silently sends a call the host cannot answer.
#[test]
fn every_schema_requires_the_ledger_argument() {
    let home = tempfile::tempdir().unwrap();
    let tools = tools(&ctx(&home));
    for name in [READ_LEDGER_TOOL, RECORD_ENTRY_TOOL, CLOSE_ENTRY_TOOL] {
        let schema = tool(&tools, name).parameters_schema();
        let required: Vec<&str> = schema["required"]
            .as_array()
            .expect("required")
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect();
        assert!(required.contains(&"ledger"), "`{name}`: {schema}");
    }
}
