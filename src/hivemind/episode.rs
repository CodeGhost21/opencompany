//! The host loop: fold, run the one turn the library authorized, append it,
//! commit the state it returned, repeat.
//!
//! That ordering is the whole contract. [`step`] never appends, never waits and
//! never calls back into this host; it hands back a `next_state` that is only
//! valid once the turn it authorized is **durably** in the journal. Committing
//! it before the append would let a failed write leave the episode believing a
//! turn happened that nothing can read back.
//!
//! Reading the transcript back out of the journal on every iteration — rather
//! than accumulating it in memory as the loop runs — is the same discipline
//! seen from the other side: what the room folds is exactly what a human
//! reading the desk would see, so the standings can never disagree with the
//! transcript they were derived from.

use std::sync::Arc;

use async_trait::async_trait;
use tinyhivemind_hive::{
    Conversation, EpisodeState, HiveStep, SESSION_WINDOW, Sequence, SessionQuery,
    desk::{Desk, DeskSet, ResponderMode},
    pins::{PIN_LIMIT, read_pinboard},
    project_for,
    roster::{Roster, RosterMember},
    step,
};

use super::log::EventLogSessionLog;
use super::prompt::{EpisodePrompt, marker_line};
use super::types::{EpisodeEnding, EpisodeOutcome, HiveDesk};
use crate::error::OpenCompanyError;
use crate::ports::events::EventLog;
use crate::ports::types::{CompanyEvent, CompanyId, EventSeq};
use crate::Result;

/// Anything that can fill one authorized turn.
///
/// Deliberately the narrowest possible seam — an agent id and a prompt in, one
/// reply out. The production implementation is a normal harness turn, with its
/// tools, its memory loop and its approval gate all intact; a test scripts
/// replies without a model. Neither knows anything about episodes, which is
/// what keeps the driver testable without a provider and keeps a deliberating
/// teammate identical to a replying one in every way but the prompt.
#[async_trait]
pub trait HiveTurnRunner: Send + Sync {
    /// Run one turn on `agent_id` and return what it said.
    ///
    /// # Errors
    ///
    /// Returns whatever the underlying turn failed with. A failed turn ends the
    /// episode: the turns already appended stay in the transcript, and no
    /// closing report claims a decision the room did not reach.
    async fn speak(&self, agent_id: &str, prompt: &str) -> Result<String>;
}

/// One deliberation episode on one desk.
pub struct EpisodeDriver<'a> {
    company: CompanyId,
    desk: HiveDesk,
    events: Arc<dyn EventLog>,
    runner: &'a dyn HiveTurnRunner,
    task: String,
    thread_root: Option<EventSeq>,
}

impl std::fmt::Debug for EpisodeDriver<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EpisodeDriver")
            .field("company", &self.company)
            .field("desk", &self.desk.id)
            .field("thread_root", &self.thread_root)
            .finish_non_exhaustive()
    }
}

impl<'a> EpisodeDriver<'a> {
    /// Open a driver over `desk`, answering `task`.
    #[must_use]
    pub fn new(
        company: CompanyId,
        desk: HiveDesk,
        events: Arc<dyn EventLog>,
        runner: &'a dyn HiveTurnRunner,
        task: impl Into<String>,
    ) -> Self {
        Self {
            company,
            desk,
            events,
            runner,
            task: task.into(),
            thread_root: None,
        }
    }

    /// Run the episode inside the thread rooted at `thread_root`.
    ///
    /// `None` — the default — is the desk channel itself, which is where every
    /// unparented message lives. Every turn is journaled with this same parent,
    /// exactly as a single-responder reply is: an answer joins the thread its
    /// question was asked in rather than opening one underneath it, and the
    /// library's own channel projection only promotes rows whose parent is a
    /// root, so a turn parented to the operator's message would be invisible to
    /// the rest of the episode.
    #[must_use]
    pub fn in_thread(mut self, thread_root: Option<EventSeq>) -> Self {
        self.thread_root = thread_root;
        self
    }

