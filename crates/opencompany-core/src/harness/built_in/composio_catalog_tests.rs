use super::*;

/// A catalogue of `count` synthetic actions for `toolkit`, each with a
/// realistically chunky description and parameter schema.
fn catalogue(toolkit: &str, count: usize) -> Vec<CatalogAction> {
    (0..count)
        .map(|i| CatalogAction {
            slug: format!("{}_ACTION_{i:03}", toolkit.to_ascii_uppercase()),
            toolkit: toolkit.to_string(),
            description: format!(
                "Performs operation {i} on the {toolkit} account. {}",
                "Long upstream prose that Composio publishes for every action. ".repeat(4)
            ),
            parameters: Some(json!({
                "type": "object",
                "properties": {
                    "owner": {"type": "string", "description": "x".repeat(200)},
                    "repo": {"type": "string", "description": "y".repeat(200)},
                },
                "required": ["owner"]
            })),
        })
        .collect()
}

fn request(search: &str, detail: Detail, toolkits: &[&str]) -> ListRequest {
    ListRequest {
        toolkits: toolkits.iter().map(|t| t.to_string()).collect(),
        search: search_terms(search),
        tags: Vec::new(),
        curated: false,
        detail,
        limit: detail.default_limit(),
    }
}


#[path = "composio_catalog_tests_part1.rs"]
mod tests_part1;
#[path = "composio_catalog_tests_part2.rs"]
mod tests_part2;
