//! The manifest knob, the desk snapshot an episode runs over, and what one
//! episode ends up having decided.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tinyhivemind_hive::{EpisodePolicy, QuorumPolicy};

use crate::ports::types::{CompanyRecord, EventSeq};

/// The `[[group_chat]].hive` block: whether this desk deliberates, and how far.
///
/// Every field is optional and every default is derived from the desk's own
/// size, because the size is the only thing the runtime reliably knows. A
/// manifest that says nothing gets a room scaled to its membership rather than
/// a fixed policy that is too tight for a desk of six and meaningless for a
/// desk of two.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HiveConfig {
    /// Whether this desk answers as a room.
    ///
    /// `None` — the default — means "yes, once there are two members to
    /// deliberate with". A desk cannot deliberate with itself, so `Some(true)`
    /// on a one-member desk is still a single turn: the flag says what the
    /// operator wants, not what the desk is able to do. `Some(false)` is the
    /// opt-out, and keeps today's single-responder path byte-for-byte.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Hard cap on turns before the episode reports itself exhausted.
    ///
    /// Defaults to three turns per member: enough for an opening position, a
    /// reply to the room, and a commit, which is the shortest sequence that can
    /// actually reach quorum. Conformity in a room of language models rises
    /// with interaction time, so a bigger budget buys correlated error rather
    /// than a better answer — raise it deliberately or not at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_budget: Option<u32>,
    /// Distinct grounded supporters a topic needs to carry.
    ///
    /// Defaults to a simple majority that still leaves somebody outside it —
    /// `(n / 2 + 1).min(n - 1)` — so a decision is never contingent on the
    /// whole room agreeing. Clamped into `1..=members` on read: a threshold of
    /// zero is refused by the fold, and one above the membership could never be
    /// met by anybody.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quorum: Option<u32>,
    /// Whether the opening round hides peers' positions.
    ///
    /// On by default, and worth keeping on. A shared transcript destroys
    /// independence — the third speaker has read the first two before it
    /// answers — and one blind round is the cheapest available repair for that,
    /// costing a projection flag rather than any concurrency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blind_round: Option<bool>,
    /// Per-member move grammar: member id → the trace kinds that member may
    /// open a line with.
    ///
    /// The knob that turns a vote back into a deliberation. A room whose
    /// members may all `!propose` produces three independent proposals and a
    /// commit — which is what the live `hive_math_lab` run produced in every
    /// one of nine episodes — because in `tinyhivemind` a proposal already
    /// counts as its own author's support, so agreement is reached without
    /// anybody ever having to engage with anybody else's reasoning.
    ///
    /// A member the map does not name may make every move, so an omitted table
    /// is a no-op for every manifest written before it existed. An entry naming
    /// no kinds at all is read the same way — an empty list is a table somebody
    /// started and never filled in far more often than it is a vow of silence.
    ///
    /// Valid kinds are [`moves::MOVE_KINDS`](crate::hivemind::moves::MOVE_KINDS);
    /// an unknown kind and an unknown member id are both refused at validation,
    /// because a typo here fails **open** — the member keeps every move, and the
    /// desk quietly goes on voting.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub moves: BTreeMap<String, Vec<String>>,
    /// Whether support must trace back to a stated fact rather than to another
    /// opinion.
    ///
    /// Off by default, matching `QuorumPolicy::DEFAULT`. On, only support whose
    /// citation chain reaches an `!evidence` counts, and an objection silences
    /// nobody unless its author has itself deposited evidence in the window.
    /// Implies `require_grounded`. It is the second half of the repair `moves`
    /// begins: assigning somebody the evidence seat is worth little if support
    /// can still be grounded in a peer's say-so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_evidential: Option<bool>,
    /// Distinct grounded refuters that cap a topic out of contention, if any.
    ///
    /// `None` — the default — still *records* refutations in the standings; it
    /// declines to let them cap anything. Left off by default deliberately:
    /// tinyhivemind's own benchmark measured the mechanism costing accuracy,
    /// because a refutation is global where an objection is local, so one
    /// member firing it on a noisy read removes an option for the whole room.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refutation_cap: Option<u32>,
    /// Turns one member may take before the attention market damps its bids.
    ///
    /// Defaults to `EpisodePolicy::DEFAULT` (50), which is effectively off for
    /// any budget a desk would actually run. Lower it on a desk where one
    /// member reliably takes the floor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dominance_cap: Option<u32>,
    /// Distinct supporters after which restating a topic scores nothing.
    ///
    /// Defaults to `EpisodePolicy::DEFAULT` (3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repetition_cap: Option<u32>,
    /// Whether a line on this desk may ask another desk a question, and how
    /// far that question may travel.
    ///
    /// Off unless the block says `enabled = true`. Every other knob on this
    /// struct changes how one room argues with itself; this is the only one
    /// that lets a turn run somewhere else, so it is the only one whose default
    /// is "no" rather than "scaled to the desk".
    #[serde(default, skip_serializing_if = "super::referral::ReferralConfig::is_default")]
    pub referral: super::referral::ReferralConfig,
}

