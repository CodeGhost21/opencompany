use super::*;

use async_trait::async_trait;

use crate::company::parse_workflow;
use crate::error::OpenCompanyError;
use crate::policy::ManifestApprovalGate;
use crate::ports::UserRecord;
use crate::ports::types::CompanyId;
use crate::ports::types::SecretValue;
use crate::runtime::channel::{
    DeskChannel, DurableOperatorChannel, OPERATOR_CHANNEL, OperatorChannel,
};
use crate::server::ops::mailer::{MailSender, RecordingMailSender};
use crate::server::ops::smtp::{SmtpCredentials, SmtpSecurity};
use crate::store::{FsInboxStore, FsOps};

/// The company's own sending address in every test below.
const COMPANY_ADDRESS: &str = "acme@opencompany.test";

/// A graph whose single `output` node carries `destination`, wired
/// `trigger → done`. `target` is omitted when `None`.
fn graph(kind: &str, target: Option<&str>) -> WorkflowFile {
    let target_line = target
        .map(|t| format!("target = \"{t}\"\n"))
        .unwrap_or_default();
    let src = format!(
        r#"
id = "report_flow"
name = "Report flow"
[[node]]
id = "start"
kind = "trigger"
name = "Start"
[[node]]
id = "done"
kind = "output"
name = "Owner summary"
[node.destination]
kind = "{kind}"
{target_line}
[[edge]]
from = "start"
to = "done"
"#
    );
    parse_workflow(&src).expect("test graph is valid")
}

/// The same graph with **no** `[node.destination]` stanza at all — the
/// pre-#170 shape every seeded company template still ships (issue #925).
fn graph_without_destination() -> WorkflowFile {
    let src = r#"
id = "report_flow"
name = "Report flow"
[[node]]
id = "start"
kind = "trigger"
name = "Start"
[[node]]
id = "done"
kind = "output"
name = "Owner summary"
[[edge]]
from = "start"
to = "done"
"#;
    parse_workflow(src).expect("a graph whose output node names no destination is still valid")
}

/// A run output in which `done` produced one text item — the reached case.
fn reached_output() -> Value {
    serde_json::json!({
        "nodes": {
            "start": { "items": [{ "json": { "seed": 1 } }] },
            "done": { "items": [{ "json": { "text": "Q3 is up 12%." } }] },
        }
    })
}

