use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::Value;
use tinyflows::graph::{
    Checkpoint, CheckpointConfig, CheckpointMetadata, CheckpointTuple, Checkpointer,
    FileCheckpointer, PendingWrite,
};

/// Durable workflow checkpoints scoped to one company bundle.
#[derive(Clone)]
pub struct WorkflowCheckpointStore {
    inner: FileCheckpointer<Value>,
}

impl WorkflowCheckpointStore {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            inner: FileCheckpointer::new(base_dir),
        }
    }

    pub fn base_dir(&self) -> &Path {
        self.inner.base_dir()
    }

    pub async fn has_resume_point(&self, thread_id: &str) -> tinyflows::graph::Result<bool> {
        Ok(self
            .get_scoped(thread_id, None, &[])
            .await?
            .is_some_and(|checkpoint| {
                !checkpoint.next_nodes.is_empty() || !checkpoint.interrupts.is_empty()
            }))
    }

    pub async fn prune_settled(&self, thread_id: &str) -> tinyflows::graph::Result<()> {
        self.delete_thread(thread_id).await
    }
}

#[async_trait]
impl Checkpointer<Value> for WorkflowCheckpointStore {
    async fn put(
        &self,
        checkpoint: Checkpoint<Value>,
    ) -> tinyflows::graph::Result<tinyflows::graph::ids::CheckpointId> {
        self.inner.put(checkpoint).await
    }

    async fn get(
        &self,
        thread_id: &str,
        checkpoint_id: Option<&str>,
    ) -> tinyflows::graph::Result<Option<Checkpoint<Value>>> {
        self.inner.get(thread_id, checkpoint_id).await
    }

    async fn get_scoped(
        &self,
        thread_id: &str,
        checkpoint_id: Option<&str>,
        namespace: &[String],
    ) -> tinyflows::graph::Result<Option<Checkpoint<Value>>> {
        Ok(self
            .inner
            .get_thread(thread_id)
            .await?
            .into_iter()
            .rev()
            .find(|checkpoint| {
                checkpoint.namespace == namespace
                    && checkpoint_id.is_none_or(|id| checkpoint.checkpoint_id.as_str() == id)
            }))
    }

    async fn list(&self, thread_id: &str) -> tinyflows::graph::Result<Vec<CheckpointMetadata>> {
        self.inner.list(thread_id).await
    }

    async fn put_writes(
        &self,
        config: &CheckpointConfig,
        writes: &[PendingWrite],
    ) -> tinyflows::graph::Result<()> {
        self.inner.put_writes(config, writes).await
    }

    async fn get_writes(
        &self,
        config: &CheckpointConfig,
    ) -> tinyflows::graph::Result<Vec<PendingWrite>> {
        self.inner.get_writes(config).await
    }

    async fn get_thread(
        &self,
        thread_id: &str,
    ) -> tinyflows::graph::Result<Vec<Checkpoint<Value>>> {
        self.inner.get_thread(thread_id).await
    }

    async fn state_history(
        &self,
        thread_id: &str,
        namespace: &[String],
        limit: Option<usize>,
    ) -> tinyflows::graph::Result<Vec<CheckpointTuple<Value>>> {
        self.inner.state_history(thread_id, namespace, limit).await
    }

    async fn list_threads(&self) -> tinyflows::graph::Result<Vec<String>> {
        self.inner.list_threads().await
    }

    async fn delete_thread(&self, thread_id: &str) -> tinyflows::graph::Result<()> {
        self.inner.delete_thread(thread_id).await
    }