impl HiveConfig {
    /// Whether a desk of `members` effective members deliberates under this
    /// config.
    ///
    /// Two members is the floor in both directions: below it there is no room,
    /// and an explicit `enabled = true` cannot conjure one.
    #[must_use]
    pub fn deliberates(&self, members: usize) -> bool {
        members >= 2 && self.enabled != Some(false)
    }

    /// The moves `member` may open a line with, in [`MOVE_KINDS`] order.
    ///
    /// [`MOVE_KINDS`]: crate::hivemind::moves::MOVE_KINDS
    #[must_use]
    pub fn moves_for(&self, member: &str) -> Vec<&'static str> {
        super::moves::allowed_for(&self.moves, member)
    }

    /// Whether `member` may open a line with `kind`.
    #[must_use]
    pub fn may(&self, member: &str, kind: &str) -> bool {
        self.moves_for(member).contains(&kind)
    }
}

/// One seat at a deliberating desk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HiveMember {
    /// The roster teammate's id — the same id its replies are journaled under.
    pub id: String,
    /// What the transcript calls it: its display name, else its id.
    pub label: String,
    /// Its job title, which is the whole of the persona the episode prompt
    /// adds. The teammate's real system prompt is built by the turn itself.
    pub role: String,
}

/// The desk an episode runs over, resolved once at the top of the episode.
///
/// A snapshot rather than a live borrow of the [`CompanyRecord`]: an episode is
/// several turns long, and a room whose membership changed underneath it would
/// hand the floor to somebody the earlier turns never saw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HiveDesk {
    /// Canonical desk id — the `chat_id` every turn is journaled under.
    pub id: String,
    /// Operator-facing desk name.
    pub name: String,
    /// What the desk is for, when the manifest says.
    pub description: Option<String>,
    /// Effective, active members in desk order.
    pub members: Vec<HiveMember>,
    /// The desk's declared hive knob, or the default for an overlay desk.
    pub config: HiveConfig,
}

impl HiveDesk {
    /// The member holding `agent_id`, if it is still seated.
    #[must_use]
    pub fn member(&self, agent_id: &str) -> Option<&HiveMember> {
        self.members.iter().find(|member| member.id == agent_id)
    }

    /// Every member id, in desk order.
    #[must_use]
    pub fn member_ids(&self) -> Vec<String> {
        self.members.iter().map(|m| m.id.clone()).collect()
    }

    /// The episode policy this desk deliberates under.
    #[must_use]
    pub fn policy(&self) -> EpisodePolicy {
        HivePolicy::from_config(&self.config, self.members.len()).episode
    }
}

/// The episode policy derived from a desk's config and its size.
///
/// A named type rather than a bare function so the derivation has somewhere to
/// be tested and documented: every default here is a function of the
/// membership, and getting one wrong is the difference between a room that
/// settles and a room that spends its whole budget restating itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HivePolicy {
    /// The policy handed to [`tinyhivemind_hive::step`].
    pub episode: EpisodePolicy,
}