/// A company record whose `[tools].allow` is exactly `grants`.
fn record(grants: &[&str]) -> CompanyRecord {
    let allow = grants
        .iter()
        .map(|g| format!("\"{g}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let manifest = toml::from_str(&format!(
        r#"
[company]
name = "Acme"

[policy]
mode = "full"

[tools]
allow = [{allow}]
"#
    ))
    .expect("valid manifest");
    CompanyRecord {
        overlay_desk_hive: Vec::new(),
        overlay_retired_agents: Vec::new(),
        overlay_agent_edits: Vec::new(),
        id: CompanyId::new("acme"),
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

fn smtp_creds() -> SmtpCredentials {
    SmtpCredentials {
        host: "smtp.example.test".into(),
        port: 587,
        security: SmtpSecurity::Starttls,
        username: "acme".into(),
        password: SecretValue("hunter2".into()),
        from_name: "Acme".into(),
        from_email: COMPANY_ADDRESS.into(),
    }
}

/// A [`MailSender`] that always refuses, for the "a send failure does not
/// fail the run" case.
struct RefusingMailSender;

#[async_trait]
impl MailSender for RefusingMailSender {
    async fn send(
        &self,
        _creds: &MailCredentials,
        _email: &OutboundEmail,
    ) -> Result<(), OpenCompanyError> {
        Err(OpenCompanyError::Config("smtp said no".into()))
    }
}

/// The offline delivery bundle: a recording mail sender (or none), tempdir
/// inbox + user stores, and the built-in operator channel.
struct Harness {
    deps: WorkflowDeliveryDeps,
    mail: RecordingMailSender,
    channel: OperatorChannel,
    /// A durable-looking channel, present only when
    /// [`with_recording_channel`](Harness::with_recording_channel) wired
    /// one. Needed by any case whose subject is what happens AFTER a send
    /// succeeds: `operator` is refused before the send, so it can no longer
    /// stand in for a channel that works.
    recording: Option<crate::runtime::channel::RecordingChannel>,
    inbox: Arc<FsInboxStore>,
    users: Arc<FsOps>,
    company: CompanyId,
    /// The approvals queue, when [`with_parking`](Harness::with_parking)
    /// wired one. The real gate and a real on-disk journal, not fakes: the
    /// point of these tests is that a workflow's park lands in the same
    /// queue an agent's does.
    gate: Option<Arc<ManifestApprovalGate>>,
    journal: Option<Arc<RuntimeJournal>>,
    /// The real on-disk event journal the write-behind delivery record
    /// (issue #529) lands in — held so a test can read back the
    /// [`CompanyEvent::WorkflowReportDelivered`] lines a dispatch appended.
    events: Arc<dyn EventLog>,
}

impl Harness {
    fn new(dir: &std::path::Path, with_mail: bool, with_channel: bool) -> Self {
        let mail = RecordingMailSender::new();
        let inbox = Arc::new(FsInboxStore::new(dir));
        let users = Arc::new(FsOps::new(dir));
        // The interactive in-memory operator buffer, kept so a test can
        // assert it stays untouched — workflow delivery must never route to
        // it. It is deliberately NOT wired into `deps.channels`.
        let channel = OperatorChannel::new();
        // A real filesystem journal, like the outcome tests use: the write
        // side must actually land on disk and read back, so the delivered
        // ledger is exercised end to end rather than against a double.
        let events: Arc<dyn EventLog> = Arc::new(crate::store::FsEventLog::new(dir));
        // The delivery adapter set the production builder wires (issue #1757):
        // the DURABLE operator channel, journaling into the event log, is the
        // owner/no-mailbox fallback's landing spot. `with_channel = false`
        // wires nothing, so the fallback has no operator adapter and reports a
        // failure row — the misconfigured-build case.
        let channels: Vec<Arc<dyn ChannelAdapter>> = if with_channel {
            vec![Arc::new(DurableOperatorChannel::new(
                CompanyId::new("acme"),
                events.clone(),
            ))]
        } else {
            Vec::new()
        };
        Self {
            deps: WorkflowDeliveryDeps {
                events: events.clone(),
                mail: with_mail.then(|| CompanyMail {
                    sender: Arc::new(mail.clone()),
                    smtp: smtp_creds(),
                }),
                inbox: inbox.clone(),
                users: users.clone(),
                bootstrap_admin: None,
                channels,
                parking: None,
            },
            mail,
            channel,
            recording: None,
            inbox,
            users,
            company: CompanyId::new("acme"),
            gate: None,
            journal: None,
            events,
        }
    }

    /// Sets the deployment's standing bootstrap-admin address (M8), the same
    /// value the production builder threads from `AppConfig::bootstrap_admin`.
    fn with_bootstrap_admin(mut self, email: &str) -> Self {
        self.deps.bootstrap_admin = Some(email.to_string());
        self
    }

    /// Sets the company record's manifest so a test can name `[users] admins`
    /// standing invites. Rebuilt from TOML rather than mutated field-by-field
    /// so the parse mirrors a real manifest load.
    fn manifest_with_admins(admins: &[&str]) -> crate::company::CompanyManifest {
        let list = admins
            .iter()
            .map(|a| format!("\"{a}\""))
            .collect::<Vec<_>>()
            .join(", ");
        toml::from_str(&format!(
            r#"
[company]
name = "Acme"

[policy]
mode = "full"

[users]
admins = [{list}]
"#
        ))
        .expect("valid manifest with [users] admins")
    }

    /// Wires the approvals queue the production builder wires: a real
    /// [`ManifestApprovalGate`] over `policy_mode` and a real
    /// [`RuntimeJournal`] on disk under `dir`.
    ///
    /// `policy_mode` is a parameter because `full` is the interesting one:
    /// it is the mode under which `evaluate` would return `Allow` for a
    /// `Send` effect, so a test that parks under `full` is the one that
    /// proves delivery does not route through `evaluate`.
    fn with_parking(mut self, dir: &std::path::Path, policy_mode: &str) -> Self {
        let policy =
            toml::from_str(&format!("mode = \"{policy_mode}\"\n")).expect("valid [policy] block");
        let gate = Arc::new(ManifestApprovalGate::new(policy));
        let journal = Arc::new(RuntimeJournal::new(dir.join("journal.jsonl")));
        self.deps.parking = Some(DeliveryParking {
            approvals: gate.clone(),
            journal: journal.clone(),
            // Issue #978: a test fixture parks into its own queues. The
            // production wiring is `RuntimeBuilder`, which hands the
            // runtime's own handles in so a park arms what the resolve
            // path releases.
            continuations: Default::default(),
            gates: Default::default(),
            blocked_nodes: Default::default(),
        });
        self.gate = Some(gate);
        self.journal = Some(journal);
        self
    }

    /// Wires a gate plus a journal whose every write **fails**, for the
    /// partial-failure case.
    ///
    /// The failure is induced by pointing the journal at a path that is
    /// already a *directory*: `append` creates the parent fine and then
    /// `OpenOptions::open` returns `EISDIR`. Deterministic, cross-platform,
    /// and it fails at the real I/O boundary rather than at a mock, so the
    /// test exercises the same error path a full disk would.
    fn with_failing_journal(mut self, dir: &std::path::Path, policy_mode: &str) -> Self {
        let policy =
            toml::from_str(&format!("mode = \"{policy_mode}\"\n")).expect("valid [policy] block");
        let gate = Arc::new(ManifestApprovalGate::new(policy));
        let blocked = dir.join("unwritable-journal.jsonl");
        std::fs::create_dir_all(&blocked).expect("journal path occupied by a directory");
        let journal = Arc::new(RuntimeJournal::new(blocked));
        self.deps.parking = Some(DeliveryParking {
            approvals: gate.clone(),
            journal: journal.clone(),
            // Issue #978: a test fixture parks into its own queues. The
            // production wiring is `RuntimeBuilder`, which hands the
            // runtime's own handles in so a park arms what the resolve
            // path releases.
            continuations: Default::default(),
            gates: Default::default(),
            blocked_nodes: Default::default(),
        });
        self.gate = Some(gate);
        self.journal = Some(journal);
        self
    }

    /// Adds an active admin with `email` to the company directory.
    async fn add_admin(&self, id: &str, email: &str) {
        self.users
            .upsert_user(
                &self.company,
                &UserRecord {
                    id: id.to_string(),
                    email: email.to_string(),
                    display_name: None,
                    avatar: None,
                    role: UserRole::Admin,
                    status: UserStatus::Active,
                    password_hash: None,
                    must_change_password: false,
                    created_at_millis: 1,
                    last_seen_at_millis: None,
                    updated_at_millis: 1,
                },
            )
            .await
            .expect("user upserted");
    }

    /// Files an INBOUND email from `from`, which is what makes that address
    /// an established thread.
    async fn receive_from(&self, from: &str) {
        self.inbox
            .append(
                &self.company,
                &EmailRecord {
                    id: generate_id(),
                    inbox: local_part(COMPANY_ADDRESS),
                    from_name: String::new(),
                    from_email: from.to_string(),
                    subject: "hello".to_string(),
                    body: "hi".to_string(),
                    at_millis: 1,
                    read: false,
                    outbound: false,
                },
            )
            .await
            .expect("inbound filed");
    }

    /// Every message in the company's own inbox.
    async fn inbox_messages(&self) -> Vec<EmailRecord> {
        self.inbox
            .messages(&self.company, &local_part(COMPANY_ADDRESS), 100, 0)
            .await
            .expect("inbox readable")
    }

    /// Every `WorkflowReportDelivered` the write-behind path journaled
    /// (issue #529) — what a re-run's fold would later read back.
    async fn journaled_deliveries(&self) -> Vec<CompanyEvent> {
        self.events
            .read_from(
                &self.company,
                crate::ports::types::EventSeq::new(0),
                usize::MAX,
            )
            .await
            .expect("journal readable")
            .into_iter()
            .map(|s| s.event)
            .filter(|e| matches!(e, CompanyEvent::WorkflowReportDelivered { .. }))
            .collect()
    }

    /// The text of every workflow report the durable operator channel
    /// journaled (issue #1757): `AgentReply`s authored by `workflow` on the
    /// dedicated `operator` line the standing Operator channel renders. This
    /// is what proves the owner/no-mailbox fallback is a real, readable
    /// delivery rather than a discard on the in-memory buffer.
    async fn operator_reports(&self) -> Vec<String> {
        self.events
            .read_from(
                &self.company,
                crate::ports::types::EventSeq::new(0),
                usize::MAX,
            )
            .await
            .expect("journal readable")
            .into_iter()
            .filter_map(|s| match s.event {
                CompanyEvent::AgentReply {
                    chat_id,
                    agent_id,
                    text,
                    ..
                } if chat_id == crate::runtime::channel::OPERATOR_CHANNEL
                    // `WORKFLOW_REPLY_AUTHOR` covers an explicit `channel`
                    // destination's report; `OWNER_FALLBACK_REPORT_AUTHOR`
                    // covers the `owner`-with-no-mailbox fallback (issue
                    // #1781 review, Codex P1) — both are still genuine,
                    // durable operator-channel reports, just gated
                    // differently on read. A test that cares which one
                    // landed reads `agent_id` itself via
                    // `operator_report_authors`.
                    && (agent_id == crate::runtime::channel::WORKFLOW_REPLY_AUTHOR
                        || agent_id == crate::runtime::OWNER_FALLBACK_REPORT_AUTHOR) =>
                {
                    Some(text)
                }
                _ => None,
            })
            .collect()
    }

    /// Every operator-channel `AgentReply`'s `(agent_id, text)` pair, for a
    /// test that needs to tell an owner-fallback report apart from an
    /// ordinary one rather than just counting them (issue #1781 review,
    /// Codex P1).
    async fn operator_report_authors(&self) -> Vec<(String, String)> {
        self.events
            .read_from(
                &self.company,
                crate::ports::types::EventSeq::new(0),
                usize::MAX,
            )
            .await
            .expect("journal readable")
            .into_iter()
            .filter_map(|s| match s.event {
                CompanyEvent::AgentReply {
                    chat_id,
                    agent_id,
                    text,
                    ..
                } if chat_id == crate::runtime::channel::OPERATOR_CHANNEL => Some((agent_id, text)),
                _ => None,
            })
            .collect()
    }

    /// Swaps the delivery bundle's event journal for one whose every append
    /// **fails**, for the "a journal failure does not fail delivery" case.
    fn with_failing_events(mut self) -> Self {
        self.deps.events = Arc::new(FailingEventLog);
        self
    }

    /// Wires a channel that accepts a send, under an ordinary channel id.
    ///
    /// The operator channel used to serve this purpose, but delivery now
    /// refuses it outright, which lands the caller in the refusal branch
    /// before the behaviour under test is reached. Anything that asserts
    /// what follows a successful send needs this instead.
    fn with_recording_channel(mut self, id: &str) -> Self {
        let channel = crate::runtime::channel::RecordingChannel::new(id);
        self.deps.channels.push(Arc::new(channel.clone()));
        self.recording = Some(channel);
        self
    }

    /// The channel [`with_recording_channel`](Harness::with_recording_channel) wired.
    fn recording(&self) -> &crate::runtime::channel::RecordingChannel {
        self.recording
            .as_ref()
            .expect("with_recording_channel was not called")
    }
}

/// An [`EventLog`] whose `append` always errors — the write-behind delivery
/// record's failure path (issue #529). Reads yield nothing; the point is the
/// append, and that a delivery survives it.
struct FailingEventLog;

#[async_trait]
impl EventLog for FailingEventLog {
    async fn append(
        &self,
        _company: &CompanyId,
        _event: CompanyEvent,
    ) -> crate::Result<crate::ports::types::EventSeq> {
        Err(OpenCompanyError::Config(
            "event journal is unwritable".into(),
        ))
    }

    async fn read_from(
        &self,
        _company: &CompanyId,
        _seq: crate::ports::types::EventSeq,
        _limit: usize,
    ) -> crate::Result<Vec<crate::ports::types::StoredEvent>> {
        Ok(Vec::new())
    }

    fn subscribe(
        &self,
        _company: &CompanyId,
    ) -> futures::stream::BoxStream<'static, crate::ports::events::EventStreamItem> {
        Box::pin(futures::stream::empty())
    }
}

/// A [`UserStore`] whose `list_users` always errors — the M8 store-error
/// path. Every other method is unreachable for these tests (the `owner`
/// resolver reads only `list_users`) and panics if a future caller leans on
/// it, rather than quietly returning an empty result that would hide a bug.
struct FailingUserStore;

#[async_trait]
impl UserStore for FailingUserStore {
    async fn list_users(&self, _company: &CompanyId) -> crate::Result<Vec<UserRecord>> {
        Err(OpenCompanyError::Config(
            "user directory is unreadable".into(),
        ))
    }
    async fn get_user(&self, _company: &CompanyId, _id: &str) -> crate::Result<Option<UserRecord>> {
        unreachable!("owner delivery reads only list_users")
    }
    async fn find_user_by_email(
        &self,
        _company: &CompanyId,
        _email: &str,
    ) -> crate::Result<Option<UserRecord>> {
        unreachable!("owner delivery reads only list_users")
    }
    async fn upsert_user(&self, _company: &CompanyId, _user: &UserRecord) -> crate::Result<()> {
        unreachable!("owner delivery reads only list_users")
    }
    async fn delete_user(&self, _company: &CompanyId, _id: &str) -> crate::Result<bool> {
        unreachable!("owner delivery reads only list_users")
    }
    async fn list_invites(
        &self,
        _company: &CompanyId,
    ) -> crate::Result<Vec<crate::ports::InviteRecord>> {
        unreachable!("owner delivery reads only list_users")
    }
    async fn find_invite_by_email(
        &self,
        _company: &CompanyId,
        _email: &str,
    ) -> crate::Result<Option<crate::ports::InviteRecord>> {
        unreachable!("owner delivery reads only list_users")
    }
    async fn upsert_invite(
        &self,
        _company: &CompanyId,
        _invite: &crate::ports::InviteRecord,
    ) -> crate::Result<()> {
        unreachable!("owner delivery reads only list_users")
    }
    async fn mark_invite_notified(
        &self,
        _company: &CompanyId,
        _id: &str,
        _at_millis: u64,
    ) -> crate::Result<bool> {
        unreachable!("owner delivery reads only list_users")
    }
    async fn delete_invite(&self, _company: &CompanyId, _id: &str) -> crate::Result<bool> {
        unreachable!("owner delivery reads only list_users")
    }
}

// --- owner ---------------------------------------------------------------

/// `owner` resolves to the company's active admins server-side and emails
/// each of them. The graph named nobody — that is the whole point.
#[tokio::test]
async fn owner_emails_every_active_admin() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), true, true);
    h.add_admin("u1", "ada@acme.test").await;
    h.add_admin("u2", "grace@acme.test").await;

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&[]),
        &graph("owner", None),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports.len(), 2, "{reports:?}");
    assert!(reports.iter().all(|r| r.status == DeliveryStatus::Sent));
    let mut addressed: Vec<String> = h.mail.sent().into_iter().map(|(_, e)| e.to).collect();
    addressed.sort();
    assert_eq!(addressed, vec!["ada@acme.test", "grace@acme.test"]);
    // The report body is the output node's text, and the subject names the
    // company, the workflow, and the step.
    let (_, email) = &h.mail.sent()[0];
    assert!(email.body.contains("Q3 is up 12%."), "{}", email.body);
    assert!(email.subject.contains("Acme"), "{}", email.subject);
    assert!(email.subject.contains("Report flow"), "{}", email.subject);
    // `owner` needs no grant: this record grants nothing at all.
}