    /// Deliberate until the room converges, deadlocks, exhausts its budget, or
    /// finds it has nothing to say.
    ///
    /// `trigger` is the journal sequence of the operator message that opened
    /// the room, and becomes the episode's watermark: everything at or below it
    /// is context the room may read and may cite, but is not folded into the
    /// episode's own traces. Without it a desk would inherit the votes of every
    /// conversation that preceded this one.
    ///
    /// # Errors
    ///
    /// Returns [`OpenCompanyError::Config`] when the roster, desk snapshot or
    /// policy this host built is malformed — which can only mean a bug here,
    /// since all three are derived from a validated manifest — and the journal's
    /// own error when a turn cannot be appended. A failed append stops the
    /// episode before the state it would have committed is taken up, so the
    /// room never believes in a turn nothing can read.
    pub async fn run(&self, trigger: EventSeq) -> Result<EpisodeOutcome> {
        let conversation = Conversation {
            desk_id: self.desk.id.clone(),
            desk_name: self.desk.name.clone(),
            thread_root: self.thread_root.map(|seq| Sequence(seq.value())),
        };
        let log = EventLogSessionLog::new(
            Arc::clone(&self.events),
            self.company.clone(),
            self.desk.id.clone(),
            self.desk.name.clone(),
        );
        let members: Vec<RosterMember> = self
            .desk
            .members
            .iter()
            .map(|member| RosterMember {
                id: member.id.clone(),
                name: Some(member.label.clone()),
            })
            .collect();
        let desks = vec![Desk {
            id: self.desk.id.clone(),
            name: self.desk.name.clone(),
            description: self.desk.description.clone(),
            members: self.desk.member_ids(),
            // `Auto`, always. The room is the responder; a lead is what a desk
            // has when exactly one member answers, which is the path this
            // episode was opened instead of.
            responder_mode: ResponderMode::Auto,
        }];
        let retired: Vec<String> = Vec::new();
        let policy = self.desk.policy();

        let mut state = EpisodeState::opened(conversation.clone(), Sequence(trigger.value()));
        let mut turns = 0_u32;
        let mut first_seq: Option<EventSeq> = None;
        let mut last_seq: Option<EventSeq> = None;

        let ending = loop {
            let transcript = tinyhivemind_hive::project_session(
                &log,
                &SessionQuery {
                    conversation: conversation.clone(),
                    before: None,
                    window: SESSION_WINDOW,
                },
            )
            .await
            .map_err(|error| self.malformed(&error))?;

            let decision = {
                let roster = Roster::new(&members, &[], &retired);
                let desk_set = DeskSet::new(&desks, &[], &[], &[], &retired);
                step(&state, &transcript, &roster, &desk_set, &policy)
                    .map_err(|error| self.malformed(&error))?
            };

            let turn = match decision {
                HiveStep::Speak { turn } => *turn,
                HiveStep::Converged { topic, standing } => {
                    break EpisodeEnding::Converged {
                        topic: topic.to_string(),
                        supporters: standing.supporters.clone(),
                    };
                }
                HiveStep::Deadlocked { topics } => {
                    break EpisodeEnding::Deadlocked {
                        topics: topics.iter().map(ToString::to_string).collect(),
                    };
                }
                HiveStep::Exhausted { .. } => break EpisodeEnding::Exhausted,
                HiveStep::Idle => break EpisodeEnding::Idle,
            };

            let visible = project_for(&turn, &transcript);
            // Folded fresh each turn from the same journal the transcript came
            // from, so a pin laid down *during* the episode is on the board for
            // the next speaker rather than the next episode.
            let pins = read_pinboard(&log, &conversation, PIN_LIMIT, None)
                .await
                .unwrap_or_default();
            let member = self.desk.member(&turn.agent_id).ok_or_else(|| {
                OpenCompanyError::Config(format!(
                    "hive episode on desk `{}`: the floor was given to `{}`, who is not seated",
                    self.desk.id, turn.agent_id,
                ))
            })?;
            let prompt =
                EpisodePrompt::new(member, &self.desk, &self.task, policy.quorum, &pins)
                    .render(&turn, &visible);

            let reply = self.runner.speak(&turn.agent_id, &prompt).await?;
            let seq = self
                .events
                .append(
                    &self.company,
                    CompanyEvent::AgentReply {
                        chat_id: self.desk.id.clone(),
                        agent_id: turn.agent_id.clone(),
                        text: marker_line(&reply),
                        // The episode's own turns carry no step timeline: the
                        // room is reading one line per turn, and a tool trace
                        // belongs to the turn's own bubble, which this path
                        // does not raise.
                        steps: Vec::new(),
                        task_id: None,
                        parent: self.thread_root,
                        // A deliberation line names topics and message numbers,
                        // not people. Left empty rather than half-resolved —
                        // and a reply's mentions are never consulted by
                        // dispatch anyway, which is the mention-loop fuse.
                        mentions: Vec::new(),
                        mention_depth: 0,
                    },
                )
                .await?;
            first_seq.get_or_insert(seq);
            last_seq = Some(seq);
            // Durably appended, so — and only now — the state the library
            // returned may be taken up.
            state = turn.next_state;
            turns = turns.saturating_add(1);
        };

        let mut outcome = EpisodeOutcome {
            ending,
            turns,
            first_seq,
            last_seq,
            report_seq: None,
        };
        outcome.report_seq = self.report(&outcome).await;
        Ok(outcome)
    }

    /// Journal the closing row, under [`HIVE_REPORT_AUTHOR`].
    ///
    /// Best-effort: the decision itself is already durable in the turns above
    /// it, and losing the room's own summary of a conversation the transcript
    /// still holds is not worth discarding the episode over.
    ///
    /// [`HIVE_REPORT_AUTHOR`]: super::HIVE_REPORT_AUTHOR
    async fn report(&self, outcome: &EpisodeOutcome) -> Option<EventSeq> {
        match self
            .events
            .append(
                &self.company,
                CompanyEvent::AgentReply {
                    chat_id: self.desk.id.clone(),
                    agent_id: super::HIVE_REPORT_AUTHOR.to_string(),
                    text: outcome.summary(),
                    steps: Vec::new(),
                    task_id: None,
                    parent: self.thread_root,
                    mentions: Vec::new(),
                    mention_depth: 0,
                },
            )
            .await
        {
            Ok(seq) => Some(seq),
            Err(error) => {
                tracing::warn!(
                    company = %self.company,
                    desk = %self.desk.id,
                    error = %error,
                    "[hive] the episode ended but its outcome row could not be journaled; \
                     the transcript still holds every turn"
                );
                None
            }
        }
    }

    /// A library error, named as what it actually is on this host.
    fn malformed(&self, error: &dyn std::fmt::Display) -> OpenCompanyError {
        OpenCompanyError::Config(format!("hive episode on desk `{}`: {error}", self.desk.id))
    }
}
