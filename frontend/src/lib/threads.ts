// Conversation threads: WhatsApp-style "chats" with the company. Every thread
// talks to the same company chat endpoint; a thread just scopes a transcript
// and gives the company side a consistent identity (a "desk" you're talking to).

import type { DeskDto, TeamMemberDto } from "../api/types";
import { MAIN_THREAD_ID, type ChatMessage } from "./chat";
import { toneFor } from "./team";

export interface ThreadContact {
  name: string;
  kind: "company" | "agent";
  /** Tailwind avatar tone key for agent desks; company uses the brand mark. */
  tone?: string;
}

export interface Thread {
  id: string;
  contact: ThreadContact;
  /** Short blurb shown under the name when the thread has no messages yet. */
  blurb: string;
  messages: ChatMessage[];
  /**
   * Whether this thread is the built-in **Operator** system channel (issue
   * #1757): a durable, read-only feed the server refuses to post to. Set from
   * the desk's own `system` flag so the legacy `#/conversation` route — which
   * builds its thread list straight from `/desks` rather than through
   * {@link resolveDesks} — still knows to disable its composer instead of
   * letting the operator type and submit before the server's guard refuses it.
   */
  readOnly?: boolean;
}

/** Avatar tones rotated across desk threads. */
const DESK_TONES = ["sky", "violet", "amber", "emerald", "rose", "cyan"];

/** The company's main line — the orchestrator you talk to for anything. */
function mainThread(): Thread {
  return {
    id: MAIN_THREAD_ID,
    contact: { name: "Your company", kind: "company" },
    blurb: "The main line — ask for anything",
    messages: [],
  };
}

/** The default chat list: the company's main line plus a few focused desks. */
export function defaultThreads(): Thread[] {
  return [
    mainThread(),
    {
      id: "strategy",
      contact: { name: "Strategy desk", kind: "agent", tone: "sky" },
      blurb: "Plans, priorities, and direction",
      messages: [],
    },
    {
      id: "creative",
      contact: { name: "Creative studio", kind: "agent", tone: "violet" },
      blurb: "Copy, design, and campaigns",
      messages: [],
    },
    {
      id: "frontdesk",
      contact: { name: "Front desk", kind: "agent", tone: "amber" },
      blurb: "Scheduling, inbox, and errands",
      messages: [],
    },
  ];
}

/** One `Thread` for a single `/desks` entry, desk or system channel alike. */
function deskThread(desk: DeskDto, i: number): Thread {
  return {
    id: desk.id,
    contact: {
      name: desk.name,
      kind: "agent",
      tone: DESK_TONES[i % DESK_TONES.length],
    },
    blurb: desk.description ?? "A desk of your company",
    messages: [],
    readOnly: desk.system,
  };
}

/**
 * Build the chat list from the company's real desks (issue #53): the main line
 * (the orchestrator) first, then one thread per desk keyed by its id. Falls back
 * to {@link defaultThreads} when the company defines no *real* desks — the fetch
 * failed and returned an empty list, or every entry is a system channel — so the
 * console always renders something.
 *
 * The fallback is keyed on non-system desk count, the same distinction
 * {@link resolveDesks} (`views/chat/model.ts`) makes for the Chat route, and for
 * the same reason: the always-present Operator system channel (issue #1757)
 * means a desk-less company's `/desks` answer is never actually `[]`, so the
 * plain `desks.length === 0` this function used to check never fired for a
 * desk-less company — the legacy `#/conversation` route this function backs
 * lost its main/strategy/creative/front-desk lines (and everything journaled
 * under them) the moment Operator shipped, while the Chat route kept them.
 * Any system desks the host did send are still threaded in either way, so
 * falling back never drops that feed — the same rule `782772aad` already
 * applied to `readOnly` here; twice missing the same host behavior on this
 * route is why the fallback belongs beside it rather than only in `resolveDesks`.
 */
export function threadsFromDesks(desks: DeskDto[]): Thread[] {
  const real = desks.filter((d) => !d.system);
  if (real.length === 0) {
    const system = desks.filter((d) => d.system).map(deskThread);
    return [...defaultThreads(), ...system];
  }
  return [mainThread(), ...desks.map(deskThread)];
}

/**
 * One DM thread per roster teammate (issue #151 §3.3): the agent's own console,
 * so an operator can follow up with the teammate who did the work instead of
 * going back through the orchestrator.
 *
 * Keyed by the **agent id**, which the host resolves straight to that teammate.
 * Desks are listed first and win any id collision — `existingIds` is what keeps
 * a teammate who is already reachable as a desk from appearing twice.
 *
 * Kept separate from {@link threadsFromDesks} so a host that exposes desks but
 * not `/team` (or fails that fetch) simply gets no DMs, rather than losing its
 * desk list too.
 */
export function agentDmThreads(
  team: TeamMemberDto[],
  existingIds: Iterable<string>,
): Thread[] {
  const taken = new Set(existingIds);
  const seen = new Set<string>();
  const threads: Thread[] = [];
  for (const member of team) {
    const id = member.id?.trim();
    // A teammate with no id has nothing the host could route to, and a
    // duplicate id would collide with the thread already added for it.
    if (!id || taken.has(id) || seen.has(id)) continue;
    seen.add(id);
    const name = member.name?.trim() || member.role;
    threads.push({
      id,
      contact: { name, kind: "agent", tone: toneFor(id) },
      blurb: member.description?.trim() || member.role,
      messages: [],
    });
  }
  return threads;
}