/// A suspended admin and a plain member are not the owner. Only active
/// admins are.
#[tokio::test]
async fn owner_ignores_suspended_admins_and_members() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), true, true);
    h.add_admin("u1", "ada@acme.test").await;
    for (id, email, role, status) in [
        (
            "u2",
            "sus@acme.test",
            UserRole::Admin,
            UserStatus::Suspended,
        ),
        ("u3", "mem@acme.test", UserRole::Member, UserStatus::Active),
    ] {
        h.users
            .upsert_user(
                &h.company,
                &UserRecord {
                    id: id.to_string(),
                    email: email.to_string(),
                    display_name: None,
                    avatar: None,
                    role,
                    status,
                    password_hash: None,
                    must_change_password: false,
                    created_at_millis: 1,
                    last_seen_at_millis: None,
                    updated_at_millis: 1,
                },
            )
            .await
            .unwrap();
    }

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&[]),
        &graph("owner", None),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(h.mail.sent().len(), 1);
    assert_eq!(h.mail.sent()[0].1.to, "ada@acme.test");
}

/// With no mailbox wired, `owner` falls back to the DURABLE operator channel
/// (issue #1757): a genuine, journal-backed delivery — not the discard-on-an-
/// in-memory-buffer failure it used to report. The report lands in the event
/// log on the dedicated Operator line, and the interactive buffer is
/// untouched.
#[tokio::test]
async fn owner_falls_back_to_the_durable_operator_channel_without_mail() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), false, true);
    h.add_admin("u1", "ada@acme.test").await;

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&[]),
        &graph("owner", None),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].status, DeliveryStatus::Sent, "{reports:?}");
    assert_eq!(reports[0].target.as_deref(), Some(OPERATOR_CHANNEL));
    assert!(reports[0].detail.contains("no mailbox"), "{reports:?}");
    assert_eq!(reports[0].reason, DeliveryReason::OwnerFellBackNoMailbox);
    // The interactive in-memory buffer is never a delivery surface.
    assert!(h.channel.sent().is_empty());
    // The report is durable: an `AgentReply` landed on the dedicated
    // Operator line — never the General desk — carrying the workflow's
    // subject header so it reads as a workflow report, not an agent's own
    // reply.
    let landed = h.operator_reports().await;
    assert_eq!(landed.len(), 1, "the report must be journaled: {landed:?}");
    assert!(landed[0].contains("Q3 is up 12%."), "{landed:?}");
    assert!(landed[0].contains("Report flow"), "{landed:?}");
}

/// Issue #1781 review (Codex P1): the `owner`-with-no-mailbox fallback must
/// journal under a distinct author from an ordinary operator-channel report,
/// so the read path (`server::chat_history::history_for_desk`) can restrict
/// exactly this row to administrators — the same audience the sibling email
/// branch already enforces (`owner_recipients` filters to active admins).
///
/// Proven against a **contrasting pair** in the same test rather than just
/// asserting the fallback's author: an explicit `channel` destination
/// naming `operator` is a workflow author's deliberate choice, general
/// audience, and must keep the ordinary `WORKFLOW_REPLY_AUTHOR` — this is
/// what shows the fallback's marker is additive, not a wholesale change to
/// every operator-channel report.
#[tokio::test]
async fn owner_fallback_report_is_authored_distinctly_from_an_ordinary_one() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), false, true);
    h.add_admin("u1", "ada@acme.test").await;

    deliver_outputs(
        Some(&h.deps),
        &record(&[]),
        &graph("owner", None),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;
    deliver_outputs(
        Some(&h.deps),
        &record(&[]),
        &graph("channel", Some(OPERATOR_CHANNEL)),
        "run-2",
        &reached_output(),
        &[],
    )
    .await;

    let authors = h.operator_report_authors().await;
    assert_eq!(authors.len(), 2, "{authors:?}");
    assert!(
        authors
            .iter()
            .any(|(agent_id, _)| agent_id == crate::runtime::OWNER_FALLBACK_REPORT_AUTHOR),
        "the owner fallback must be marked distinctly: {authors:?}"
    );
    assert!(
        authors
            .iter()
            .any(|(agent_id, _)| agent_id == crate::runtime::channel::WORKFLOW_REPLY_AUTHOR),
        "an explicit `channel: operator` destination must keep the ordinary \
         author — the marker is additive, not a wholesale change: {authors:?}"
    );
}

/// Issue #1781 review (CodeRabbit Major + Codex P2): a company whose roster
/// already grandfathers a **teammate** at the literal id `operator` (no desk
/// of the same id — see `CompanyRecord::operator_feed_channel`) must not
/// have the durable Operator system feed land on that same address. Proven
/// for **both** report shapes that can reach the operator channel — the
/// `owner` fallback and an explicit `channel: operator` destination — since
/// review found the collision on the desk-list/read side, not the write
/// guard, and either shape re-opens it if only one were fixed.
///
/// Pre-fix, both reports journaled at `chat_id == OPERATOR_CHANNEL`
/// (`"operator"`) — exactly the address `ChatView` addresses that teammate's
/// own DM by (issue #364). This test's whole point is that the two lines
/// now diverge.
#[tokio::test]
async fn a_report_diverts_off_a_grandfathered_teammates_own_operator_line() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), false, true);
    h.add_admin("u1", "ada@acme.test").await;

    let mut collided = record(&[]);
    collided.manifest = toml::from_str(
        r#"
[company]
name = "Acme"

[policy]
mode = "full"

[[agent]]
id = "operator"
role = "Chief of Staff"
"#,
    )
    .expect("valid manifest with a grandfathered `operator` teammate");
    assert!(
        collided.is_roster_agent(OPERATOR_CHANNEL) && !collided.desk_exists(OPERATOR_CHANNEL),
        "fixture must actually be in the collision state this test exercises"
    );

    deliver_outputs(
        Some(&h.deps),
        &collided,
        &graph("owner", None),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;
    deliver_outputs(
        Some(&h.deps),
        &collided,
        &graph("channel", Some(OPERATOR_CHANNEL)),
        "run-2",
        &reached_output(),
        &[],
    )
    .await;

    let landed = h
        .events
        .read_from(
            &h.company,
            crate::ports::types::EventSeq::new(0),
            usize::MAX,
        )
        .await
        .expect("journal readable")
        .into_iter()
        .filter_map(|s| match s.event {
            CompanyEvent::AgentReply { chat_id, .. } => Some(chat_id),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(landed.len(), 2, "{landed:?}");
    assert!(
        landed
            .iter()
            .all(|chat_id| chat_id == crate::runtime::OPERATOR_CHANNEL_COLLISION_FALLBACK),
        "every report bound for the system feed must land off the \
         grandfathered teammate's own `operator` line, not on it: {landed:?}"
    );
    assert!(
        landed.iter().all(|chat_id| chat_id != OPERATOR_CHANNEL),
        "the literal `operator` line must stay untouched by the durable \
         feed — that is the teammate's own DM address: {landed:?}"
    );
}

/// Issue #1781 review (fresh P2 on `operator_feed_channel_fallback_shadowed`
/// itself): the residual double collision that predicate detects must not
/// merely be logged while the report ships anyway. Reuses this fixture's
/// manifest shape from `CompanyRecord`'s own
/// `operator_feed_channel_fallback_shadowed_detects_a_double_collision` test
/// (`ports::types`) — one grandfathered desk named "Operator" (shadowing the
/// primary address) and a second, different desk named "operator-feed"
/// (shadowing the collision fallback) — and proves the delivery layer
/// refuses the send rather than journaling into that second desk's own
/// transcript while still reporting `Sent`.
#[tokio::test]
async fn a_double_collision_refuses_delivery_instead_of_misrouting() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), false, true);

    let mut collided = record(&[]);
    collided.manifest = toml::from_str(
        r#"
[company]
name = "Acme"

[policy]
mode = "full"

[[group_chat]]
id = "legacy_ops"
name = "Operator"
members = []

[[group_chat]]
id = "ops2"
name = "operator-feed"
members = []
"#,
    )
    .expect("valid manifest with a double grandfathered collision");
    assert!(
        collided.operator_feed_channel_fallback_shadowed(),
        "fixture must actually be in the double-collision state this test \
         exercises, or it proves nothing"
    );

    let reports = deliver_outputs(
        Some(&h.deps),
        &collided,
        &graph("channel", Some(OPERATOR_CHANNEL)),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(
        reports[0].status,
        DeliveryStatus::Failed,
        "a shadowed fallback must be reported as a failed delivery, never \
         `Sent` — a `Sent` row here is exactly the silent misroute this test \
         guards against: {reports:?}"
    );
    assert_eq!(
        reports[0].reason,
        DeliveryReason::ChannelCollisionShadowed,
        "{reports:?}"
    );

    let landed = h
        .events
        .read_from(
            &h.company,
            crate::ports::types::EventSeq::new(0),
            usize::MAX,
        )
        .await
        .expect("journal readable")
        .into_iter()
        .filter_map(|s| match s.event {
            CompanyEvent::AgentReply { chat_id, .. } => Some(chat_id),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        landed.is_empty(),
        "a refused delivery must not append anything to the event log — in \
         particular nothing must land on \"operator-feed\", the second \
         desk's own transcript: {landed:?}"
    );
}

/// A company with a mailbox but no admin address also delivers durably to the
/// operator channel rather than failing — the report still reaches the one
/// human who could act on it.
#[tokio::test]
async fn owner_falls_back_to_the_durable_operator_channel_when_no_admin_has_an_address() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), true, true);

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&[]),
        &graph("owner", None),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].status, DeliveryStatus::Sent, "{reports:?}");
    assert_eq!(
        reports[0].reason,
        DeliveryReason::OwnerFellBackNoAdminAddress
    );
    assert!(reports[0].detail.contains("no active admin"), "{reports:?}");
    assert!(h.mail.sent().is_empty(), "nothing should have been emailed");
    assert!(h.channel.sent().is_empty());
    let landed = h.operator_reports().await;
    assert_eq!(landed.len(), 1, "the report must be journaled: {landed:?}");
}

