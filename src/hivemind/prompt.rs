//! What one authorized turn is shown.
//!
//! This is a port of the reference host's prompt builder
//! (`tinyhivemind-hive/examples/bench/live.rs`), and it is deliberately a port
//! rather than a fresh design: every block here exists because a live room
//! failed without it.
//!
//! - **The move list is phase-gated.** `!commit` is absent while the room is
//!   still deliberating, because the library authorizes a commit turn by
//!   setting the phase — a `!commit` deposited before that adds no supporter to
//!   anything, and a room that reaches for it early spends its whole budget
//!   recording a decision it never reached. In the Commit phase it is present
//!   for **every** seat, and the block names the carried topic outright: the
//!   fold hands the floor to whoever the attention market picks, so the seat
//!   asked to record a decision is regularly not the seat that reached it.
//! - **The task's topic id is stated.** A room that coins `#euler12`,
//!   `#euler12-triangle` and `#euler12-triangular` for one number splits its
//!   own support three ways and never carries anything. The id is derived once
//!   from the operator's message and every prompt of the episode repeats it.
//! - **The floor is rendered with its standings.** Models coin a fresh topic id
//!   for an idea the room already has one for (`#rollout` and
//!   `#rollout-strategy` in one episode), and support split across two names
//!   never adds up to a quorum. A member also cannot tell that one more
//!   supporter would settle a question unless it is shown how far each option
//!   is from carrying.
//! - **A member is shown its own last line.** Live models restate their
//!   previous line verbatim when they have nothing new; `repetition_cap` damps
//!   a restated *support* and cannot see this at all.
//! - **The pinboard is rendered.** The transcript window is thirty messages, so
//!   what the desk settled long ago is otherwise gone; a `!pin` is how it stays
//!   unavoidable.
//!
//! The whole prompt is assembled from the projection the library handed this
//! turn, so a host driving a live run can predict exactly what an agent sees
//! from the transcript plus the turn's own visibility.

use tinyhivemind_hive::{
    HiveTurn, Phase, QuorumPolicy, Sequence, SessionAuthor, SessionMessage, Visibility, pins::Pin,
    quorum::standings, trace::resolve,
};

use super::memory::{HiveMemoryHit, MAX_RECALL_CHARS};
use super::types::{HiveDesk, HiveMember};

/// One line of the move list, per kind.
///
/// Rendered per seat rather than as one fixed block: a desk that assigns moves
/// must not show a member a marker it is about to be corrected for using. The
/// prompt and the enforcement therefore read the same table, so what a member
/// is shown is exactly what it is allowed to deposit.
fn move_line(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "propose" => "!propose #topic  then one sentence putting a new option on the floor",
        "support" => "!support #topic ^N  then why, citing message N as grounds",
        "object" => "!object >N ^M  then why, objecting to message N and citing message M",
        "refute" => {
            "!refute #topic ^N  then the fact that argues against the option itself, citing \
             message N"
        }
        "evidence" => {
            "!evidence #topic ^N  then a fact, adding grounds without taking a side; keep the # \
             when the fact bears on a named option"
        }
        "question" => "!question  then what you need that nobody has established",
        "defer" => "!defer #topic  then who should answer instead, when this is not your area",
        "pin" => {
            "!pin ^N  then why message N must stay on the board once the window has scrolled past \
             it"
        }
        _ => return None,
    })
}

/// The rules those moves are read under.
const DELIBERATE_RULES: &str = "\
The # on a topic and the ^ on a citation are part of the grammar: `!propose \
#canary ...` names an option, `!propose canary ...` names nothing and is \
discarded. A support with no ^citation does not count, and only support moves \
an option towards a decision. Angle brackets are not part of any line — write \
the sentence itself, not a placeholder in brackets. The marker is what the \
room counts and your prose is not, so never write !support for one option \
while arguing for another: put the marker on the option you actually mean. Do \
not write !commit: the room has not reached a decision yet, and a commit line \
now counts for nothing. !defer costs you this turn and adds no support to \
anything. Use it when the question on the floor turns on something you do not \
hold and somebody here does: a confident guess from outside your area is \
worse for the room than saying so and standing aside. If you have nothing to \
add, reply !defer #topic naming who should act next, or !question. Prose \
without a marker counts for nothing and costs the room a turn. Write nothing \
before or after the single marker line.";

