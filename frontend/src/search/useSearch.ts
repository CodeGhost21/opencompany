// The asynchronous half: what has to be fetched before the pure half can rank it.
//
// Everything about *what counts as a hit* lives in `sources.ts` and `rank.ts`,
// which are pure. What lives here is the part that cannot be: reading the
// roster and the desks once the modal opens, asking the host to search the
// workspace, and reading one conversation when the query names one.
//
// ## Two rules this file exists to keep
//
// **A late answer never wins.** Every fetch is tagged with the generation that
// asked for it and drops itself if the query has moved on. Typing is a stream
// of queries and the network reorders them freely, so without this an operator
// who types `box` sees results for `bo` land on top of them a moment later.
//
// **Nothing is fetched for a query that cannot use it.** Workspace search is a
// host round trip, so it waits for a term and never runs for a scoped query,
// where the question is about a conversation rather than about files.

import { useEffect, useMemo, useRef, useState } from "react";

import type { OpenCompanyClient } from "@/api/client";
import type { ChatHistoryMessageDto } from "@/api/types";
import { searchWorkspace, type SearchHit } from "@/api/workspace";
import type { Desk } from "@/lib/desks";
import { fromDto, type TeamMember } from "@/lib/team";
import { deskFromDto, dmChannelId, dmThreadId } from "@/views/room/channels";
import { isScopedMessageSearch, type SearchQuery } from "./query";
import { score } from "./rank";
import {
  agentResults,
  channelResults,
  fileResults,
  messageResults,
  type MessageContext,
} from "./sources";
import { RESULT_LABEL, RESULT_ORDER, type ResultGroup, type SearchResult } from "./types";

/**
 * How long to wait after a keystroke before asking the host anything.
 *
 * Local matching is synchronous and unaffected — channels and agents update on
 * every keystroke, which is what makes the modal feel immediate. This bounds
 * only the round trips.
 */
const DEBOUNCE_MS = 160;

/**
 * How much of a conversation to read when searching inside it.
 *
 * Fetched a page at a time, because the host clamps every request to
 * `CHAT_HISTORY_PAGE_LIMIT` (200, `src/server/chat_history.rs`) — so asking for
 * 500 silently returned the newest 200 and reported "Nothing matched" for
 * anything older, which is the one answer a search must never give wrongly.
 *
 * Bounded rather than exhaustive: a search is not an export, and a person
 * waiting on a modal will not wait out a year of a busy channel. When the cap
 * is reached the older messages are genuinely unsearched — the honest fix for
 * that is a host-side message search, not more pages here.
 */
const HISTORY_PAGE = 200;
const HISTORY_PAGES = 5;

/** What the modal needs to draw itself. */
export interface SearchState {
  groups: ResultGroup[];
  /** Whether a host round trip is outstanding. Local results are already shown. */
  loading: boolean;
  /** The roster, for the resting state's shortcuts. */
  members: TeamMember[];
  /** The channels, likewise. */
  desks: Desk[];
}

/**
 * Everything the search modal shows, for one query.
 *
 * `enabled` is the modal's open state: nothing is fetched while it is shut, and
 * what was loaded is kept, so reopening is instant rather than a second load of
 * the same roster.
 */