/// Both fallbacks unavailable: no mail, no operator channel wired at all
/// (a misconfigured build). Still a row — `failed`, naming the gap — never
/// silence.
#[tokio::test]
async fn owner_with_neither_mail_nor_a_channel_reports_failure() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), false, false);

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&[]),
        &graph("owner", None),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].status, DeliveryStatus::Failed);
    assert!(reports[0].detail.contains("operator"), "{reports:?}");
}

// --- owner: standing admin invites (issue #661 / M8) ---------------------

/// **The M8 headline.** A fresh platform-provisioned tenant has nobody in
/// its manifest and nobody in the user store yet, but the platform injected
/// a bootstrap admin. An `owner` report must reach that address — not fall
/// back to the operator channel, which is the one human who could act on it
/// never hearing about it.
#[tokio::test]
async fn owner_emails_the_standing_bootstrap_admin_on_a_fresh_tenant() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), true, true).with_bootstrap_admin("founder@acme.test");
    // No admins in the store, no `[users] admins` in the manifest.

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&[]),
        &graph("owner", None),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].status, DeliveryStatus::Sent, "{reports:?}");
    assert_eq!(reports[0].reason, DeliveryReason::OwnerEmailed);
    assert_eq!(reports[0].target.as_deref(), Some("founder@acme.test"));
    assert_eq!(h.mail.sent().len(), 1);
    assert_eq!(h.mail.sent()[0].1.to, "founder@acme.test");
    // The operator channel must be untouched — the whole bug is that the
    // report fell back to it.
    assert!(
        h.channel.sent().is_empty(),
        "the standing admin was mailed, so nothing goes to the operator channel"
    );
    // The send is mirrored into the inbox as outbound, and journaled.
    let outbound: Vec<_> = h
        .inbox_messages()
        .await
        .into_iter()
        .filter(|m| m.outbound)
        .collect();
    assert_eq!(outbound.len(), 1, "the send must leave an audit record");
    let journaled = h.journaled_deliveries().await;
    assert_eq!(journaled.len(), 1, "{journaled:?}");
}

/// A manifest `[users] admins` entry is a standing invite too, and is mailed
/// the same way — even before that person has ever signed in.
#[tokio::test]
async fn owner_emails_a_manifest_admin_standing_invite() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), true, true);
    let mut rec = record(&[]);
    rec.manifest = Harness::manifest_with_admins(&["grace@acme.test"]);

    let reports = deliver_outputs(
        Some(&h.deps),
        &rec,
        &graph("owner", None),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].status, DeliveryStatus::Sent, "{reports:?}");
    assert_eq!(reports[0].reason, DeliveryReason::OwnerEmailed);
    assert_eq!(h.mail.sent().len(), 1);
    assert_eq!(h.mail.sent()[0].1.to, "grace@acme.test");
}

/// **User-record-wins.** A bootstrap admin who has since signed in and been
/// *suspended* is not mailed through the leftover standing invite: their
/// record wins, and a suspended admin is not an active one. `owner` then has
/// no address to email and falls back to the durable operator channel with
/// the M8 wording — a real delivery (issue #1757), not the failure it once
/// reported.
#[tokio::test]
async fn owner_does_not_email_a_suspended_bootstrap_admin() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), true, true).with_bootstrap_admin("founder@acme.test");
    // The bootstrap admin signed in, then was suspended: a record exists.
    h.users
        .upsert_user(
            &h.company,
            &UserRecord {
                id: "founder".to_string(),
                email: "founder@acme.test".to_string(),
                display_name: None,
                avatar: None,
                role: UserRole::Admin,
                status: UserStatus::Suspended,
                password_hash: None,
                must_change_password: false,
                created_at_millis: 1,
                last_seen_at_millis: None,
                updated_at_millis: 1,
            },
        )
        .await
        .unwrap();

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&[]),
        &graph("owner", None),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].status, DeliveryStatus::Sent, "{reports:?}");
    assert_eq!(
        reports[0].reason,
        DeliveryReason::OwnerFellBackNoAdminAddress
    );
    assert!(
        reports[0].detail.contains("standing admin invite"),
        "the fallback wording must name standing invites now: {reports:?}"
    );
    assert!(
        h.mail.sent().is_empty(),
        "a suspended admin must not be mailed, invite or not"
    );
    assert!(
        h.channel.sent().is_empty(),
        "the interactive operator buffer is not delivery"
    );
    // The report still lands, durably, on the operator channel.
    assert_eq!(h.operator_reports().await.len(), 1);
}

/// **Dedupe.** An address named both as an active admin and as the bootstrap
/// admin is one person, and is mailed exactly once.
#[tokio::test]
async fn owner_dedupes_an_active_admin_that_is_also_the_bootstrap_admin() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), true, true).with_bootstrap_admin("ada@acme.test");
    h.add_admin("u1", "ada@acme.test").await;

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&[]),
        &graph("owner", None),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports.len(), 1, "one recipient, one row: {reports:?}");
    assert_eq!(reports[0].status, DeliveryStatus::Sent);
    assert_eq!(h.mail.sent().len(), 1, "mailed once, not twice");
    assert_eq!(h.mail.sent()[0].1.to, "ada@acme.test");
}

/// A manifest admin address is normalized the same way the login path
/// normalizes it, so `Grace@ACME.test` and `grace@acme.test` are one address
/// — the send goes to the normalized form.
#[tokio::test]
async fn owner_normalizes_a_manifest_admin_address() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), true, true);
    let mut rec = record(&[]);
    rec.manifest = Harness::manifest_with_admins(&["Grace@ACME.test"]);

    let reports = deliver_outputs(
        Some(&h.deps),
        &rec,
        &graph("owner", None),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].status, DeliveryStatus::Sent, "{reports:?}");
    assert_eq!(h.mail.sent()[0].1.to, "grace@acme.test");
}

/// **Store-error stance (the M8 bug's worst case).** When the user store
/// cannot be read, the standing invites are mailed anyway — dropping the only
/// humans the company is known to have back to the operator channel is
/// exactly the silent drop M8 fixes.
#[tokio::test]
async fn owner_still_emails_standing_invites_when_the_user_store_errors() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = Harness::new(dir.path(), true, true).with_bootstrap_admin("founder@acme.test");
    h.deps.users = Arc::new(FailingUserStore);

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&[]),
        &graph("owner", None),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(
        reports[0].status,
        DeliveryStatus::Sent,
        "an unreadable store must still mail the standing invite: {reports:?}"
    );
    assert_eq!(reports[0].reason, DeliveryReason::OwnerEmailed);
    assert_eq!(h.mail.sent().len(), 1);
    assert_eq!(h.mail.sent()[0].1.to, "founder@acme.test");
}

// --- email ---------------------------------------------------------------

/// The happy path: granted AND established. The mail goes out and is
/// mirrored into the company inbox as outbound, for audit.
#[tokio::test]
async fn email_granted_and_established_sends_and_records_outbound() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), true, true);
    h.receive_from("ada@example.com").await;

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&["email.send"]),
        &graph("email", Some("ada@example.com")),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].status, DeliveryStatus::Sent);
    assert_eq!(h.mail.sent().len(), 1);
    assert_eq!(h.mail.sent()[0].1.to, "ada@example.com");

    let messages = h.inbox_messages().await;
    let outbound: Vec<&EmailRecord> = messages.iter().filter(|m| m.outbound).collect();
    assert_eq!(outbound.len(), 1, "the send must leave an audit record");
    assert!(outbound[0].body.contains("Q3 is up 12%."));
}

/// **The security boundary.** With no `email` grant the send is REFUSED
/// outright — before the mailbox, before the thread check — and nothing
/// leaves the process.
#[tokio::test]
async fn email_without_the_grant_is_denied_and_nothing_is_sent() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), true, true);
    // Established thread AND a wired mailbox: the ONLY thing missing is the
    // grant, so a pass here could only come from the grant check.
    h.receive_from("ada@example.com").await;

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&["docs.*", "web"]),
        &graph("email", Some("ada@example.com")),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].status, DeliveryStatus::Denied);
    assert!(reports[0].detail.contains("[tools].allow"), "{reports:?}");
    assert!(h.mail.sent().is_empty(), "a denied send must not go out");
    assert!(
        h.inbox_messages().await.iter().all(|m| !m.outbound),
        "a denied send must leave no outbound record"
    );
}

/// **The security boundary, second gate.** Granted but COLD: the company's
/// inbox holds nothing from this address, so the workflow may not open the
/// conversation. Skipped and reported — never sent.
#[tokio::test]
async fn email_to_a_cold_recipient_is_skipped_and_nothing_is_sent() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), true, true);
    // A different address wrote in; the target never did.
    h.receive_from("someone-else@example.com").await;

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&["*"]),
        &graph("email", Some("stranger@example.com")),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].status, DeliveryStatus::Skipped);
    assert!(reports[0].detail.contains("never written"), "{reports:?}");
    assert!(
        h.mail.sent().is_empty(),
        "a cold recipient must not be mailed"
    );
}

// --- email: cold recipients park (issue #227) ----------------------------

