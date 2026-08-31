import { describe, expect, it } from "vitest";

import type { IrreversibleEffect } from "@/api/tasks";
import { dispatchNeedsConfirm, patchDispatchesRun } from "@/lib/task-edit";

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

  it("still confirms when only one half was wired and it says nothing is known", () => {
    expect(dispatchNeedsConfirm({ column: "working" }, { historyIncomplete: true })).toBe(true);
    expect(dispatchNeedsConfirm({ column: "working" }, { irreversible: [] })).toBe(false);
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