export function useSearch(
  client: OpenCompanyClient,
  company: string | null,
  query: SearchQuery,
  enabled: boolean,
): SearchState {
  const [desks, setDesks] = useState<Desk[]>([]);
  const [members, setMembers] = useState<TeamMember[]>([]);
  const [files, setFiles] = useState<SearchHit[]>([]);
  const [messages, setMessages] = useState<{ context: MessageContext; rows: ChatHistoryMessageDto[] } | null>(null);
  const [loading, setLoading] = useState(false);

  // The scope the collections were loaded for. A company switch while the modal
  // is shut must not leave the previous company's channels in it.
  const loadedFor = useRef<{ client: unknown; company: string | null } | null>(null);
  const generation = useRef(0);

  useEffect(() => {
    if (!enabled) return;
    const scope = loadedFor.current;
    if (scope && scope.client === client && scope.company === company) return;

    // Emptied before the new company's read, never left standing during it:
    // `groups` is computed from whatever is held, so the previous company's
    // channels and agents would otherwise be listed — and activatable — under
    // the new one's query.
    if (scope) {
      setDesks([]);
      setMembers([]);
      setFiles([]);
      setMessages(null);
    }

    let cancelled = false;
    void Promise.allSettled([client.listDesks(company), client.listTeam(company)]).then(
      ([deskResult, teamResult]) => {
        if (cancelled) return;
        // Settled rather than awaited together: a host with no `.../desks` route
        // still has a roster, and one failure should not empty both lists.
        if (deskResult.status === "fulfilled") setDesks(deskResult.value.map(deskFromDto));
        if (teamResult.status === "fulfilled") setMembers(teamResult.value.map(fromDto));
        // Marked loaded only when both answered. Recording the scope on a
        // partial read would make one transient failure permanent for the rest
        // of the session: every later opening returns at the guard above, so a
        // failed roster read leaves no agents — and with them no `@` scope that
        // can resolve — until the console is reloaded.
        if (deskResult.status === "fulfilled" && teamResult.status === "fulfilled") {
          loadedFor.current = { client, company };
        }
      },
    );
    return () => {
      cancelled = true;
    };
  }, [client, company, enabled]);

  /** The teammate or desk a scope names, or null while it names nobody. */
  const scoped = useMemo(() => resolveScope(query, desks, members), [query, desks, members]);

  useEffect(() => {
    if (!enabled) return;
    const mine = ++generation.current;
    const current = () => generation.current === mine;

    // Nothing to ask about: clear what the last query fetched rather than
    // leaving it under a query it does not answer.
    if (query.isEmpty) {
      setFiles([]);
      setMessages(null);
      setLoading(false);
      return;
    }

    // Cleared as the query changes rather than when its replacement lands: a
    // result list that keeps answering the previous question through the
    // debounce is one an operator can click, and it opens the wrong thing.
    setFiles([]);
    setMessages(null);

    const timer = setTimeout(() => {
      if (!current()) return;

      if (isScopedMessageSearch(query) && scoped) {
        setLoading(true);
        void readConversation(client, company, scoped.threadId)
          .then((rows) => {
            if (!current()) return;
            setMessages({ context: scoped.context, rows });
            setFiles([]);
          })
          .catch(() => {
            if (!current()) return;
            setMessages(null);
          })
          .finally(() => {
            if (current()) setLoading(false);
          });
        return;
      }

      setMessages(null);
      // `/` is a file query in its own right; `@` and `#` half-typed are
      // pickers, and a picker is not a question for the host.
      const fileTerm = query.scope?.kind === "file" ? query.scope.name : query.scope ? "" : query.term;
      if (!fileTerm) {
        setFiles([]);
        setLoading(false);
        return;
      }

      setLoading(true);
      void searchWorkspace(client, company, fileTerm, { limit: 12 })
        .then((results) => {
          if (!current()) return;
          setFiles(results.hits);
        })
        .catch(() => {
          // A host without the workspace route answers 404. That is a missing
          // source, not a failed search — the other groups still stand.
          if (current()) setFiles([]);
        })
        .finally(() => {
          if (current()) setLoading(false);
        });
    }, DEBOUNCE_MS);

    return () => clearTimeout(timer);
  }, [client, company, enabled, query, scoped]);

  const groups = useMemo(() => {
    const byKind: Record<string, SearchResult[]> = {
      channel: channelResults(desks, query),
      agent: agentResults(members, query),
      message: messages ? messageResults(messages.rows, query, messages.context) : [],
      file: fileResults(files, query),
    };
    return RESULT_ORDER.map((kind) => ({
      kind,
      label: RESULT_LABEL[kind],
      results: byKind[kind] ?? [],
    })).filter((group) => group.results.length > 0);
  }, [desks, members, messages, files, query]);

  return { groups, loading, members, desks };
}

/**
 * As much of one conversation as a search is willing to wait for.
 *
 * Walks the `before` cursor, which the host keys on a message's sequence — the
 * bare id it answers with. Stops early on a short page, because that is the
 * start of the conversation and there is nothing behind it.
 */
async function readConversation(
  client: OpenCompanyClient,
  company: string | null,
  threadId: string,
): Promise<ChatHistoryMessageDto[]> {
  const all: ChatHistoryMessageDto[] = [];
  let before: string | undefined;
  for (let page = 0; page < HISTORY_PAGES; page += 1) {
    const rows = await client.getChatHistory(threadId, company, {
      before,
      limit: HISTORY_PAGE,
    });
    if (rows.length === 0) break;
    // Pages arrive oldest-first within themselves and each page is older than
    // the last, so earlier pages go in front.
    all.unshift(...rows);
    if (rows.length < HISTORY_PAGE) break;
    before = rows[0]?.id;
    if (!before) break;
  }
  return all;
}

/** A resolved scope: which conversation to read, and what to call it. */
interface ResolvedScope {
  /** The **host thread** to read history from. */
  threadId: string;
  context: MessageContext;
}

/**
 * The one conversation a scope names, or null when it names none yet.
 *
 * The thread and the link are deliberately different strings for a person:
 * `dmThreadId` is what the host answers history on, `dmChannelId` is what this
 * console routes — and they diverge for a teammate whose id spells General
 * (`views/room/channels.ts`). Reading the wrong one gives an empty history for
 * a DM that plainly has messages.
 */
function resolveScope(
  query: SearchQuery,
  desks: readonly Desk[],
  members: readonly TeamMember[],
): ResolvedScope | null {
  const scope = query.scope;
  if (!scope || !scope.name) return null;

  if (scope.kind === "person") {
    const member = best(members, (m) => Math.max(score(m.name, scope.name), score(m.id, scope.name)));
    if (!member) return null;
    const nameById = new Map(members.map((m) => [m.id, m.name]));
    return {
      threadId: dmThreadId(member),
      context: {
        channelId: dmChannelId(member),
        label: member.name,
        nameFor: (author) => nameById.get(author) ?? (author === "operator" ? "You" : author),
      },
    };
  }

  const desk = best(desks, (d) => Math.max(score(d.channel, scope.name), score(d.name, scope.name)));
  if (!desk) return null;
  const nameById = new Map(members.map((m) => [m.id, m.name]));
  return {
    threadId: desk.id,
    context: {
      channelId: desk.id,
      label: `#${desk.channel}`,
      nameFor: (author) => nameById.get(author) ?? (author === "operator" ? "You" : author),
    },
  };
}

/** The highest-scoring item, or null when nothing scored at all. */
function best<T>(items: readonly T[], scoreOf: (item: T) => number): T | null {
  let winner: T | null = null;
  let top = 0;
  for (const item of items) {
    const value = scoreOf(item);
    if (value > top) {
      top = value;
      winner = item;
    }
  }
  return winner;
}