/// **Issue #227, the headline.** Cold and granted, with an approvals queue
/// wired: the report is PARKED rather than dropped. One `pending` row,
/// nothing mailed, and a real card in the journal the operator's
/// `/approvals` list reads.
///
/// Note the policy mode: `full`. That is deliberately the mode under which
/// `ApprovalGate::evaluate` returns `Allow` for a `Send` effect, so if this
/// path ever grew an evaluate-then-dispatch step the mail would go out here
/// and this test would fail on `sent()`.
#[tokio::test]
async fn a_cold_recipient_is_parked_for_approval_and_nothing_is_sent() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), true, true).with_parking(dir.path(), "full");
    h.receive_from("someone-else@example.com").await;

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&["*"]),
        &graph("email", Some("stranger@example.com")),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].status, DeliveryStatus::Pending, "{reports:?}");
    assert_eq!(
        reports[0].target.as_deref(),
        Some("stranger@example.com"),
        "{reports:?}"
    );
    // The row has to point somewhere, or `pending` is just a nicer word for
    // dropped.
    assert!(reports[0].detail.contains("Approvals"), "{reports:?}");
    // THE INVARIANT: a cold recipient never auto-sends.
    assert!(
        h.mail.sent().is_empty(),
        "a cold recipient must not be mailed, parked or not"
    );
    assert!(
        h.inbox_messages().await.iter().all(|m| !m.outbound),
        "nothing was sent, so there is no outbound record to leave"
    );

    // And the card really is in the durable queue — this is what
    // `/approvals` lists and what boot replay rehydrates.
    let pending = h.journal.as_ref().unwrap().pending();
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0].effect.kind, EMAIL_SEND_KIND);
}

/// The parked effect must have the **same shape as the agent path's**
/// (`CycleHostImpl::send_email`), field for field. Not cosmetic: the
/// operator sees one kind of card either way, and `perform_effect` keys on
/// `kind` plus the `to`/`subject`/`body` payload to actually mail it on
/// approval. A drift here parks cards that do nothing when approved.
#[tokio::test]
async fn the_parked_effect_matches_the_agent_paths_shape() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), true, true).with_parking(dir.path(), "full");

    deliver_outputs(
        Some(&h.deps),
        &record(&["*"]),
        &graph("email", Some("stranger@example.com")),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    let pending = h.journal.as_ref().unwrap().pending();
    let effect = &pending[0].effect;
    assert_eq!(effect.kind, EMAIL_SEND_KIND, "same kind constant");
    assert_eq!(effect.group, EffectGroup::Send);
    assert_eq!(effect.amount_usd, None, "a send costs nothing to gate on");
    // The two flags that say *why* this parked: cold counterparty.
    assert!(!effect.established_thread);
    assert!(effect.first_time_counterparty);
    // The payload `perform_effect` reads.
    assert_eq!(effect.payload["to"], "stranger@example.com");
    assert!(
        effect.payload["subject"].as_str().unwrap().contains("Acme"),
        "{effect:?}"
    );
    assert!(
        effect.payload["body"]
            .as_str()
            .unwrap()
            .contains("Q3 is up 12%."),
        "the report itself is the body, or approving sends an empty mail: {effect:?}"
    );
    // The gate holds the identical effect under the same id, so resolving
    // the approval returns something executable.
    let parked = h
        .gate
        .as_ref()
        .unwrap()
        .parked_effect(&pending[0].id)
        .expect("the gate holds the same id the journal recorded");
    assert_eq!(parked.payload, effect.payload);
    assert_eq!(parked.kind, effect.kind);
}

/// **Data integrity (PR #256 review).** A journal write that fails AFTER the
/// gate accepted the park must leave **no gate entry behind**.
///
/// The half-wired state the bundled [`DeliveryParking`] makes unrepresentable
/// is a *construction* mistake; this is the *runtime* version of it, and
/// bundling does nothing for it. An orphaned gate entry is the worst of the
/// three outcomes: an executable effect sitting in the queue with no durable
/// record, visible now and gone on the next restart, backing a row that
/// promises a card which will not survive.
///
/// Asserts all four halves of the rollback: `skipped` (not `pending`), no
/// gate entry, no in-memory queue entry, and nothing sent.
#[tokio::test]
async fn a_failed_journal_write_leaves_no_orphaned_gate_entry() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), true, true).with_failing_journal(dir.path(), "full");
    h.receive_from("someone-else@example.com").await;

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&["*"]),
        &graph("email", Some("stranger@example.com")),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    // Degrades to the pre-#227 row, never `pending`: there is no durable
    // card to point the operator at.
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(
        reports[0].status,
        DeliveryStatus::Skipped,
        "a park that could not be journaled is not pending: {reports:?}"
    );
    assert!(
        reports[0].detail.contains("could not be queued"),
        "{reports:?}"
    );

    // THE FINDING: the gate must not still hold the effect.
    assert!(
        h.gate.as_ref().unwrap().parked_ids().is_empty(),
        "a journal write failure must retract the gate entry, not orphan it"
    );
    // …and the operator's queue must not list a card the gate can no longer
    // execute. `record_parked` inserts before it appends, so this only holds
    // because the rollback clears it too.
    assert!(
        h.journal.as_ref().unwrap().pending().is_empty(),
        "no phantom card may be left in the approvals queue"
    );
    // The refusal still held throughout.
    assert!(h.mail.sent().is_empty(), "nothing may leave the process");
}

/// **Fail-closed (issue #227).** With no approvals queue wired, delivery
/// degrades to the pre-#227 `skipped` row rather than promising a `pending`
/// card that nothing is backing. A `pending` row on a runtime with no queue
/// would send the operator to an empty Approvals list.
#[tokio::test]
async fn a_cold_recipient_without_a_queue_falls_back_to_skipped() {
    let dir = tempfile::tempdir().unwrap();
    // No `with_parking`: exactly the shape every non-production
    // construction site builds.
    let h = Harness::new(dir.path(), true, true);
    assert!(h.deps.parking.is_none());

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&["*"]),
        &graph("email", Some("stranger@example.com")),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(
        reports[0].status,
        DeliveryStatus::Skipped,
        "never `pending` with no queue to back it: {reports:?}"
    );
    assert!(reports[0].detail.contains("never written"), "{reports:?}");
    assert!(h.mail.sent().is_empty());
}

/// The established-thread gate is unchanged by #227: a recipient who DID
/// write in still sends immediately, and parks nothing. Parking is what
/// happens to the refusal, not a new hurdle in front of a legitimate send.
#[tokio::test]
async fn an_established_recipient_still_sends_without_parking() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), true, true).with_parking(dir.path(), "full");
    h.receive_from("ada@example.com").await;

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&["*"]),
        &graph("email", Some("ada@example.com")),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports[0].status, DeliveryStatus::Sent, "{reports:?}");
    assert_eq!(h.mail.sent().len(), 1);
    assert!(
        h.journal.as_ref().unwrap().pending().is_empty(),
        "an established send must not clutter the approvals queue"
    );
}

/// The grant gate is unchanged by #227 too, and still runs FIRST: an
/// ungranted company's cold send is `denied` outright, never parked. Parking
/// an effect the company has no grant for would put a card in front of the
/// operator that policy already refused — approving it would be an end-run
/// around `[tools].allow`.
#[tokio::test]
async fn an_ungranted_cold_recipient_is_denied_not_parked() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), true, true).with_parking(dir.path(), "full");

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&["docs.*"]),
        &graph("email", Some("stranger@example.com")),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports[0].status, DeliveryStatus::Denied, "{reports:?}");
    assert!(
        h.journal.as_ref().unwrap().pending().is_empty(),
        "a denied destination must not reach the approvals queue"
    );
    assert!(h.mail.sent().is_empty());
}

/// A company with no mailbox is still `skipped`, not parked: there is
/// nothing to send from, so approving a card would fail at the transport.
/// That arm is checked before the thread gate and #227 does not move it.
#[tokio::test]
async fn a_company_without_a_mailbox_is_skipped_not_parked() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), false, true).with_parking(dir.path(), "full");

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&["*"]),
        &graph("email", Some("stranger@example.com")),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports[0].status, DeliveryStatus::Skipped, "{reports:?}");
    assert!(reports[0].detail.contains("no mailbox"), "{reports:?}");
    assert!(h.journal.as_ref().unwrap().pending().is_empty());
}

/// **Regression (PR #226 review).** A busy company's inbox must not lose an
/// established recipient. `InboxStore::messages` returns oldest-first, so a
/// capped read takes the OLDEST page — and an inbox that outgrows the cap
/// silently stops finding anyone whose mail arrived after it. The failure
/// is fail-closed (never a wrong send) but it is still wrong, and it bites
/// exactly the longest-lived tenants.
///
/// Note the direction: the sender's message must be buried *past* the cap,
/// i.e. among the NEWEST mail. A sender whose message is the oldest sits at
/// index 0 and was always found, cap or no cap.
#[tokio::test]
async fn an_established_sender_is_found_past_the_old_scan_cap() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), true, true);
    // 600 older messages from other people fill the first page…
    for i in 0..600 {
        h.receive_from(&format!("filler{i}@example.com")).await;
    }
    // …so the real correspondent's mail lands well past a 500-message cap.
    h.receive_from("ada@example.com").await;

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&["*"]),
        &graph("email", Some("ada@example.com")),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(
        reports[0].status,
        DeliveryStatus::Sent,
        "a correspondent buried past the scan cap is still an established \
         thread: {reports:?}"
    );
    assert_eq!(h.mail.sent().len(), 1);
}

/// **The default-configuration case (after #230).** A company with no
/// `[tools]` section at all now defaults to the globals `default_allow`,
/// and `*` satisfies the `email` grant — so on the majority of tenants the
/// grant gate is open and the established-thread gate is the one actually
/// holding the line. Pin that it does: a default-configured company still
/// cannot cold-email a stranger.
#[tokio::test]
async fn a_default_configured_company_still_cannot_cold_email() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), true, true);
    let manifest = toml::from_str(
        r#"
[company]
name = "Acme"

[policy]
mode = "full"
"#,
    )
    .expect("valid manifest");
    let mut rec = record(&[]);
    rec.manifest = manifest;
    // Sanity: the default really does grant `email` — if this ever stops
    // being true the test below would pass for the wrong reason.
    assert!(
        crate::harness::build::grants_cover(&rec.manifest.tools.allow, "email"),
        "expected the post-#230 default belt to cover `email`, got {:?}",
        rec.manifest.tools.allow
    );

    let reports = deliver_outputs(
        Some(&h.deps),
        &rec,
        &graph("email", Some("stranger@example.com")),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(
        reports[0].status,
        DeliveryStatus::Skipped,
        "the established-thread gate must still refuse a stranger: {reports:?}"
    );
    assert!(h.mail.sent().is_empty(), "nothing may leave the process");
}