impl HivePolicy {
    /// Derive the policy for a desk of `members` members.
    ///
    /// `members` is clamped to at least two before anything is derived from it:
    /// the caller has already refused to open an episode below that, and a
    /// zero-member arithmetic underflow here would produce a threshold the fold
    /// rejects rather than a smaller room.
    #[must_use]
    pub fn from_config(config: &HiveConfig, members: usize) -> Self {
        let members = members.max(2);
        let count = u32::try_from(members).unwrap_or(u32::MAX);
        // A simple majority that still leaves somebody outside it, so a
        // decision never requires unanimity. Both operands are at least 1 for
        // `members >= 2`, so the clamp below can only ever tighten an
        // operator's own number.
        let default_threshold = (count / 2 + 1).min(count.saturating_sub(1)).max(1);
        let threshold = config
            .quorum
            .map_or(default_threshold, |asked| asked.clamp(1, count));
        // Three turns per member: an opening position, a reply to the room, and
        // a commit. Anything an operator asks for is honoured, except zero —
        // a budget of nothing is an episode that is exhausted before it starts.
        let turn_budget = config
            .turn_budget
            .unwrap_or_else(|| count.saturating_mul(3))
            .max(1);
        // `require_evidential` implies `require_grounded` in the library, but
        // the flag is set here as well rather than left to be implied: a policy
        // that says one and not the other reads, in a log or a test, as a
        // policy that meant it.
        let require_evidential = config.require_evidential.unwrap_or(false);
        Self {
            episode: EpisodePolicy {
                turn_budget,
                blind_round: config.blind_round.unwrap_or(true),
                // Both caps are the library's own defaults unless the manifest
                // says otherwise, and a zero is refused at validation rather
                // than clamped here — a `dominance_cap = 0` would damp every
                // member's first bid, which is a room that never speaks.
                dominance_cap: config
                    .dominance_cap
                    .unwrap_or(EpisodePolicy::DEFAULT.dominance_cap)
                    .max(1),
                repetition_cap: config
                    .repetition_cap
                    .unwrap_or(EpisodePolicy::DEFAULT.repetition_cap)
                    .max(1),
                quorum: QuorumPolicy {
                    threshold,
                    require_grounded: require_evidential || QuorumPolicy::DEFAULT.require_grounded,
                    require_evidential,
                    refutation_cap: config.refutation_cap,
                    ..QuorumPolicy::DEFAULT
                },
                ..EpisodePolicy::DEFAULT
            },
        }
    }
}

/// How an episode ended, and what it ended on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EpisodeEnding {
    /// One topic carried and the room recorded it.
    Converged {
        /// The topic that carried.
        topic: String,
        /// The members whose grounded support carried it.
        supporters: Vec<String>,
    },
    /// Two or more topics carried at once and nobody broke the tie.
    Deadlocked {
        /// Every tied topic.
        topics: Vec<String>,
    },
    /// The turn budget ran out first.
    Exhausted,
    /// Nobody's urge to speak cleared their threshold.
    Idle,
}

impl EpisodeEnding {
    /// A stable one-word label, for logs and tests.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Converged { .. } => "converged",
            Self::Deadlocked { .. } => "deadlocked",
            Self::Exhausted => "exhausted",
            Self::Idle => "idle",
        }
    }
}

/// What one episode cost and what it decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EpisodeOutcome {
    /// How it ended.
    pub ending: EpisodeEnding,
    /// Turns actually taken — never more than the policy's budget.
    pub turns: u32,
    /// The journal sequence of the episode's first turn, if it took one.
    pub first_seq: Option<EventSeq>,
    /// The journal sequence of its last turn, if it took one.
    pub last_seq: Option<EventSeq>,
    /// The journal sequence of the closing `hive-report` row, when it was
    /// written. `None` only when the append itself failed, which is logged and
    /// not fatal — a decision the room actually reached is already durable in
    /// the turns above it.
    pub report_seq: Option<EventSeq>,
    /// Lines a member deposited that its seat was not entitled to make, after
    /// the correction and the second attempt both failed.
    ///
    /// Reported rather than merely logged: a desk whose move grammar is wrong
    /// for the work looks, from the transcript alone, exactly like a desk whose
    /// members are being unhelpful, and the operator is the only one who can
    /// tell the two apart.
    pub violations: Vec<super::moves::MoveViolation>,
    /// Turns whose member did not answer at all.
    ///
    /// A failed turn does not end the room — the live case was one member
    /// hitting the harness's per-turn wall-clock ceiling, which propagated and
    /// threw away three good turns — so the episode records the miss on the
    /// desk and steps on. Counted here so the operator is told the answer was
    /// reached with a seat missing rather than left to infer it.
    pub failed_turns: u32,
    /// What this episode asked of other desks, if anything.
    ///
    /// Empty for every desk that did not opt in to referral, which is the
    /// default and the overwhelmingly common case.
    pub referrals: super::referral::ReferralLedger,
}