/// The extra sentence a room under `require_evidential` is given.
///
/// Rendered only when the desk actually requires it, because on a desk that
/// does not, citing a proposal is a perfectly good support and a rule saying
/// otherwise would be false.
const EVIDENTIAL_RULE: &str = "\
This desk counts a !support only when its ^citation reaches an !evidence line \
— either the evidence itself, or a support that cites it. A !support that \
cites only a !propose counts for nothing, however well argued: the room needs \
a fact under the option, not a second opinion about it. If no evidence is on \
the floor yet, put one there with !evidence #topic ^N.";

/// The extra sentence a seat with an assigned grammar is given.
///
/// Only rendered when the desk actually narrowed this member, because a member
/// that may make every move is not being restricted and telling it so would be
/// a rule about nothing.
const ASSIGNED_MOVES_RULE: &str = "\
These are the ONLY markers this desk gives you. A line opening with any other \
marker is handed back to you once for correction, and on a second attempt it \
is journaled with its marker stripped — it will say what you wrote and count \
for nothing.";

/// The move every seat has once the room has reached quorum, naming the topic
/// that carried.
///
/// The topic is named rather than described because the seat holding the floor
/// in the Commit phase is regularly not one of the seats that carried it: the
/// attention market picks the speaker, so the cheapest member on the desk can
/// be the one asked to do the bookkeeping. Told *which* id to record, it needs
/// to re-derive nothing.
fn commit_protocol(topic: &str) -> String {
    format!(
        "The room has reached quorum and carried `#{topic}`; record it. Reply with ONE line \
         only:\n!commit #{topic} ^N  then why, citing the evidence it rests on\nKeep the # on the \
         topic and the ^ on the citation; without them the line records nothing. Angle brackets \
         are not part of the line — write the sentence itself. This is bookkeeping, not a fresh \
         judgement: record the topic the room actually settled on rather than the one you would \
         have preferred, and do not re-derive the answer. Write nothing before or after the \
         single marker line."
    )
}

/// Everything about one seat in a room except how its answer is fetched.
#[derive(Debug)]
pub struct EpisodePrompt<'a> {
    /// The member taking this turn.
    member: &'a HiveMember,
    /// The desk it is taking it on.
    desk: &'a HiveDesk,
    /// What the operator asked, verbatim — the reason the room opened.
    task: &'a str,
    /// The room's quorum rule. Public on purpose: a participant is entitled to
    /// know how many grounded supporters settle a question, and a member that
    /// cannot see how close an option is has no way to know one more supporter
    /// would end it.
    quorum: QuorumPolicy,
    /// The desk's pinboard, folded from the same journal the transcript came
    /// from.
    pins: &'a [Pin],
    /// What the desk remembers, recalled once at the top of the episode.
    recall: &'a [HiveMemoryHit],
    /// Members who have not taken a turn yet in this episode, when the fold has
    /// just handed the floor back to whoever spoke last.
    unspoken: &'a [String],
}

impl<'a> EpisodePrompt<'a> {
    /// Build the prompt state for one seat.
    #[must_use]
    pub fn new(
        member: &'a HiveMember,
        desk: &'a HiveDesk,
        task: &'a str,
        quorum: QuorumPolicy,
        pins: &'a [Pin],
    ) -> Self {
        Self {
            member,
            desk,
            task,
            quorum,
            pins,
            recall: &[],
            unspoken: &[],
        }
    }

    /// Render what the desk remembers from earlier episodes into this prompt.
    ///
    /// Recalled once per episode by the driver and handed to every turn, so a
    /// room does not pay one store round-trip per speaker for an answer that
    /// cannot change mid-episode.
    #[must_use]
    pub fn with_recall(mut self, recall: &'a [HiveMemoryHit]) -> Self {
        self.recall = recall;
        self
    }

