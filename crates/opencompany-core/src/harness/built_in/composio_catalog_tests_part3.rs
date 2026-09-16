use super::*;

/// The exact shape the live GitHub call returns: records two levels down,
/// under Composio's own `data` envelope.
#[test]
fn projects_records_nested_under_the_composio_envelope() {
    let body = serde_json::json!({
        "data": { "details": [
            { "number": 1, "title": "a", "url": "https://x", "html_url": "https://y",
              "user": { "login": "octocat", "avatar_url": "https://z", "id": 5 },
              "labels": [ { "name": "bug", "url": "https://l" } ] },
            { "number": 2, "title": "b", "url": "https://x2", "html_url": "https://y2",
              "user": { "login": "hubot", "avatar_url": "https://z2", "id": 6 },
              "labels": [ { "name": "p2", "url": "https://l2" } ] }
        ]}
    })
    .to_string();

    let projected = project_records(&body).expect("records nested under `data.details`");
    assert!(
        projected.len() < body.len(),
        "must shrink: {} -> {}",
        body.len(),
        projected.len()
    );
    // These used to assert that `avatar_url` and `html_url` were dropped.
    // They are answers a caller can ask for, and this projection runs ahead
    // of the task-aware extractor and the artifact store, so dropping them
    // was unrecoverable (codex and CodeRabbit on
    // tinyhumansai/opencompany#2153). What goes now is API plumbing only.
    assert!(
        projected.contains("html_url"),
        "a browser link can be the answer and must survive: {projected}"
    );
    assert!(
        !projected.contains("comments_url"),
        "self-referential collection endpoints still go: {projected}"
    );
    assert!(
        projected.contains("octocat"),
        "nested user collapses to its login: {projected}"
    );
    assert!(
        projected.contains("bug"),
        "label array collapses to names: {projected}"
    );
    assert!(
        projected.contains("\"title\""),
        "answering fields survive: {projected}"
    );
}
