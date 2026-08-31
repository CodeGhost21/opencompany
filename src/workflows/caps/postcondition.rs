//! The deterministic tier of issue #1866's sufficiency gate.
//!
//! The engine advances the moment a node returns `Ok`; nothing checks whether
//! the output is actually enough to hand downstream. The extreme case is
//! already fixed independently for the iteration-cap signal (#1865): a node
//! that stops at `max_tool_iterations` settles `Failed` rather than flowing a
//! truncated reply on as if it were a finished answer. This module is the
//! general form of that same idea, expressed as an author-declared,
//! **mechanical** check rather than a signal the engine happens to expose —
//! "the output has this shape, or it isn't good enough to advance."
//!
//! Deliberately narrow: three predicates, no LLM call, no network, no state.
//! The semantic judge tier the issue also describes (a tool-less model call
//! for nodes whose sufficiency cannot be expressed as a predicate) is Wave 3,
//! gated on #1861's blocker-park plumbing landing first — this module only
//! ever returns `Ok` or a plain-English gap sentence.
//!
//! # Fail-open on the unknown case
//!
//! [`evaluate_postcondition`] is validated at author time
//! ([`crate::company::workflow_file::validate`] rejects an unknown `require`
//! before a graph is ever saved), so an unrecognized `require` reaching this
//! function at runtime can only mean a graph saved by an older or newer
//! version of the validator disagreeing with this binary. Observability must
//! never be able to fail the work it is observing (the same rule
//! [`super::HarnessAgentRunner::run_turn`] already applies to a failed
//! attempt-row mint) — so the unknown case warns and lets the node through
//! rather than halting a run over a predicate this binary cannot evaluate.

use serde_json::Value;

/// Evaluates a node's declared `postcondition` against its output envelope.
///
/// `spec` is the raw `postcondition` config node ({ "require": ..., "field":
/// ... (optional) }); `output` is the node's output value — for an agent node
/// today, the `{ "text", "agent_ref" }` envelope [`super::HarnessAgentRunner::run_turn`]
/// builds. `Ok(())` means the output clears the gate; `Err(gap)` carries a
/// plain-English sentence naming what is missing, suitable to surface as the
/// halting attempt's error message.
///
/// Three predicates:
/// - `non_empty` — the envelope's `text` is present and non-empty after
///   trimming whitespace. Catches the truncation class this issue opens
///   with: a capped or refused turn that still produced *some* prose.
/// - `field_present` — the dotted `field` path resolves to a present,
///   non-null value in `output`.
/// - `non_empty_list` — the target (the whole `output`, or the dotted
///   `field` path within it when given) is a JSON array with at least one
///   element.
///
/// Any other `require` value fails OPEN: a `tracing::warn!` is emitted and
/// the node is allowed to proceed. See the module doc for why.
pub(crate) fn evaluate_postcondition(spec: &Value, output: &Value) -> Result<(), String> {
    let require = spec.get("require").and_then(Value::as_str).unwrap_or("");
    let field = spec
        .get("field")
        .and_then(Value::as_str)
        .filter(|f| !f.is_empty());

    match require {
        "non_empty" => {
            let text = output.get("text").and_then(Value::as_str).unwrap_or("");
            if text.trim().is_empty() {
                Err("the node's output was empty — nothing was produced to advance on.".to_string())
            } else {
                Ok(())
            }
        }
        "field_present" => {
            let Some(path) = field else {
                // Author-time validation requires `field` on `field_present` —
                // see `WorkflowPostconditionDef` validation. A spec reaching here
                // without one is the same "binary disagrees with the validator
                // that saved this graph" case the module doc describes.
                tracing::warn!(
                    require,
                    "workflow postcondition: `field_present` declared with no `field` — \
                     passing the node through unevaluated"
                );
                return Ok(());
            };
            match resolve_path(output, path) {
                Some(value) if !value.is_null() => Ok(()),
                _ => Err(format!(
                    "the node's output is missing `{path}` — the expected field never landed."
                )),
            }
        }
        "non_empty_list" => {
            let target = match field {
                Some(path) => resolve_path(output, path),
                // No `field` given: for the standard `{ json, text, raw }`
                // envelope, "the output" means the structured payload under
                // `json`, not the envelope wrapper itself — the wrapper is
                // always an object (it also carries `text`/`agent_ref`), so
                // checking it directly could never see a `Value::Array` even
                // when the underlying result genuinely is a list. Falls back
                // to the raw value for any caller not using that envelope
                // shape (no `json` key at all), which is the pre-existing
                // behavior.
                None => output.get("json").or(Some(output)),
            };
            let described = field
                .map(|path| format!("`{path}`"))
                .unwrap_or_else(|| "the output".to_string());
            match target {
                Some(Value::Array(items)) if !items.is_empty() => Ok(()),
                Some(Value::Array(_)) => Err(format!(
                    "{described} is an empty list — nothing came back to advance on."
                )),
                Some(_) => Err(format!(
                    "{described} is not a list — the shape does not match."
                )),
                None => Err(format!(
                    "{described} is missing — nothing came back to advance on."
                )),
            }
        }
        other => {
            tracing::warn!(
                require = other,
                "workflow postcondition: unknown `require` — passing the node through unevaluated"
            );
            Ok(())
        }
    }
}

