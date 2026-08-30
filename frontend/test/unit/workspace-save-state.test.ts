/**
 * The workspace editor's save indicator (issue #1372).
 *
 * The rule this pins is about *silence*. Before #1372 the indicator had four
 * states and rendered nothing for `idle` — and `onEdit` set `idle` on every
 * keystroke, so the header was blank for the whole autosave debounce plus the
 * round trip after it, and the first thing it ever said was "Saved". Measured
 * against a live host, a reload 150 ms after typing lost the text and nothing
 * had warned. The line was silent for exactly the window in which the words
 * existed only in the tab.
 *
 * So the property under test is not "each state has a nice label" but "the only
 * state that renders nothing is the one with nothing at risk".
 */
import { describe, expect, it } from "vitest";

import { saveStatusLabel, type SaveState } from "@/views/WorkspaceView";

const EVERY_STATE: SaveState[] = ["idle", "dirty", "saving", "saved", "error"];

describe("saveStatusLabel", () => {
  it("says nothing only for an untouched note", () => {
    const silent = EVERY_STATE.filter((state) => saveStatusLabel(state) === null);
    expect(silent).toEqual(["idle"]);
  });

  it("announces buffered typing before the write is even attempted", () => {
    // The regression that mattered: `dirty` is what a keystroke sets, and it
    // must be visible immediately rather than waiting for the debounce.
    expect(saveStatusLabel("dirty")).toBe("Unsaved");
  });

  it("distinguishes in-flight from acknowledged", () => {
    // Two different facts. "Saving…" means the host has the bytes and has not
    // answered; "Saved" means it has. Collapsing them would let a failed write
    // spend its whole round trip claiming success.
    expect(saveStatusLabel("saving")).toBe("Saving…");
    expect(saveStatusLabel("saved")).toBe("Saved");
  });

  it("says what happens next when a write failed", () => {
    // Not just "Error": the buffer is kept and the next edit retries it, and an
    // operator who does not know that has no reason to keep typing rather than
    // copying their paragraph out by hand.
    expect(saveStatusLabel("error")).toBe("Not saved — retrying on edit");
  });

  it("never reports a state as saved while text is still buffered", () => {
    // The dishonesty this file exists to prevent, stated directly: no state
    // that means "the host does not have this yet" may read as "Saved".
    for (const state of ["dirty", "saving", "error"] as SaveState[]) {
      expect(saveStatusLabel(state)).not.toBe("Saved");
    }
  });
});
