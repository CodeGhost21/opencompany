import { useCallback, useEffect, useRef, useState } from "react";

import type { OpenCompanyClient } from "@/api/client";
import {
  applyPresence,
  livePeers,
  presenceToAnnounce,
  PRESENCE_HEARTBEAT_MS,
  PRESENCE_TTL_MS,
  type Peer,
  type PresencePreference,
  type PresenceStatus,
} from "@/lib/awareness";

const PREFERENCE_KEY = "oc.presence.override";

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

  /** Apply one live frame. */
  const onFrame = useCallback(
    (frame: { userId: string; status: PresenceStatus; atMillis: number }) => {
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

  // The heartbeat, plus the initial roster read.
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
      void client.announcePresence(status, company).catch(() => {
        // A host without the route. Stop asking rather than retrying every
        // minute for the life of the tab.
        if (live) setSupported(false);
      });
    };

    void client
      .presence(company)
      .then((res) => {
        if (!live) return;
        setPeers(
          new Map(res.people.map((p) => [p.userId, { status: p.status, atMillis: p.atMillis }])),
        );
      })
      .catch(() => {
        if (live) setSupported(false);
      });

    announce();
    const beat = setInterval(announce, PRESENCE_HEARTBEAT_MS);
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
      client.disconnectPresenceBeacon(company);
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
        client.disconnectPresenceBeacon(company);
        setPeers((current) => current);
      }
    },
    [client, company],
  );

  return { peers, preference, choose, onFrame, supported };
}
