use super::*;
use std::sync::Mutex as StdMutex;

use crate::company::{CompanyManifest, RawEdge, RawNode, load_workflow_union};
use crate::ports::types::{
    CompanyRecord, CompanySummary, EventSeq, LedgerEntry, OverlayDesk, ResponderMode,
    StoredEvent,
};
use async_trait::async_trait;
use futures::stream::{self, BoxStream};

// --- test doubles --------------------------------------------------------

/// An in-memory `CompanyStore` seeded with one record; `save` can be told to
/// fail so the file-rollback path is exercised.
#[derive(Default)]
struct MemStore {
    record: StdMutex<Option<CompanyRecord>>,
    fail_save: bool,
}

impl MemStore {
    fn seeded(record: CompanyRecord) -> Self {
        Self {
            record: StdMutex::new(Some(record)),
            fail_save: false,
        }
    }
    fn failing(record: CompanyRecord) -> Self {
        Self {
            record: StdMutex::new(Some(record)),
            fail_save: true,
        }
    }
}

#[async_trait]
impl CompanyStore for MemStore {
    async fn load(&self, _id: &CompanyId) -> Result<Option<CompanyRecord>> {
        Ok(self.record.lock().unwrap().clone())
    }
    async fn save(&self, record: &CompanyRecord) -> Result<()> {
        if self.fail_save {
            return Err(OpenCompanyError::InvalidRequest("save boom".into()));
        }
        *self.record.lock().unwrap() = Some(record.clone());
        Ok(())
    }
    async fn list(&self) -> Result<Vec<CompanySummary>> {
        Ok(Vec::new())
    }
    async fn append_ledger(&self, _id: &CompanyId, _entry: LedgerEntry) -> Result<()> {
        Ok(())
    }
}

/// An in-memory `EventLog` that records appended events so the audit journal
/// can be asserted.
#[derive(Default)]
struct MemLog {
    events: StdMutex<Vec<CompanyEvent>>,
}

#[async_trait]
impl EventLog for MemLog {
    async fn append(&self, _id: &CompanyId, event: CompanyEvent) -> Result<EventSeq> {
        let mut guard = self.events.lock().unwrap();
        guard.push(event);
        Ok(EventSeq::new(guard.len() as u64))
    }
    async fn read_from(
        &self,
        _id: &CompanyId,
        _seq: EventSeq,
        _limit: usize,
    ) -> Result<Vec<StoredEvent>> {
        Ok(Vec::new())
    }
    fn subscribe(
        &self,
        _id: &CompanyId,
    ) -> BoxStream<'static, crate::ports::events::EventStreamItem> {
        Box::pin(stream::empty())
    }
}

/// An in-memory [`EventLog`] whose [`subscribe`](EventLog::subscribe) stream
/// actually delivers what [`append`](EventLog::append) writes — the property
/// [`MemLog`] above deliberately lacks (its `subscribe` is empty). This is
/// what lets a test stand in for the live SSE fan-out: the console's picker
/// re-reads off exactly this broadcast (issue #1045), so a create/delete
/// that reaches a live subscriber here is evidence that in-process delivery
/// is intact and a stale picker is a console-side defect, not a lost frame.
struct BroadcastMemLog {
    tx: tokio::sync::broadcast::Sender<StoredEvent>,
    next_seq: StdMutex<u64>,
}

impl BroadcastMemLog {
    fn new() -> Self {
        Self {
            tx: tokio::sync::broadcast::channel(64).0,
            next_seq: StdMutex::new(0),
        }
    }
}

#[async_trait]
impl EventLog for BroadcastMemLog {
    async fn append(&self, id: &CompanyId, event: CompanyEvent) -> Result<EventSeq> {
        let seq = {
            let mut n = self.next_seq.lock().unwrap();
            *n += 1;
            *n
        };
        let stored = StoredEvent {
            seq: EventSeq::new(seq),
            company: id.clone(),
            event,
            at_millis: now_millis(),
        };
        // No live subscriber is not an error — a send with zero receivers
        // just means nobody is watching yet.
        let _ = self.tx.send(stored);
        Ok(EventSeq::new(seq))
    }
    async fn read_from(
        &self,
        _id: &CompanyId,
        _seq: EventSeq,
        _limit: usize,
    ) -> Result<Vec<StoredEvent>> {
        Ok(Vec::new())
    }
    fn subscribe(
        &self,
        _id: &CompanyId,
    ) -> BoxStream<'static, crate::ports::events::EventStreamItem> {
        let rx = self.tx.subscribe();
        Box::pin(stream::unfold(rx, |mut rx| async move {
            // Each call to this closure produces exactly one item and hands
            // the receiver back as continuation state, so there is no loop
            // here.
            match rx.recv().await {
                Ok(event) => Some((crate::ports::events::EventStreamItem::Event(event), rx)),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    Some((crate::ports::events::EventStreamItem::Gap { missed }, rx))
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => None,
            }
        }))
    }
}

/// An in-memory [`WorkflowRevisionStore`] so the capture, prune, cascade and
/// rollback behaviour can be asserted without a real backend. Pruning to the
/// cap is applied on push, mirroring the durable backends.
#[derive(Default)]
struct MemRevisions {
    rows: StdMutex<Vec<WorkflowRevisionRecord>>,
}

#[async_trait]
impl WorkflowRevisionStore for MemRevisions {
    async fn push_revision(
        &self,
        _company: &CompanyId,
        revision: &WorkflowRevisionRecord,
    ) -> Result<()> {
        use crate::ports::workflow_revisions::{MAX_WORKFLOW_REVISIONS, sort_newest_first};
        let mut rows = self.rows.lock().unwrap();
        rows.push(revision.clone());
        let mut mine: Vec<WorkflowRevisionRecord> = rows
            .iter()
            .filter(|r| r.workflow_id == revision.workflow_id)
            .cloned()
            .collect();
        if mine.len() > MAX_WORKFLOW_REVISIONS {
            sort_newest_first(&mut mine);
            let keep: std::collections::HashSet<String> = mine
                .into_iter()
                .take(MAX_WORKFLOW_REVISIONS)
                .map(|r| r.id)
                .collect();
            rows.retain(|r| r.workflow_id != revision.workflow_id || keep.contains(&r.id));
        }
        Ok(())
    }
    async fn list_revisions(
        &self,
        _company: &CompanyId,
        workflow_id: &str,
    ) -> Result<Vec<WorkflowRevisionRecord>> {
        use crate::ports::workflow_revisions::sort_newest_first;
        let mut mine: Vec<WorkflowRevisionRecord> = self
            .rows
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.workflow_id == workflow_id)
            .cloned()
            .collect();
        sort_newest_first(&mut mine);
        Ok(mine)
    }
    async fn get_revision(
        &self,
        _company: &CompanyId,
        workflow_id: &str,
        revision_id: &str,
    ) -> Result<Option<WorkflowRevisionRecord>> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.workflow_id == workflow_id && r.id == revision_id)
            .cloned())
    }
    async fn delete_revisions(&self, _company: &CompanyId, workflow_id: &str) -> Result<u64> {
        let mut rows = self.rows.lock().unwrap();
        let before = rows.len();
        rows.retain(|r| r.workflow_id != workflow_id);
        Ok((before - rows.len()) as u64)
    }
}

/// A throwaway revision store for tests that do not assert on revision
/// capture — the common case. Tests that DO assert capture/prune/rollback
/// hold their own `Arc<MemRevisions>` so they can read it back.
fn revs() -> Arc<dyn WorkflowRevisionStore> {
    Arc::new(MemRevisions::default())
}

/// An in-memory [`ScheduleFireStore`] so the delete-time fire-ledger purge
/// (issue #708) can be asserted without a real backend. Only the verbs the
/// delete path exercises need real behaviour; `claim_fire` seeds a ledger
/// and `delete_schedule_fires` purges one schedule's rows.
#[derive(Default)]
struct MemFires {
    /// `(company, schedule_id) -> claimed minutes`.
    rows: StdMutex<std::collections::HashMap<(String, String), std::collections::HashSet<u64>>>,
    /// Arm the next `delete_schedule_fires` call to error, to prove the
    /// delete succeeds even when the purge cascade fails.
    fail_delete: std::sync::atomic::AtomicBool,
}

impl MemFires {
    fn seed(&self, company: &CompanyId, schedule_id: &str, minute: u64) {
        self.rows
            .lock()
            .unwrap()
            .entry((company.as_ref().to_string(), schedule_id.to_string()))
            .or_default()
            .insert(minute);
    }
    fn minutes(&self, company: &CompanyId, schedule_id: &str) -> Vec<u64> {
        let rows = self.rows.lock().unwrap();
        let mut ms: Vec<u64> = rows
            .get(&(company.as_ref().to_string(), schedule_id.to_string()))
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default();
        ms.sort_unstable();
        ms
    }
    fn arm_delete_failure(&self) {
        self.fail_delete
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

#[async_trait]
impl ScheduleFireStore for MemFires {
    async fn claim_fire(&self, c: &CompanyId, s: &str, m: u64) -> Result<bool> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .entry((c.as_ref().to_string(), s.to_string()))
            .or_default()
            .insert(m))
    }
    async fn latest_fire(&self, c: &CompanyId, s: &str) -> Result<Option<u64>> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .get(&(c.as_ref().to_string(), s.to_string()))
            .and_then(|set| set.iter().max().copied()))
    }
    async fn prune_fires_before(&self, _c: &CompanyId, _m: u64) -> Result<usize> {
        Ok(0)
    }
    async fn delete_schedule_fires(&self, c: &CompanyId, s: &str) -> Result<usize> {
        if self
            .fail_delete
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(OpenCompanyError::Store("flaky fire-ledger purge".into()));
        }
        Ok(self
            .rows
            .lock()
            .unwrap()
            .remove(&(c.as_ref().to_string(), s.to_string()))
            .map_or(0, |set| set.len()))
    }
}

/// A throwaway fire store for delete tests that do not assert on the purge —
/// the common case. Tests that DO assert the purge hold their own
/// `Arc<MemFires>` so they can read it back.
fn fires() -> Arc<dyn ScheduleFireStore> {
    Arc::new(MemFires::default())
}

// --- fixtures ------------------------------------------------------------

/// A committed seed graph, id `seeded` / name `Seeded flow`.
const SEED_TOML: &str = r#"
id = "seeded"
name = "Seeded flow"
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

/// A manifest with an `assistant` roster agent so `agent`-node graphs pass
/// the roster check.
fn manifest_with_assistant() -> CompanyManifest {
    toml::from_str(
        "[company]\nname = \"Acme\"\n[[agent]]\nid = \"assistant\"\nrole = \"Assistant\"\n",
    )
    .expect("valid manifest")
}

fn record(id: &CompanyId, manifest: CompanyManifest) -> CompanyRecord {
    CompanyRecord {
        overlay_desk_hive: Vec::new(),
        overlay_retired_agents: Vec::new(),
        overlay_agent_edits: Vec::new(),
        id: id.clone(),
        manifest,
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
        setup: None,
        name_confirmed: false,
        activation_completed_at: None,
        created_at_millis: None,
    }
}

/// A valid trigger → agent → output draft naming the `assistant` teammate.
fn valid_draft(id: &str, name: &str) -> RawWorkflow {
    RawWorkflow {
        id: id.to_string(),
        name: name.to_string(),
        description: Some("A tiny graph.".to_string()),
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
                agent: Some("assistant".to_string()),
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
                destination: None,
                postcondition: None,
                verify: None,
            },
        ],
        edges: vec![
            RawEdge {
                from: "start".to_string(),
                to: "worker".to_string(),
                label: None,
            },
            RawEdge {
                from: "worker".to_string(),
                to: "done".to_string(),
                label: Some("ok".to_string()),
            },
        ],
    }
}

fn store_of(store: MemStore) -> Arc<dyn CompanyStore> {
    Arc::new(store)
}

// --- happy path ----------------------------------------------------------

#[tokio::test]
async fn creates_enables_and_journals() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    let log = Arc::new(MemLog::default());
    let log_dyn: Arc<dyn EventLog> = log.clone();

    let file = create_company_workflow(
        &company,
        Some(dir.path()),
        &store,
        Some(&log_dyn),
        valid_draft("greeter", "Greeter"),
        None,
        None,
    )
    .await
    .expect("creates");

    assert_eq!(file.id, "greeter");
    assert_eq!(file.nodes.len(), 3);

    // The body landed on the RECORD, not in the (read-only in hosted mode)
    // source tree — the whole point of #168.
    let record = store.load(&company).await.unwrap().unwrap();
    assert_eq!(record.overlay_workflows.len(), 1);
    assert_eq!(record.overlay_workflows[0].id, "greeter");
    assert!(
        !dir.path().join("workflows").exists(),
        "creation must not write into the company source tree"
    );

    // The persisted body re-loads to exactly what we returned (contract).
    let reloaded = load_workflow_union(Some(dir.path()), &record.overlay_workflows, &file.id)
        .expect("reloads")
        .expect("one file");
    assert_eq!(
        reloaded, file,
        "returned WorkflowFile must equal what the union read path serves"
    );

    // Enabled on the record.
    assert!(
        record
            .manifest
            .workflows
            .enabled
            .contains(&"greeter".to_string())
    );

    // Journaled a WorkflowCreated audit event.
    let events = log.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    match &events[0] {
        CompanyEvent::WorkflowCreated {
            workflow_id,
            name,
            by,
        } => {
            assert_eq!(workflow_id, "greeter");
            assert_eq!(name, "Greeter");
            assert!(
                by.is_none(),
                "the orchestrator/no-actor path must journal an unattributed create"
            );
        }
        other => panic!("expected WorkflowCreated, got {other:?}"),
    }
}

/// Issue #1843: the REST create path passes `ScopedCompany::actor` through
/// as `by`, and it must land verbatim on the journaled event — this is the
/// per-user attribution the activation funnel's `IntegrationConnected`-style
/// signals eventually build on. Sibling of `creates_enables_and_journals`
/// above, which pins the complementary `None` (orchestrator/platform) path.
#[tokio::test]
async fn a_signed_in_actor_is_attributed_on_the_journaled_create() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    let log = Arc::new(MemLog::default());
    let log_dyn: Arc<dyn EventLog> = log.clone();
    let actor = crate::ports::types::Actor {
        kind: crate::ports::types::ActorKind::User,
        id: "user-42".to_string(),
    };

    create_company_workflow(
        &company,
        Some(dir.path()),
        &store,
        Some(&log_dyn),
        valid_draft("greeter", "Greeter"),
        None,
        Some(actor.clone()),
    )
    .await
    .expect("creates");

    let events = log.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    match &events[0] {
        CompanyEvent::WorkflowCreated { by, .. } => {
            assert_eq!(
                by.as_ref(),
                Some(&actor),
                "the signed-in actor must be attributed on the journaled create"
            );
        }
        other => panic!("expected WorkflowCreated, got {other:?}"),
    }
}

// --- guardrail failures --------------------------------------------------

/// A second create with the same id collides against the record's overlay.
#[tokio::test]
async fn duplicate_id_is_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));

    create_company_workflow(
        &company,
        Some(dir.path()),
        &store,
        None,
        valid_draft("dup", "First"),
        None,
        None,
    )
    .await
    .expect("first create");
    let err = create_company_workflow(
        &company,
        Some(dir.path()),
        &store,
        None,
        valid_draft("dup", "Second name"),
        None,
        None,
    )
    .await
    .expect_err("second create with same id");
    assert!(matches!(err, OpenCompanyError::Conflict(_)), "{err:?}");
}

#[tokio::test]
async fn duplicate_name_case_insensitive_is_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));

    create_company_workflow(
        &company,
        Some(dir.path()),
        &store,
        None,
        valid_draft("one", "Greeter"),
        None,
        None,
    )
    .await
    .expect("first");
    let err = create_company_workflow(
        &company,
        Some(dir.path()),
        &store,
        None,
        valid_draft("two", "  GREETER  "),
        None,
        None,
    )
    .await
    .expect_err("name collides case-insensitively");
    assert!(matches!(err, OpenCompanyError::Conflict(_)), "{err:?}");
}

#[tokio::test]
async fn unknown_roster_teammate_is_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    let mut draft = valid_draft("wf", "WF");
    draft.nodes[1].agent = Some("ghost".to_string());

    let err =
        create_company_workflow(&company, Some(dir.path()), &store, None, draft, None, None)
            .await
            .expect_err("unknown teammate");
    assert!(
        matches!(err, OpenCompanyError::InvalidRequest(_)),
        "{err:?}"
    );
    assert!(err.to_string().contains("ghost"), "{err}");
}

#[tokio::test]
async fn missing_agent_on_agent_node_is_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    let mut draft = valid_draft("wf", "WF");
    draft.nodes[1].agent = None;

    let err =
        create_company_workflow(&company, Some(dir.path()), &store, None, draft, None, None)
            .await
            .expect_err("agent node with no teammate");
    assert!(
        matches!(err, OpenCompanyError::InvalidRequest(_)),
        "{err:?}"
    );
}

