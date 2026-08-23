#[tokio::test]
async fn probe_child_gate_call() {
    use crate::workflows::caps::resolver::{ChildGateRecord, child_gate_call, descend};
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
    // Build a gated record like apply_policy_gates would.
    let mut child = translate(&parse_workflow(&child_with_shell_toml()).expect("child parses"));
    let _ = &mut child;
    let registry = crate::workflows::caps::resolver::ChildGateRegistry::default();
    // A fake gated list naming the child's own node id `work`.
    use crate::workflows::gate::GatedCall;
    let call = GatedCall {
        node_id: "work".to_string(),
        slug: "shell".to_string(),
        reason: "policy".to_string(),
        args: serde_json::json!({}),
        target: None,
    };
    registry.record("child", ChildGateRecord {
        graph: child.clone(),
        gated: vec![call],
    });
    let input = serde_json::json!({ "target": "child" });
    let d = descend(&registry, &g, "sub::work", Some(&input));
    eprintln!("descend -> {:?}", d.as_ref().map(|(_, gate)| gate.clone()));
    let c = child_gate_call(&registry, &g, "sub::work", Some(&input));
    eprintln!("child_gate_call -> {:?}", c.as_ref().map(|c| (c.node_id.clone(), c.slug.clone())));
    panic!("probe done");
}

fn child_with_shell_toml() -> String {
    r#"
id = "child"
name = "child"
[[node]]
id = "start"
kind = "trigger"
name = "Start"
[[node]]
id = "work"
kind = "tool_call"
name = "Work"
[node.config]
slug = "shell"
[node.config.args]
command = "echo hi"
[[edge]]
from = "start"
to = "work"
"#.to_string()
}