/// Resolves a dot-separated path (`"a.b.c"`) through nested JSON objects.
/// Does not index into arrays — every hop is an object-field lookup, which is
/// all the two field-aware predicates above need.
fn resolve_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(value, |acc, key| acc.get(key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec(require: &str) -> Value {
        json!({ "require": require })
    }

    fn spec_with_field(require: &str, field: &str) -> Value {
        json!({ "require": require, "field": field })
    }

    #[test]
    fn non_empty_passes_on_real_text() {
        let output = json!({ "text": "the report is done", "agent_ref": "a" });
        assert_eq!(evaluate_postcondition(&spec("non_empty"), &output), Ok(()));
    }

    #[test]
    fn non_empty_fails_on_blank_text() {
        let output = json!({ "text": "   ", "agent_ref": "a" });
        assert!(evaluate_postcondition(&spec("non_empty"), &output).is_err());
    }

    #[test]
    fn non_empty_fails_on_missing_text() {
        let output = json!({ "agent_ref": "a" });
        assert!(evaluate_postcondition(&spec("non_empty"), &output).is_err());
    }

    #[test]
    fn field_present_passes_when_the_field_resolves() {
        let output = json!({ "items": [1, 2] });
        assert_eq!(
            evaluate_postcondition(&spec_with_field("field_present", "items"), &output),
            Ok(())
        );
    }

    #[test]
    fn field_present_fails_when_the_field_is_absent() {
        let output = json!({ "text": "prose only" });
        assert!(
            evaluate_postcondition(&spec_with_field("field_present", "items"), &output).is_err()
        );
    }

    #[test]
    fn field_present_fails_when_the_field_is_explicitly_null() {
        let output = json!({ "items": null });
        assert!(
            evaluate_postcondition(&spec_with_field("field_present", "items"), &output).is_err()
        );
    }

    #[test]
    fn field_present_resolves_a_dotted_path() {
        let output = json!({ "json": { "result": { "count": 3 } } });
        assert_eq!(
            evaluate_postcondition(
                &spec_with_field("field_present", "json.result.count"),
                &output
            ),
            Ok(())
        );
    }

    #[test]
    fn field_present_dotted_path_fails_partway_through() {
        let output = json!({ "json": { "result": {} } });
        assert!(
            evaluate_postcondition(
                &spec_with_field("field_present", "json.result.count"),
                &output
            )
            .is_err()
        );
    }

    #[test]
    fn non_empty_list_passes_on_a_populated_array() {
        let output = json!(["a"]);
        assert_eq!(
            evaluate_postcondition(&spec("non_empty_list"), &output),
            Ok(())
        );
    }

    #[test]
    fn non_empty_list_fails_on_an_empty_array() {
        let output = json!([]);
        assert!(evaluate_postcondition(&spec("non_empty_list"), &output).is_err());
    }

    #[test]
    fn non_empty_list_fails_on_a_non_array() {
        let output = json!({ "text": "not a list" });
        assert!(evaluate_postcondition(&spec("non_empty_list"), &output).is_err());
    }

    /// Codex review on #1937 (issue #1866): the no-`field` form must look at
    /// the standard envelope's structured `json` payload, not the envelope
    /// object itself — an agent-node envelope always carries `text`/
    /// `agent_ref` alongside `json`, so checking the envelope directly could
    /// never see a `Value::Array` even when the agent's parsed reply
    /// genuinely is a non-empty list.
    #[test]
    fn non_empty_list_with_no_field_checks_the_envelopes_json_payload() {
        let output = json!({ "text": "[\"a\",\"b\"]", "agent_ref": "a", "json": ["a", "b"] });
        assert_eq!(
            evaluate_postcondition(&spec("non_empty_list"), &output),
            Ok(())
        );
    }

    /// Companion RED-shape: when the envelope's `json` payload didn't parse
    /// (a plain-prose reply), the no-`field` form still fails honestly —
    /// it must not silently pass just because a `json` key exists.
    #[test]
    fn non_empty_list_with_no_field_fails_when_the_envelopes_json_is_null() {
        let output = json!({ "text": "just prose, no list here", "agent_ref": "a", "json": null });
        assert!(evaluate_postcondition(&spec("non_empty_list"), &output).is_err());
    }

    #[test]
    fn non_empty_list_checks_the_named_field_when_given() {
        let output = json!({ "items": ["a", "b"] });
        assert_eq!(
            evaluate_postcondition(&spec_with_field("non_empty_list", "items"), &output),
            Ok(())
        );

        let empty = json!({ "items": [] });
        assert!(
            evaluate_postcondition(&spec_with_field("non_empty_list", "items"), &empty).is_err()
        );
    }

    #[test]
    fn unknown_require_fails_open() {
        let output = json!({});
        assert_eq!(
            evaluate_postcondition(&spec("some_future_predicate"), &output),
            Ok(())
        );
    }
}
