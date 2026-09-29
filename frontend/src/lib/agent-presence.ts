/**
 * What an agent is doing right now, as one word the console can draw.
 *
 * Pure: no React, no store, no clock. The Room store feeds it the live state it
 * already holds (open turns, live tool rows, the frames' agent and reply
 * markers, the turn-bracket ledger, the pending approvals) and gets back an
 * index; `room/store.ts` memoises it and exposes `useAgentPresence`.
 *
 * Precedence, strongest first: approval, working, typing, thinking, queued,
 * inactive.
 *
 * - **approval**: a pending approval names the agent, or a live tool row is
 *   parked `awaiting_approval`. It lasts while the approval is in the list, so
 *   it outlives the turn that raised it.
 * - **working**: a live tool row is running. An agent seen only through the
 *   turn-bracket ledger (a hive seat, which streams no frames) reads here too:
 *   its turn is open and nothing finer is known.
 * - **typing**: the host's `replying` frame arrived and nothing has reset it
 *   (a tool call or thinking frame does, a settle clears it). Tied to the turn,
 *   deliberately not to a timer: an agent's reply can stream for longer than a
 *   person's 8-second `typing` frame.
 * - **thinking**: an open, non-queued turn with no running tool row.
 * - **queued**: every open turn attributed to the agent is waiting on the
 *   per-company serial lock.
 * - **inactive**: none of the above. No timer, no open turn.
 */

/** The six states, weakest last. */
export type AgentPresenceState =
  | "approval"
  | "working"
  | "typing"
  | "thinking"
  | "queued"
  | "inactive";

const RANK: Record<AgentPresenceState, number> = {
  approval: 5,
  working: 4,
  typing: 3,
  thinking: 2,
  queued: 1,
  inactive: 0,
};

/** The stronger of two states, by the precedence above. */
export function strongerPresence(a: AgentPresenceState, b: AgentPresenceState): AgentPresenceState {
  return RANK[b] > RANK[a] ? b : a;
}

/**
 * How long a turn may go without a frame before it stops counting as live.
 *
 * A proposal, tuned by nothing yet: a missed `turn_settled` leaves a turn open
 * until reload (`coordination.ts`), and a stuck "working" dot is worse than a
 * dot that gives up on a turn silent for ten minutes.
 */
export const STALE_TURN_MS = 10 * 60 * 1000;

/** What the console knows about one live turn from its frames (keyed as its rows are). */
export interface TurnMeta {
  /** The host thread the frames named. */
  chatId: string;
  /** The last frame was `replying` (nothing has reset it since). */
  replying: boolean;
  /** Wall-clock of the last frame, for the age-out. */
  lastFrameAt: number;
}

/** The slice of a live tool row presence reads. */
export interface PresenceStep {
  status?: string;
}

/** An open turn as the console's Room store holds it. */
export interface PresenceOpenTurn {
  queued: boolean;
  chatId: string;
  agentId?: string;
}

/** A turn the bracket ledger saw open. */
export interface PresenceLedgerTurn {
  agentId?: string;
  chatId?: string;
  startedAtMillis: number;
}

/** Everything {@link derivePresence} reads. */
export interface PresenceInputs {
  openTurns: Record<string, readonly PresenceOpenTurn[]>;
  liveStepsByThread: Record<string, readonly PresenceStep[]>;
  liveStepsByMessage: Record<string, readonly PresenceStep[]>;
  /** Who last reported on each turn, keyed like {@link turnMeta}. */
  liveAgentByTurn: Record<string, string>;
  turnMeta: Record<string, TurnMeta>;
  ledgerTurns: readonly PresenceLedgerTurn[];
  /** Agent id to how many approvals of theirs are pending. */
  approvalAgents: Record<string, number>;
  /** Host thread id to the teammate whose DM it is. */
  threadAgents: Record<string, string>;
  now: number;
}

/** The derived answer: one state per agent, and one per (agent, chat). */
export interface PresenceIndex {
  byAgent: ReadonlyMap<string, AgentPresenceState>;
  byAgentChat: ReadonlyMap<string, AgentPresenceState>;
  /** Agents with at least one pending approval, for the per-chat lookup. */
  approving: ReadonlySet<string>;
  /** Host thread id to the teammate whose DM it is, for {@link presenceChatKey}. */
  threadAgents: Readonly<Record<string, string>>;
}

/** The `byAgentChat` key. */
export function presenceKey(agentId: string, chatId: string): string {
  return `${agentId}\u0000${chatId}`;
}

/**
 * One spelling for a DM, whichever the thread id arrived in.
 *
 * A DM has two: the console addresses an ordinary teammate's DM by its **bare**
 * id (`dmThreadId`), so the chat route's turns and frames name `ceo`, while the
 * hive seat that answers it journals its turn brackets under its desk id,
 * `dm:ceo` (`hive/host.rs`). Keyed as they arrived, a real DM turn lit
 * `(ceo, dm:ceo)` and the DM row, which asks for `(ceo, ceo)`, stayed dark.
 *
 * Folded to `dm:<teammate>`, which cannot collide with a desk: a thread the
 * roster says is a teammate's DM maps to it, and a `dm:<id>` single-seat key is
 * already in that form. A pair conversation (`dm:a+b`) is not a teammate's DM
 * and keeps its own key. The one teammate addressed prefixed (a General
 * spelling, `dm:general`) folds to itself, and the bare `general` channel stays
 * #general because the roster never maps it.
 */
