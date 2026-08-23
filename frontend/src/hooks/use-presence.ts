import { useCallback, useEffect, useRef, useState } from "react";

import type { OpenCompanyClient } from "@/api/client";
import { ApiError } from "@/api/types";
import {
  applyPresence,
  livePeers,
  presenceToAnnounce,
  reconcilePresenceSnapshot,
  PRESENCE_HEARTBEAT_MS,
  PRESENCE_TTL_MS,
  type Peer,
  type PresencePreference,
  type PresenceStatus,
} from "@/lib/awareness";

const PREFERENCE_KEY = "oc.presence.override";
const CONSOLE_ID_KEY = "oc.presence.consoleId";

/** The viewer's stored appear-as choice, defaulting to automatic. */
function storedPreference(): PresencePreference {
  try {
    const raw = localStorage.getItem(PREFERENCE_KEY);
    return raw === "away" || raw === "offline" ? raw : "auto";
  } catch {
    // A private window, or storage disabled. Not a reason to fail.
    return "auto";
  }
}

/**
 * This tab's lease key — never a second identity, just which of this
 * person's open consoles is announcing (see the server's presence module
 * header). `sessionStorage` rather than `localStorage`: it is already scoped
 * per tab, so a reload keeps the same lease (no double-announce) while a
 * second tab gets its own key (no shared lease to steal on close). A private
 * window or storage disabled falls back to a key that lives only for this
 * render — still correct, just unable to survive a reload without minting a
 * new one.
 */
function consoleId(): string {
  const mint = () =>
    typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID()
      : `${Date.now()}-${Math.random().toString(36).slice(2)}`;
  try {
    const existing = sessionStorage.getItem(CONSOLE_ID_KEY);
    if (existing) return existing;
    const fresh = mint();
    sessionStorage.setItem(CONSOLE_ID_KEY, fresh);
    return fresh;
  } catch {
    return mint();
  }
}

/**
 * Who is here, and announcing that this console is.
 *
 * # Activity lives in a ref, never in state
 *
 * The idle timer needs the last time the human touched the page, which means a
 * listener on every `keydown`, `pointermove` and `scroll` in the document. Held
 * in state, that re-renders this hook's host on **every keystroke anywhere in
 * the application** — and this hook's host is the app shell. `block/buzz` has a
 * performance regression test for exactly this mistake. So the timestamp is a
 * ref, and only a status *transition* is allowed to call `setState`.
 *
 * # A departure is best-effort, and does not need to work
 *
 * `pagehide` fires a beacon so a closed tab clears its dot at once instead of
 * lingering for the lease. If it does not fire — a crash, a killed process, a
 * browser that skipped it — the host's TTL collects it anyway. That is the
 * whole reason presence is a lease: no disconnect path has to be correct.
 */