#[tokio::test]
async fn zero_or_two_triggers_is_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));

    // Zero triggers.
    let mut zero = valid_draft("z", "Z");
    zero.nodes[0].kind = "output".to_string();
    let err =
        create_company_workflow(&company, Some(dir.path()), &store, None, zero, None, None)
            .await
            .expect_err("no trigger");
    assert!(err.to_string().contains("exactly one `trigger`"), "{err}");

    // Two triggers.
    let mut two = valid_draft("t", "T");
    two.nodes[2].kind = "trigger".to_string();
    let err =
        create_company_workflow(&company, Some(dir.path()), &store, None, two, None, None)
            .await
            .expect_err("two triggers");
    assert!(err.to_string().contains("exactly one `trigger`"), "{err}");
}

#[tokio::test]
async fn traversal_id_is_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    let err = create_company_workflow(
        &company,
        Some(dir.path()),
        &store,
        None,
        valid_draft("../secrets", "Escape"),
        None,
        None,
    )
    .await
    .expect_err("traversal id");
    assert!(
        matches!(err, OpenCompanyError::InvalidRequest(_)),
        "{err:?}"
    );
}

#[tokio::test]
async fn oversized_node_count_is_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    let mut draft = valid_draft("big", "Big");
    for i in 0..MAX_WORKFLOW_NODES {
        draft.nodes.push(RawNode {
            id: format!("n{i}"),
            kind: "output".to_string(),
            name: format!("N{i}"),
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
        });
    }
    assert!(draft.nodes.len() > MAX_WORKFLOW_NODES);
    let err =
        create_company_workflow(&company, Some(dir.path()), &store, None, draft, None, None)
            .await
            .expect_err("too many nodes");
    assert!(err.to_string().contains("at most"), "{err}");
}

#[tokio::test]
async fn oversized_toml_bytes_is_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    // Stay within the node cap but blow the byte cap with a huge summary.
    let mut draft = valid_draft("fat", "Fat");
    draft.nodes[0].summary = Some("x".repeat(MAX_WORKFLOW_TOML_BYTES + 10));
    let err =
        create_company_workflow(&company, Some(dir.path()), &store, None, draft, None, None)
            .await
            .expect_err("too many bytes");
    assert!(err.to_string().contains("byte"), "{err}");
}

/// The body and the enabled id land in ONE save, so a failing save leaves
/// the record exactly as it was — no orphaned body, no orphaned enabled id,
/// nothing to roll back. (Before #168 this needed a file-removal dance.)
#[tokio::test]
async fn store_save_failure_leaves_the_record_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::failing(record(
        &company,
        manifest_with_assistant(),
    )));

    let err = create_company_workflow(
        &company,
        Some(dir.path()),
        &store,
        None,
        valid_draft("rollback", "Rollback"),
        None,
        None,
    )
    .await
    .expect_err("save fails");
    assert!(
        matches!(err, OpenCompanyError::InvalidRequest(_)),
        "{err:?}"
    );

    let record = store.load(&company).await.unwrap().unwrap();
    assert!(record.overlay_workflows.is_empty(), "no orphaned body");
    assert!(
        record.manifest.workflows.enabled.is_empty(),
        "no orphaned enabled id"
    );
    assert!(
        !dir.path().join("workflows").join("rollback.toml").exists(),
        "nothing was written to the source tree"
    );
}

// --- #168: no source directory at all (the hosted case) ------------------

/// The direct #168 regression at the core level: a hosted tenant has no
/// source directory (its crate mount is read-only), and creation must still
/// succeed by persisting the body on the record.
#[tokio::test]
async fn creates_with_no_source_dir() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));

    let file = create_company_workflow(
        &company,
        None,
        &store,
        None,
        valid_draft("hosted", "Hosted"),
        None,
        None,
    )
    .await
    .expect("creates with no source dir");
    assert_eq!(file.id, "hosted");

    let record = store.load(&company).await.unwrap().unwrap();
    assert_eq!(record.overlay_workflows.len(), 1);
    assert_eq!(record.overlay_workflows[0].id, "hosted");
    assert!(
        record
            .manifest
            .workflows
            .enabled
            .contains(&"hosted".to_string())
    );
    // And it reads back as a full graph through the union path.
    let loaded = load_workflow_union(None, &record.overlay_workflows, "hosted")
        .expect("loads")
        .expect("present");
    assert_eq!(loaded, file);
}

/// An id already taken by a *seed* file is a 409 even though the new body
/// would live somewhere else entirely — the seed would shadow it on read.
#[tokio::test]
async fn id_colliding_with_a_seed_file_is_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let workflows = dir.path().join("workflows");
    std::fs::create_dir_all(&workflows).unwrap();
    std::fs::write(workflows.join("seeded.toml"), SEED_TOML).unwrap();

    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    let err = create_company_workflow(
        &company,
        Some(dir.path()),
        &store,
        None,
        valid_draft("seeded", "Different name"),
        None,
        None,
    )
    .await
    .expect_err("id is taken by a seed file");
    assert!(matches!(err, OpenCompanyError::Conflict(_)), "{err:?}");
    assert!(err.to_string().contains("seeded"), "{err}");
}

/// A *name* already used by a seed file collides too — the picker would show
/// two indistinguishable entries.
#[tokio::test]
async fn name_colliding_with_a_seed_file_is_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let workflows = dir.path().join("workflows");
    std::fs::create_dir_all(&workflows).unwrap();
    std::fs::write(workflows.join("seeded.toml"), SEED_TOML).unwrap();

    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    let err = create_company_workflow(
        &company,
        Some(dir.path()),
        &store,
        None,
        valid_draft("other", "  seeded FLOW  "),
        None,
        None,
    )
    .await
    .expect_err("name collides with the seed file's name");
    assert!(matches!(err, OpenCompanyError::Conflict(_)), "{err:?}");
}

/// With no source tree at all, the name guard still works — it degrades to
/// overlay ∪ enabled rather than erroring or silently allowing duplicates.
#[tokio::test]
async fn name_guard_works_without_a_source_tree() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));

    create_company_workflow(
        &company,
        None,
        &store,
        None,
        valid_draft("one", "Greeter"),
        None,
        None,
    )
    .await
    .expect("first");
    let err = create_company_workflow(
        &company,
        None,
        &store,
        None,
        valid_draft("two", "GREETER"),
        None,
        None,
    )
    .await
    .expect_err("name collides with the overlay body");
    assert!(matches!(err, OpenCompanyError::Conflict(_)), "{err:?}");
}

/// A graph whose only node is its trigger, carrying a schedule — the
/// `campaign` shape from staging (issue #976).
fn stageless_scheduled_draft(id: &str, name: &str) -> RawWorkflow {
    let mut draft = valid_draft(id, name);
    // Keep only the trigger, and put a schedule on it. This is what the
    // console produces when somebody drops a Start node, sets a cron, and
    // saves before adding any stage.
    draft.nodes.retain(|n| n.kind == "trigger");
    draft.edges.clear();
    draft.nodes[0].schedule = Some("0 9 * * *".to_string());
    draft
}

/// Saving one is **allowed**, and that is the deliberate half of the fix.
/// Authoring is incremental: the console drops a Start node first and adds
/// stages after, `parse_workflow` was made lenient on purpose (#661) to
/// support exactly that, and refusing at save would also refuse every
/// existing seed and legacy body on its next edit.
#[tokio::test]
async fn a_stageless_graph_still_saves() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));

    create_company_workflow(
        &company,
        Some(dir.path()),
        &store,
        None,
        stageless_scheduled_draft("campaign", "Campaign"),
        None,
        None,
    )
    .await
    .expect("a stub mid-authoring is legitimate and must save");
}

/// ...but switching its schedule on is refused. Arming is where the promise
/// is made, so it is where the promise is checked: resume this and it fires
/// on time, runs nothing, and reports nothing.
#[tokio::test]
async fn arming_a_stageless_schedule_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));

    create_company_workflow(
        &company,
        Some(dir.path()),
        &store,
        None,
        stageless_scheduled_draft("campaign", "Campaign"),
        None,
        None,
    )
    .await
    .expect("saves");

    let err = set_company_workflow_enabled(
        &company,
        Some(dir.path()),
        &store,
        None,
        "campaign",
        true,
        true,
        &[],
    )
    .await
    .expect_err("a schedule that cannot run must not be armed");

    let rendered = err.to_string();
    assert!(
        rendered.contains("no stage to run"),
        "the operator is told WHAT is wrong: {rendered}"
    );
    assert!(
        rendered.contains("Add at least one node"),
        "...and the one thing they can do about it: {rendered}"
    );
}

/// Switching such a workflow **off** stays allowed. An operator must always
/// be able to stop a thing — the same call the unparseable-body case makes
/// — and a guard that trapped a workflow in the armed state would be worse
/// than the silence it replaced.
#[tokio::test]
async fn a_stageless_schedule_can_still_be_switched_off() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));

    create_company_workflow(
        &company,
        Some(dir.path()),
        &store,
        None,
        stageless_scheduled_draft("campaign", "Campaign"),
        None,
        None,
    )
    .await
    .expect("saves");

    set_company_workflow_enabled(
        &company,
        Some(dir.path()),
        &store,
        None,
        "campaign",
        false,
        true,
        &[],
    )
    .await
    .expect("pausing must never be refused");
}

/// A graph with a real stage arms normally. Without this the refusal above
/// would pass against a build that refused every schedule.
#[tokio::test]
async fn arming_a_scheduled_graph_with_a_stage_still_works() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));

    let mut draft = valid_draft("greeter", "Greeter");
    draft.nodes[0].schedule = Some("0 9 * * *".to_string());
    create_company_workflow(&company, Some(dir.path()), &store, None, draft, None, None)
        .await
        .expect("saves");

    set_company_workflow_enabled(
        &company,
        Some(dir.path()),
        &store,
        None,
        "greeter",
        true,
        true,
        &[],
    )
    .await
    .expect("a graph that can actually run may be armed");
}

// --- Undeliverable-schedule refusal (issue #1046) ------------------------

/// A scheduled `trigger → agent → output` draft whose output delivers to
/// `(dest_kind, dest_target)`. The shape #1046 guards: a graph that runs a
/// stage but whose only report may land nowhere.
fn scheduled_output_draft(
    id: &str,
    name: &str,
    dest_kind: &str,
    dest_target: Option<&str>,
) -> RawWorkflow {
    let mut draft = draft_with_destination(id, name, dest_kind, dest_target);
    draft.nodes[0].schedule = Some("0 9 * * *".to_string());
    draft
}

/// Issue #1757 reverses one arm of #1046: a scheduled graph whose only report
/// goes to the owner on a company with **no mailbox** now ARMS. `owner` no
/// longer dead-ends on an in-memory buffer — it falls back to the durable
/// operator channel, which journals the report into the operator's main line,
/// so a scheduled owner report reliably lands and the schedule is honest.
#[tokio::test]
async fn arming_a_scheduled_owner_output_with_no_mailbox_now_arms() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));

    create_company_workflow(
        &company,
        Some(dir.path()),
        &store,
        None,
        scheduled_output_draft("digest", "Digest", "owner", None),
        None,
        None,
    )
    .await
    .expect("saving a stub is legitimate and must succeed");

    set_company_workflow_enabled(
        &company,
        Some(dir.path()),
        &store,
        None,
        "digest",
        true,
        // No mailbox, no wired channels: the owner report still lands, on the
        // durable operator channel (issue #1757).
        false,
        &[],
    )
    .await
    .expect("an owner report always lands, so its schedule must arm");
}

/// The manual half of the fix: the same undeliverable graph, but with **no**
/// schedule on its trigger, enables freely. Running a stub by hand — knowing
/// its report only reaches the run drawer — is the operator's own business;
/// only a *schedule* makes a delivery promise nobody is watching.
#[tokio::test]
async fn a_manual_undeliverable_graph_is_not_refused() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));

    // A genuinely undeliverable graph — a channel output to an unwired desk —
    // but drop the schedule so it is manual. (`owner` no longer qualifies:
    // since issue #1757 it always lands on the durable operator channel.)
    let mut draft = scheduled_output_draft("digest", "Digest", "channel", Some("marketing"));
    draft.nodes[0].schedule = None;
    create_company_workflow(&company, Some(dir.path()), &store, None, draft, None, None)
        .await
        .expect("saves");

    set_company_workflow_enabled(
        &company,
        Some(dir.path()),
        &store,
        None,
        "digest",
        true,
        false,
        &[],
    )
    .await
    .expect("a manual graph is never refused for undeliverable output");
}

/// The refusal is delivery-capability-specific, not a blanket ban on
/// owner outputs: the identical scheduled owner graph arms once a mailbox is
/// configured, because owner delivery can then email the company's admins.
#[tokio::test]
async fn arming_a_scheduled_owner_output_with_a_mailbox_arms() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));

    create_company_workflow(
        &company,
        Some(dir.path()),
        &store,
        None,
        scheduled_output_draft("digest", "Digest", "owner", None),
        None,
        None,
    )
    .await
    .expect("saves");

    set_company_workflow_enabled(
        &company,
        Some(dir.path()),
        &store,
        None,
        "digest",
        true,
        // A mailbox is configured: owner reports can be emailed.
        true,
        &[],
    )
    .await
    .expect("an owner report can land once a mailbox exists");
}

/// A scheduled output to a wired channel arms even with no mailbox — the
/// channel is a real write path.
#[tokio::test]
async fn arming_a_scheduled_channel_output_to_a_wired_channel_arms() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));

    create_company_workflow(
        &company,
        Some(dir.path()),
        &store,
        None,
        scheduled_output_draft("digest", "Digest", "channel", Some("engineering")),
        None,
        None,
    )
    .await
    .expect("saves");

    set_company_workflow_enabled(
        &company,
        Some(dir.path()),
        &store,
        None,
        "digest",
        true,
        false,
        &["engineering".to_string()],
    )
    .await
    .expect("a report to a wired channel can land");
}

/// A scheduled output to the operator channel now ARMS (issue #1757): the
/// operator channel is a durable, journal-backed surface the company always
/// wires, so `deliverable_channel_ids` lists it and a report posted there
/// lands in the standing Operator channel.
#[tokio::test]
async fn arming_a_scheduled_channel_output_to_operator_arms() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));

    create_company_workflow(
        &company,
        Some(dir.path()),
        &store,
        None,
        scheduled_output_draft("digest", "Digest", "channel", Some("operator")),
        None,
        None,
    )
    .await
    .expect("saves");

    set_company_workflow_enabled(
        &company,
        Some(dir.path()),
        &store,
        None,
        "digest",
        true,
        false,
        // `operator` is a wired, durable channel now.
        &["operator".to_string()],
    )
    .await
    .expect("a report to the durable operator channel can land, so the schedule arms");
}

/// Switching an undeliverable scheduled graph **off** is always allowed — an
/// operator must be able to stop a thing, the same rule the stage-less guard
/// keeps.
#[tokio::test]
async fn an_undeliverable_scheduled_graph_can_still_be_switched_off() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));

    create_company_workflow(
        &company,
        Some(dir.path()),
        &store,
        None,
        scheduled_output_draft("digest", "Digest", "owner", None),
        None,
        None,
    )
    .await
    .expect("saves");

    set_company_workflow_enabled(
        &company,
        Some(dir.path()),
        &store,
        None,
        "digest",
        false,
        false,
        &[],
    )
    .await
    .expect("pausing must never be refused");
}

/// A manifest-`enabled` id with no body in either source is shown by the
/// picker under its id, so a new workflow can't take that name either.
#[tokio::test]
async fn name_collides_with_a_bodiless_enabled_id() {
    let company = CompanyId::new("acme");
    let mut rec = record(&company, manifest_with_assistant());
    rec.manifest.workflows.enabled.push("legacy".to_string());
    let store = store_of(MemStore::seeded(rec));

    let err = create_company_workflow(
        &company,
        None,
        &store,
        None,
        valid_draft("new", "  LEGACY  "),
        None,
        None,
    )
    .await
    .expect_err("name collides with the enabled-id fallback name");
    assert!(matches!(err, OpenCompanyError::Conflict(_)), "{err:?}");
}

#[tokio::test]
async fn no_company_record_is_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("ghost");
    let store: Arc<dyn CompanyStore> = Arc::new(MemStore::default());
    let err = create_company_workflow(
        &company,
        Some(dir.path()),
        &store,
        None,
        valid_draft("wf", "WF"),
        None,
        None,
    )
    .await
    .expect_err("no record");
    assert!(
        matches!(err, OpenCompanyError::CompanyNotFound(_)),
        "{err:?}"
    );
}

// --- #259: update ------------------------------------------------------