impl EpisodeOutcome {
    /// The operator-facing sentence the closing `hive-report` row carries.
    ///
    /// Deliberately says what happened rather than restating the decision's
    /// content: the argument is in the transcript directly above it, and a
    /// summary that paraphrases it would be a second, unattributed account of a
    /// conversation that already has one.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "{}{}{}{}",
            self.ending_summary(),
            self.failure_summary(),
            self.violation_summary(),
            self.referral_summary()
        )
    }

    /// The sentence naming what the room asked of other desks, or nothing when
    /// it asked nothing.
    ///
    /// Reported rather than left to the transcript because a crossing question
    /// is the one thing an episode does that an operator reading *this* desk
    /// cannot see: the far turn was journaled on the far desk, and only the
    /// answer came home.
    #[must_use]
    pub fn referral_summary(&self) -> String {
        let ledger = &self.referrals;
        if ledger.asked.is_empty() && ledger.failed == 0 && ledger.over_cap == 0 {
            return String::new();
        }
        let mut parts = Vec::new();
        if !ledger.asked.is_empty() {
            let named = ledger
                .asked
                .iter()
                .map(|question| format!("@{} on {}", question.target, question.desk))
                .collect::<Vec<_>>()
                .join(", ");
            let count = ledger.asked.len();
            let plural = if count == 1 { "question" } else { "questions" };
            parts.push(format!("asked {count} {plural} of another desk ({named})"));
        }
        if ledger.failed > 0 {
            parts.push(format!("{} went unanswered", ledger.failed));
        }
        if ledger.over_cap > 0 {
            parts.push(format!(
                "{} more were declined by this desk's `referral.peer_cap`",
                ledger.over_cap
            ));
        }
        format!(" The room {}.", parts.join("; "))
    }

    /// The sentence naming turns that did not finish, or nothing when they all
    /// did.
    #[must_use]
    pub fn failure_summary(&self) -> String {
        match self.failed_turns {
            0 => String::new(),
            1 => " 1 turn did not finish and the room continued without it.".to_owned(),
            count => format!(" {count} turns did not finish and the room continued without them."),
        }
    }

    /// The sentence naming demoted lines, or nothing when there were none.
    #[must_use]
    pub fn violation_summary(&self) -> String {
        if self.violations.is_empty() {
            return String::new();
        }
        let count = self.violations.len();
        let plural = if count == 1 { "line" } else { "lines" };
        let named = self
            .violations
            .iter()
            .map(|violation| format!("@{} !{}", violation.agent_id, violation.attempted))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            " {count} {plural} demoted for a move its author may not make on this desk: {named}."
        )
    }

    /// How it ended, without the move-grammar postscript.
    #[must_use]
    pub fn ending_summary(&self) -> String {
        let turns = self.turns;
        let plural = if turns == 1 { "turn" } else { "turns" };
        match &self.ending {
            EpisodeEnding::Converged { topic, supporters } => {
                let backing = if supporters.is_empty() {
                    "the room".to_owned()
                } else {
                    supporters.join(", ")
                };
                format!(
                    "The desk settled on #{topic} after {turns} {plural} (backed by {backing})."
                )
            }
            EpisodeEnding::Deadlocked { topics } => format!(
                "The desk deadlocked after {turns} {plural}: {} carried together and nobody broke the tie.",
                topics
                    .iter()
                    .map(|topic| format!("#{topic}"))
                    .collect::<Vec<_>>()
                    .join(" and "),
            ),
            EpisodeEnding::Exhausted => {
                format!("The desk spent its {turns}-turn budget without reaching a decision.",)
            }
            EpisodeEnding::Idle => {
                "Nobody on the desk had anything to add, so the room did not open.".to_owned()
            }
        }
    }
}

