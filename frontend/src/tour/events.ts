// The run-level half of react-joyride's protocol, kept in one small module so
// the controller's wiring is testable without a browser.
//
// # Why these are bare string literals
//
// `TourController` lazy-loads react-joyride (most operators never run the
// tour), so it must not statically `import { EVENTS, STATUS }` — that pulls the
// runtime module into the main chunk and defeats the split. The literals below
// are those enums' values, written out.
//
// A copied constant can drift from the package it copies, so
// `test/unit/tour-terminal.test.ts` imports the real `EVENTS`/`STATUS` and
// asserts each of these still matches. The test is not shipped, so it can
// import the runtime module freely.
//
// # Why `tour:end` and not "watch the status"
//
// react-joyride v3 has **no `callback` prop**. The run-level hook is `onEvent`,
// and the terminal moment it reports is `EVENTS.TOUR_END` — emitted exactly
// once, when `status` changes into `finished` or `skipped`
// (`useLifecycleEffect`). joyride then calls `controls.reset()` for an
// uncontrolled tour, which moves `status` straight back to `ready`; so the
// event is the only reliable place to read the outcome, and gating on it also
// stops the same outcome being recorded twice.
//
// `options.after` — where this used to live — is joyride's **per-step** hook.
// It runs around an individual step and its `TourData` never carries a
// run-level `finished`/`skipped`, which is issue #1408: completing the tour
// recorded nothing at all.

/** `EVENTS.TOUR_END` — the run is over; `status` says how. */
export const EVENT_TOUR_END = "tour:end";
/** `STATUS.FINISHED` — the operator reached the last stop and pressed Done. */
export const STATUS_FINISHED = "finished";
/** `STATUS.SKIPPED` — the operator pressed "Skip tour" from inside a run. */
export const STATUS_SKIPPED = "skipped";

/**
 * How a tour run ended.
 *
 * Both outcomes mean "do not offer this again" — see `terminalOutcome`.
 */
export type TourOutcome = "completed" | "skipped";

/**
 * The outcome to record for a joyride event, or `null` if the run is not over.
 *
 * Deliberately structural rather than typed against `EventData`: the shape it
 * needs is two strings, and keeping it that way is what lets the unit suite
 * exercise it without constructing a whole joyride step.
 *
 * **Completing and skipping record different flags but mean the same thing.**
 * `tourSeen` is `completed || skipped`, so either one silences the welcome
 * card. They stay distinguishable because "watched the whole tour" and "bailed
 * out at step 3" are different facts about an operator, and the day there is a
 * backend preference to sync (or a "you skipped this — want to finish it?"
 * nudge) the distinction is the thing that makes it possible. What must never
 * differ is whether the tour is re-offered.
 */
export function terminalOutcome(data: { type: string; status: string }): TourOutcome | null {
  if (data.type !== EVENT_TOUR_END) return null;
  if (data.status === STATUS_SKIPPED) return "skipped";
  if (data.status === STATUS_FINISHED) return "completed";
  return null;
}