export function usePresence(
  client: OpenCompanyClient,
  company: string | null | undefined,
  options: { enabled?: boolean } = {},
) {
  const enabled = options.enabled ?? true;
  const [peers, setPeers] = useState<ReadonlyMap<string, Peer>>(new Map());
  const [preference, setPreference] = useState<PresencePreference>(storedPreference);
  const [supported, setSupported] = useState(true);

  // The one value that must not be state. See the header.
  const lastActivity = useRef(Date.now());
  const preferenceRef = useRef(preference);
  preferenceRef.current = preference;
  // This tab's lease key, minted once per mount. See `consoleId`'s own doc.
  const tabId = useRef(consoleId());

  /**
   * Whether a failed request means "this host predates the route" (permanent,
   * disable) versus a network blip or a proxy hiccup (transient, try again on
   * the next beat). Only a 404 is the former: everything else — a dropped
   * connection, a 5xx, a timeout — says nothing about whether the route
   * exists, and treating it as permanent would leave presence disabled for
   * the rest of the tab's life over one bad request.
   */
  const isRouteMissing = (error: unknown): boolean =>
    error instanceof ApiError && error.status === 404;

  // The `atMillis` of the newest known `offline` frame per user, so a
  // snapshot reconciled later (see `reconcilePresenceSnapshot`) knows a
  // departure happened even though the departed peer has no entry left in
  // `peers` to compare a timestamp against.
  const tombstones = useRef(new Map<string, number>());

  /** Apply one live frame. */
  const onFrame = useCallback(
    (frame: { userId: string; status: PresenceStatus; atMillis: number }) => {
      if (frame.status === "offline") {
        const known = tombstones.current.get(frame.userId) ?? 0;
        if (frame.atMillis > known) tombstones.current.set(frame.userId, frame.atMillis);
      } else {
        // Present again — a stale tombstone must not keep suppressing a
        // future snapshot row for them once they're genuinely back.
        tombstones.current.delete(frame.userId);
      }
      setPeers((current) => applyPresence(current, frame) ?? current);
    },
    [],
  );

  // Document-level activity. Passive and capture-phase so it sees input
  // wherever it lands, and writes only to the ref.
  useEffect(() => {
    if (!enabled) return;
    const bump = () => {
      lastActivity.current = Date.now();
    };
    const events = ["keydown", "pointerdown", "pointermove", "scroll"] as const;
    for (const name of events) {
      document.addEventListener(name, bump, { capture: true, passive: true });
    }
    return () => {
      for (const name of events) {
        document.removeEventListener(name, bump, { capture: true });
      }
    };
  }, [enabled]);

  // The heartbeat, plus a snapshot re-read on every beat.
  //
  // The re-read (not just the announce) is what keeps a *steady* peer's clock
  // from lapsing. The host only publishes a live frame when somebody's status
  // changes — a renewal that doesn't move anything is deliberately silent, see
  // `PresenceRegistry::beat` — so a peer who stays `online` the whole time
  // never gets a fresh frame, and this hook's `atMillis` for them would freeze
  // at whatever the very first snapshot said. `livePeers`'s sweep then reads
  // that frozen timestamp against a moving clock and prunes them once the
  // lease looks stale, even though the host still has them live. Re-fetching
  // the snapshot every heartbeat renews `atMillis` for everybody still
  // present, so the local sweep never outruns the truth.
  useEffect(() => {
    if (!enabled || !supported) return;
    let live = true;

    const announce = () => {
      const status = presenceToAnnounce(
        preferenceRef.current,
        Date.now() - lastActivity.current,
      );
      // Appearing offline means sending nothing at all rather than sending
      // "offline" every minute — a lapsed lease says it more cheaply, and says
      // the same thing to every reader.
      if (status === "offline") return;
      void client.announcePresence(status, company, tabId.current).catch((error) => {
        if (live && isRouteMissing(error)) setSupported(false);
        // Otherwise a transient failure: the next beat tries again rather
        // than disabling presence for the rest of the tab's life.
      });
    };

    const refresh = () => {
      // Captured before the request goes out: see `reconcilePresenceSnapshot`
      // for why a peer this snapshot never mentions is only dropped when what
      // we already knew about them predates this moment, rather than the
      // moment the response happens to land.
      const requestSentAt = Date.now();
      void client
        .presence(company)
        .then((res) => {
          if (!live) return;
          setPeers((current) =>
            reconcilePresenceSnapshot(current, tombstones.current, res?.people, requestSentAt),
          );
        })
        .catch((error) => {
          if (live && isRouteMissing(error)) setSupported(false);
        });
    };

    refresh();
    announce();
    const beat = setInterval(() => {
      announce();
      refresh();
    }, PRESENCE_HEARTBEAT_MS);
    return () => {
      live = false;
      clearInterval(beat);
    };
  }, [client, company, enabled, supported]);

  // Expire lapsed peers. Runs on the heartbeat rather than every second: a
  // presence dot going stale is not urgent, and a frame usually arrives first.
  useEffect(() => {
    if (!enabled) return;
    const sweep = setInterval(() => {
      setPeers((current) => {
        const next = livePeers(current, Date.now());
        return next.size === current.size ? current : next;
      });
    }, PRESENCE_TTL_MS);
    return () => clearInterval(sweep);
  }, [enabled]);

  // Clear the dot on the way out, best-effort.
  useEffect(() => {
    if (!enabled || !supported) return;
    const leave = () => {
      client.disconnectPresenceBeacon(company, tabId.current);
    };
    window.addEventListener("pagehide", leave);
    return () => {
      window.removeEventListener("pagehide", leave);
    };
  }, [client, company, enabled, supported]);

  const choose = useCallback(
    (next: PresencePreference) => {
      setPreference(next);
      try {
        localStorage.setItem(PREFERENCE_KEY, next);
      } catch {
        // Storage refused; the choice still holds for this tab.
      }
      if (next === "offline") {
        client.disconnectPresenceBeacon(company, tabId.current);
        setPeers((current) => current);
      }
    },
    [client, company],
  );

  return { peers, preference, choose, onFrame, supported };
}