/// The company's OWN prior outbound mail to an address does not make that
/// address established — otherwise one send would bootstrap the next.
#[tokio::test]
async fn a_prior_outbound_does_not_establish_a_thread() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), true, true);
    h.inbox
        .append(
            &h.company,
            &EmailRecord {
                id: generate_id(),
                inbox: local_part(COMPANY_ADDRESS),
                from_name: String::new(),
                from_email: "stranger@example.com".to_string(),
                subject: "earlier".to_string(),
                body: "earlier".to_string(),
                at_millis: 1,
                read: true,
                outbound: true,
            },
        )
        .await
        .unwrap();

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&["*"]),
        &graph("email", Some("stranger@example.com")),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports[0].status, DeliveryStatus::Skipped);
    assert!(h.mail.sent().is_empty());
}

/// Granted and established, but the company has no mailbox: skipped, with a
/// reason distinct from the cold-recipient one.
#[tokio::test]
async fn email_without_a_mailbox_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), false, true);
    h.receive_from("ada@example.com").await;

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&["*"]),
        &graph("email", Some("ada@example.com")),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports[0].status, DeliveryStatus::Skipped);
    assert!(reports[0].detail.contains("no mailbox"), "{reports:?}");
}

/// A transport refusal is reported as `failed` — and, critically,
/// `deliver_outputs` still returns normally, because the run's work is done
/// and must not be thrown away over a mail hiccup.
#[tokio::test]
async fn a_send_failure_is_reported_and_does_not_abort_delivery() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = Harness::new(dir.path(), true, true);
    h.deps.mail = Some(CompanyMail {
        sender: Arc::new(RefusingMailSender),
        smtp: smtp_creds(),
    });
    h.receive_from("ada@example.com").await;

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&["*"]),
        &graph("email", Some("ada@example.com")),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].status, DeliveryStatus::Failed);
    assert!(reports[0].detail.contains("smtp said no"), "{reports:?}");
    assert_eq!(reports[0].reason, DeliveryReason::MailTransportRefused);
    // A refused send leaves no outbound audit record — the mail never went.
    assert!(h.inbox_messages().await.iter().all(|m| !m.outbound));
}

/// **Issue #248 at the source.** A real SMTP refusal quotes the mailbox it
/// refused, so the transport's own words are an address-bearing string. This
/// asserts the split holds where the row is built: `detail` keeps the reply
/// (the operator needs it), `reason` cannot carry it.
///
/// `.invalid` is reserved by RFC 2606 and can never resolve, so the fixture
/// names nobody even if it escapes.
#[tokio::test]
async fn a_refusal_that_quotes_the_address_keeps_it_out_of_the_loggable_half() {
    const ADDRESS: &str = "recipient@example.invalid";

    /// Refuses the way a real MTA does: `550` with the rejected mailbox
    /// echoed back inside the reply.
    struct AddressQuotingMailSender;

    #[async_trait]
    impl MailSender for AddressQuotingMailSender {
        async fn send(
            &self,
            _creds: &MailCredentials,
            email: &OutboundEmail,
        ) -> Result<(), OpenCompanyError> {
            Err(OpenCompanyError::Config(format!(
                "550 5.1.1 <{}>: Recipient address rejected: User unknown in local recipient \
                 table",
                email.to
            )))
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let mut h = Harness::new(dir.path(), true, true);
    h.deps.mail = Some(CompanyMail {
        sender: Arc::new(AddressQuotingMailSender),
        smtp: smtp_creds(),
    });
    h.receive_from(ADDRESS).await;

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&["*"]),
        &graph("email", Some(ADDRESS)),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports.len(), 1, "{reports:?}");
    let row = &reports[0];
    assert_eq!(row.status, DeliveryStatus::Failed);

    // The operator's half is untouched: the reply is what makes this
    // fixable, and the run response goes to the tenant, not the platform.
    assert!(row.detail.contains(ADDRESS), "{row:?}");
    assert!(row.detail.contains("550 5.1.1"), "{row:?}");

    // The loggable half classifies the same failure and cannot carry the
    // address — not by scrubbing it, but by having nowhere to put it.
    assert_eq!(row.reason, DeliveryReason::MailTransportRefused);
    let reason = row.reason.to_string();
    assert!(!reason.contains(ADDRESS), "{reason}");
    assert!(!reason.contains('@'), "{reason}");
    assert!(
        reason.contains("the mail transport refused the message"),
        "{reason}"
    );
}

// --- channel -------------------------------------------------------------

#[tokio::test]
async fn channel_posts_to_the_wired_adapter() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = Harness::new(dir.path(), true, true);
    h.deps.channels = vec![Arc::new(DeskChannel::new(
        h.company.clone(),
        "engineering".to_string(),
        h.events.clone(),
    ))];

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&[]),
        &graph("channel", Some("engineering")),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].status, DeliveryStatus::Sent);
    let events = h
        .events
        .read_from(&h.company, crate::ports::types::EventSeq::new(0), 20)
        .await
        .unwrap();
    assert!(events.iter().any(|event| matches!(
        &event.event,
        CompanyEvent::AgentReply { chat_id, text, .. }
            if chat_id == "engineering" && text.contains("Q3 is up 12%.")
    )));
}

/// A channel the deployment never wired cannot be conjured by a graph. The
/// failure names what IS wired, so the fix is obvious from the run result.
#[tokio::test]
async fn channel_that_is_not_wired_fails_with_the_wired_list() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), true, true);

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&["*"]),
        &graph("channel", Some("telegram")),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].status, DeliveryStatus::Failed);
    // The unwired failure now speaks in the same sentence the console's
    // picker pre-flight shows (issue #981), naming what IS wired — which,
    // since #1757, includes the durable `operator` channel this Harness wires.
    assert!(
        reports[0]
            .detail
            .contains("is not an automation delivery channel"),
        "{reports:?}"
    );
    assert!(reports[0].detail.contains(OPERATOR_CHANNEL), "{reports:?}");
    // The two channel failures are classified apart: "you named a channel
    // that does not exist" and "the channel said no" want different fixes,
    // and the log line only ever sees this half.
    assert_eq!(reports[0].reason, DeliveryReason::ChannelNotWired);
    // The channel id — which for this arm IS the target — stays off the
    // loggable half, same rule as a recipient address (issue #248).
    assert!(
        !reports[0].reason.to_string().contains("telegram"),
        "{reports:?}"
    );
    assert!(h.channel.sent().is_empty());
}

// --- reachability & wiring ----------------------------------------------

/// An `output` node on a branch the run never took gets no attempt and NO
/// ROW. An absent row means "not reached", never "silently dropped".
#[tokio::test]
async fn an_unreached_output_node_produces_no_row() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), true, true);
    h.add_admin("u1", "ada@acme.test").await;
    // The engine reached `start` but never `done`.
    let output = serde_json::json!({
        "nodes": { "start": { "items": [{ "json": { "seed": 1 } }] } }
    });

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&["*"]),
        &graph("owner", None),
        "run-1",
        &output,
        &[],
    )
    .await;

    assert!(reports.is_empty(), "{reports:?}");
    assert!(h.mail.sent().is_empty());
    assert!(h.channel.sent().is_empty());
}

/// An `output` node with no `destination` is the pre-#170 shape. It still
/// shows in the run drawer, still sends nothing, and — since #925 — says so
/// with a `Skipped` row instead of contributing nothing at all.
///
/// **This assertion is inverted from what it was.** It previously read
/// `reports.is_empty()`, which is the behaviour #925 was filed against:
/// silence made "the author routed nothing on purpose" and "the author never
/// configured a destination" the same observation. The transport assertions
/// below are the part that must not change — nothing is sent either way.
#[tokio::test]
async fn an_output_node_without_a_destination_reports_the_gap_and_sends_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), true, true);
    let plain = parse_workflow(
        r#"
id = "plain"
name = "Plain"
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
"#,
    )
    .expect("parses");

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&["*"]),
        &plain,
        "run-1",
        &reached_output(),
        &[],
    )
    .await;
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].status, DeliveryStatus::Skipped);
    assert_eq!(reports[0].reason, DeliveryReason::NoDestinationConfigured);
    assert!(
        h.mail.sent().is_empty() && h.channel.sent().is_empty(),
        "the row is a statement about configuration; nothing may leave the process"
    );
}

/// The #169 lesson: an unwired delivery bundle must be LOUD. It writes a
/// `failed` row onto the run result — where an operator actually looks —
/// rather than skipping in a debug log.
#[tokio::test]
async fn unwired_delivery_reports_loudly_instead_of_skipping() {
    let reports = deliver_outputs(
        None,
        &record(&["*"]),
        &graph("owner", None),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].status, DeliveryStatus::Failed);
    assert_eq!(reports[0].node, "done");
    assert_eq!(reports[0].kind, "owner");
    assert!(reports[0].detail.contains("not wired"), "{reports:?}");
    assert!(
        reports[0].detail.contains("nothing was sent"),
        "{reports:?}"
    );
}

// --- issue #438: one delivery per approval lineage ------------------------

/// One ledger row naming `node`, as a continuation's trigger input carries.
fn already(node: &str, kind: &str) -> Vec<DeliveredReport> {
    vec![DeliveredReport {
        node: node.to_string(),
        kind: kind.to_string(),
    }]
}

/// **The regression.** A continuation reaches the same `output` node again,
/// and must not mail the report a second time. The row says so, and the
/// transport is never touched.
#[tokio::test]
async fn a_report_this_lineage_already_sent_is_not_sent_again() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), true, true);
    h.add_admin("u1", "ada@acme.test").await;

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&[]),
        &graph("owner", None),
        "run-1",
        &reached_output(),
        &already("done", "owner"),
    )
    .await;

    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].status, DeliveryStatus::Skipped);
    assert_eq!(reports[0].reason, DeliveryReason::AlreadyDelivered);
    assert_eq!(reports[0].node, "done");
    assert!(
        h.mail.sent().is_empty(),
        "the transport must never be reached: {:?}",
        h.mail.sent()
    );
    assert!(h.channel.sent().is_empty());
}