/// Seeds a company with one created workflow and hands back the store plus
/// the version token a `GET` would have returned for it.
async fn with_one_workflow(
    company: &CompanyId,
    id: &str,
    name: &str,
) -> (Arc<dyn CompanyStore>, String) {
    let store = store_of(MemStore::seeded(record(company, manifest_with_assistant())));
    create_company_workflow(
        company,
        None,
        &store,
        None,
        valid_draft(id, name),
        None,
        None,
    )
    .await
    .expect("seed create");
    let record = store.load(company).await.unwrap().unwrap();
    let version = workflow_version(&record.overlay_workflows[0].toml);
    (store, version)
}

#[tokio::test]
async fn updates_the_body_in_place_and_journals() {
    let company = CompanyId::new("acme");
    let (store, version) = with_one_workflow(&company, "greeter", "Greeter").await;
    let log = Arc::new(MemLog::default());
    let log_dyn: Arc<dyn EventLog> = log.clone();

    let mut draft = valid_draft("greeter", "Greeter");
    draft.nodes[0].schedule = Some("0 9 * * *".to_string());
    draft.description = Some("Now on a cron.".to_string());

    let file = update_company_workflow(
        &company,
        None,
        &store,
        &revs(),
        Some(&log_dyn),
        draft,
        Some(&version),
        None,
    )
    .await
    .expect("updates");
    assert_eq!(file.nodes[0].schedule.as_deref(), Some("0 9 * * *"));

    let record = store.load(&company).await.unwrap().unwrap();
    // Replaced, not appended — an edit must never fork the graph in two.
    assert_eq!(record.overlay_workflows.len(), 1);
    assert_eq!(record.overlay_workflows[0].id, "greeter");
    // The manifest declaration is untouched — it says which workflows this
    // company has, not which of them are armed.
    assert_eq!(record.manifest.workflows.enabled, vec!["greeter"]);
    // …but this edit added a cron to a manual graph, so issue #276's disarm
    // rule switched it off in the same save. The draft above is exactly the
    // manual→automatic transition the rule exists for, which is why this
    // assertion lives on the general update test rather than only on the
    // dedicated one.
    assert!(
        !record.workflow_enabled("greeter"),
        "an edit that adds a schedule must leave the workflow switched off"
    );

    // What the union read path serves is what we returned.
    let reloaded = load_workflow_union(None, &record.overlay_workflows, "greeter")
        .expect("reloads")
        .expect("present");
    assert_eq!(reloaded, file);

    let events = log.events.lock().unwrap();
    assert_eq!(events.len(), 2, "the edit, then the disarm it triggered");
    match &events[0] {
        CompanyEvent::WorkflowUpdated {
            workflow_id, name, ..
        } => {
            assert_eq!(workflow_id, "greeter");
            assert_eq!(name, "Greeter");
        }
        other => panic!("expected WorkflowUpdated, got {other:?}"),
    }
    match &events[1] {
        CompanyEvent::WorkflowEnabledChanged {
            workflow_id,
            enabled,
            reason,
            ..
        } => {
            assert_eq!(workflow_id, "greeter");
            assert!(!enabled);
            assert_eq!(*reason, WorkflowEnabledReason::Disarmed);
        }
        other => panic!("expected WorkflowEnabledChanged, got {other:?}"),
    }
}

/// The version token is what makes concurrent edits safe. A caller holding a
/// token from before someone else's write must be refused, not silently win.
#[tokio::test]
async fn a_stale_version_is_refused_and_changes_nothing() {
    let company = CompanyId::new("acme");
    let (store, stale) = with_one_workflow(&company, "greeter", "Greeter").await;

    // Someone else edits first, unconditionally.
    let mut theirs = valid_draft("greeter", "Greeter");
    theirs.description = Some("Theirs landed first.".to_string());
    update_company_workflow(&company, None, &store, &revs(), None, theirs, None, None)
        .await
        .expect("first writer wins");

    // Our stale token is now wrong.
    let mut ours = valid_draft("greeter", "Greeter");
    ours.description = Some("Ours would clobber.".to_string());
    let err = update_company_workflow(
        &company,
        None,
        &store,
        &revs(),
        None,
        ours,
        Some(&stale),
        None,
    )
    .await
    .expect_err("stale version must be refused");
    assert!(matches!(err, OpenCompanyError::Conflict(_)), "{err:?}");

    // And the other writer's edit is intact — the refusal is not partial.
    let record = store.load(&company).await.unwrap().unwrap();
    let current = load_workflow_union(None, &record.overlay_workflows, "greeter")
        .unwrap()
        .unwrap();
    assert_eq!(current.description.as_deref(), Some("Theirs landed first."));
}

/// The fresh token from the *previous* write is accepted, so the
/// reload-and-retry loop the console offers actually terminates.
#[tokio::test]
async fn a_fresh_version_is_accepted() {
    let company = CompanyId::new("acme");
    let (store, first) = with_one_workflow(&company, "greeter", "Greeter").await;

    let mut once = valid_draft("greeter", "Greeter");
    once.description = Some("One.".to_string());
    update_company_workflow(
        &company,
        None,
        &store,
        &revs(),
        None,
        once,
        Some(&first),
        None,
    )
    .await
    .expect("first conditional write");

    let record = store.load(&company).await.unwrap().unwrap();
    let second = workflow_version(&record.overlay_workflows[0].toml);
    assert_ne!(second, first, "the token must move when the body does");

    let mut twice = valid_draft("greeter", "Greeter");
    twice.description = Some("Two.".to_string());
    update_company_workflow(
        &company,
        None,
        &store,
        &revs(),
        None,
        twice,
        Some(&second),
        None,
    )
    .await
    .expect("refreshed token is accepted");
}

/// No token at all is an unconditional write — the `curl` contract.
#[tokio::test]
async fn no_version_is_an_unconditional_write() {
    let company = CompanyId::new("acme");
    let (store, _) = with_one_workflow(&company, "greeter", "Greeter").await;
    let mut draft = valid_draft("greeter", "Greeter");
    draft.description = Some("No token needed.".to_string());
    update_company_workflow(&company, None, &store, &revs(), None, draft, None, None)
        .await
        .expect("unconditional write");
}

/// Re-saving without renaming must not collide with the workflow's own name.
#[tokio::test]
async fn keeping_the_same_name_is_not_a_self_conflict() {
    let company = CompanyId::new("acme");
    let (store, version) = with_one_workflow(&company, "greeter", "Greeter").await;
    let mut draft = valid_draft("greeter", "  greeter  ");
    draft.description = Some("Same name, different case and padding.".to_string());
    update_company_workflow(
        &company,
        None,
        &store,
        &revs(),
        None,
        draft,
        Some(&version),
        None,
    )
    .await
    .expect("own name must not conflict with itself");
}

/// …but a *sibling's* name is still guarded.
#[tokio::test]
async fn taking_another_workflows_name_is_a_conflict() {
    let company = CompanyId::new("acme");
    let (store, _) = with_one_workflow(&company, "greeter", "Greeter").await;
    create_company_workflow(
        &company,
        None,
        &store,
        None,
        valid_draft("other", "Other"),
        None,
        None,
    )
    .await
    .expect("second workflow");

    let err = update_company_workflow(
        &company,
        None,
        &store,
        &revs(),
        None,
        valid_draft("greeter", "OTHER"),
        None,
        None,
    )
    .await
    .expect_err("sibling name is taken");
    assert!(matches!(err, OpenCompanyError::Conflict(_)), "{err:?}");
}

/// An edit runs the same shape validation a create does.
#[tokio::test]
async fn a_bad_edit_is_refused_on_the_same_terms_as_a_bad_create() {
    let company = CompanyId::new("acme");
    let (store, _) = with_one_workflow(&company, "greeter", "Greeter").await;

    // Zero triggers.
    let mut no_trigger = valid_draft("greeter", "Greeter");
    no_trigger.nodes[0].kind = "output".to_string();
    let err = update_company_workflow(
        &company,
        None,
        &store,
        &revs(),
        None,
        no_trigger,
        None,
        None,
    )
    .await
    .expect_err("no trigger");
    assert!(err.to_string().contains("exactly one `trigger`"), "{err}");

    // Off-roster teammate.
    let mut ghost = valid_draft("greeter", "Greeter");
    ghost.nodes[1].agent = Some("ghost".to_string());
    let err = update_company_workflow(&company, None, &store, &revs(), None, ghost, None, None)
        .await
        .expect_err("off-roster teammate");
    assert!(
        matches!(err, OpenCompanyError::InvalidRequest(_)),
        "{err:?}"
    );

    // And nothing was persisted by either attempt.
    let record = store.load(&company).await.unwrap().unwrap();
    assert_eq!(record.overlay_workflows.len(), 1);
    let current = load_workflow_union(None, &record.overlay_workflows, "greeter")
        .unwrap()
        .unwrap();
    assert_eq!(current.nodes.len(), 3);
}

/// **The core overlay-only rule for update.** A seed-backed id is refused,
/// because `load_workflow_union` gives the seed file precedence — persisting
/// the edit would store a graph the read path never serves.
#[tokio::test]
async fn updating_a_seed_backed_workflow_is_a_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let workflows = dir.path().join("workflows");
    std::fs::create_dir_all(&workflows).unwrap();
    std::fs::write(workflows.join("seeded.toml"), SEED_TOML).unwrap();

    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));

    let err = update_company_workflow(
        &company,
        Some(dir.path()),
        &store,
        &revs(),
        None,
        valid_draft("seeded", "Seeded flow"),
        None,
        None,
    )
    .await
    .expect_err("a source-defined workflow is not editable");
    assert!(matches!(err, OpenCompanyError::Conflict(_)), "{err:?}");
    assert!(err.to_string().contains("source tree"), "{err}");
}

#[tokio::test]
async fn updating_an_unknown_workflow_is_not_found() {
    let company = CompanyId::new("acme");
    let (store, _) = with_one_workflow(&company, "greeter", "Greeter").await;
    let err = update_company_workflow(
        &company,
        None,
        &store,
        &revs(),
        None,
        valid_draft("ghost", "Ghost"),
        None,
        None,
    )
    .await
    .expect_err("unknown id");
    assert!(
        matches!(err, OpenCompanyError::CompanyNotFound(_)),
        "{err:?}"
    );
}

/// A manifest-`enabled` id with no body in either source has nothing to
/// replace — a 409 that says so beats a 404 that implies it never existed.
#[tokio::test]
async fn updating_a_bodiless_enabled_id_is_a_conflict() {
    let company = CompanyId::new("acme");
    let mut rec = record(&company, manifest_with_assistant());
    rec.manifest.workflows.enabled.push("legacy".to_string());
    let store = store_of(MemStore::seeded(rec));

    let err = update_company_workflow(
        &company,
        None,
        &store,
        &revs(),
        None,
        valid_draft("legacy", "Legacy"),
        None,
        None,
    )
    .await
    .expect_err("no body to replace");
    assert!(matches!(err, OpenCompanyError::Conflict(_)), "{err:?}");
}

/// An edit must not reshuffle the picker.
#[tokio::test]
async fn an_edit_preserves_overlay_order() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    for (id, name) in [("a", "Alpha"), ("b", "Bravo"), ("c", "Charlie")] {
        create_company_workflow(
            &company,
            None,
            &store,
            None,
            valid_draft(id, name),
            None,
            None,
        )
        .await
        .expect("seed");
    }

    let mut draft = valid_draft("a", "Alpha");
    draft.description = Some("Edited.".to_string());
    update_company_workflow(&company, None, &store, &revs(), None, draft, None, None)
        .await
        .expect("edit the first");

    let record = store.load(&company).await.unwrap().unwrap();
    let ids: Vec<&str> = record
        .overlay_workflows
        .iter()
        .map(|w| w.id.as_str())
        .collect();
    assert_eq!(ids, vec!["a", "b", "c"], "an edit must not reorder");
}

// --- #259: delete ------------------------------------------------------

#[tokio::test]
async fn deletes_the_body_and_the_enabled_id_and_journals() {
    let company = CompanyId::new("acme");
    let (store, version) = with_one_workflow(&company, "greeter", "Greeter").await;
    let log = Arc::new(MemLog::default());
    let log_dyn: Arc<dyn EventLog> = log.clone();

    let name = delete_company_workflow(
        &company,
        None,
        &store,
        &revs(),
        Some(&fires()),
        Some(&log_dyn),
        "greeter",
        Some(&version),
    )
    .await
    .expect("deletes");
    assert_eq!(name, "Greeter");

    let record = store.load(&company).await.unwrap().unwrap();
    // BOTH halves gone, in one save. Either alone would leave a workflow
    // that is half-present: a listed id with no graph, or a graph the
    // scheduler still fires.
    assert!(record.overlay_workflows.is_empty(), "body must be gone");
    assert!(
        record.manifest.workflows.enabled.is_empty(),
        "enabled id must be gone"
    );
    assert!(
        load_workflow_union(None, &record.overlay_workflows, "greeter")
            .unwrap()
            .is_none(),
        "the union read path must no longer serve it"
    );

    let events = log.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    match &events[0] {
        CompanyEvent::WorkflowDeleted {
            workflow_id, name, ..
        } => {
            assert_eq!(workflow_id, "greeter");
            assert_eq!(name, "Greeter");
        }
        other => panic!("expected WorkflowDeleted, got {other:?}"),
    }
}

/// Issue #1017: deleting a paused workflow must purge its pause flag, so a
/// workflow later re-created under the same id starts armed instead of
/// inheriting a stale pause the operator never asked for. `disabled_workflows`
/// is keyed by id, and a re-created id reuses it, so a leftover entry would
/// silently keep the fresh workflow off its schedule.
#[tokio::test]
async fn deleting_a_paused_workflow_purges_the_stale_pause_for_a_re_create() {
    let company = CompanyId::new("acme");
    let (store, version) = with_one_workflow(&company, "greeter", "Greeter").await;

    // Pause it — the id lands in `disabled_workflows`.
    set_company_workflow_enabled(&company, None, &store, None, "greeter", false, true, &[])
        .await
        .expect("pausing must never be refused");
    let paused = store.load(&company).await.unwrap().unwrap();
    assert!(
        !paused.workflow_enabled("greeter"),
        "precondition: the workflow is paused"
    );

    // Delete it, then re-create the same id from scratch.
    delete_company_workflow(
        &company,
        None,
        &store,
        &revs(),
        None,
        None,
        "greeter",
        Some(&version),
    )
    .await
    .expect("deletes");
    create_company_workflow(
        &company,
        None,
        &store,
        None,
        valid_draft("greeter", "Greeter"),
        None,
        None,
    )
    .await
    .expect("re-create");

    let record = store.load(&company).await.unwrap().unwrap();
    assert!(
        !record.disabled_workflows.iter().any(|id| id == "greeter"),
        "the delete must purge the stale pause flag"
    );
    assert!(
        record.workflow_enabled("greeter"),
        "a re-created workflow must start armed, not inherit the deleted one's pause"
    );
}

/// Issue #1045: the REST create/delete persist path puts
/// `WorkflowCreated` / `WorkflowDeleted` on a stream a **live subscriber**
/// actually receives — the in-process delivery the console's SSE picker
/// depends on. A green characterization: it locates the reported "graph
/// authored elsewhere stays invisible" defect on the console side, not in a
/// dropped host frame.
///
/// The projection of these variants onto the `{type, workflowId, name}` wire
/// frame the console keys on is asserted next to `project_event` itself
/// (`server::operator` — `projects_workflow_created_without_the_actor`,
/// `projects_workflow_updated_and_deleted_without_the_actor`). This test
/// closes the remaining link: that the persist path emits those variants,
/// carrying the same id and name, onto a stream `subscribe` delivers.
#[tokio::test]
async fn create_and_delete_reach_a_live_subscriber() {
    use futures::StreamExt;

    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    let log = Arc::new(BroadcastMemLog::new());
    let log_dyn: Arc<dyn EventLog> = log.clone();
    // Subscribe before the writes, exactly as the SSE handler does.
    let mut stream = log_dyn.subscribe(&company);

    create_company_workflow(
        &company,
        None,
        &store,
        Some(&log_dyn),
        valid_draft("greeter", "Greeter"),
        None,
        None,
    )
    .await
    .expect("creates");

    let created = stream
        .next()
        .await
        .expect("workflow_created delivered live");
    let created_event = match created {
        crate::ports::events::EventStreamItem::Event(ev) => ev,
        other => panic!("expected a live Event frame, got {other:?}"),
    };
    match &created_event.event {
        CompanyEvent::WorkflowCreated {
            workflow_id, name, ..
        } => {
            assert_eq!(workflow_id, "greeter");
            assert_eq!(name, "Greeter");
        }
        other => panic!("expected WorkflowCreated on the wire, got {other:?}"),
    }

    // Delete the same graph over the same persist path. `None` expected
    // version skips the optimistic-concurrency check — this test is about
    // the emitted frame, not the token.
    delete_company_workflow(
        &company,
        None,
        &store,
        &revs(),
        Some(&fires()),
        Some(&log_dyn),
        "greeter",
        None,
    )
    .await
    .expect("deletes");

    let deleted = stream
        .next()
        .await
        .expect("workflow_deleted delivered live");
    let deleted_event = match deleted {
        crate::ports::events::EventStreamItem::Event(ev) => ev,
        other => panic!("expected a live Event frame, got {other:?}"),
    };
    match &deleted_event.event {
        CompanyEvent::WorkflowDeleted {
            workflow_id, name, ..
        } => {
            assert_eq!(workflow_id, "greeter");
            assert_eq!(name, "Greeter");
        }
        other => panic!("expected WorkflowDeleted on the wire, got {other:?}"),
    }
}

