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
 * "Later" means later, not never: dismissing a staged update hides it until the
 * flow moves on. It comes back when the phase re-enters an actionable state
 * from a non-actionable one — the next check that finds a newer build, or an
 * install the operator started themselves.
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
