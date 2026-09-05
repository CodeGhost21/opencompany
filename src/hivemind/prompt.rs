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
//!   recording a decision it never reached.
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

use super::types::{HiveDesk, HiveMember};

/// The moves available while the room is still deliberating.
const DELIBERATE_MOVES: &str = "\
Reply with ONE line only, beginning with exactly one of these markers:
!propose #topic  then one sentence putting a new option on the floor
!support #topic ^N  then why, citing message N as grounds
!object >N ^M  then why, objecting to message N and citing message M
!evidence #topic ^N  then a fact, adding grounds without taking a side; keep the # when the fact bears on a named option
!defer #topic  then who should answer instead, when this is not your area
!question  then what you need that nobody has established";

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
worse for the room than saying so and standing aside. Write nothing before or \
after the single marker line.";

/// The move available once the room has reached quorum.
const COMMIT_PROTOCOL: &str = "\
The room has reached quorum. Reply with ONE line only, recording the option \
that carried:
!commit #topic ^N  then why, citing message N
Keep the # on the topic and the ^ on the citation; without them the line \
records nothing. Angle brackets are not part of the line — write the sentence \
itself. Use the topic the room actually settled on, not the one you would have \
preferred. Write nothing before or after the single marker line.";

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
        }
    }

    /// Render exactly what this turn is allowed to see.
    #[must_use]
    pub fn render(&self, turn: &HiveTurn, visible: &[&SessionMessage]) -> String {
        let sight = match turn.visibility {
            Visibility::Blind => "You cannot yet see your peers' positions. Form your own first.",
            Visibility::Full => "You can see the whole room.",
        };
        let protocol = match turn.phase {
            Phase::Deliberate => format!("{DELIBERATE_MOVES}\n{DELIBERATE_RULES}"),
            Phase::Commit => COMMIT_PROTOCOL.to_owned(),
        };
        format!(
            "You are @{}, the {} on the {} desk. {sight}\n\n{}{}\n\n{protocol}\n\n{}{}\
             Shared attributed transcript:\n{}\n\nYour one line:",
            self.member.id,
            self.member.role,
            self.desk.name,
            self.room(),
            self.board(),
            self.floor(visible),
            self.last_line(visible),
            render_transcript(visible),
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

    /// The topics on the floor, with the standing the library gives each.
    ///
    /// Folded here with the same [`standings`] the episode itself uses, so the
    /// numbers a member reads are the numbers the room will decide on.
    fn floor(&self, visible: &[&SessionMessage]) -> String {
        let traces: Vec<_> = visible
            .iter()
            .flat_map(|message| resolve(&message.content, None, &message.author, message.sequence))
            .collect();
        let at = visible
            .last()
            .map_or(Sequence(0), |message| message.sequence);
        let Ok(standings) = standings(&traces, at, &self.quorum) else {
            return String::new();
        };
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