    /// Name the members who have not spoken yet in this episode.
    ///
    /// Rendered only when the driver has something to say with it — the fold
    /// gave the floor back to the member who just held it while somebody has
    /// still not used theirs. It is a *prompt*, never an override: the library
    /// picked this speaker under invariants this host does not get to break, so
    /// the repair available is to tell the speaker who is missing and let it
    /// `!question` or `!defer` to them.
    #[must_use]
    pub fn with_unspoken(mut self, unspoken: &'a [String]) -> Self {
        self.unspoken = unspoken;
        self
    }

    /// Render exactly what this turn is allowed to see.
    #[must_use]
    pub fn render(&self, turn: &HiveTurn, visible: &[&SessionMessage]) -> String {
        let sight = match turn.visibility {
            Visibility::Blind => "You cannot yet see your peers' positions. Form your own first.",
            Visibility::Full => "You can see the whole room.",
        };
        // Folded once and read twice: the block a member reads its standings
        // off and the topic the Commit phase names have to be the same fold,
        // or a seat could be told to record a topic the floor does not show.
        let standings = self.standings(visible);
        let protocol = match turn.phase {
            Phase::Commit => commit_protocol(&self.carried(&standings)),
            Phase::Deliberate => self.deliberate_protocol(),
        };
        format!(
            "You are @{}, the {} on the {} desk. {sight}\n\n{}{}{}\n\n{protocol}\n\n{}{}{}{}\
             Shared attributed transcript:\n{}\n\nYour one line:",
            self.member.id,
            self.member.role,
            self.desk.name,
            self.room(),
            self.remembered(),
            self.board(),
            self.topic_discipline(turn.phase),
            self.floor(&standings),
            self.missing(),
            self.last_line(visible),
            render_transcript(visible),
        )
    }

    /// The topic id this task's answer is named by, and the rule for coining
    /// another.
    ///
    /// Rendered only while the room is deliberating: the Commit block names the
    /// carried id outright, so repeating the derivation rule there would be a
    /// second, weaker instruction about the same string.
    fn topic_discipline(&self, phase: Phase) -> String {
        if phase == Phase::Commit {
            return String::new();
        }
        let topic = canonical_topic(self.task);
        format!(
            "Topic id for this task's answer: `#{topic}`. Every !propose, !support and !evidence \
             about the answer uses exactly this id. Only a genuinely different candidate value \
             gets a different id (`#{topic}-2`). Never invent a synonym for an id already on the \
             floor.\n\n",
        )
    }

    /// The topic the room has carried, for the Commit phase to name.
    ///
    /// Falls back to the task's canonical id when the fold shows nothing
    /// carried — which the Commit phase should make impossible, but a prompt
    /// naming `#answer` is a better failure than one naming an empty string.
    fn carried(&self, standings: &[TopicStanding]) -> String {
        standings
            .iter()
            .find(|standing| standing.carried(&self.quorum))
            .map_or_else(
                || canonical_topic(self.task),
                |standing| standing.topic.to_string(),
            )
    }

    /// The markers this seat may open a line with while the room deliberates,
    /// and the rules they are read under.
    ///
    /// Phase-gated on top of the per-member grammar, in that order: `!commit`
    /// is the library's to authorize, and the deliberation markers are the
    /// desk's to assign. `!question` and `!defer` are in every seat's list
    /// whatever the table says (see
    /// [`UNGATED_KINDS`](super::moves::UNGATED_KINDS)), so this block always
    /// offers a member with nothing to add something to say that is not prose.
    fn deliberate_protocol(&self) -> String {
        let allowed = self.desk.config.moves_for(&self.member.id);
        let assigned = allowed.len() < super::moves::MOVE_KINDS.len();
        let lines: Vec<&str> = allowed
            .iter()
            .filter(|kind| **kind != "commit")
            .filter_map(|kind| move_line(kind))
            .collect();
        debug_assert!(
            !lines.is_empty(),
            "every seat keeps !question and !defer, so a deliberating member always has a marker",
        );
        let head = "Reply with ONE line only, beginning with exactly one of these markers:";
        let mut tail = DELIBERATE_RULES.to_owned();
        if self.quorum.require_evidential {
            tail.push('\n');
            tail.push_str(EVIDENTIAL_RULE);
        }
        if assigned {
            tail.push('\n');
            tail.push_str(ASSIGNED_MOVES_RULE);
        }
        format!("{head}\n{}\n{tail}", lines.join("\n"))
    }