/// #708: a committed delete purges the schedule's durable fire ledger under
/// the exact `workflow-<id>` key, so a recreated same-id workflow inherits
/// no anchor and no stale claim.
#[tokio::test]
async fn deleting_a_workflow_purges_its_schedule_fire_ledger() {
    let company = CompanyId::new("acme");
    let (store, _) = with_one_workflow(&company, "greeter", "Greeter").await;

    // A ledger for greeter's schedule, plus a sibling schedule's row to
    // prove the purge is scoped to exactly the deleted workflow's key.
    let fires = Arc::new(MemFires::default());
    let greeter_key = workflow_schedule_id("greeter");
    fires.seed(&company, &greeter_key, 100);
    fires.seed(&company, &greeter_key, 101);
    fires.seed(&company, &workflow_schedule_id("other"), 100);
    let fires_dyn: Arc<dyn ScheduleFireStore> = fires.clone();

    delete_company_workflow(
        &company,
        None,
        &store,
        &revs(),
        Some(&fires_dyn),
        None,
        "greeter",
        None,
    )
    .await
    .expect("deletes");

    assert!(
        fires.minutes(&company, &greeter_key).is_empty(),
        "the deleted workflow's whole fire ledger is purged"
    );
    assert_eq!(
        fires.minutes(&company, &workflow_schedule_id("other")),
        vec![100],
        "a sibling workflow's schedule ledger is untouched"
    );
}

/// #708: the purge is best-effort. A purge failure is logged, never rolled
/// back — the workflow is already gone, so the delete still succeeds.
#[tokio::test]
async fn a_failing_fire_ledger_purge_still_deletes_the_workflow() {
    let company = CompanyId::new("acme");
    let (store, _) = with_one_workflow(&company, "greeter", "Greeter").await;

    let fires = Arc::new(MemFires::default());
    fires.seed(&company, &workflow_schedule_id("greeter"), 100);
    fires.arm_delete_failure();
    let fires_dyn: Arc<dyn ScheduleFireStore> = fires.clone();

    let name = delete_company_workflow(
        &company,
        None,
        &store,
        &revs(),
        Some(&fires_dyn),
        None,
        "greeter",
        None,
    )
    .await
    .expect("delete succeeds even when the purge cascade errors");
    assert_eq!(name, "Greeter");

    // The graph is gone despite the purge error.
    let record = store.load(&company).await.unwrap().unwrap();
    assert!(record.overlay_workflows.is_empty(), "body must be gone");
    assert!(record.manifest.workflows.enabled.is_empty());
}

/// The delete is durable across the #208 boot rebuild *because* the overlay
/// body is gone: `merge_enabled_workflows` re-derives `enabled` from seed
/// ids ∪ surviving overlay ids, so there is nothing left to resurrect. This
/// pins the invariant the delete's correctness rests on.
#[tokio::test]
async fn a_deleted_workflow_has_nothing_left_for_the_boot_merge_to_re_enable() {
    let company = CompanyId::new("acme");
    let (store, _) = with_one_workflow(&company, "greeter", "Greeter").await;
    delete_company_workflow(
        &company,
        None,
        &store,
        &revs(),
        Some(&fires()),
        None,
        "greeter",
        None,
    )
    .await
    .expect("deletes");

    let record = store.load(&company).await.unwrap().unwrap();
    let surviving: Vec<&str> = record
        .overlay_workflows
        .iter()
        .map(|w| w.id.as_str())
        .collect();
    assert!(
        !surviving.contains(&"greeter"),
        "no overlay body means the boot merge cannot re-enable it"
    );
    assert!(list_workflows_union(None, &record.overlay_workflows).is_empty());
}

#[tokio::test]
async fn deleting_with_a_stale_version_is_refused_and_keeps_the_workflow() {
    let company = CompanyId::new("acme");
    let (store, stale) = with_one_workflow(&company, "greeter", "Greeter").await;

    let mut theirs = valid_draft("greeter", "Greeter");
    theirs.description = Some("Edited after you loaded it.".to_string());
    update_company_workflow(&company, None, &store, &revs(), None, theirs, None, None)
        .await
        .expect("someone edits first");

    // A held fire store, seeded under the workflow's schedule key, proves the
    // refused delete purges NOTHING — the purge runs only after a committed
    // save, so a version-refused delete (which never saves) leaves the live
    // workflow's ledger intact (#708).
    let fires = Arc::new(MemFires::default());
    fires.seed(&company, &workflow_schedule_id("greeter"), 42);
    let fires_dyn: Arc<dyn ScheduleFireStore> = fires.clone();

    let err = delete_company_workflow(
        &company,
        None,
        &store,
        &revs(),
        Some(&fires_dyn),
        None,
        "greeter",
        Some(&stale),
    )
    .await
    .expect_err("stale delete must be refused");
    assert!(matches!(err, OpenCompanyError::Conflict(_)), "{err:?}");

    let record = store.load(&company).await.unwrap().unwrap();
    assert_eq!(record.overlay_workflows.len(), 1, "nothing was removed");
    assert_eq!(
        fires.minutes(&company, &workflow_schedule_id("greeter")),
        vec![42],
        "a version-refused delete never reaches the purge — the ledger is intact"
    );
}

/// Deleting a source-defined workflow is refused: `merge_enabled_workflows`
/// would re-enable it from the seed id on the next boot, so the console
/// would be promising a removal it cannot keep.
#[tokio::test]
async fn deleting_a_seed_backed_workflow_is_a_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let workflows = dir.path().join("workflows");
    std::fs::create_dir_all(&workflows).unwrap();
    std::fs::write(workflows.join("seeded.toml"), SEED_TOML).unwrap();

    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));

    let err = delete_company_workflow(
        &company,
        Some(dir.path()),
        &store,
        &revs(),
        Some(&fires()),
        None,
        "seeded",
        None,
    )
    .await
    .expect_err("a source-defined workflow is not deletable");
    assert!(matches!(err, OpenCompanyError::Conflict(_)), "{err:?}");
    assert!(err.to_string().contains("source tree"), "{err}");
    // And the seed file is untouched — this path never writes to the tree.
    assert!(workflows.join("seeded.toml").is_file());
}

#[tokio::test]
async fn deleting_an_unknown_workflow_is_not_found() {
    let company = CompanyId::new("acme");
    let (store, _) = with_one_workflow(&company, "greeter", "Greeter").await;
    let err = delete_company_workflow(
        &company,
        None,
        &store,
        &revs(),
        Some(&fires()),
        None,
        "ghost",
        None,
    )
    .await
    .expect_err("unknown id");
    assert!(
        matches!(err, OpenCompanyError::CompanyNotFound(_)),
        "{err:?}"
    );
}

#[tokio::test]
async fn deleting_a_traversal_id_is_invalid() {
    let company = CompanyId::new("acme");
    let (store, _) = with_one_workflow(&company, "greeter", "Greeter").await;
    let err = delete_company_workflow(
        &company,
        None,
        &store,
        &revs(),
        Some(&fires()),
        None,
        "../secrets",
        None,
    )
    .await
    .expect_err("traversal id");
    assert!(
        matches!(err, OpenCompanyError::InvalidRequest(_)),
        "{err:?}"
    );
}

/// Only the named workflow goes; siblings keep their bodies and their
/// enabled ids.
#[tokio::test]
async fn deleting_one_leaves_the_others_alone() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    for (id, name) in [("a", "Alpha"), ("b", "Bravo"), ("c", "Charlie")] {
        create_company_workflow(
            &company,
            None,
            &store,
            None,
            valid_draft(id, name),
            None,
            None,
        )
        .await
        .expect("seed");
    }

    delete_company_workflow(
        &company,
        None,
        &store,
        &revs(),
        Some(&fires()),
        None,
        "b",
        None,
    )
    .await
    .expect("deletes the middle one");

    let record = store.load(&company).await.unwrap().unwrap();
    let ids: Vec<&str> = record
        .overlay_workflows
        .iter()
        .map(|w| w.id.as_str())
        .collect();
    assert_eq!(ids, vec!["a", "c"]);
    assert_eq!(record.manifest.workflows.enabled, vec!["a", "c"]);
}

/// A save failure leaves the record exactly as it was — the same one-save
/// atomicity create relies on.
#[tokio::test]
async fn a_failing_save_leaves_the_workflow_in_place() {
    let company = CompanyId::new("acme");
    let mut rec = record(&company, manifest_with_assistant());
    rec.overlay_workflows.push(OverlayWorkflow {
        id: "greeter".to_string(),
        toml: render_workflow(&valid_draft("greeter", "Greeter")).unwrap(),
    });
    rec.manifest.workflows.enabled.push("greeter".to_string());
    let store = store_of(MemStore::failing(rec));

    delete_company_workflow(
        &company,
        None,
        &store,
        &revs(),
        Some(&fires()),
        None,
        "greeter",
        None,
    )
    .await
    .expect_err("save fails");

    let record = store.load(&company).await.unwrap().unwrap();
    assert_eq!(record.overlay_workflows.len(), 1, "nothing was removed");
    assert_eq!(record.manifest.workflows.enabled, vec!["greeter"]);
}

// --- #259: the version token itself ------------------------------------

#[test]
fn the_version_token_is_stable_and_body_derived() {
    let a = workflow_version("id = \"x\"\n");
    assert_eq!(a, workflow_version("id = \"x\"\n"), "must be deterministic");
    assert_ne!(
        a,
        workflow_version("id = \"y\"\n"),
        "a different body must produce a different token"
    );
    // Hex sha256: 64 lowercase hex characters, so it is safe in a URL query
    // without escaping (the DELETE route passes it as `?expectedVersion=`).
    assert_eq!(a.len(), 64, "{a}");
    assert!(
        a.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
        "{a}"
    );
}

// --- #540: author-time tool_call validation ----------------------------

/// A manifest with the `assistant` roster agent AND an explicit
/// `[tools].allow`, so tool_call grant coverage can be exercised precisely.
///
/// Only the tool_call grant-coverage tests use this, and every one of them
/// is gated on `openhuman` (the tool catalogue lives behind that feature);
/// without the gate the helper is dead code at default features and trips
/// `-D warnings` in the default `Rust` CI lane.
#[cfg(feature = "openhuman")]
fn manifest_with_allow(allow: &[&str]) -> CompanyManifest {
    let list = allow
        .iter()
        .map(|grant| format!("\"{grant}\""))
        .collect::<Vec<_>>()
        .join(", ");
    toml::from_str(&format!(
        "[company]\nname = \"Acme\"\n[tools]\nallow = [{list}]\n[[agent]]\nid = \"assistant\"\nrole = \"Assistant\"\n"
    ))
    .expect("valid manifest")
}

/// A `trigger → tool_call` draft. `slug` of `None` omits `config` entirely,
/// so the ungated slug-presence check fires. Otherwise the node carries a
/// generic `config.args` table with every workflow-tool required-arg key set
/// (issue #813), so a positive-control slug clears the required-args arm — the
/// arm checks only presence, so the extra keys are harmless and this stays
/// feature-agnostic (no catalogue reference).
fn tool_call_draft(id: &str, name: &str, slug: Option<&str>) -> RawWorkflow {
    let mut args = toml::map::Map::new();
    for key in [
        "command",
        "edits",
        "operation",
        "data",
        "filename",
        "url",
        "path",
        "query",
    ] {
        args.insert(key.to_string(), toml::Value::String("x".to_string()));
    }
    tool_call_draft_args(id, name, slug, Some(toml::Value::Table(args)))
}

/// A `trigger → tool_call` draft with explicit control over `config.args` —
/// used to exercise the #813 required-args arm (absent args, present args)
/// directly. `args` of `None` omits the `args` table entirely.
fn tool_call_draft_args(
    id: &str,
    name: &str,
    slug: Option<&str>,
    args: Option<toml::Value>,
) -> RawWorkflow {
    let config = slug.map(|slug| {
        let mut table = toml::map::Map::new();
        table.insert("slug".to_string(), toml::Value::String(slug.to_string()));
        if let Some(args) = &args {
            table.insert("args".to_string(), args.clone());
        }
        toml::Value::Table(table)
    });
    RawWorkflow {
        id: id.to_string(),
        name: name.to_string(),
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
                id: "call".to_string(),
                kind: "tool_call".to_string(),
                name: "Call".to_string(),
                summary: None,
                agent: None,
                schedule: None,
                config,
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
            to: "call".to_string(),
            label: None,
        }],
    }
}

/// UNGATED: a tool_call with no `slug` is refused before any feature-specific
/// namespace resolution, so it fails the same way in every build.
#[tokio::test]
async fn tool_call_without_slug_is_invalid() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    let err = create_company_workflow(
        &company,
        None,
        &store,
        None,
        tool_call_draft("wf", "WF", None),
        None,
        None,
    )
    .await
    .expect_err("tool_call with no slug");
    assert!(
        matches!(err, OpenCompanyError::InvalidRequest(_)),
        "{err:?}"
    );
    assert!(err.to_string().contains("no `slug`"), "{err}");
}

/// A slug that maps to no toolbelt namespace is unwired — the run would halt
/// on it, so the save is refused with the run gate's own wording.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn tool_call_with_bogus_slug_is_invalid() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    let err = create_company_workflow(
        &company,
        None,
        &store,
        None,
        tool_call_draft("wf", "WF", Some("totally_bogus")),
        None,
        None,
    )
    .await
    .expect_err("unwired slug");
    assert!(
        matches!(err, OpenCompanyError::InvalidRequest(_)),
        "{err:?}"
    );
    assert!(
        err.to_string().contains("not a wired workflow tool"),
        "{err}"
    );
}

/// A wired slug whose namespace the company's `[tools].allow` does not cover
/// is refused; granting that namespace lets the same slug through.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn tool_call_slug_outside_the_granted_namespace_is_invalid() {
    let company = CompanyId::new("acme");
    // `web.*` grants `web` but NOT `code`; `csv_export` is a `code` tool.
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_allow(&["web.*"]),
    )));
    let err = create_company_workflow(
        &company,
        None,
        &store,
        None,
        tool_call_draft("wf", "WF", Some("csv_export")),
        None,
        None,
    )
    .await
    .expect_err("code slug not granted");
    assert!(
        matches!(err, OpenCompanyError::InvalidRequest(_)),
        "{err:?}"
    );
    assert!(err.to_string().contains("does not grant"), "{err}");

    // Positive control: grant `code` and the same slug is accepted.
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_allow(&["code"]),
    )));
    create_company_workflow(
        &company,
        None,
        &store,
        None,
        tool_call_draft("wf2", "WF2", Some("csv_export")),
        None,
        None,
    )
    .await
    .expect("code slug is granted");
}

/// Mirrors `caps/tools.rs`'s
/// `the_search_namespace_requires_an_explicit_grant_not_a_wildcard`: the
/// catch-all `*` never confers the priced `search` family, but an explicit
/// `search` grant does.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn the_search_namespace_needs_an_explicit_grant_not_a_wildcard() {
    let company = CompanyId::new("acme");
    // `*` covers ordinary namespaces but must NOT buy a managed search call.
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_allow(&["*"]),
    )));
    let err = create_company_workflow(
        &company,
        None,
        &store,
        None,
        tool_call_draft("wf", "WF", Some("web_search")),
        None,
        None,
    )
    .await
    .expect_err("wildcard never confers search");
    assert!(
        matches!(err, OpenCompanyError::InvalidRequest(_)),
        "{err:?}"
    );
    assert!(err.to_string().contains("does not grant"), "{err}");

    // An explicit `search` grant alongside the belt passes.
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_allow(&["*", "search"]),
    )));
    create_company_workflow(
        &company,
        None,
        &store,
        None,
        tool_call_draft("wf2", "WF2", Some("web_search")),
        None,
        None,
    )
    .await
    .expect("explicit search grant is honored");
}

/// The shared helper gates BOTH surfaces: an update into a graph with an
/// unwired tool_call slug is refused the same way a create is.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn update_gates_tool_calls_through_the_shared_helper() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    create_company_workflow(
        &company,
        None,
        &store,
        None,
        valid_draft("wf", "WF"),
        None,
        None,
    )
    .await
    .expect("seed create");

    let err = update_company_workflow(
        &company,
        None,
        &store,
        &revs(),
        None,
        tool_call_draft("wf", "WF", Some("totally_bogus")),
        None,
        None,
    )
    .await
    .expect_err("update must gate tool_calls too");
    assert!(
        matches!(err, OpenCompanyError::InvalidRequest(_)),
        "{err:?}"
    );
    assert!(
        err.to_string().contains("not a wired workflow tool"),
        "{err}"
    );
}