/// The desk a hive episode should answer `chat` on, or `None` to keep today's
/// single-responder path.
///
/// Every rung here is a reason NOT to open a room, and each one matters:
///
/// - **No addressed chat.** An unaddressed message is the company's own line
///   and is answered by the orchestrator, which is not a desk.
/// - **A General spelling.** The main thread is a conversation with the
///   company, not a deliberation among a department — and it folds four
///   spellings into one channel, so a room opened there would have no stable
///   membership to open over.
/// - **A key that names no desk.** A bare teammate id or a `dm:` thread is
///   addressed to one teammate by construction.
/// - **Fewer than two effective members.** The overwhelmingly common shape:
///   every desk in a one-teammate-per-desk company, and every desk whose
///   members have since left the roster. There is nobody to deliberate with, so
///   the desk answers exactly as it did before.
/// - **`hive.enabled = false`.** The operator's own opt-out.
///
/// Membership is read through [`CompanyRecord::effective_desk_members`], the
/// same source `desk_lead` and the console's desk list read, so who is in the
/// room and who the console says is in it cannot drift.
#[must_use]
pub fn desk_episode(record: &CompanyRecord, chat: Option<&str>) -> Option<HiveDesk> {
    let chat = chat?;
    if crate::server::chat_history::is_general_chat(Some(chat)) {
        return None;
    }
    let desk_id = record.resolve_desk_id(chat)?;
    let declared = record
        .manifest
        .group_chats
        .iter()
        .find(|group| group.id == desk_id);
    let config = declared.map(|group| group.hive.clone()).unwrap_or_default();
    let members: Vec<HiveMember> = record
        .effective_desk_members(&desk_id)
        .into_iter()
        .filter(|id| record.is_roster_agent(id))
        .map(|id| member_of(record, &id))
        .collect();
    if !config.deliberates(members.len()) {
        return None;
    }
    Some(HiveDesk {
        name: declared.map_or_else(|| desk_id.clone(), |group| group.name.clone()),
        description: declared.and_then(|group| group.description.clone()),
        id: desk_id,
        members,
        config,
    })
}

/// Every desk and teammate the episode on `home` may refer a turn to.
///
/// Built from the same two sources [`desk_episode`] reads — the manifest's
/// declared desks and [`CompanyRecord::effective_desk_members`] — so a desk a
/// member can name is a desk the console agrees exists, and `@#id` can never
/// resolve to a room nobody is in.
///
/// Returns `None` when there is nothing to refer to: referral not enabled on
/// the home desk, or no peer desk with a member on it. `None` is what keeps the
/// whole mechanism inert for the overwhelmingly common company — one that never
/// wrote the block.
#[must_use]
pub fn desk_federation(
    record: &CompanyRecord,
    home: &HiveDesk,
) -> Option<super::referral::HiveFederation> {
    if !home.config.referral.enabled() {
        return None;
    }
    let desks: Vec<super::referral::FederationDesk> = record
        .manifest
        .group_chats
        .iter()
        .map(|group| super::referral::FederationDesk {
            id: group.id.clone(),
            name: group.name.clone(),
            description: group.description.clone(),
            members: record
                .effective_desk_members(&group.id)
                .into_iter()
                .filter(|id| record.is_roster_agent(id))
                .collect(),
        })
        .collect();
    let federation = super::referral::HiveFederation {
        agents: desks
            .iter()
            .flat_map(|desk| desk.members.iter())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(|id| {
                let seat = member_of(record, id);
                (seat.id, seat.label)
            })
            .collect(),
        desks,
    };
    if federation.peers_of(&home.id).is_empty() {
        return None;
    }
    Some(federation)
}

/// One seat, built from whichever roster half declares the teammate.
fn member_of(record: &CompanyRecord, id: &str) -> HiveMember {
    if let Some(agent) = record.manifest.agents.iter().find(|a| a.id == id) {
        return HiveMember {
            id: agent.id.clone(),
            label: agent.name.clone().unwrap_or_else(|| agent.id.clone()),
            role: agent.role.clone(),
        };
    }
    if let Some(agent) = record.overlay_agents.iter().find(|a| a.id == id) {
        return HiveMember {
            id: agent.id.clone(),
            label: agent.name.clone(),
            role: agent.role.clone(),
        };
    }
    // Unreachable through `desk_episode`, which filters on `is_roster_agent`
    // first. Kept total anyway rather than panicking: a seat with no persona is
    // a worse prompt, not a reason to fail the operator's message.
    HiveMember {
        id: id.to_owned(),
        label: id.to_owned(),
        role: "teammate".to_owned(),
    }
}