    /// What the desk remembers, or nothing when it remembers nothing.
    ///
    /// Attributed as memory rather than rendered into the transcript, and
    /// carrying no sequence, because it is not a message on this desk: a member
    /// that could cite it with `^N` would be grounding a decision in a number
    /// nothing in this conversation answers to.
    fn remembered(&self) -> String {
        if self.recall.is_empty() {
            return String::new();
        }
        let lines = self
            .recall
            .iter()
            .map(|hit| {
                let flat = hit.snippet.split_whitespace().collect::<Vec<_>>().join(" ");
                format!("- {}", truncate_chars(&flat, MAX_RECALL_CHARS))
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "\nThe desk remembers:\n(From earlier episodes on this desk. This is memory, not a \
             line in this conversation — it has no message number and cannot be cited with ^.)\n\
             {lines}\n",
        )
    }

    /// Who has not spoken yet, when the driver asked for it to be said.
    fn missing(&self) -> String {
        if self.unspoken.is_empty() {
            return String::new();
        }
        let who = self
            .unspoken
            .iter()
            .map(|id| format!("@{id}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "Members who have not spoken yet: {who}. You have the floor twice in a row. If what \
             the room is missing is theirs to supply, !question them or !defer #topic to them \
             rather than restating your own position.\n\n",
        )
    }

    /// Who else is in the room, what the desk is for, and what was asked.
    fn room(&self) -> String {
        let teammates: Vec<String> = self
            .desk
            .members
            .iter()
            .filter(|member| member.id != self.member.id)
            .map(|member| format!("@{} ({})", member.id, member.role))
            .collect();
        let mut room = String::new();
        if let Some(description) = &self.desk.description {
            room.push_str(&format!("This desk is for: {description}\n"));
        }
        if !teammates.is_empty() {
            room.push_str(&format!(
                "In the room with you: {}. Address them by the ids above.\n",
                teammates.join(", "),
            ));
        }
        room.push_str(&format!("The operator asked the desk:\n{}\n", self.task));
        room
    }

    /// The desk's pinboard, or nothing when it holds nothing.
    ///
    /// Rendered from the fold rather than restated as prose: a pin is a
    /// sequence the room can cite with `^N`, so the sequence has to be visible.
    fn board(&self) -> String {
        if self.pins.is_empty() {
            return String::new();
        }
        let lines = self
            .pins
            .iter()
            .map(|pin| {
                let label = pin
                    .label
                    .as_ref()
                    .map_or_else(String::new, |label| format!(" #{label}"));
                let body = pin
                    .excerpt
                    .as_deref()
                    .or(pin.note.as_deref())
                    .unwrap_or("(pinned)");
                format!("[{}]{label} {body}", pin.sequence)
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!("\nPinned on this desk, whatever else has scrolled away:\n{lines}\n")
    }

    /// This turn's standings, folded with the same [`standings`] the episode
    /// itself uses, so the numbers a member reads are the numbers the room will
    /// decide on.
    ///
    /// An unfoldable transcript yields nothing rather than an error: the floor
    /// block and the carried topic both degrade to their "nothing yet" shapes,
    /// which is a worse prompt and not a failed turn.
    fn standings(&self, visible: &[&SessionMessage]) -> Vec<TopicStanding> {
        let traces: Vec<_> = visible
            .iter()
            .flat_map(|message| resolve(&message.content, None, &message.author, message.sequence))
            .collect();
        let at = visible
            .last()
            .map_or(Sequence(0), |message| message.sequence);
        standings(&traces, at, &self.quorum).unwrap_or_default()
    }

    /// The topics on the floor, with the standing the library gives each.
    fn floor(&self, standings: &[TopicStanding]) -> String {
        if standings.is_empty() {
            return format!(
                "No option is on the floor yet. The topic id you coin becomes the room's name \
                 for that option, so keep it short. An option carries once {} different members \
                 have backed it with grounds.\n\n",
                self.quorum.threshold,
            );
        }
        let floor = standings
            .iter()
            .map(|standing| {
                format!(
                    "#{} — {} of the {} supporters it needs ({})",
                    standing.topic,
                    standing.supporters.len(),
                    self.quorum.threshold,
                    if standing.supporters.is_empty() {
                        "nobody counted yet".to_owned()
                    } else {
                        standing.supporters.join(", ")
                    },
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "Options already on the floor. Reuse one of these ids exactly if your point is about \
             it — support split across two names for one idea never adds up to a decision — and \
             only coin a new id for a genuinely different option:\n{floor}\n\n",
        )
    }

    /// The line this member last authored, if any.
    fn last_line(&self, visible: &[&SessionMessage]) -> String {
        visible
            .iter()
            .rev()
            .find(|message| match &message.author {
                SessionAuthor::Agent { id, .. } => id == &self.member.id,
                SessionAuthor::Operator
                | SessionAuthor::Person { .. }
                | SessionAuthor::System { .. } => false,
            })
            .map(|message| message.content.trim())
            .map_or_else(String::new, |line| {
                format!(
                    "You already said this, so do not repeat it — say something that moves the \
                     room on:\n{line}\n\n",
                )
            })
    }
}

/// Render an attributed transcript the way every prompt here shows one.
///
/// `[sequence] author: content`, because the sequence IS the citation: a
/// `!support #topic ^12` names message 12, and a member that cannot see the
/// numbers cannot ground anything.
#[must_use]
pub fn render_transcript(visible: &[&SessionMessage]) -> String {
    visible
        .iter()
        .map(|message| {
            let author = match &message.author {
                SessionAuthor::Agent { label, .. }
                | SessionAuthor::Person { label, .. }
                | SessionAuthor::System { label, .. } => label.as_str(),
                SessionAuthor::Operator => "operator",
            };
            format!("[{}] {author}: {}", message.sequence, message.content)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The one line a turn's answer contributes to the transcript.
///
/// A harness turn can return a paragraph however firmly it was asked for one
/// line, and the whole paragraph would then be rendered back into every
/// subsequent prompt — thirty of those is the transcript window spent on one
/// answer. The marker is what the room counts, so the marker line is what the
/// transcript keeps.
///
/// Colour and banners are stripped first (a harness reply can carry escapes
/// from a tool's captured output), then the marker line is taken if the agent
/// wrapped it in prose. A turn that deposits no trace is still a legal turn, so
/// prose falls through to the first thing the agent actually said.
#[must_use]
pub fn marker_line(text: &str) -> String {
    let text = plain(text);
    let marker = text
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with('!') || line.starts_with('@'));
    marker
        .or_else(|| {
            text.lines()
                .map(str::trim)
                .find(|line| !line.is_empty() && !line.starts_with('>'))
        })
        .unwrap_or("(no answer)")
        .to_owned()
}

/// Strip ANSI escape sequences from a turn's reply.
fn plain(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character != '\u{1b}' {
            plain.push(character);
            continue;
        }
        // CSI sequences end at their final byte in `@`..=`~`; anything else
        // after the escape is a two-character sequence.
        if characters.next() == Some('[') {
            for byte in characters.by_ref() {
                if ('@'..='~').contains(&byte) {
                    break;
                }
            }
        }
    }
    plain
}

/// Truncate `text` to at most `max` characters, on a char boundary.
///
/// The ellipsis is budgeted *inside* `max`, so the cap never quietly exceeds
/// the bound it advertises — the same accounting
/// `memory_loop::truncate_chars` does
/// for injected prior work, duplicated because that module is harness-gated and
/// this one compiles in every build.
fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    if max == 0 {
        return String::new();
    }
    let head: String = text.chars().take(max - 1).collect();
    format!("{head}\u{2026}")
}