// --- output destinations (issue #981) ------------------------------------

/// [`valid_draft`] with the `output` node routed to `kind` / `target`.
fn draft_with_destination(
    id: &str,
    name: &str,
    kind: &str,
    target: Option<&str>,
) -> RawWorkflow {
    let mut draft = valid_draft(id, name);
    let output = draft
        .nodes
        .iter_mut()
        .find(|node| node.kind == "output")
        .expect("valid_draft has an output node");
    output.destination = Some(WorkflowDestinationDef {
        kind: kind.to_string(),
        target: target.map(str::to_string),
    });
    draft
}

/// Issue #1191, the core of the fix: an unwired `channel` target is refused
/// by the shared authoring core — so EVERY caller of it is held to the rule,
/// not just the two write routes that used to run it themselves.
///
/// The refusal is a located `WorkflowInvalid`, which is the second half of
/// the defect: it used to be a bare `InvalidRequest` with no `problems`
/// array, so the console got a flat banner for exactly the class of error
/// #836 asked for a highlight on.
#[tokio::test]
async fn an_unwired_channel_target_is_refused_by_the_authoring_core() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));

    let err = create_company_workflow(
        &company,
        None,
        &store,
        None,
        draft_with_destination("wf", "WF", "channel", Some("engineering-desk")),
        Some(&["engineering".to_string()]),
        None,
    )
    .await
    .expect_err("a channel nobody wired must not persist");

    let OpenCompanyError::WorkflowInvalid { problems } = &err else {
        panic!("expected a located `WorkflowInvalid`, got: {err}");
    };
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert_eq!(problems[0].node_id.as_deref(), Some("done"));
    assert_eq!(problems[0].field.as_deref(), Some("destination.target"));
    assert!(
        problems[0]
            .message
            .contains("is not an automation delivery channel"),
        "{:?}",
        problems[0]
    );
    // The live set rides in the sentence, so the fix is legible from the
    // refusal alone — the same message a failed delivery would have carried.
    assert!(
        problems[0].message.contains("engineering"),
        "{:?}",
        problems[0]
    );
}

/// A wired target on the same company saves. The rule refuses what delivery
/// would refuse and nothing more.
#[tokio::test]
async fn a_wired_channel_target_saves() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));

    create_company_workflow(
        &company,
        None,
        &store,
        None,
        draft_with_destination("wf", "WF", "channel", Some("engineering")),
        Some(&["engineering".to_string()]),
        None,
    )
    .await
    .expect("a wired channel is a destination this runtime can deliver to");
}

/// `None` is "the caller cannot see this deployment's wiring", not "nothing
/// is wired" — the same meaning `workflow_effective_tool_slugs` gives its
/// `wired` argument. The agent tool surfaces pass it, and their behaviour is
/// deliberately unchanged by #1191.
#[tokio::test]
async fn an_unseen_wiring_skips_the_channel_rule() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));

    create_company_workflow(
        &company,
        None,
        &store,
        None,
        draft_with_destination("wf", "WF", "channel", Some("engineering-desk")),
        None,
        None,
    )
    .await
    .expect("with no deliverable set in hand the rule is skipped, not guessed");
}

/// `Some(&[])` is a real answer, not a missing one: a company with no desk
/// and no provider channel can deliver nowhere, so every channel target is
/// refused and the sentence says so.
#[tokio::test]
async fn an_empty_deliverable_set_refuses_every_channel_target() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));

    let err = create_company_workflow(
        &company,
        None,
        &store,
        None,
        draft_with_destination("wf", "WF", "channel", Some("engineering")),
        Some(&[]),
        None,
    )
    .await
    .expect_err("nowhere to deliver means no channel target is honourable");
    assert!(err.to_string().contains("no durable channels"), "{err}");
}

/// The update path runs the same rule through the same helper, so an edit
/// cannot introduce a destination a create would have refused.
#[tokio::test]
async fn update_refuses_an_unwired_channel_target_too() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));

    create_company_workflow(
        &company,
        None,
        &store,
        None,
        valid_draft("wf", "WF"),
        None,
        None,
    )
    .await
    .expect("the base graph has no destination at all");

    let err = update_company_workflow(
        &company,
        None,
        &store,
        &revs(),
        None,
        draft_with_destination("wf", "WF", "channel", Some("engineering-desk")),
        None,
        Some(&["engineering".to_string()]),
    )
    .await
    .expect_err("an edit must be held to the create rule");
    assert!(
        matches!(err, OpenCompanyError::WorkflowInvalid { .. }),
        "{err}"
    );
}

/// The builder's courtesy pass runs the rule too (issue #1191), so a
/// proposal naming a channel nobody wired never reaches In Review — it
/// settles the card back to To-do with the reason instead.
#[cfg(feature = "openhuman")]
#[test]
fn courtesy_validation_refuses_an_unwired_channel_target() {
    let company = CompanyId::new("acme");
    let record = record(&company, manifest_with_assistant());

    let err = courtesy_validate_draft(
        &draft_with_destination("wf", "WF", "channel", Some("engineering-desk")),
        &record,
        None,
        Some(&["engineering".to_string()]),
        None,
    )
    .expect_err("the courtesy pass must refuse what apply would refuse");
    assert!(
        err.to_string()
            .contains("is not an automation delivery channel"),
        "{err}"
    );

    courtesy_validate_draft(
        &draft_with_destination("wf", "WF", "channel", Some("engineering")),
        &record,
        None,
        Some(&["engineering".to_string()]),
        None,
    )
    .expect("a wired target passes the same pass");
}

/// **Regression, issue #1882 review (PR #1882 bot finding, comment
/// 3879878907).** The courtesy pre-flight now takes the caller's stored
/// owning desk, so a caller that holds the saved body — the fix-from-run
/// copilot — gets the SAME grandfathering `update_company_workflow` applies:
/// a desk that went stale under an untouched field is carried, not refused.
///
/// RED-FIRST: pre-fix this function took no such argument and always
/// validated as a create, so the unchanged arm below was a `400`.
#[cfg(feature = "openhuman")]
#[test]
fn courtesy_validation_grandfathers_an_unchanged_stale_owner_desk() {
    let company = CompanyId::new("acme");
    let record = record(&company, manifest_with_assistant());
    let mut draft = valid_draft("wf", "WF");
    draft.owner_desk = Some("ghost-desk".to_string());

    courtesy_validate_draft(&draft, &record, None, None, Some("ghost-desk"))
        .expect("an unchanged owning desk is carried, not refused");

    // The create-shaped caller keeps the old, stricter verdict: with no
    // stored body to grandfather against, a desk naming nothing is a refusal.
    let err = courtesy_validate_draft(&draft, &record, None, None, None)
        .expect_err("a create-shaped pre-flight still refuses an unknown desk");
    let problems = problems_of(&err);
    assert_eq!(problems[0].field.as_deref(), Some("owner_desk"));

    // And grandfathering is scoped to the value that was already on file: a
    // DIFFERENT bad desk on the same edit is still refused.
    let err = courtesy_validate_draft(&draft, &record, None, None, Some("some-other-desk"))
        .expect_err("a newly named bad desk is not grandfathered by an edit");
    let problems = problems_of(&err);
    assert_eq!(problems[0].field.as_deref(), Some("owner_desk"));
}

/// Issue #1191: a `channel` destination with no `target` is refused with a
/// LOCATED problem — the node id and the config field — not a bare sentence.
///
/// The rule itself is old and lived only on the load path, where it is built
/// as a flat `String` and converted with `node_id: None, field: None`. The
/// console reads `problems` to highlight the offending node, so the one class
/// of error #836 was filed about rendered as prose it could not anchor.
#[tokio::test]
async fn a_channel_destination_with_no_target_names_the_node_and_the_field() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));

    let err = create_company_workflow(
        &company,
        None,
        &store,
        None,
        draft_with_destination("wf", "WF", "channel", None),
        None,
        None,
    )
    .await
    .expect_err("a channel destination with no target must not persist");

    let OpenCompanyError::WorkflowInvalid { problems } = &err else {
        panic!("expected a located `WorkflowInvalid`, got: {err}");
    };
    let problem = problems
        .iter()
        .find(|p| p.message.contains("name the channel to post the report to"))
        .unwrap_or_else(|| panic!("no channel-target problem in {problems:?}"));
    assert_eq!(problem.node_id.as_deref(), Some("done"));
    assert_eq!(problem.field.as_deref(), Some("destination.target"));
}

/// Issue #981: an `email` destination on a company whose `[tools].allow`
/// does not grant `email` is refused at SAVE, not silently accepted and then
/// denied on every run. Granting `email` lets the same graph through, which
/// is what proves the refusal is about the grant rather than about the kind.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn an_email_destination_needs_the_email_grant() {
    let company = CompanyId::new("acme");
    // `web.*` grants something, just not `email` — so a bare "no grants at
    // all" is not what the refusal is keying off.
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_allow(&["web.*"]),
    )));
    let err = create_company_workflow(
        &company,
        None,
        &store,
        None,
        draft_with_destination("wf", "WF", "email", Some("ops@example.com")),
        None,
        None,
    )
    .await
    .expect_err("email destination without the grant");
    assert!(
        matches!(err, OpenCompanyError::InvalidRequest(_)),
        "{err:?}"
    );
    assert!(err.to_string().contains("does not grant"), "{err}");
    assert!(err.to_string().contains("`done`"), "{err}");

    // Positive control: grant `email` and the same graph saves.
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_allow(&["email"]),
    )));
    create_company_workflow(
        &company,
        None,
        &store,
        None,
        draft_with_destination("wf2", "WF2", "email", Some("ops@example.com")),
        None,
        None,
    )
    .await
    .expect("email is granted");
}

/// The grant gate is scoped to `email`. An `owner` destination resolves
/// through the company's own directory and never sends to a named address,
/// so it must not be caught by the `email` rule — otherwise the fix for
/// #981 would refuse graphs that deliver perfectly well.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn an_owner_destination_is_not_gated_on_the_email_grant() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_allow(&["web.*"]),
    )));
    create_company_workflow(
        &company,
        None,
        &store,
        None,
        draft_with_destination("wf", "WF", "owner", None),
        None,
        None,
    )
    .await
    .expect("owner delivery needs no `email` grant");
}

/// The shared helper gates BOTH surfaces: an update that introduces an
/// ungranted `email` destination is refused the same way a create is.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn update_gates_email_destinations_through_the_shared_helper() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_allow(&["web.*"]),
    )));
    create_company_workflow(
        &company,
        None,
        &store,
        None,
        valid_draft("wf", "WF"),
        None,
        None,
    )
    .await
    .expect("seed create");

    let err = update_company_workflow(
        &company,
        None,
        &store,
        &revs(),
        None,
        draft_with_destination("wf", "WF", "email", Some("ops@example.com")),
        None,
        None,
    )
    .await
    .expect_err("update must gate email destinations too");
    assert!(
        matches!(err, OpenCompanyError::InvalidRequest(_)),
        "{err:?}"
    );
    assert!(err.to_string().contains("does not grant"), "{err}");
}

/// A slug padded with leading/trailing whitespace is rejected outright rather
/// than silently trimmed: the persisted config and the run-time lookup are
/// literal, so a padded slug that "passed" a trim-normalized check would halt
/// the run on the very lookup this save-time gate promised to catch.
/// (Regression, #540.)
#[tokio::test]
async fn tool_call_with_a_whitespace_padded_slug_is_rejected() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    let err = create_company_workflow(
        &company,
        None,
        &store,
        None,
        tool_call_draft("wf", "WF", Some(" csv_export ")),
        None,
        None,
    )
    .await
    .expect_err("padded slug");
    assert!(
        matches!(err, OpenCompanyError::InvalidRequest(_)),
        "{err:?}"
    );
    assert!(err.to_string().contains("whitespace"), "{err}");
}

/// `media` and `composio` are agent-turn tool families the workflow invoker
/// never wires (see `WORKFLOW_TOOL_NAMESPACES`), so a `tool_call` naming one
/// would clear the run-time grant gate and then ALWAYS miss the lookup.
/// Author-time validation rejects it up front even when the namespace is
/// explicitly granted, so the save mirrors the run. (Regression, #540.)
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn tool_call_in_an_agent_turn_only_family_is_rejected() {
    let company = CompanyId::new("acme");
    // Grant BOTH families explicitly, so the rejection is about the workflow
    // surface — not a missing grant.
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_allow(&["media", "composio"]),
    )));
    let err = create_company_workflow(
        &company,
        None,
        &store,
        None,
        tool_call_draft("wf", "WF", Some("media_generate_image")),
        None,
        None,
    )
    .await
    .expect_err("media is not a workflow tool family");
    assert!(
        matches!(err, OpenCompanyError::InvalidRequest(_)),
        "{err:?}"
    );
    assert!(err.to_string().contains("cannot run"), "{err}");
}

// --- #813: required `config.args` on a workflow tool_call -----------------

/// A granted `tool_call` whose required `config.args` are absent is refused at
/// author time, naming the missing args — the same gate the create-time
/// copilot hears via courtesy validation. `csv_export` needs `data` and
/// `filename`; the run would otherwise export nothing.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn tool_call_missing_required_args_is_invalid() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_allow(&["code"]),
    )));
    let err = create_company_workflow(
        &company,
        None,
        &store,
        None,
        tool_call_draft_args("wf", "WF", Some("csv_export"), None),
        None,
        None,
    )
    .await
    .expect_err("missing required args");
    assert!(
        matches!(err, OpenCompanyError::InvalidRequest(_)),
        "{err:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("config.args") && msg.contains("data") && msg.contains("filename"),
        "the missing args are named: {msg}"
    );
}

/// The same slug WITH its required args under `config.args` is accepted — the
/// arm gates the absence, not the tool. A `=`-expression counts as present.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn tool_call_with_required_args_is_accepted() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_allow(&["code"]),
    )));
    let mut args = toml::map::Map::new();
    args.insert(
        "data".to_string(),
        toml::Value::String("=nodes.pick.items".to_string()),
    );
    args.insert(
        "filename".to_string(),
        toml::Value::String("out.csv".to_string()),
    );
    create_company_workflow(
        &company,
        None,
        &store,
        None,
        tool_call_draft_args(
            "wf",
            "WF",
            Some("csv_export"),
            Some(toml::Value::Table(args)),
        ),
        None,
        None,
    )
    .await
    .expect("required args present");
}

/// `read_workspace_state` has NO required args, so an empty-args node is not
/// blocked by the arm — its inability to read a file is a grounding concern
/// (the copilot's honest capability line), not an author-time gate.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn tool_call_with_no_required_args_is_accepted_empty() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_allow(&["shell"]),
    )));
    create_company_workflow(
        &company,
        None,
        &store,
        None,
        tool_call_draft_args("wf", "WF", Some("read_workspace_state"), None),
        None,
        None,
    )
    .await
    .expect("read_workspace_state needs no args");
}

/// A required arg that is PRESENT but blank (a whitespace-only string or an
/// empty array/table) counts as missing — presence alone is not enough, since
/// a `""` filename would export to nowhere. This is the branch that carries
/// the real difference from a plain `contains_key` check.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn tool_call_with_a_blank_required_arg_is_invalid() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_allow(&["code"]),
    )));
    let mut args = toml::map::Map::new();
    // Empty array and a whitespace-only string: both present, both unusable.
    args.insert("data".to_string(), toml::Value::Array(Vec::new()));
    args.insert(
        "filename".to_string(),
        toml::Value::String("   ".to_string()),
    );
    let err = create_company_workflow(
        &company,
        None,
        &store,
        None,
        tool_call_draft_args(
            "wf",
            "WF",
            Some("csv_export"),
            Some(toml::Value::Table(args)),
        ),
        None,
        None,
    )
    .await
    .expect_err("blank required args count as missing");
    let msg = err.to_string();
    assert!(
        msg.contains("data") && msg.contains("filename"),
        "both blank args are named: {msg}"
    );
}

// --- issue #661/#682: required config + condition labels on the draft path

/// A minimal draft — trigger → condition `gate` → two outputs — with the
/// gate's `config.field` and both branch labels parameterised. Since
/// `parse_workflow` is now lenient on the #661 rules (issue #682), these are
/// the graphs that prove the create/update path still enforces them strictly.
fn condition_draft(
    field: Option<&str>,
    yes_label: Option<&str>,
    no_label: Option<&str>,
) -> RawWorkflow {
    let config = field.map(|field| {
        let mut table = toml::map::Map::new();
        table.insert("field".to_string(), toml::Value::String(field.to_string()));
        toml::Value::Table(table)
    });
    let node = |id: &str, kind: &str, config: Option<toml::Value>| RawNode {
        id: id.to_string(),
        kind: kind.to_string(),
        name: id.to_string(),
        summary: None,
        agent: None,
        schedule: None,
        config,
        on_error: None,
        retry: None,
        requires_approval: None,
        repeatable: None,
        destination: None,
        postcondition: None,
        verify: None,
    };
    RawWorkflow {
        id: "wf".to_string(),
        name: "WF".to_string(),
        description: None,
        owner_desk: None,
        nodes: vec![
            node("start", "trigger", None),
            node("gate", "condition", config),
            node("a", "output", None),
            node("b", "output", None),
        ],
        edges: vec![
            RawEdge {
                from: "start".to_string(),
                to: "gate".to_string(),
                label: None,
            },
            RawEdge {
                from: "gate".to_string(),
                to: "a".to_string(),
                label: yes_label.map(str::to_string),
            },
            RawEdge {
                from: "gate".to_string(),
                to: "b".to_string(),
                label: no_label.map(str::to_string),
            },
        ],
    }
}

