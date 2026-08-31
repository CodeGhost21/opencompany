import { describe, expect, it } from "vitest";

import type { IrreversibleEffect } from "@/api/tasks";
import { dispatchNeedsConfirm, patchDispatchesRun, readEffectHistory } from "@/lib/task-edit";

/**
 * The edit dialog's Column select is a second route to the write issue #351
 * wrapped in a confirmation.
 *
 * The Task Detail screen's Retry button sends `PATCH { column: "working" }` and
 * asks first when the journal recorded something irreversible. The edit
 * dialog's Column select can emit the identical patch, and its Save button was
 * a plain button — so the protection was one dropdown away from being walked
 * around on the very screen it was built for.
 *
 * These pin the two halves of the fix separately: *what counts as a dispatch*,
 * and *when a dispatch has to stop and say what already happened*. The second
 * matters as much as the first: a dialog that confirmed on every dispatch would
 * train the operator to click through it, and the confirmation would then be
 * gone on the card where it counts.
 */

function effect(kind: string, atMillis = 1_700_000_000_000): IrreversibleEffect {
  return { kind, atMillis };
}

describe("patchDispatchesRun", () => {
  it("recognises the phase word Retry sends", () => {
    expect(patchDispatchesRun({ column: "working" })).toBe(true);
  });

  it("does not treat the parks as a dispatch", () => {
    expect(patchDispatchesRun({ column: "pending" })).toBe(false);
    expect(patchDispatchesRun({ column: "done" })).toBe(false);
  });

  /**
   * `computeTaskPatch` omits a field nobody touched, so a rename on a card
   * already sitting in Working must not read as a re-dispatch. Confirming there
   * would ask about an effect the save cannot cause.
   */
  it("ignores a patch that does not move the column at all", () => {
    expect(patchDispatchesRun({ title: "renamed" })).toBe(false);
    expect(patchDispatchesRun({})).toBe(false);
  });
});

describe("dispatchNeedsConfirm", () => {
  it("confirms a dispatch on a card that already did something irreversible", () => {
    expect(
      dispatchNeedsConfirm(
        { column: "working" },
        { irreversible: [effect("payment.send")], historyIncomplete: false },
      ),
    ).toBe(true);
  });

  it("saves in one click when the journal says the card is clean", () => {
    expect(
      dispatchNeedsConfirm({ column: "working" }, { irreversible: [], historyIncomplete: false }),
    ).toBe(false);
  });

  /**
   * The honest half. A journal written before #351 holds executed keys with no
   * description, so an empty list there means "cannot say" rather than
   * "nothing happened" — and the dialog opens to say exactly that.
   */
  it("confirms when the journal admits it cannot describe its own history", () => {
    expect(
      dispatchNeedsConfirm({ column: "working" }, { irreversible: [], historyIncomplete: true }),
    ).toBe(true);
  });

  /**
   * The same reasoning one level out: a caller that has not wired the read
   * knows nothing about this card, which is not the same claim as a read that
   * came back empty. Defaulting to "clean" would leave the guard silently dead
   * until somebody wired it, which is the failure mode the guard exists to
   * prevent.
   */
  it("confirms when the effect history was never read at all", () => {
    expect(dispatchNeedsConfirm({ column: "working" }, {})).toBe(true);
  });

  /**
   * The partial reads, which is where a falsiness check fails open.
   *
   * `{ irreversible: [] }` on its own is *not* "this card is clean" — it is
   * "no effect was described, and nobody asked whether the journal could
   * describe them". `{ historyIncomplete: false }` on its own is the mirror:
   * the journal can describe its history, and nothing read what it says. Each
   * is one empty-looking field away from an all-clear it has no standing to
   * give, and each has to confirm.
   */
  it("confirms when only one half of the history was read", () => {
    expect(dispatchNeedsConfirm({ column: "working" }, { irreversible: [] })).toBe(true);
    expect(dispatchNeedsConfirm({ column: "working" }, { historyIncomplete: false })).toBe(true);
  });

  /**
   * A half that *was* read and says something happened settles it on its own.
   * The unread other half cannot make a recorded payment un-happen.
   */
  it("confirms on a dirty half whatever the unread half would have said", () => {
    expect(dispatchNeedsConfirm({ column: "working" }, { historyIncomplete: true })).toBe(true);
    expect(
      dispatchNeedsConfirm({ column: "working" }, { irreversible: [effect("payment.send")] }),
    ).toBe(true);
  });

  /**
   * The gate is about *re-entering the run*, not about the card's past. Moving
   * a card that spent money into Done is a park, and asking there would be
   * noise attached to the wrong gesture.
   */
  it("never confirms a save that is not a dispatch, however dirty the card's past", () => {
    const history = { irreversible: [effect("payment.send")], historyIncomplete: true };
    expect(dispatchNeedsConfirm({ column: "done" }, history)).toBe(false);
    expect(dispatchNeedsConfirm({ title: "renamed" }, history)).toBe(false);
    expect(dispatchNeedsConfirm({}, history)).toBe(false);
  });
});

/**
 * The three states, pinned by name rather than through the boolean.
 *
 * `dispatchNeedsConfirm` collapses `"dirty"` and `"unknown"` into the same
 * answer, which is correct and also exactly what hid the bug this replaced: a
 * partial read returned `false` and read as a considered "clean" instead of the
 * gap it was. Naming the verdict makes "cannot say" a state a test can see.
 */
describe("readEffectHistory", () => {
  it("calls a card clean only when both halves were read and both were empty", () => {
    expect(readEffectHistory({ irreversible: [], historyIncomplete: false })).toBe("clean");
  });

  it("calls a recorded effect, or an admitted gap in the journal, dirty", () => {
    expect(
      readEffectHistory({ irreversible: [effect("payment.send")], historyIncomplete: false }),
    ).toBe("dirty");
    expect(readEffectHistory({ irreversible: [], historyIncomplete: true })).toBe("dirty");
  });

  it("calls an unread half unknown, never clean", () => {
    expect(readEffectHistory({})).toBe("unknown");
    expect(readEffectHistory({ irreversible: [] })).toBe("unknown");
    expect(readEffectHistory({ historyIncomplete: false })).toBe("unknown");
  });
});