/// A report the first run **parked** counts as delivered too. Otherwise
/// every continuation stacks a second identical cold-send card, and
/// approving both mails the stranger twice — `park_cold_recipient` has no
/// dedupe of its own.
#[tokio::test]
async fn a_report_this_lineage_already_parked_is_not_parked_again() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), true, false).with_parking(dir.path(), "full");
    // Cold: the company has never heard from this address.
    let cold = graph("email", Some("stranger@example.test"));

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&["email"]),
        &cold,
        "run-1",
        &reached_output(),
        &already("done", "email"),
    )
    .await;

    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].status, DeliveryStatus::Skipped);
    assert_eq!(reports[0].reason, DeliveryReason::AlreadyDelivered);
    assert!(
        h.journal
            .as_ref()
            .expect("parking wired")
            .pending()
            .is_empty(),
        "a continuation must not stack a second card for one send"
    );
    assert!(h.mail.sent().is_empty());
}

/// The ledger suppresses the node it names and nothing else. A second
/// output node in the same graph still delivers — otherwise one earlier
/// send would silence the whole graph.
#[tokio::test]
async fn a_node_the_ledger_does_not_name_still_delivers() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), true, true);
    h.add_admin("u1", "ada@acme.test").await;

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&[]),
        &graph("owner", None),
        "run-1",
        &reached_output(),
        &already("some_other_node", "owner"),
    )
    .await;

    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].status, DeliveryStatus::Sent);
    assert_eq!(h.mail.sent().len(), 1);
}

/// A run nobody resumed carries an empty ledger, and behaves exactly as it
/// did before #438. Every other test in this module passes `&[]`, so this
/// states the invariant they all rely on.
#[tokio::test]
async fn a_run_with_no_ledger_delivers_normally() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), true, true);
    h.add_admin("u1", "ada@acme.test").await;

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&[]),
        &graph("owner", None),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].status, DeliveryStatus::Sent);
    assert_eq!(h.mail.sent().len(), 1);
}

/// An unreached node on the ledger produces no row at all: "not reached"
/// outranks "already delivered", because there was nothing to deliver this
/// time either way.
#[tokio::test]
async fn an_unreached_node_on_the_ledger_still_produces_no_row() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), true, true);
    let unreached = serde_json::json!({ "nodes": { "start": { "items": [] } } });

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&[]),
        &graph("owner", None),
        "run-1",
        &unreached,
        &already("done", "owner"),
    )
    .await;

    assert!(reports.is_empty(), "{reports:?}");
}

// --- report extraction ---------------------------------------------------

/// Several items concatenate in order; the doubly-wrapped `json.json.text`
/// the engine sometimes emits is read too, with the outer value winning.
#[test]
fn report_text_reads_plain_and_doubly_wrapped_items() {
    let output = serde_json::json!({
        "nodes": { "done": { "items": [
            { "json": { "text": "first" } },
            { "json": { "json": { "text": "second" } } },
            { "json": { "text": "outer", "json": { "text": "inner" } } },
        ] } }
    });
    assert_eq!(report_text(&output, "done"), "first\n\nsecond\n\nouter");
}

/// A data-shaped item with no `text` is delivered as JSON rather than
/// dropped — an empty report would be worse than an ugly one.
#[test]
fn report_text_falls_back_to_json_for_a_textless_item() {
    let output = serde_json::json!({
        "nodes": { "done": { "items": [{ "json": { "revenue": 12 } }] } }
    });
    assert!(report_text(&output, "done").contains("revenue"));
}

#[test]
fn report_text_of_a_node_with_no_items_says_so() {
    let output = serde_json::json!({ "nodes": { "done": { "items": [] } } });
    assert!(report_text(&output, "done").contains("no output"));
}

/// Truncation is character-indexed: a byte slice here would panic
/// mid-codepoint on any multi-byte report.
#[test]
fn truncation_never_splits_a_codepoint() {
    let text = "é".repeat(50);
    let cut = truncate_chars(&text, 10);
    assert!(cut.starts_with(&"é".repeat(10)));
    assert!(cut.ends_with(TRUNCATION_MARKER));
    // Untouched when it fits.
    assert_eq!(truncate_chars("short", 10), "short");
}

// --- issue #529: the write-behind delivery record ------------------------

/// A `Sent` dispatch journals exactly one `WorkflowReportDelivered`, shaped
/// from the row — the durable record a crashed run leaves so a re-run can
/// skip it.
#[tokio::test]
async fn a_sent_delivery_journals_one_record() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = Harness::new(dir.path(), false, true);
    h.deps.channels = vec![Arc::new(DeskChannel::new(
        h.company.clone(),
        "engineering".to_string(),
        h.events.clone(),
    ))];

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&[]),
        &graph("channel", Some("engineering")),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;
    assert_eq!(reports[0].status, DeliveryStatus::Sent, "{reports:?}");

    let journaled = h.journaled_deliveries().await;
    assert_eq!(
        journaled.len(),
        1,
        "exactly one record per dispatch: {journaled:?}"
    );
    let CompanyEvent::WorkflowReportDelivered {
        workflow_id,
        run_id,
        node,
        kind,
        target,
    } = &journaled[0]
    else {
        panic!("expected a WorkflowReportDelivered, got {:?}", journaled[0]);
    };
    assert_eq!(workflow_id, "report_flow");
    assert_eq!(run_id, "run-1");
    assert_eq!(node, "done");
    assert_eq!(kind, "channel");
    assert_eq!(target.as_deref(), Some("engineering"));
    let events = h
        .events
        .read_from(&h.company, crate::ports::types::EventSeq::new(0), 20)
        .await
        .unwrap();
    assert!(events.iter().any(|event| matches!(
        &event.event,
        CompanyEvent::AgentReply { chat_id, text, .. }
            if chat_id == "engineering" && text.contains("Q3 is up 12%.")
    )));
}

/// A `Pending` park journals a record too: the card is durable and approving
/// it sends, so a re-run must treat the report as already delivered —
/// exactly as issue #438's in-lineage ledger counts a `Pending` row.
#[tokio::test]
async fn a_pending_park_journals_a_record() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), true, true).with_parking(dir.path(), "full");
    h.receive_from("someone-else@example.com").await;

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&["*"]),
        &graph("email", Some("stranger@example.com")),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;
    assert_eq!(reports[0].status, DeliveryStatus::Pending, "{reports:?}");

    let journaled = h.journaled_deliveries().await;
    assert_eq!(
        journaled.len(),
        1,
        "a park is a delivery for the ledger: {journaled:?}"
    );
    let CompanyEvent::WorkflowReportDelivered { node, kind, .. } = &journaled[0] else {
        panic!("expected a WorkflowReportDelivered");
    };
    assert_eq!(node, "done");
    assert_eq!(kind, "email");
}

/// A row that did NOT leave the process journals nothing. A `Failed` channel
/// (unwired target) leaves no record, so a re-run is free to retry it.
#[tokio::test]
async fn a_failed_delivery_journals_nothing() {
    let dir = tempfile::tempdir().unwrap();
    // No channel wired, so a `channel` destination fails.
    let h = Harness::new(dir.path(), false, false);

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&[]),
        &graph("channel", Some("operator")),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;
    assert_eq!(reports[0].status, DeliveryStatus::Failed, "{reports:?}");
    assert!(
        h.journaled_deliveries().await.is_empty(),
        "a failed dispatch left the process nothing to record"
    );
}

/// An `AlreadyDelivered` skip journals nothing — the report is on the ledger
/// precisely because it already went out, so recording it again would double
/// the very thing the ledger exists to prevent.
#[tokio::test]
async fn an_already_delivered_skip_journals_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path(), false, true);

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&[]),
        &graph("channel", Some("operator")),
        "run-1",
        &reached_output(),
        &[DeliveredReport {
            node: "done".to_string(),
            kind: "channel".to_string(),
        }],
    )
    .await;
    assert_eq!(reports[0].status, DeliveryStatus::Skipped, "{reports:?}");
    assert_eq!(reports[0].reason, DeliveryReason::AlreadyDelivered);
    assert!(
        h.journaled_deliveries().await.is_empty(),
        "a report skipped because it already went out is not re-recorded"
    );
}

/// A journal that cannot be written does not fail the delivery: the report
/// still sends and its row is still `Sent`. Losing the record risks one
/// duplicate on a later re-run — the accepted write-behind cost, never a
/// failed send.
#[tokio::test]
async fn a_journal_failure_does_not_fail_a_delivery() {
    let dir = tempfile::tempdir().unwrap();
    // A channel that accepts the send, so the journal write is what this
    // case actually reaches. Pointed at `operator` it would fail on the
    // refusal instead and pass for the wrong reason.
    let h = Harness::new(dir.path(), false, true)
        .with_recording_channel("engineering")
        .with_failing_events();

    let reports = deliver_outputs(
        Some(&h.deps),
        &record(&[]),
        &graph("channel", Some("engineering")),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].status, DeliveryStatus::Sent, "{reports:?}");
    assert_eq!(
        h.recording().sent().len(),
        1,
        "the report reached the channel despite the journal"
    );
}

/// Issue #542: the dry router runs the routing half only. A reached output
/// node yields one `Skipped`/`DryRun` row naming where the report WOULD have
/// gone — no deps, no transport, no journal.
#[test]
fn deliver_outputs_dry_routes_a_reached_node_without_sending() {
    let workflow = graph("email", Some("ada@example.com"));
    let reports = deliver_outputs_dry(&record(&["email"]), &workflow, &reached_output());
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].node, "done");
    assert_eq!(reports[0].status, DeliveryStatus::Skipped);
    assert_eq!(reports[0].reason, DeliveryReason::DryRun);
    assert_eq!(reports[0].target.as_deref(), Some("ada@example.com"));
    assert!(
        reports[0].detail.contains("email ada@example.com"),
        "the row should name where it would have gone: {}",
        reports[0].detail
    );
}

