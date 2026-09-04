// When a desktop update is worth interrupting somebody for, and what to say.
//
// # The contract this file encodes
//
// The update flow has eight states and the operator is shown **three** of them.
// Checking, finding something, and downloading it are all work the application
// can do without anybody's permission, so it does them quietly: a banner that
// appears to announce a download nobody can act on is a banner that trains
// people to dismiss the one that matters.
//
// What is left is the moment there is a real decision — the bytes are on disk,
// verified, and applying them costs a restart — plus the two states that follow
// from the operator's own click: the install running, and the install failing.
//
// Kept here rather than in the component because it is the part worth pinning
// in a test: `AppUpdatePrompt` renders whatever this says is renderable, and
// `test/unit/app-update-visibility.test.ts` asserts the silence directly rather
// than through six layers of render.

/**
 * Where the update flow has got to.
 *
 * `checking`, `available` and `downloading` are background work.
 * `ready`, `installing` and `error` are the operator's business.
 * `idle` and `up-to-date` are the resting states either side of a probe.
 */
export type AppUpdatePhase =
  | "idle"
  | "checking"
  | "available"
  | "downloading"
  | "ready"
  | "installing"
  | "up-to-date"
  | "error";

/**
 * Whether this phase has anything to say to the person using the application.
 *
 * The single rule the whole UX rests on. Everything false here happens with no
 * banner at all, which is why a desktop update is normally invisible until the
 * one moment it asks for a restart.
 */
export function isActionable(phase: AppUpdatePhase): boolean {
  return phase === "ready" || phase === "installing" || phase === "error";
}

/**
 * Whether the download owns the phase, so a probe must keep its hands off it.
 *
 * `busy` is the download's own flag: bytes are being fetched, or are already
 * staged and waiting on a restart. `ready` and `installing` are the phases a
 * probe must not disturb even in the instant before that flag is set.
 *
 * This is the entry guard — asked before a probe starts. What an answer needs
 * on the way back is [`probeMayReport`], which asks this *and* whether a newer
 * probe has started since.
 */
export function probeIsSuperseded(phase: AppUpdatePhase, busy: boolean): boolean {
  return busy || phase === "ready" || phase === "installing";
}

/**
 * Whether a probe that has just answered may still write what it learned.
 *
 * Two probes can be in flight at once and it takes no unusual timing: a check
 * that was waiting on the network when the laptop's lid closed lands whenever
 * the machine wakes, and `setInterval` fires its overdue tick on the same wake.
 * One timer, never meant to overlap, now has two answers coming back in an
 * order nobody chose.
 *
 * **Staleness is a property of which probe this is, not of what phase we happen
 * to be in**, and that distinction is the whole reason `generation` exists.
 * Phase alone leaves gaps, because a phase is only a snapshot of how far the
 * newer probe has got. Guarding on the *staged* phases closed the window where
 * the older answer overwrites `ready`; it left the one a beat earlier, where
 * the newer probe has set `available` but the effect that starts its download
 * has not run yet — nothing is `busy`, nothing is `ready`, and a stale
 * `up-to-date` lands on top. The auto-download never fires, and the update
 * stays invisible until another quarter of an hour goes by. Same root cause,
 * one frame earlier. A monotonic counter has no such frames: a probe is either
 * the newest one or it is not.
 *
 * So the rule is both halves. `generation === latest` settles probe against
 * probe. [`probeIsSuperseded`] settles probe against the download, which the
 * counter cannot see — a "Try again" click stages bytes without starting a
 * probe at all, so nothing would have bumped it.
 */
export function probeMayReport(
  generation: number,
  latest: number,
  phase: AppUpdatePhase,
  busy: boolean,
): boolean {
  return generation === latest && !probeIsSuperseded(phase, busy);
}

/** The banner's heading for a phase. Only actionable phases have one. */
export function updateHeadline(phase: AppUpdatePhase): string {
  switch (phase) {
    case "ready":
      return "Update ready to install";
    case "installing":
      return "Installing the update";
    case "error":
      return "Update failed";
    default:
      return "Update";
  }
}

/**
 * The line under the heading when an update is staged.
 *
 * Version-first, because that is the fact somebody deciding whether to restart
 * now actually wants — and a build that did not report one still gets a
 * sentence rather than a gap.
 */
export function updateSummary(version: string | null): string {
  return version
    ? `Version ${version} is downloaded and ready.`
    : "A new version is downloaded and ready.";
}

/**
 * Whether a banner that was dismissed should come back for this transition.
 *
 * A dismissed banner comes back when the phase re-enters an actionable state
 * from a non-actionable one — a check that then stages a build, or an install
 * the operator started themselves.
 *
 * Which is reachable from `error` and not from `ready`, and that asymmetry is
 * in `useAppUpdate` rather than here: once bytes are staged the hook stops
 * probing, so nothing moves the phase again and "Later" on a staged update
 * means later than this session. That is the deliberate half — a re-check that
 * found a newer release would discard ~100 MB of verified download — and
 * `docs/spec/runtime/desktop-updates.md` says so where an operator will read
 * it. This function stays a pure rule about transitions and does not know it.
 *
 * The one exception is an error the operator has already dismissed. A failing
 * background download retries on the same cadence as the check, so re-showing
 * the same message every fifteen minutes would be the pestering this file
 * exists to prevent. A *different* error is a different fact and shows.
 */
export function shouldResurface(
  previous: AppUpdatePhase,
  next: AppUpdatePhase,
  message: string | null,
  dismissedMessage: string | null,
): boolean {
  if (!isActionable(next) || isActionable(previous)) return false;
  if (next === "error") return message !== dismissedMessage;
  return true;
}