/// A condition draft with no `config.field` is refused at author time even
/// though `parse_workflow` would now let it load.
#[tokio::test]
async fn draft_condition_without_field_is_invalid() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    let err = create_company_workflow(
        &company,
        None,
        &store,
        None,
        condition_draft(None, Some("yes"), Some("no")),
        None,
        None,
    )
    .await
    .expect_err("condition with no field");
    // Issue #1016: the config gate now raises a structured `WorkflowInvalid`.
    let problems = match &err {
        OpenCompanyError::WorkflowInvalid { problems } => problems,
        other => panic!("{other:?}"),
    };
    assert_eq!(problems[0].node_id.as_deref(), Some("gate"));
    assert_eq!(problems[0].field.as_deref(), Some("config.field"));
    assert!(err.to_string().contains("config.field"), "{err}");
}

/// A condition branch labeled anything but `yes`/`no` is refused at author
/// time — the load path is lenient, so this rule now lives entirely here.
#[tokio::test]
async fn draft_condition_with_non_yes_no_label_is_invalid() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    let err = create_company_workflow(
        &company,
        None,
        &store,
        None,
        condition_draft(Some("=item.ok"), Some("pass"), Some("no")),
        None,
        None,
    )
    .await
    .expect_err("off-vocabulary condition label");
    assert!(
        matches!(err, OpenCompanyError::InvalidRequest(_)),
        "{err:?}"
    );
    assert!(err.to_string().contains("labeled `yes` or `no`"), "{err}");
}

/// The positive control: a condition with a `field` and `yes`/`no` branches
/// is accepted by the same author path.
#[tokio::test]
async fn draft_condition_with_field_and_yes_no_labels_is_valid() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    create_company_workflow(
        &company,
        None,
        &store,
        None,
        condition_draft(Some("=item.approved"), Some("yes"), Some("no")),
        None,
        None,
    )
    .await
    .expect("a well-formed condition draft is accepted");
}

/// An http_request draft missing BOTH `method` and `url` reports both in one
/// 400 — the draft path collects every required-config problem for a node,
/// not just the first, so a human/model iterating hears the full list.
#[tokio::test]
async fn draft_http_request_missing_method_and_url_reports_both() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    let draft = RawWorkflow {
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
                id: "fetch".to_string(),
                kind: "http_request".to_string(),
                name: "Fetch".to_string(),
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
            to: "fetch".to_string(),
            label: None,
        }],
    };
    let err = create_company_workflow(&company, None, &store, None, draft, None, None)
        .await
        .expect_err("http_request with no method or url");
    // Issue #1016: structured `WorkflowInvalid`, one problem per field, both
    // pinned to the offending node.
    let problems = match &err {
        OpenCompanyError::WorkflowInvalid { problems } => problems,
        other => panic!("{other:?}"),
    };
    assert!(
        problems
            .iter()
            .all(|p| p.node_id.as_deref() == Some("fetch")),
        "{problems:?}"
    );
    let fields: Vec<&str> = problems.iter().filter_map(|p| p.field.as_deref()).collect();
    assert!(fields.contains(&"config.method"), "{fields:?}");
    assert!(fields.contains(&"config.url"), "{fields:?}");
    let message = err.to_string();
    assert!(message.contains("config.method"), "{message}");
    assert!(message.contains("config.url"), "{message}");
}

// --- issue #276: arming, disarming, and the switch -----------------------

/// A draft whose trigger fires on `cron`.
fn scheduled_draft(id: &str, name: &str, cron: &str) -> RawWorkflow {
    let mut draft = valid_draft(id, name);
    draft.nodes[0].schedule = Some(cron.to_string());
    draft
}

/// A graph authored with a cron lands switched OFF.
///
/// The half of the disarm rule OpenHuman does not have, and the one that
/// matters most here: this function is also the orchestrator's
/// `create_workflow` tool, so this assertion is what stops an agent putting a
/// cron into production by writing one.
async fn create_scheduled(
    company: &CompanyId,
    dir: &std::path::Path,
    store: &Arc<dyn CompanyStore>,
    log: &Arc<dyn EventLog>,
    id: &str,
    name: &str,
    cron: &str,
) -> WorkflowFile {
    create_company_workflow(
        company,
        Some(dir),
        store,
        Some(log),
        scheduled_draft(id, name, cron),
        None,
        None,
    )
    .await
    .expect("creates")
}

#[tokio::test]
async fn creating_a_scheduled_workflow_leaves_it_switched_off() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    let log = Arc::new(MemLog::default());
    let log_dyn: Arc<dyn EventLog> = log.clone();

    create_scheduled(
        &company,
        dir.path(),
        &store,
        &log_dyn,
        "digest",
        "Digest",
        "0 9 * * *",
    )
    .await;

    let saved = store.load(&company).await.unwrap().unwrap();
    assert!(
        !saved.workflow_enabled("digest"),
        "a created schedule must not be armed"
    );
    // Still a normal, complete workflow otherwise — pausing stops the
    // schedule, not the workflow.
    assert_eq!(saved.overlay_workflows.len(), 1);
    assert!(
        saved
            .manifest
            .workflows
            .enabled
            .contains(&"digest".to_string()),
        "the manifest declaration is untouched by the arming decision"
    );

    // Journaled, and journaled as the rule rather than as a person.
    let events = log.events.lock().unwrap();
    let disarm = events
        .iter()
        .find(|e| matches!(e, CompanyEvent::WorkflowEnabledChanged { .. }))
        .expect("a disarm is journaled");
    match disarm {
        CompanyEvent::WorkflowEnabledChanged {
            workflow_id,
            enabled,
            reason,
            by,
            ..
        } => {
            assert_eq!(workflow_id, "digest");
            assert!(!enabled);
            assert_eq!(*reason, WorkflowEnabledReason::Disarmed);
            assert!(by.is_none(), "the rule is not a person");
        }
        other => panic!("expected WorkflowEnabledChanged, got {other:?}"),
    }
}

/// A graph authored WITHOUT a cron is armed, because there is nothing to
/// arm. The disarm rule must not make every manual workflow look paused in
/// the console.
#[tokio::test]
async fn creating_a_manual_workflow_leaves_it_switched_on() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    let log = Arc::new(MemLog::default());
    let log_dyn: Arc<dyn EventLog> = log.clone();

    create_company_workflow(
        &company,
        Some(dir.path()),
        &store,
        Some(&log_dyn),
        valid_draft("greeter", "Greeter"),
        None,
        None,
    )
    .await
    .expect("creates");

    let saved = store.load(&company).await.unwrap().unwrap();
    assert!(saved.workflow_enabled("greeter"));
    assert!(
        saved.disabled_workflows.is_empty(),
        "a manual workflow must not be listed as paused"
    );
    assert!(
        !log.events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, CompanyEvent::WorkflowEnabledChanged { .. })),
        "nothing changed, so nothing is journaled"
    );
}

/// **The safety-relevant half of issue #276.** An edit that turns a manual
/// workflow into a scheduled one switches it off, so a cron introduced by an
/// edit cannot fire before anyone has looked at it.
#[tokio::test]
async fn an_edit_that_adds_a_schedule_switches_the_workflow_off() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    let log = Arc::new(MemLog::default());
    let log_dyn: Arc<dyn EventLog> = log.clone();

    create_company_workflow(
        &company,
        Some(dir.path()),
        &store,
        Some(&log_dyn),
        valid_draft("greeter", "Greeter"),
        None,
        None,
    )
    .await
    .expect("creates");
    assert!(
        store
            .load(&company)
            .await
            .unwrap()
            .unwrap()
            .workflow_enabled("greeter"),
        "armed before the edit, so the assertion below is about the edit"
    );

    update_company_workflow(
        &company,
        Some(dir.path()),
        &store,
        &revs(),
        Some(&log_dyn),
        scheduled_draft("greeter", "Greeter", "0 8 * * *"),
        None,
        None,
    )
    .await
    .expect("updates");

    let saved = store.load(&company).await.unwrap().unwrap();
    assert!(
        !saved.workflow_enabled("greeter"),
        "an edit that adds a schedule must disarm it"
    );
}

/// Correcting an already-armed workflow's cron leaves it armed.
///
/// The deliberate limit of the rule: the reviewed decision is "automatic at
/// all", and that one has not changed. Disarming here would put a re-enable
/// click behind every typo fix.
#[tokio::test]
async fn changing_an_existing_schedule_does_not_disarm() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    let log = Arc::new(MemLog::default());
    let log_dyn: Arc<dyn EventLog> = log.clone();

    create_scheduled(
        &company,
        dir.path(),
        &store,
        &log_dyn,
        "digest",
        "Digest",
        "0 9 * * *",
    )
    .await;
    // The operator reviews it and arms it.
    set_company_workflow_enabled(
        &company,
        Some(dir.path()),
        &store,
        Some(&log_dyn),
        "digest",
        true,
        true,
        &[],
    )
    .await
    .expect("arms");

    update_company_workflow(
        &company,
        Some(dir.path()),
        &store,
        &revs(),
        Some(&log_dyn),
        scheduled_draft("digest", "Digest", "0 3 * * *"),
        None,
        None,
    )
    .await
    .expect("updates");

    assert!(
        store
            .load(&company)
            .await
            .unwrap()
            .unwrap()
            .workflow_enabled("digest"),
        "a cron correction must not disarm an already-armed workflow"
    );
}

/// An edit never arms. A paused workflow stays paused across a re-save, even
/// one that removes the schedule entirely — the rule has no arming
/// direction, which is what stops it arming by accident.
#[tokio::test]
async fn an_edit_never_re_arms_a_paused_workflow() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    let log = Arc::new(MemLog::default());
    let log_dyn: Arc<dyn EventLog> = log.clone();

    create_scheduled(
        &company,
        dir.path(),
        &store,
        &log_dyn,
        "digest",
        "Digest",
        "0 9 * * *",
    )
    .await;

    // Re-save with the schedule removed: still paused.
    update_company_workflow(
        &company,
        Some(dir.path()),
        &store,
        &revs(),
        Some(&log_dyn),
        valid_draft("digest", "Digest"),
        None,
        None,
    )
    .await
    .expect("updates");

    assert!(
        !store
            .load(&company)
            .await
            .unwrap()
            .unwrap()
            .workflow_enabled("digest"),
        "removing a schedule must not re-arm the workflow"
    );
}

/// The operator switch round-trips, journals once, and is idempotent.
#[tokio::test]
async fn the_operator_switch_toggles_and_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    let log = Arc::new(MemLog::default());
    let log_dyn: Arc<dyn EventLog> = log.clone();

    create_company_workflow(
        &company,
        Some(dir.path()),
        &store,
        Some(&log_dyn),
        valid_draft("greeter", "Greeter"),
        None,
        None,
    )
    .await
    .expect("creates");
    let before = log.events.lock().unwrap().len();

    let changed = set_company_workflow_enabled(
        &company,
        Some(dir.path()),
        &store,
        Some(&log_dyn),
        "greeter",
        false,
        true,
        &[],
    )
    .await
    .expect("pauses");
    assert!(changed, "the first toggle changes the record");
    assert!(
        !store
            .load(&company)
            .await
            .unwrap()
            .unwrap()
            .workflow_enabled("greeter")
    );

    // Setting the state it already holds writes nothing and journals nothing
    // — a double-click is a no-op, not a second audit entry.
    let changed = set_company_workflow_enabled(
        &company,
        Some(dir.path()),
        &store,
        Some(&log_dyn),
        "greeter",
        false,
        true,
        &[],
    )
    .await
    .expect("no-ops");
    assert!(!changed);
    assert_eq!(
        log.events.lock().unwrap().len(),
        before + 1,
        "only the real transition is journaled"
    );

    // And back on, journaled as an operator decision rather than the rule.
    assert!(
        set_company_workflow_enabled(
            &company,
            Some(dir.path()),
            &store,
            Some(&log_dyn),
            "greeter",
            true,
            true,
            &[],
        )
        .await
        .expect("arms")
    );
    let events = log.events.lock().unwrap();
    match events.last().expect("an event") {
        CompanyEvent::WorkflowEnabledChanged {
            enabled, reason, ..
        } => {
            assert!(enabled);
            assert_eq!(*reason, WorkflowEnabledReason::Operator);
        }
        other => panic!("expected WorkflowEnabledChanged, got {other:?}"),
    }
}

/// An id with no graph anywhere is a 404, and a manifest-`enabled` id with no
/// body is a 409 — there is no schedule to switch off in either case.
#[tokio::test]
async fn toggling_an_id_with_no_graph_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let mut seed = record(&company, manifest_with_assistant());
    seed.manifest.workflows.enabled.push("ghost".to_string());
    let store = store_of(MemStore::seeded(seed));

    let err = set_company_workflow_enabled(
        &company,
        Some(dir.path()),
        &store,
        None,
        "nowhere",
        false,
        true,
        &[],
    )
    .await
    .expect_err("unknown id");
    assert!(
        matches!(err, OpenCompanyError::CompanyNotFound(_)),
        "{err:?}"
    );

    let err = set_company_workflow_enabled(
        &company,
        Some(dir.path()),
        &store,
        None,
        "ghost",
        false,
        true,
        &[],
    )
    .await
    .expect_err("bodiless id");
    assert!(matches!(err, OpenCompanyError::Conflict(_)), "{err:?}");
}

/// A stored graph that no longer parses is pausable, and is NOT reported as
/// "provisioned by name only".
///
/// The bodiless-409 message says the id was provisioned by name — true for a
/// manifest entry with no graph, and false for a workflow whose saved body
/// simply broke. An earlier revision collapsed both into one branch by
/// swallowing `load_workflow_union`'s error, so a corrupt graph read back as
/// the wrong explanation with no way to act on it.
#[tokio::test]
async fn a_workflow_whose_stored_graph_no_longer_parses_can_still_be_paused() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let mut seed = record(&company, manifest_with_assistant());
    seed.overlay_workflows.push(OverlayWorkflow {
        id: "broken".to_string(),
        toml: "id = \"broken\"\nname = \"Broken\"\n".to_string(), // no nodes: fails validation
    });
    seed.manifest.workflows.enabled.push("broken".to_string());
    let store = store_of(MemStore::seeded(seed));
    let log = Arc::new(MemLog::default());
    let log_dyn: Arc<dyn EventLog> = log.clone();

    assert!(
        set_company_workflow_enabled(
            &company,
            Some(dir.path()),
            &store,
            Some(&log_dyn),
            "broken",
            false,
            true,
            &[],
        )
        .await
        .expect("an unreadable graph is still pausable")
    );
    assert!(
        !store
            .load(&company)
            .await
            .unwrap()
            .unwrap()
            .workflow_enabled("broken")
    );
    // Journals under the id, since there is no readable name to use.
    match log.events.lock().unwrap().last().expect("an event") {
        CompanyEvent::WorkflowEnabledChanged { name, .. } => assert_eq!(name, "broken"),
        other => panic!("expected WorkflowEnabledChanged, got {other:?}"),
    }
}

/// A **seed-defined** workflow can be paused, even though `PUT`/`DELETE`
/// refuse it with a 409.
///
/// This is the deliberate asymmetry, and the reason for it: an edit or a
/// delete would be undone by the read path's seed precedence and by the boot
/// rebuild, so refusing them is honesty about what the reader will do.
/// Pausing writes to the record, leaves the source tree alone, and only ever
/// removes capability — and without it an operator cannot stop a committed
/// cron without a redeploy, which is issue #276(a) with extra steps.
#[tokio::test]
async fn a_seed_defined_workflow_can_be_paused_even_though_it_cannot_be_edited() {
    let dir = tempfile::tempdir().unwrap();
    let workflows = dir.path().join("workflows");
    std::fs::create_dir_all(&workflows).unwrap();
    std::fs::write(workflows.join("seeded.toml"), SEED_TOML).unwrap();

    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));

    // Edit is refused, as it has been since #259 …
    let err = update_company_workflow(
        &company,
        Some(dir.path()),
        &store,
        &revs(),
        None,
        valid_draft("seeded", "Seeded flow"),
        None,
        None,
    )
    .await
    .expect_err("a seed-defined graph cannot be replaced");
    assert!(matches!(err, OpenCompanyError::Conflict(_)), "{err:?}");

    // … and the switch still works.
    assert!(
        set_company_workflow_enabled(
            &company,
            Some(dir.path()),
            &store,
            None,
            "seeded",
            false,
            true,
            &[],
        )
        .await
        .expect("pauses a seed-defined workflow")
    );
    let saved = store.load(&company).await.unwrap().unwrap();
    assert!(!saved.workflow_enabled("seeded"));
    assert!(
        saved.overlay_workflows.is_empty(),
        "pausing must not materialize an overlay body for a seed graph"
    );
}