// --- issue #925: an unconfigured destination is not the same as no report --

/// **The regression.** A run that reaches an output node naming no
/// destination used to return an empty `deliveries` list, which the console
/// renders as `Finished — this run routed no reports.` — the same sentence
/// it shows a workflow that deliberately routed nothing. The row is what
/// tells the two apart, and it carries the reason as a closed token so a
/// reader does not have to parse prose.
///
/// Deps are `None` here on purpose: the check must land *before* anything
/// touches a transport, so this passes on a runtime with no delivery ports
/// and would fail with a `NotWired` row if the arms were ever reordered.
#[tokio::test]
async fn a_reached_output_node_with_no_destination_says_so() {
    let reports = deliver_outputs(
        None,
        &record(&["*"]),
        &graph_without_destination(),
        "run-1",
        &reached_output(),
        &[],
    )
    .await;

    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].node, "done");
    assert_eq!(reports[0].status, DeliveryStatus::Skipped);
    assert_eq!(reports[0].reason, DeliveryReason::NoDestinationConfigured);
    assert_eq!(
        reports[0].target, None,
        "there is no destination, so there is no target to name"
    );
    assert!(
        reports[0].detail.contains("no destination"),
        "the row has to say what is missing: {}",
        reports[0].detail
    );
}

/// The other half of the same rule: an output node with no destination that
/// the run never reached still contributes nothing. "Never configured" is
/// only worth reporting about a node the run actually arrived at — otherwise
/// every untaken branch would file a complaint.
#[tokio::test]
async fn an_unreached_output_node_with_no_destination_stays_silent() {
    let output = serde_json::json!({ "nodes": { "start": { "items": [] } } });
    let reports = deliver_outputs(
        None,
        &record(&["*"]),
        &graph_without_destination(),
        "run-1",
        &output,
        &[],
    )
    .await;

    assert!(
        reports.is_empty(),
        "an unreached node owes no report either way: {reports:?}"
    );
}

/// An `output` node that only exists to pause for approval is control flow,
/// not a report-back that lost its address. It contributes no row, so a
/// correct gated workflow does not grow a "not delivered" badge on every
/// continuation run.
#[tokio::test]
async fn an_approval_gate_with_no_destination_is_not_reported_as_misconfigured() {
    let gate = parse_workflow(
        r#"
id = "gated"
name = "Gated"
[[node]]
id = "start"
kind = "trigger"
name = "Start"
[[node]]
id = "done"
kind = "output"
name = "Gate"
requires_approval = true
[[edge]]
from = "start"
to = "done"
"#,
    )
    .expect("a gate graph is valid");
    let reports = deliver_outputs(
        None,
        &record(&["*"]),
        &gate,
        "run-1",
        &reached_output(),
        &[],
    )
    .await;
    assert!(
        reports.is_empty(),
        "a gate is not a report-back with a missing address: {reports:?}"
    );
}

/// A test run is where an author most wants to find this, so the dry router
/// takes the same rule.
#[test]
fn deliver_outputs_dry_reports_a_node_with_no_destination() {
    let reports = deliver_outputs_dry(
        &record(&["email"]),
        &graph_without_destination(),
        &reached_output(),
    );
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].node, "done");
    assert_eq!(reports[0].status, DeliveryStatus::Skipped);
    assert_eq!(reports[0].reason, DeliveryReason::NoDestinationConfigured);
}

/// An output node the dry run never reached contributes no row at all —
/// exactly the "absent means not reached" rule the live path takes.
#[test]
fn deliver_outputs_dry_skips_an_unreached_node() {
    let workflow = graph("owner", None);
    // Output where `done` was NOT reached.
    let output = serde_json::json!({ "nodes": { "start": { "items": [] } } });
    let reports = deliver_outputs_dry(&record(&["email"]), &workflow, &output);
    assert!(
        reports.is_empty(),
        "an unreached node routes nothing: {reports:?}"
    );
}

/// Issue #1825 (P1, fifth follow-up — found by chatgpt-codex-connector):
/// "Prevent the synthetic hold from consuming a real decision".
///
/// Pre-fix, `park_and_journal` called `self.approvals.park` — which is what
/// makes an approval id exist for an operator to resolve — strictly
/// *before* arming this card's own `ContinuationQueue` slot (that arm ran
/// only after `record_parked` returned, on the success path). A resolve
/// racing in on another tokio worker thread during `record_parked`'s own
/// async durable append therefore saw a turn whose only armed slot was
/// `park_gated_calls`'s pre-loop synthetic hold, decided against it, and
/// released the batch before this card had been counted; this card's own
/// arm then still landed once the journal write returned, into a fresh,
/// orphaned queue entry no further decision would ever redeem.
///
/// This spies on the approval gate `park_and_journal` calls first and
/// captures `continuations.outstanding(turn)` at that exact point —
/// deterministic, no wall-clock race needed, on the same principle as
/// `approving_the_first_card_of_a_multi_call_node_does_not_complete_the_batch_early`
/// in `workflows::caps::mod`. Pre-fix this captures `0` (nothing armed
/// yet); post-fix it must capture `1`.
#[tokio::test]
async fn park_and_journal_arms_the_continuation_slot_before_the_card_is_parkable() {
    use crate::ports::types::PolicyDecision;

    /// Delegates every call to `inner`, except that `park` first records
    /// how many decisions `turn` is already counted as blocking on —
    /// the moment an operator's resolve could first reach this approval.
    struct Spy {
        inner: Arc<dyn ApprovalGate>,
        continuations: crate::runtime::continuation::ContinuationQueue,
        turn: String,
        outstanding_at_park: std::sync::Mutex<Option<usize>>,
    }

    #[async_trait]
    impl ApprovalGate for Spy {
        async fn evaluate(
            &self,
            company: &CompanyId,
            effect: &Effect,
        ) -> crate::Result<PolicyDecision> {
            self.inner.evaluate(company, effect).await
        }

        async fn park(&self, company: &CompanyId, effect: Effect) -> crate::Result<ApprovalId> {
            *self.outstanding_at_park.lock().expect("spy lock") =
                Some(self.continuations.outstanding(&self.turn));
            self.inner.park(company, effect).await
        }

        async fn resolve(
            &self,
            id: &ApprovalId,
            verdict: Verdict,
            by: Actor,
        ) -> crate::Result<Option<Effect>> {
            self.inner.resolve(id, verdict, by).await
        }
    }

    let dir = tempfile::Builder::new()
        .prefix("oc-1825-p1-5-")
        .tempdir()
        .expect("tempdir");
    let h = Harness::new(dir.path(), false, false).with_parking(dir.path(), "full");
    let parking = h.deps.parking.clone().expect("with_parking wired it");

    let turn = "workflow-node:run-1825-p1-5:work".to_string();
    let spy = Arc::new(Spy {
        inner: parking.approvals.clone(),
        continuations: parking.continuations.clone(),
        turn: turn.clone(),
        outstanding_at_park: std::sync::Mutex::new(None),
    });
    let mut spied_parking = parking.clone();
    spied_parking.approvals = spy.clone();

    let effect = Effect {
        kind: "shell".to_string(),
        group: EffectGroup::Other,
        amount_usd: None,
        established_thread: false,
        first_time_counterparty: false,
        payload: serde_json::json!({ "call": "shell" }),
        agent: Some("ceo".to_string()),
        run_id: None,
    };

    let approval_id = spied_parking
        .park_and_journal(
            &CompanyId::new("acme"),
            effect,
            crate::runtime::journal::TaskLink::Unlinked,
            None,
            Some(turn.clone()),
        )
        .await
        .expect("parks");

    let captured = spy
        .outstanding_at_park
        .lock()
        .expect("spy lock")
        .expect("park was called");
    assert_eq!(
        captured, 1,
        "this card's continuation slot must already be armed by the time the approval \
         gate's park() runs, before record_parked's synchronous insert can make the card \
         resolvable to a concurrent operator — otherwise a decision racing in during \
         record_parked's async durable append can consume a hold this card was never \
         counted against"
    );

    // Sanity: the ordinary, non-racing shape is unchanged — one card on
    // this turn, one decision, releases it immediately.
    assert_eq!(parking.continuations.outstanding(&turn), 1);
    let event = CompanyEvent::ApprovalResolved {
        approval_id,
        verdict: Verdict::Approve,
        by: Actor {
            kind: ActorKind::Operator,
            id: "operator".to_string(),
        },
    };
    assert!(
        parking.continuations.decide(&turn, Some(event)).is_some(),
        "the only card parked on this turn must still release it on its own decision"
    );
}

/// Companion to the test above: when the durable journal write fails, the
/// slot armed before the attempt must be released rather than left
/// blocking the turn on a card that will now never exist.
#[tokio::test]
async fn park_and_journal_releases_the_continuation_slot_when_the_journal_write_fails() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1825-p1-5-fail-")
        .tempdir()
        .expect("tempdir");
    let h = Harness::new(dir.path(), false, false).with_failing_journal(dir.path(), "full");
    let parking = h
        .deps
        .parking
        .clone()
        .expect("with_failing_journal wired it");

    let turn = "workflow-node:run-1825-p1-5-fail:work".to_string();
    let effect = Effect {
        kind: "shell".to_string(),
        group: EffectGroup::Other,
        amount_usd: None,
        established_thread: false,
        first_time_counterparty: false,
        payload: serde_json::json!({ "call": "shell" }),
        agent: Some("ceo".to_string()),
        run_id: None,
    };

    let result = parking
        .park_and_journal(
            &CompanyId::new("acme"),
            effect,
            crate::runtime::journal::TaskLink::Unlinked,
            None,
            Some(turn.clone()),
        )
        .await;
    assert!(
        result.is_err(),
        "the failing journal must still fail the park"
    );
    assert_eq!(
        parking.continuations.outstanding(&turn),
        0,
        "a park whose durable write failed leaves no card for an operator to ever decide, \
         so the slot armed for it before the attempt must be released — otherwise the turn \
         is left permanently blocked on a decision that can never arrive"
    );
}
