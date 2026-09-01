import { useEffect, useState } from "react";

import type { OpenCompanyClient } from "@/api/client";
import { getPolicy, isPolicyStatus, type PolicyStatus } from "@/api/policy";
import { startVisiblePolling } from "@/lib/visible-poll";

/**
 * How often the title row re-reads the policy.
 *
 * Deliberately slower than {@link useApprovalDeadline}'s 5s. That hook runs
 * while the approvals view is open, beside a feed already polling at that
 * cadence; this one is mounted for the entire life of the console, on every
 * view. The tier changes when an operator changes it — minutes or days apart,
 * never seconds — so a faster cadence would buy nothing and cost a request
 * every five seconds forever. `startVisiblePolling` gates on visibility, so a
 * backgrounded window costs the host nothing at all.
 *
 * It is also why {@link applyAutonomy} exists: 30s is the right cadence for
 * *noticing* someone else's change and far too long to wait for your own.
 */
const POLL_MS = 30000;

/** One mounted {@link useAutonomy}, with the scope it is reading. */
interface Reader {
  client: OpenCompanyClient;
  company: string | null;
  /** Take a policy the host has just returned, in place of the next poll. */
  accept: (next: PolicyStatus) => void;
}

/**
 * A policy **and the scope it describes**, held together.
 *
 * The scope travels with the value because a company switch is a render, not
 * an effect. `useState` survives the change of `company` — nothing about a new
 * dependency array empties it — and the clear below runs in a *passive* effect,
 * which React schedules AFTER paint. So the first frame of the new company was
 * drawn with the previous company's tier still in state, and the title row
 * names the company an inch to the left of the pill: a confident, attributed,
 * wrong answer about what a different company's agents may do.
 *
 * Pairing the two makes that unrepresentable rather than merely unlikely. The
 * hook returns the policy only while the snapshot's scope is still the scope
 * being asked about, so the switch frame answers `null` — which this hook
 * documents as a real answer, and which the pill renders as nothing at all.
 */
interface Snapshot {
  client: OpenCompanyClient;
  company: string | null;
  policy: PolicyStatus;
}

/**
 * Every mounted reader. In practice there is exactly one — the shell's — but
 * the scope is compared anyway (see {@link applyAutonomy}) so that a second
 * console mounted beside it could never be handed another company's tier.
 */
const readers = new Set<Reader>();

/**
 * Hand a just-written policy to the readers of that scope, now.
 *
 * The title-bar tier switcher writes through `setPolicy`, which **returns the
 * host's own resulting `PolicyStatus`** — so there is nothing to guess at and
 * nothing optimistic here: this is the same value the next poll would fetch,
 * arriving one round trip earlier instead of up to {@link POLL_MS} later.
 *
 * Pushing it into the hook rather than keeping it beside the pill is what
 * keeps a single source of truth for the tier. A local copy in the control
 * would have to be reconciled against every poll, and the failure mode of
 * getting that wrong is the one this component cannot have: a pill stating an
 * autonomy level the company is not actually under.
 *
 * A write that **fails** calls nothing, so the displayed tier is never
 * anything but a value the host returned.
 *
 * Scope is matched by object identity on `client` plus the company string,
 * because the caller and the reader are handed the very same client instance
 * by the shell. A string scope key would collide across two hosts serving the
 * same company id.
 */
export function applyAutonomy(
  client: OpenCompanyClient,
  company: string | null,
  next: PolicyStatus,
): void {
  for (const reader of readers) {
    if (reader.client === client && reader.company === company) reader.accept(next);
  }
}

/**
 * The company's effective autonomy policy, or `null` while it is unknown.
 *
 * **`null` is a real answer here, not a loading placeholder**, and the pill
 * renders nothing for it. The alternative — defaulting to a tier — would put a
 * confident sentence about what the agents are allowed to do on screen at the
 * exact moment the console does not know, which is the one failure mode a
 * standing policy indicator cannot have. An older or unreachable host, a
 * revoked session, and a company that has just been switched away from all land
 * here, and all three deserve an empty space rather than a guess.
 *
 * This is the same read the approvals settings page makes
 * (`GET {scope}/policy`), so the pill and the page it describes cannot disagree
 * about the tier: there is one endpoint and it returns the *effective* policy —
 * the console override where one is set, the manifest everywhere else.
 */
export function useAutonomy(
  client: OpenCompanyClient,
  company: string | null,
): PolicyStatus | null {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  useEffect(() => {
    let live = true;
    // Serialize refreshes, exactly as `useApprovalDeadline` does: a slow
    // request must not be invalidated by the next polling tick, or a
    // consistently slow host would never update the row.
    let refreshing = false;
    let queued = false;
    // Bumped every time a write hands us the host's answer directly. A read
    // *issued* before that bump is stale by construction however late it
    // lands, and dropping it is what stops a poll that was already in flight
    // when the operator changed the tier from putting the old tier back on
    // screen for the rest of the polling interval.
    let generation = 0;
    // Clear on a company switch. Carrying the previous company's tier into the
    // new one's title bar would be a wrong answer rather than a stale one —
    // the row names the company beside it.
    //
    // This is the second of two clears, not the only one, and it is the weaker
    // of the two: a passive effect runs after paint, so on its own it lets one
    // frame of the new company render under the old company's tier. The scope
    // check on the returned value is what actually fences that frame. This call
    // still earns its place — it drops the reference rather than leaving a
    // previous company's policy alive in state, so switching A → B → A does not
    // paint A's minutes-old tier back before the first read of the new mount
    // lands.
    setSnapshot(null);
    const refresh = () => {
      if (refreshing) {
        queued = true;
        return;
      }
      refreshing = true;
      const issued = generation;
      void getPolicy(client, company)
        .then((next) => {
          // `isPolicyStatus`, not a cast. A body that is not a policy is
          // treated exactly as an unreachable host is — the pill keeps what it
          // had and states nothing new — because the alternative is putting it
          // on screen, where the first `tiers.find` throws and takes the whole
          // console down with it. See the note on the predicate.
          if (live && issued === generation && isPolicyStatus(next)) {
            setSnapshot({ client, company, policy: next });
          }
        })
        .catch(() => {
          // Silent, like the deadline read. A host that cannot answer leaves
          // the pill absent; it must not blank an answer it already has, so a
          // transient failure mid-session keeps the last known tier rather
          // than flickering the row.
        })
        .finally(() => {
          refreshing = false;
          if (queued && live) {
            queued = false;
            refresh();
          }
        });
    };
    const reader: Reader = {
      client,
      company,
      accept: (next) => {
        // Same fence on the write path. `applyAutonomy` is handed whatever the
        // host returned from a PUT, and a PUT can answer with rubbish exactly
        // as a GET can.
        if (!live || !isPolicyStatus(next)) return;
        generation += 1;
        setSnapshot({ client, company, policy: next });
      },
    };
    readers.add(reader);
    refresh();
    const dispose = startVisiblePolling(refresh, POLL_MS);
    return () => {
      live = false;
      readers.delete(reader);
      dispose();
    };
  }, [client, company]);
  // The scope check, not a bare `return snapshot?.policy`. See {@link Snapshot}:
  // state outlives the dependency change that invalidates it, and the effect
  // that clears it runs after the frame that would have shown it.
  return snapshot && snapshot.client === client && snapshot.company === company
    ? snapshot.policy
    : null;
}