// --- issue #274: revision capture + rollback -----------------------------

/// The overlay TOML currently stored for `wid`.
async fn current_toml(store: &Arc<dyn CompanyStore>, company: &CompanyId, wid: &str) -> String {
    store
        .load(company)
        .await
        .unwrap()
        .unwrap()
        .overlay_workflows
        .into_iter()
        .find(|w| w.id == wid)
        .expect("overlay body exists")
        .toml
}

/// Creates `greeter`, then returns `(store, revisions, body_a)` ready to edit.
async fn seeded_greeter() -> (Arc<dyn CompanyStore>, Arc<MemRevisions>, String) {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    create_company_workflow(
        &company,
        None,
        &store,
        None,
        valid_draft("greeter", "Greeter"),
        None,
        None,
    )
    .await
    .expect("create");
    let body_a = current_toml(&store, &company, "greeter").await;
    (store, Arc::new(MemRevisions::default()), body_a)
}

#[tokio::test]
async fn update_snapshots_the_prior_body_exactly_once() {
    let company = CompanyId::new("acme");
    let (store, revs, body_a) = seeded_greeter().await;
    let revs_dyn: Arc<dyn WorkflowRevisionStore> = revs.clone();

    // Edit the description so the rendered body differs from A.
    let mut edit = valid_draft("greeter", "Greeter");
    edit.description = Some("edited once".to_string());
    update_company_workflow(&company, None, &store, &revs_dyn, None, edit, None, None)
        .await
        .expect("update");

    let history = revs.list_revisions(&company, "greeter").await.unwrap();
    assert_eq!(history.len(), 1, "one edit captures one snapshot");
    assert_eq!(
        history[0].toml, body_a,
        "the snapshot must hold the prior body byte-for-byte"
    );
    assert_eq!(history[0].workflow_id, "greeter");
    assert_eq!(history[0].name, "Greeter");
}

#[tokio::test]
async fn a_byte_identical_resave_snapshots_nothing() {
    let company = CompanyId::new("acme");
    let (store, revs, _body_a) = seeded_greeter().await;
    let revs_dyn: Arc<dyn WorkflowRevisionStore> = revs.clone();

    // Re-save the exact same graph: the rendered body is byte-identical, so
    // there is nothing to lose and no snapshot is taken.
    update_company_workflow(
        &company,
        None,
        &store,
        &revs_dyn,
        None,
        valid_draft("greeter", "Greeter"),
        None,
        None,
    )
    .await
    .expect("no-op resave");
    assert!(
        revs.list_revisions(&company, "greeter")
            .await
            .unwrap()
            .is_empty(),
        "a byte-identical re-save must not snapshot"
    );
}

#[tokio::test]
async fn the_ring_prunes_the_oldest_past_the_cap() {
    use crate::ports::workflow_revisions::MAX_WORKFLOW_REVISIONS;
    let company = CompanyId::new("acme");
    let (store, revs, _body_a) = seeded_greeter().await;
    let revs_dyn: Arc<dyn WorkflowRevisionStore> = revs.clone();

    // MAX+1 distinct edits capture MAX+1 prior bodies; the ring keeps MAX.
    for i in 0..=MAX_WORKFLOW_REVISIONS {
        let mut edit = valid_draft("greeter", "Greeter");
        edit.description = Some(format!("edit {i}"));
        update_company_workflow(&company, None, &store, &revs_dyn, None, edit, None, None)
            .await
            .expect("update");
    }
    let history = revs.list_revisions(&company, "greeter").await.unwrap();
    assert_eq!(
        history.len(),
        MAX_WORKFLOW_REVISIONS,
        "the ring is capped at MAX_WORKFLOW_REVISIONS"
    );
}

#[tokio::test]
async fn rollback_restores_the_body_and_is_itself_undoable() {
    let company = CompanyId::new("acme");
    let (store, revs, body_a) = seeded_greeter().await;
    let revs_dyn: Arc<dyn WorkflowRevisionStore> = revs.clone();

    // A → edit → B. One revision now holds A.
    let mut edit_b = valid_draft("greeter", "Greeter");
    edit_b.description = Some("this is B".to_string());
    update_company_workflow(&company, None, &store, &revs_dyn, None, edit_b, None, None)
        .await
        .expect("edit to B");
    let body_b = current_toml(&store, &company, "greeter").await;
    let rev_a = revs.list_revisions(&company, "greeter").await.unwrap()[0]
        .id
        .clone();

    // Restore A: the live body becomes A again, and B is captured as the new
    // newest revision — so the rollback can itself be rolled back.
    let restored = rollback_company_workflow(
        &company, None, &store, &revs_dyn, None, "greeter", &rev_a, None,
    )
    .await
    .expect("rollback to A");
    assert_eq!(restored.description.as_deref(), Some("A tiny graph."));
    assert_eq!(current_toml(&store, &company, "greeter").await, body_a);

    let history = revs.list_revisions(&company, "greeter").await.unwrap();
    assert_eq!(
        history.len(),
        2,
        "the restore captured the body it replaced"
    );
    assert_eq!(history[0].toml, body_b, "B is the newest snapshot now");

    // …and restoring that B snapshot puts B back.
    let rev_b = history[0].id.clone();
    rollback_company_workflow(
        &company, None, &store, &revs_dyn, None, "greeter", &rev_b, None,
    )
    .await
    .expect("rollback the rollback");
    assert_eq!(current_toml(&store, &company, "greeter").await, body_b);
}

#[tokio::test]
async fn rollback_of_a_revision_naming_a_removed_teammate_is_400_and_leaves_current() {
    let company = CompanyId::new("acme");
    let (store, revs, _body_a) = seeded_greeter().await;
    let revs_dyn: Arc<dyn WorkflowRevisionStore> = revs.clone();

    // Edit to capture a revision (which still names `assistant`), then set B.
    let mut edit_b = valid_draft("greeter", "Greeter");
    edit_b.description = Some("B, assistant still valid here".to_string());
    update_company_workflow(&company, None, &store, &revs_dyn, None, edit_b, None, None)
        .await
        .expect("edit to B");
    let body_b = current_toml(&store, &company, "greeter").await;
    let rev_a = revs.list_revisions(&company, "greeter").await.unwrap()[0]
        .id
        .clone();

    // Remove `assistant` from the roster: the captured revision now names a
    // teammate the current record does not know about.
    let mut rec = store.load(&company).await.unwrap().unwrap();
    rec.manifest = toml::from_str("[company]\nname = \"Acme\"\n").unwrap();
    store.save(&rec).await.unwrap();

    let err = rollback_company_workflow(
        &company, None, &store, &revs_dyn, None, "greeter", &rev_a, None,
    )
    .await
    .expect_err("a revision naming a removed teammate must not restore");
    assert!(
        matches!(err, OpenCompanyError::InvalidRequest(_)),
        "{err:?}"
    );
    // The current body is untouched — a rejected rollback writes nothing.
    assert_eq!(current_toml(&store, &company, "greeter").await, body_b);
}

#[tokio::test]
async fn rollback_with_a_stale_expected_version_is_409_and_writes_nothing() {
    let company = CompanyId::new("acme");
    let (store, revs, body_a) = seeded_greeter().await;
    let revs_dyn: Arc<dyn WorkflowRevisionStore> = revs.clone();

    let mut edit_b = valid_draft("greeter", "Greeter");
    edit_b.description = Some("B".to_string());
    update_company_workflow(&company, None, &store, &revs_dyn, None, edit_b, None, None)
        .await
        .expect("edit to B");
    let body_b = current_toml(&store, &company, "greeter").await;
    let rev_a = revs.list_revisions(&company, "greeter").await.unwrap()[0]
        .id
        .clone();

    let err = rollback_company_workflow(
        &company,
        None,
        &store,
        &revs_dyn,
        None,
        "greeter",
        &rev_a,
        Some("deadbeef-not-the-current-token"),
    )
    .await
    .expect_err("a stale token must refuse the restore");
    assert!(matches!(err, OpenCompanyError::Conflict(_)), "{err:?}");
    assert_eq!(
        current_toml(&store, &company, "greeter").await,
        body_b,
        "a 409 must leave the live body unchanged"
    );

    // The response token of a successful restore is the hash of the restored
    // body — echo the CURRENT token and the restore lands.
    let current = workflow_version(&body_b);
    let restored = rollback_company_workflow(
        &company,
        None,
        &store,
        &revs_dyn,
        None,
        "greeter",
        &rev_a,
        Some(&current),
    )
    .await
    .expect("the current token lets the restore through");
    // The restored live body is the captured A body, and the token the write
    // response carries is that body's hash.
    let restored_toml = current_toml(&store, &company, "greeter").await;
    assert_eq!(restored_toml, body_a, "restoring A puts A back verbatim");
    assert_eq!(restored.id, "greeter");
    assert_eq!(
        workflow_version(&restored_toml),
        workflow_version(&body_a),
        "the response token is the restored body's hash"
    );
}

#[tokio::test]
async fn rollback_disarms_a_restored_schedule() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    let revs: Arc<MemRevisions> = Arc::new(MemRevisions::default());
    let revs_dyn: Arc<dyn WorkflowRevisionStore> = revs.clone();

    // A scheduled workflow lands disarmed on create (issue #276); arm it, so
    // the "restored cron re-arms" hazard is real to test.
    let mut scheduled = valid_draft("greeter", "Greeter");
    scheduled.nodes[0].schedule = Some("0 9 * * *".to_string());
    create_company_workflow(&company, None, &store, None, scheduled, None, None)
        .await
        .expect("create scheduled");
    set_company_workflow_enabled(&company, None, &store, None, "greeter", true, true, &[])
        .await
        .expect("arm it");

    // Edit the schedule away — the workflow stays armed (removal never
    // disarms), and the scheduled body is captured as a revision.
    let mut unscheduled = valid_draft("greeter", "Greeter");
    unscheduled.description = Some("no schedule now".to_string());
    update_company_workflow(
        &company,
        None,
        &store,
        &revs_dyn,
        None,
        unscheduled,
        None,
        None,
    )
    .await
    .expect("remove schedule");
    assert!(
        store
            .load(&company)
            .await
            .unwrap()
            .unwrap()
            .workflow_enabled("greeter"),
        "removing a schedule must not disarm"
    );
    let rev_scheduled = revs.list_revisions(&company, "greeter").await.unwrap()[0]
        .id
        .clone();

    // Restoring the scheduled body re-introduces a cron the live graph lacked
    // → it lands switched off pending review.
    rollback_company_workflow(
        &company,
        None,
        &store,
        &revs_dyn,
        None,
        "greeter",
        &rev_scheduled,
        None,
    )
    .await
    .expect("restore scheduled body");
    assert!(
        !store
            .load(&company)
            .await
            .unwrap()
            .unwrap()
            .workflow_enabled("greeter"),
        "a restored schedule must land disarmed (issue #276)"
    );
}

#[tokio::test]
async fn rollback_unknown_revision_is_not_found() {
    let company = CompanyId::new("acme");
    let (store, revs, _body_a) = seeded_greeter().await;
    let revs_dyn: Arc<dyn WorkflowRevisionStore> = revs.clone();
    let err = rollback_company_workflow(
        &company,
        None,
        &store,
        &revs_dyn,
        None,
        "greeter",
        "no-such-rev",
        None,
    )
    .await
    .expect_err("unknown revision");
    assert!(
        matches!(err, OpenCompanyError::CompanyNotFound(_)),
        "{err:?}"
    );
}

// --- issue #1016: structured, per-node/field workflow problems -----------

/// A bare node with an optional config table.
fn oc_node(id: &str, kind: &str, config: Option<toml::Value>) -> RawNode {
    RawNode {
        id: id.to_string(),
        kind: kind.to_string(),
        name: id.to_string(),
        summary: None,
        agent: None,
        schedule: None,
        config,
        on_error: None,
        retry: None,
        requires_approval: None,
        repeatable: None,
        destination: None,
        postcondition: None,
        verify: None,
    }
}

/// A `trigger → <node>` two-node draft, so the node under test sits on a
/// reachable, single-trigger graph the shape check accepts.
fn one_node_draft(node: RawNode) -> RawWorkflow {
    let to = node.id.clone();
    RawWorkflow {
        id: "wf".to_string(),
        name: "WF".to_string(),
        description: None,
        owner_desk: None,
        nodes: vec![oc_node("start", "trigger", None), node],
        edges: vec![RawEdge {
            from: "start".to_string(),
            to,
            label: None,
        }],
    }
}

fn problems_of(err: &OpenCompanyError) -> &[WorkflowProblem] {
    match err {
        OpenCompanyError::WorkflowInvalid { problems } => problems,
        other => panic!("expected WorkflowInvalid, got {other:?}"),
    }
}

/// RED-FIRST #1 (headline): a `transform` with no `config.set` is now rejected
/// at save, naming the node and `config.set`. Accepted on unpatched code.
#[tokio::test]
async fn draft_transform_without_set_is_rejected() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    let err = create_company_workflow(
        &company,
        None,
        &store,
        None,
        one_node_draft(oc_node("tf", "transform", None)),
        None,
        None,
    )
    .await
    .expect_err("transform with no config.set");
    let problems = problems_of(&err);
    assert_eq!(problems[0].node_id.as_deref(), Some("tf"));
    assert_eq!(problems[0].field.as_deref(), Some("config.set"));
}

/// RED-FIRST #1 (split_out half): a `split_out` with no `config.path` is
/// rejected, naming `config.path`.
#[tokio::test]
async fn draft_split_out_without_path_is_rejected() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    let err = create_company_workflow(
        &company,
        None,
        &store,
        None,
        one_node_draft(oc_node("so", "split_out", None)),
        None,
        None,
    )
    .await
    .expect_err("split_out with no config.path");
    let problems = problems_of(&err);
    assert_eq!(problems[0].node_id.as_deref(), Some("so"));
    assert_eq!(problems[0].field.as_deref(), Some("config.path"));
}

fn http_bad_url_draft() -> RawWorkflow {
    let mut config = toml::map::Map::new();
    config.insert("method".to_string(), toml::Value::String("GET".to_string()));
    config.insert(
        "url".to_string(),
        toml::Value::String("not-a-url".to_string()),
    );
    one_node_draft(oc_node(
        "greet",
        "http_request",
        Some(toml::Value::Table(config)),
    ))
}

/// RED-FIRST #2 (create): a create with an http_request `url` of `not-a-url`
/// yields a `WorkflowInvalid` whose first problem is pinned to `greet` /
/// `config.url`. Asserted on the STRUCT, not the joined message.
#[tokio::test]
async fn draft_http_request_bad_url_is_rejected_on_create() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    let err = create_company_workflow(
        &company,
        None,
        &store,
        None,
        http_bad_url_draft(),
        None,
        None,
    )
    .await
    .expect_err("http_request url = not-a-url");
    let problems = problems_of(&err);
    assert_eq!(problems[0].node_id.as_deref(), Some("greet"));
    assert_eq!(problems[0].field.as_deref(), Some("config.url"));
}

/// RED-FIRST #2 (update): the same structured rejection on the update path.
#[tokio::test]
async fn draft_http_request_bad_url_is_rejected_on_update() {
    let company = CompanyId::new("acme");
    let (store, version) = with_one_workflow(&company, "wf", "WF").await;
    let err = update_company_workflow(
        &company,
        None,
        &store,
        &revs(),
        None,
        http_bad_url_draft(),
        Some(&version),
        None,
    )
    .await
    .expect_err("update to a bad url");
    let problems = problems_of(&err);
    assert_eq!(problems[0].node_id.as_deref(), Some("greet"));
    assert_eq!(problems[0].field.as_deref(), Some("config.url"));
}

/// RED-FIRST #3: a create whose edge has a dangling `from` yields a structured
/// problem naming the endpoint (`old-id`) and the `from` field, and the
/// message no longer leads with `edge #N`.
#[tokio::test]
async fn draft_dangling_from_edge_is_structured() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    let draft = RawWorkflow {
        id: "wf".to_string(),
        name: "WF".to_string(),
        description: None,
        owner_desk: None,
        nodes: vec![oc_node("start", "trigger", None)],
        edges: vec![RawEdge {
            from: "old-id".to_string(),
            to: "start".to_string(),
            label: None,
        }],
    };
    let err = create_company_workflow(&company, None, &store, None, draft, None, None)
        .await
        .expect_err("dangling from");
    let problems = problems_of(&err);
    assert_eq!(problems[0].node_id.as_deref(), Some("old-id"));
    assert_eq!(problems[0].field.as_deref(), Some("from"));
    assert!(!err.to_string().contains("edge #"), "{err}");
}