export function presenceChatKey(chatId: string, threadAgents: Readonly<Record<string, string>>): string {
  const owner = threadAgents[chatId];
  return owner === undefined ? chatId : `dm:${owner}`;
}

/** Folds the live state into a {@link PresenceIndex}. */
export function derivePresence(inputs: PresenceInputs): PresenceIndex {
  const byAgent = new Map<string, AgentPresenceState>();
  const byAgentChat = new Map<string, AgentPresenceState>();
  const fresh = (at: number) => inputs.now - at < STALE_TURN_MS;
  const chatKey = (chat: string) => presenceChatKey(chat, inputs.threadAgents);
  const bump = (agent: string, chat: string | undefined, state: AgentPresenceState) => {
    byAgent.set(agent, strongerPresence(byAgent.get(agent) ?? "inactive", state));
    if (chat === undefined) return;
    const key = presenceKey(agent, chatKey(chat));
    byAgentChat.set(key, strongerPresence(byAgentChat.get(key) ?? "inactive", state));
  };

  // 1. Turns the frames speak for: the agent is whoever the frames named, which
  //    is truer than the one the host started the turn on.
  const framedChats = new Set<string>();
  const framedAgents = new Set<string>();
  const lastFrameByAgent = new Map<string, number>();
  for (const [key, meta] of Object.entries(inputs.turnMeta)) {
    const agent = inputs.liveAgentByTurn[key];
    if (!agent || !fresh(meta.lastFrameAt)) continue;
    framedChats.add(chatKey(meta.chatId));
    framedAgents.add(agent);
    lastFrameByAgent.set(agent, Math.max(lastFrameByAgent.get(agent) ?? 0, meta.lastFrameAt));
    const steps = inputs.liveStepsByMessage[key] ?? inputs.liveStepsByThread[key] ?? [];
    let state: AgentPresenceState = meta.replying ? "typing" : "thinking";
    if (steps.some((s) => s.status === "running")) state = "working";
    if (steps.some((s) => s.status === "awaiting_approval")) state = "approval";
    bump(agent, meta.chatId, state);
  }

  // 2. Open turns nobody has framed yet (just accepted, or queued): the guess
  //    the host recorded, else the teammate whose DM this is.
  for (const turns of Object.values(inputs.openTurns)) {
    for (const turn of turns) {
      if (framedChats.has(chatKey(turn.chatId))) continue;
      const agent = turn.agentId ?? inputs.threadAgents[turn.chatId];
      if (!agent) continue;
      bump(agent, turn.chatId, turn.queued ? "queued" : "thinking");
    }
  }

  // 3. The bracket ledger: seats that stream no frames. Open and nothing finer
  //    is known, so "working". An agent the frames already describe keeps that.
  for (const turn of inputs.ledgerTurns) {
    if (!turn.agentId || framedAgents.has(turn.agentId)) continue;
    if (!fresh(Math.max(turn.startedAtMillis, lastFrameByAgent.get(turn.agentId) ?? 0))) continue;
    bump(turn.agentId, turn.chatId, "working");
  }

  // 4. Pending approvals outlive the turn, so they are read last and win.
  const approving = new Set<string>();
  for (const [agent, count] of Object.entries(inputs.approvalAgents)) {
    if (count <= 0) continue;
    approving.add(agent);
    byAgent.set(agent, "approval");
  }
  return { byAgent, byAgentChat, approving, threadAgents: inputs.threadAgents };
}

/** One agent's state across every chat. */
export function presenceOf(index: PresenceIndex, agentId: string): AgentPresenceState {
  return index.byAgent.get(agentId) ?? "inactive";
}

/**
 * One agent's state in one chat: what it is doing there, or "approval" when it
 * has a pending approval anywhere (an approval is the operator's to act on
 * wherever they are looking, so it is not scoped to the conversation).
 */
export function presenceIn(index: PresenceIndex, agentId: string, chatId: string): AgentPresenceState {
  if (index.approving.has(agentId)) return "approval";
  const key = presenceKey(agentId, presenceChatKey(chatId, index.threadAgents));
  return index.byAgentChat.get(key) ?? "inactive";
}

/**
 * `meta` without the entries `drop` names, or the same object when none match
 * (so the store's identity check skips the notify).
 */
export function dropTurnMeta(
  meta: Record<string, TurnMeta>,
  drop: (key: string, entry: TurnMeta) => boolean,
): Record<string, TurnMeta> {
  let next: Record<string, TurnMeta> | null = null;
  for (const [key, entry] of Object.entries(meta)) {
    if (!drop(key, entry)) continue;
    next ??= { ...meta };
    delete next[key];
  }
  return next ?? meta;
}

/** Counts approvals per asking agent, ignoring the ones no agent raised. */
export function approvalAgentCounts(
  approvals: readonly { agent?: string | null }[],
): Record<string, number> {
  const out: Record<string, number> = {};
  for (const approval of approvals) {
    if (approval.agent) out[approval.agent] = (out[approval.agent] ?? 0) + 1;
  }
  return out;
}

/** Whether two count maps say the same thing, so a poll that changed nothing changes nothing. */
export function sameCounts(a: Record<string, number>, b: Record<string, number>): boolean {
  const keys = Object.keys(a);
  return keys.length === Object.keys(b).length && keys.every((k) => a[k] === b[k]);
}
