#[tokio::test]
async fn probe_child_id_of() {
    use crate::workflows::caps::resolver::{child_id_of, ChildGateRegistry};
    use crate::company::parse_workflow;
    use crate::workflows::translate::translate;
    let parent = r#"
id = "parent"
name = "Parent"
[[node]]
id = "start"
kind = "trigger"
name = "Start"
[[node]]
id = "sub"
kind = "sub_workflow"
name = "Sub"
[node.config]
workflow_id = "=item.target"
[[edge]]
from = "start"
to = "sub"
"#;
    let file = parse_workflow(parent).expect("parent parses");
    let g = translate(&file);
    for n in &g.nodes {
        eprintln!("node {} kind {:?} config {:?}", n.id, n.kind, n.config);
    }
    let resolved = child_id_of(&g, "sub", Some(&serde_json::json!({"target": "child"})));
    eprintln!("child_id_of -> {:?}", resolved);
    let registry = ChildGateRegistry::default();
    let _ = registry;
    panic!("probe");
}