fn sub_workflow_draft(workflow_id: &str) -> RawWorkflow {
    let mut config = toml::map::Map::new();
    config.insert(
        "workflow_id".to_string(),
        toml::Value::String(workflow_id.to_string()),
    );
    one_node_draft(oc_node(
        "child",
        "sub_workflow",
        Some(toml::Value::Table(config)),
    ))
}

/// A `sub_workflow` naming a workflow id that this company cannot resolve is
/// rejected, pinned to the node and `workflow_id`.
#[tokio::test]
async fn draft_sub_workflow_with_unknown_id_is_rejected() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    let mut draft = sub_workflow_draft("nope");
    draft.id = "parent".to_string();
    let err = create_company_workflow(&company, None, &store, None, draft, None, None)
        .await
        .expect_err("unknown sub-workflow id");
    let problems = problems_of(&err);
    assert_eq!(problems[0].node_id.as_deref(), Some("child"));
    assert_eq!(problems[0].field.as_deref(), Some("workflow_id"));
}

/// A `sub_workflow` referencing an existing saved workflow passes the record
/// cross-check.
#[tokio::test]
async fn draft_sub_workflow_with_existing_id_is_accepted() {
    let company = CompanyId::new("acme");
    let (store, _) = with_one_workflow(&company, "greeter", "Greeter").await;
    let mut draft = sub_workflow_draft("greeter");
    draft.id = "parent".to_string();
    draft.name = "Parent".to_string();
    create_company_workflow(&company, None, &store, None, draft, None, None)
        .await
        .expect("sub_workflow referencing a saved workflow is accepted");
}

/// A `sub_workflow` referencing its own id is still rejected (regression): the
/// structural self-reference check owns that message.
#[tokio::test]
async fn draft_sub_workflow_self_reference_is_rejected() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    // draft.id defaults to "wf" — a self reference to the same id.
    let draft = sub_workflow_draft("wf");
    let err = create_company_workflow(&company, None, &store, None, draft, None, None)
        .await
        .expect_err("self-referencing sub_workflow");
    assert!(err.to_string().contains("run itself"), "{err}");
}

/// An `output_parser` with no schema (a pass-through identity parser) is
/// accepted; a `merge` with no config is accepted — the gate does not
/// over-reject the config-optional kinds.
#[tokio::test]
async fn draft_output_parser_and_merge_are_config_optional() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    create_company_workflow(
        &company,
        None,
        &store,
        None,
        one_node_draft(oc_node("op", "output_parser", None)),
        None,
        None,
    )
    .await
    .expect("schema-less output_parser is accepted");

    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    create_company_workflow(
        &company,
        None,
        &store,
        None,
        one_node_draft(oc_node("mg", "merge", None)),
        None,
        None,
    )
    .await
    .expect("config-less merge is accepted");
}

/// A manifest with an `assistant` roster agent AND an `ops` desk that
/// agent sits on — issue #1862 prerequisite's "accept wired" case needs a
/// real desk to resolve against.
fn manifest_with_assistant_and_desk() -> CompanyManifest {
    toml::from_str(
        "[company]\nname = \"Acme\"\n[[agent]]\nid = \"assistant\"\nrole = \"Assistant\"\n\
         [[group_chat]]\nid = \"ops\"\nname = \"Ops\"\nmembers = [\"assistant\"]\n",
    )
    .expect("valid manifest")
}

/// Issue #1862 prerequisite, RED-FIRST: a draft naming an `owner_desk` that
/// resolves against NO desk on the company is rejected at author time,
/// naming the `owner_desk` field. Accepted on unpatched code, because the
/// field — and this check — did not exist.
#[tokio::test]
async fn draft_with_unknown_owner_desk_is_rejected() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    let mut draft = valid_draft("wf", "WF");
    draft.owner_desk = Some("ghost-desk".to_string());
    let err = create_company_workflow(&company, None, &store, None, draft, None, None)
        .await
        .expect_err("owner_desk naming no real desk");
    let problems = problems_of(&err);
    assert_eq!(problems[0].node_id, None, "graph-level, not node-scoped");
    assert_eq!(problems[0].field.as_deref(), Some("owner_desk"));
}

/// The accept half of the same gate: an `owner_desk` that resolves against
/// a real desk is accepted, and the saved graph carries it through.
#[tokio::test]
async fn draft_with_a_wired_owner_desk_is_accepted() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant_and_desk(),
    )));
    let mut draft = valid_draft("wf", "WF");
    draft.owner_desk = Some("ops".to_string());
    let file = create_company_workflow(&company, None, &store, None, draft, None, None)
        .await
        .expect("owner_desk naming a real desk is accepted");
    assert_eq!(file.owner_desk.as_deref(), Some("ops"));
}

/// A blank/whitespace `owner_desk` is treated as unset rather than
/// resolved against the desk set — the same "empty means absent" leniency
/// the rest of this draft's optional fields get.
#[tokio::test]
async fn draft_with_blank_owner_desk_is_accepted() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant(),
    )));
    let mut draft = valid_draft("wf", "WF");
    draft.owner_desk = Some("   ".to_string());
    create_company_workflow(&company, None, &store, None, draft, None, None)
        .await
        .expect("a blank owner_desk is not resolved against the desk set");
}

/// **Regression, issue #1882 review.** An edit to a field that has
/// nothing to do with `owner_desk` must still save when the workflow's
/// STORED desk has since been renamed or removed — the same "a field
/// nobody looked at" leniency `parse_workflow`'s lenient load path
/// already grants. Before the fix, the strict desk-exists check
/// re-validated the untouched, unchanged desk on every save and refused
/// unconditionally — so once a desk went stale, an operator could not
/// save ANY edit from an editor that (correctly, per the round-trip fix)
/// carries `ownerDesk` forward with no control to clear it.
#[tokio::test]
async fn an_unrelated_update_survives_a_desk_that_went_stale() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant_and_desk(),
    )));
    let mut created = valid_draft("wf", "WF");
    created.owner_desk = Some("ops".to_string());
    create_company_workflow(&company, None, &store, None, created, None, None)
        .await
        .expect("creates with a real desk");
    let saved = store.load(&company).await.unwrap().unwrap();
    let version = workflow_version(&saved.overlay_workflows[0].toml);

    // The desk is renamed/removed underneath the workflow — nothing about
    // the workflow itself is touched.
    let mut stale_record = store.load(&company).await.unwrap().unwrap();
    stale_record.manifest = manifest_with_assistant();
    store.save(&stale_record).await.unwrap();

    // An edit to a completely unrelated field, carrying the stored
    // owner_desk forward unchanged rather than re-typing it — exactly
    // what a console round-tripping the read does.
    let mut edit = valid_draft("wf", "WF");
    edit.owner_desk = Some("ops".to_string());
    edit.description = Some("Renamed the description only.".to_string());
    let file = update_company_workflow(
        &company,
        None,
        &store,
        &revs(),
        None,
        edit,
        Some(&version),
        None,
    )
    .await
    .expect("an unrelated edit must save even though the stored desk went stale");
    assert_eq!(
        file.owner_desk.as_deref(),
        Some("ops"),
        "the stale desk is grandfathered, not cleared"
    );

    // A DIFFERENT bad desk is still a refusal — grandfathering only covers
    // the value already on file, never a newly typed/selected one.
    let saved2 = store.load(&company).await.unwrap().unwrap();
    let version2 = workflow_version(&saved2.overlay_workflows[0].toml);
    let mut bad_edit = valid_draft("wf", "WF");
    bad_edit.owner_desk = Some("a-totally-different-ghost-desk".to_string());
    let err = update_company_workflow(
        &company,
        None,
        &store,
        &revs(),
        None,
        bad_edit,
        Some(&version2),
        None,
    )
    .await
    .expect_err("a newly typed desk that resolves to nothing is still refused");
    let problems = problems_of(&err);
    assert_eq!(problems[0].field.as_deref(), Some("owner_desk"));
}

/// **Regression, issue #1882 review ("preserve padded stale owners"),
/// RED-FIRST.** The grandfathering above compares the draft's
/// `owner_desk` against the STORED body's. The draft side is trimmed on
/// the way in (`normalize_owner_desk` at the top of
/// `update_company_workflow`); the stored side used to be whatever the
/// saved TOML literally held. A stored value with surrounding whitespace
/// therefore never compared equal to the same value round-tripped through
/// a GET/PUT, so the grandfathering did not apply and an unrelated edit
/// was refused once the padded desk went stale. `parse_workflow` now
/// normalizes the stored side too, so both are trimmed by construction.
#[tokio::test]
async fn an_unrelated_update_survives_a_padded_stored_desk_that_went_stale() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant_and_desk(),
    )));
    let mut created = valid_draft("wf", "WF");
    created.owner_desk = Some("ops".to_string());
    create_company_workflow(&company, None, &store, None, created, None, None)
        .await
        .expect("creates with a real desk");

    // Pad the STORED value. Every write boundary trims, so this stands in
    // for a body that reached the record by any other route — a
    // hand-authored graph, an import, a body written before the trim.
    let mut padded_record = store.load(&company).await.unwrap().unwrap();
    padded_record.overlay_workflows[0].toml = padded_record.overlay_workflows[0]
        .toml
        .replace("owner_desk = \"ops\"", "owner_desk = \"  ops  \"");
    assert!(
        padded_record.overlay_workflows[0]
            .toml
            .contains("owner_desk = \"  ops  \""),
        "the padded stored body must actually be in place for this test to mean anything"
    );
    // The desk goes stale underneath the workflow at the same time.
    padded_record.manifest = manifest_with_assistant();
    store.save(&padded_record).await.unwrap();
    let version = workflow_version(&padded_record.overlay_workflows[0].toml);

    // What a console round-trip sends back: the desk exactly as the read
    // route hands it out (trimmed), with an unrelated field edited.
    let mut edit = valid_draft("wf", "WF");
    edit.owner_desk = Some("ops".to_string());
    edit.description = Some("Renamed the description only.".to_string());
    let file = update_company_workflow(
        &company,
        None,
        &store,
        &revs(),
        None,
        edit,
        Some(&version),
        None,
    )
    .await
    .expect("a padded stored desk must grandfather the same as an unpadded one");
    assert_eq!(
        file.owner_desk.as_deref(),
        Some("ops"),
        "the stale desk is carried forward, not cleared"
    );
}

/// **Regression, issue #1882 review (PR #1882 bot finding, comment
/// 3878829353), RED-FIRST.** The grandfathering above (previous test)
/// only covers a stored `owner_desk` that stays UNRESOLVABLE. This
/// covers the sharper case the bot flagged: the desk that used to own
/// the stored raw string is deleted, and a *different*, later desk is
/// created whose display name happens to equal that same string (desk
/// creation enforces id uniqueness, not name uniqueness — nothing stops
/// this). The stored value is now newly RESOLVABLE again, just to the
/// wrong desk. An unrelated edit that round-trips `owner_desk` unchanged
/// must not let that resolution through — a PUT that never touched the
/// field must never reassign a workflow's owning desk.
#[tokio::test]
async fn an_unrelated_update_does_not_retarget_a_desk_id_recycled_as_a_name() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant_and_desk(),
    )));
    let mut created = valid_draft("wf", "WF");
    created.owner_desk = Some("ops".to_string());
    create_company_workflow(&company, None, &store, None, created, None, None)
        .await
        .expect("creates with a real desk");
    let saved = store.load(&company).await.unwrap().unwrap();
    let version = workflow_version(&saved.overlay_workflows[0].toml);

    // The "ops" desk is deleted, and a brand-new, UNRELATED desk is
    // created whose display name happens to be the literal string
    // "ops" — the old desk's id, now recycled as someone else's name.
    let mut stale_record = store.load(&company).await.unwrap().unwrap();
    stale_record.manifest = manifest_with_assistant();
    stale_record.overlay_desks = vec![OverlayDesk {
        id: "sales_new".to_string(),
        name: "ops".to_string(),
        description: None,
        members: vec!["assistant".to_string()],
        responder: ResponderMode::default(),
        hive: Default::default(),
    }];
    store.save(&stale_record).await.unwrap();

    // An edit to a completely unrelated field, carrying the stored
    // owner_desk forward unchanged — exactly what a console
    // round-tripping the read does.
    let mut edit = valid_draft("wf", "WF");
    edit.owner_desk = Some("ops".to_string());
    edit.description = Some("Renamed the description only.".to_string());
    let file = update_company_workflow(
        &company,
        None,
        &store,
        &revs(),
        None,
        edit,
        Some(&version),
        None,
    )
    .await
    .expect("an unrelated edit must save even though the stored desk was recycled");
    assert_eq!(
        file.owner_desk.as_deref(),
        Some("ops"),
        "the raw stored value must be carried forward untouched, not resolved to the \
         unrelated desk that recycled it as a display name"
    );
}

/// **Regression, issue #1882 review (PR #1882 bot finding).** A draft
/// naming its owner desk by DISPLAY NAME — `resolve_desk_id` accepts
/// either the id or a case-insensitive name — must be normalized to the
/// desk's canonical id before it is persisted. `render_workflow`
/// serializes `owner_desk` verbatim and has no `record` to re-resolve an
/// alias at save time, so leaving the alias in place would mean: if this
/// desk is later deleted and a new one created reusing the same display
/// name (desk creation enforces id uniqueness, not name uniqueness), the
/// stored alias would silently start resolving to the NEW desk on the
/// next load, re-routing this workflow's future blocker DMs to the wrong
/// team with no edit ever made to the workflow itself.
#[tokio::test]
async fn draft_naming_owner_desk_by_display_name_is_normalized_to_its_id() {
    let company = CompanyId::new("acme");
    let store = store_of(MemStore::seeded(record(
        &company,
        manifest_with_assistant_and_desk(),
    )));
    // The desk's id is "ops", its display name "Ops" (see
    // `manifest_with_assistant_and_desk`) — supply the alias, not the id.
    let mut draft = valid_draft("wf", "WF");
    draft.owner_desk = Some("Ops".to_string());
    let file = create_company_workflow(&company, None, &store, None, draft, None, None)
        .await
        .expect("owner_desk naming a real desk by display name is accepted");
    assert_eq!(
        file.owner_desk.as_deref(),
        Some("ops"),
        "the stored owner_desk must be the canonical id, not the display-name alias supplied"
    );

    // Same normalization on the update path, where the alias is
    // re-typed on an otherwise-untouched edit rather than the id the
    // create above just normalized.
    let saved = store.load(&company).await.unwrap().unwrap();
    let version = workflow_version(&saved.overlay_workflows[0].toml);
    let mut edit = valid_draft("wf", "WF");
    edit.owner_desk = Some("Ops".to_string());
    edit.description = Some("Renamed the description only.".to_string());
    let file2 = update_company_workflow(
        &company,
        None,
        &store,
        &revs(),
        None,
        edit,
        Some(&version),
        None,
    )
    .await
    .expect("owner_desk re-typed as a display name alias is accepted on update");
    assert_eq!(
        file2.owner_desk.as_deref(),
        Some("ops"),
        "the update path must also normalize the alias to the canonical id"
    );
}

/// **Regression, issue #1882 review (PR #1882 bot finding, comment
/// 3878620688), RED-FIRST.** Two desks sharing the same display name are
/// not a future-recreation hazard like the test above — desk creation
/// enforces id uniqueness, not name uniqueness (see the comment above
/// `resolved_owner_desk` in `validate_draft_against_record`), so both can
/// coexist right now. `resolve_desk_id`'s alias pass answers with
/// whichever of the two it iterates to first; unpatched, this write
/// silently persists that arbitrary desk instead of refusing the
/// ambiguous name, which would route this workflow's future blocker DMs
/// to a team the caller never actually named.
#[tokio::test]
async fn draft_naming_an_ambiguous_desk_display_name_is_rejected() {
    let company = CompanyId::new("acme");
    let mut seed = record(&company, manifest_with_assistant());
    seed.overlay_desks = vec![
        OverlayDesk {
            id: "sales_us".to_string(),
            name: "Sales".to_string(),
            description: None,
            members: vec!["assistant".to_string()],
            responder: ResponderMode::default(),
            hive: Default::default(),
        },
        OverlayDesk {
            id: "sales_eu".to_string(),
            name: "Sales".to_string(),
            description: None,
            members: vec!["assistant".to_string()],
            responder: ResponderMode::default(),
            hive: Default::default(),
        },
    ];
    let store = store_of(MemStore::seeded(seed));
    let mut draft = valid_draft("wf", "WF");
    draft.owner_desk = Some("Sales".to_string());
    let err = create_company_workflow(&company, None, &store, None, draft, None, None)
        .await
        .expect_err(
            "an owner_desk display name naming two desks must be refused, not silently \
             resolved to whichever one is iterated to first",
        );
    let problems = problems_of(&err);
    assert_eq!(problems[0].node_id, None, "graph-level, not node-scoped");
    assert_eq!(problems[0].field.as_deref(), Some("owner_desk"));
}
