import { useCallback, useEffect, useRef, useState } from "react";

import type { OpenCompanyClient } from "@/api/client";
import { ApiError } from "@/api/types";
import {
  pruneTypers,
  shouldShowTyping,
  typerKey,
  typingExpiry,
  TYPING_PRUNE_INTERVAL_MS,
  TYPING_SEND_THROTTLE_MS,
  type Typer,
} from "@/lib/awareness";

/**
 * Who is typing, and telling the company that this console is.
 *
 * Sending is throttled per channel, and skipped entirely while the event
 * stream is down: a typing ping is not worth waking a dead connection for, and
 * an indicator nobody will receive is not worth a request.
 *
 * The prune interval is armed **only while somebody is typing**. An interval
 * that runs once a second for the life of every tab, to filter an empty list,
 * is a real cost paid by every idle console in the company.
 */
export function useTyping(
  client: OpenCompanyClient,
  company: string | null | undefined,
  options: { connected?: boolean } = {},
) {
  const connected = options.connected ?? true;
  const [typers, setTypers] = useState<Typer[]>([]);

  // Last time this console announced typing, per channel. A ref: throttling is
  // not something the UI renders.
  const lastSent = useRef(new Map<string, number>());
  // Each author's most recent message per channel, for the de-flicker rules.
  const lastMessage = useRef(new Map<string, number>());
  // The newest `atMillis` already applied per typer, so a redelivered frame —
  // an SSE reconnect replaying its last event is the real-world case — cannot
  // renew an indicator that a genuine new ping did not actually buy. This is
  // the de-flicker rule `typingExpiry` used to get by capping expiry at
  // `frame.atMillis + TTL`; that comparison read the *server's* clock against
  // nothing of the browser's, but broke for anyone whose workstation clock ran
  // ahead of the host (see `awareness.ts`). Comparing a frame's stamp only to
  // the previous frame's stamp — both server-issued — keeps the guard without
  // ever touching the browser's own clock.
  const lastFrameAt = useRef(new Map<string, number>());
  const supported = useRef(true);

  /** Apply one live typing frame. */
  const onFrame = useCallback(
    (frame: {
      userId: string;
      chatId: string;
      parentId?: string;
      atMillis: number;
    }) => {
      const now = Date.now();
      const seen = lastMessage.current.get(`${frame.userId}:${frame.chatId}`);
      if (!shouldShowTyping(frame, seen)) return;
      const key = typerKey(frame);
      const previousAt = lastFrameAt.current.get(key);
      if (previousAt !== undefined && frame.atMillis <= previousAt) return;
      lastFrameAt.current.set(key, frame.atMillis);
      setTypers((current) => {
        const existing = current.find((t) => typerKey(t) === key);
        const next: Typer = {
          userId: frame.userId,
          chatId: frame.chatId,
          parentId: frame.parentId,
          expiresAt: typingExpiry(now),
          // Preserved across renewals so the displayed order stays put.
          firstSeenAt: existing?.firstSeenAt ?? now,
        };
        return [...current.filter((t) => typerKey(t) !== key), next];
      });
    },
    [],
  );

  /**
   * Record that somebody's message landed, so their in-flight typing ping does
   * not leave a ghost indicator behind it.
   */
  const onMessage = useCallback(
    (userId: string, chatId: string, atMillis: number) => {
      lastMessage.current.set(`${userId}:${chatId}`, atMillis);
      setTypers((current) =>
        current.filter((t) => !(t.userId === userId && t.chatId === chatId)),
      );
    },
    [],
  );

  /** Say this console is typing in `chatId`, at most once per throttle window. */
  const announce = useCallback(
    (chatId: string, parentId?: string) => {
      if (!connected || !supported.current || !chatId) return;
      const key = `${chatId}:${parentId ?? ""}`;
      const now = Date.now();
      const sent = lastSent.current.get(key) ?? 0;
      if (now - sent < TYPING_SEND_THROTTLE_MS) return;
      lastSent.current.set(key, now);
      void client.typing(chatId, parentId, company).catch((error) => {
        // Only a 404 means this host predates the route — disable it, the
        // same distinction `usePresence`'s heartbeat makes. Anything else
        // (a network blip, a proxy hiccup, a 5xx) is fire-and-forget: this
        // one ping is not worth retrying, but the *route* is not disabled,
        // so the next keystroke's ping still goes out.
        if (error instanceof ApiError && error.status === 404) supported.current = false;
      });
    },
    [client, company, connected],
  );

  // Armed only while there is something to expire.
  useEffect(() => {
    if (typers.length === 0) return;
    const prune = setInterval(() => {
      setTypers((current) => {
        const next = pruneTypers(current, Date.now());
        if (next.length === current.length) return current;
        // Forget the dedup memory for whoever just expired, so a genuinely
        // new burst of typing from them later is never mistaken for a
        // replay of the one that just timed out.
        const stillTyping = new Set(next.map((t) => typerKey(t)));
        for (const key of lastFrameAt.current.keys()) {
          if (!stillTyping.has(key)) lastFrameAt.current.delete(key);
        }
        return next;
      });
    }, TYPING_PRUNE_INTERVAL_MS);
    return () => clearInterval(prune);
  }, [typers.length]);

  return { typers, onFrame, onMessage, announce };
}