    async fn delete_checkpoints(
        &self,
        thread_id: &str,
        ids: &[String],
    ) -> tinyflows::graph::Result<usize> {
        self.inner.delete_checkpoints(thread_id, ids).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use serde_json::json;
    use tinyflows::caps::mock::mock_capabilities;
    use tinyflows::graph::ids::NodeId;
    use tinyflows::graph::{Checkpoint, Checkpointer};
    use tinyflows::model::{Edge, Node, NodeKind, WorkflowGraph};
    use tinyflows::observability::RunObserver;

    use super::*;

    fn checkpoint(id: &str, namespace: &[&str], value: u64) -> Checkpoint<Value> {
        Checkpoint {
            thread_id: "lineage".to_string(),
            checkpoint_id: id.to_string(),
            run_id: Some("attempt".to_string()),
            parent_checkpoint_id: None,
            namespace: namespace.iter().map(|part| (*part).to_string()).collect(),
            state: json!({ "value": value }),
            next_nodes: vec![NodeId::new("next")],
            completed_tasks: Vec::new(),
            pending_writes: Vec::new(),
            interrupts: Vec::new(),
            pending_activations: None,
            barrier_arrivals: Vec::new(),
            metadata: Value::Null,
        }
    }

    #[tokio::test]
    async fn survives_reopen_and_round_trips_scoped_namespaces() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = WorkflowCheckpointStore::new(temp.path());
        store
            .put(checkpoint("shared", &[], 1))
            .await
            .expect("put root");
        store
            .put(checkpoint("shared", &["loop", "child"], 2))
            .await
            .expect("put child");

        let reopened = WorkflowCheckpointStore::new(temp.path());
        let root = reopened
            .get_scoped("lineage", Some("shared"), &[])
            .await
            .expect("get root")
            .expect("root checkpoint");
        let child_namespace = vec!["loop".to_string(), "child".to_string()];
        let child = reopened
            .get_scoped("lineage", Some("shared"), &child_namespace)
            .await
            .expect("get child")
            .expect("child checkpoint");

        assert_eq!(root.state, json!({ "value": 1 }));
        assert_eq!(child.state, json!({ "value": 2 }));
        assert!(reopened.has_resume_point("lineage").await.unwrap());
    }

    #[tokio::test]
    async fn clean_settle_prunes_the_whole_lineage() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = WorkflowCheckpointStore::new(temp.path());
        store.put(checkpoint("one", &[], 1)).await.expect("put");

        store.prune_settled("lineage").await.expect("prune");

        assert!(store.list("lineage").await.unwrap().is_empty());
        assert!(!store.has_resume_point("lineage").await.unwrap());
    }

    #[derive(Default)]
    struct FinishedNodes(Mutex<Vec<String>>);

    impl RunObserver for FinishedNodes {
        fn on_step_finish(&self, step: &tinyflows::observability::ExecutionStep) {
            self.0.lock().unwrap().push(step.node_id.clone());
        }
    }

    fn parser_node(id: &str) -> Node {
        Node {
            id: id.to_string(),
            kind: NodeKind::OutputParser,
            type_version: 1,
            name: id.to_string(),
            config: Value::Null,
            ports: Vec::new(),
            position: None,
        }
    }

    #[tokio::test]
    async fn reopens_at_node_seven_without_replaying_completed_nodes() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = WorkflowCheckpointStore::new(temp.path());
        let mut nodes = vec![Node {
            id: "trigger".to_string(),
            kind: NodeKind::Trigger,
            type_version: 1,
            name: "trigger".to_string(),
            config: Value::Null,
            ports: Vec::new(),
            position: None,
        }];
        nodes.extend((1..=9).map(|index| {
            let mut node = parser_node(&format!("node-{index}"));
            if index == 7 {
                node.config = json!({ "requires_approval": true });
            }
            node
        }));
        let mut edges = vec![Edge {
            from_node: "trigger".to_string(),
            from_port: "main".to_string(),
            to_node: "node-1".to_string(),
            to_port: "main".to_string(),
        }];
        edges.extend((1..9).map(|index| Edge {
            from_node: format!("node-{index}"),
            from_port: "main".to_string(),
            to_node: format!("node-{}", index + 1),
            to_port: "main".to_string(),
        }));
        let compiled = tinyflows::compiler::compile(&WorkflowGraph {
            nodes,
            edges,
            ..Default::default()
        })
        .expect("compile");
        let checkpointer: Arc<dyn Checkpointer<Value>> = Arc::new(store);
        let paused = tinyflows::engine::run_with_checkpointer(
            &compiled,
            json!({ "request": "restart" }),
            &mock_capabilities(),
            checkpointer,
            "node-seven-lineage",
        )
        .await
        .expect("pause at node seven");
        assert_eq!(paused.pending_approvals, vec!["node-7".to_string()]);

        let reopened: Arc<dyn Checkpointer<Value>> =
            Arc::new(WorkflowCheckpointStore::new(temp.path()));
        let observer = Arc::new(FinishedNodes::default());
        let dyn_observer: Arc<dyn RunObserver> = observer.clone();
        tinyflows::engine::resume_with_checkpointer_journaled_observed(
            &compiled,
            &mock_capabilities(),
            reopened,
            "node-seven-lineage",
            vec!["node-7".to_string()],
            Vec::new(),
            Arc::new(tinyflows::engine::InMemoryGraphEventJournal::new()),
            &dyn_observer,
        )
        .await
        .expect("resume from durable node boundary");

        assert_eq!(*observer.0.lock().unwrap(), ["node-7", "node-8", "node-9"]);
    }
}
