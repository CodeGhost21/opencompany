import { describe, expect, it } from "vitest";

import { planOpenNote } from "@/views/WorkspaceView";
import type { FsNode } from "@/lib/workspace";
import type { WorkspaceEvent } from "@/views/WorkspaceView";

/**
 * What a live workspace write (issue #327) means for the note in the pane.
 *
 * The case this file exists for is the **ancestor delete**: the host announces
 * one `removed` frame naming the node somebody deleted — a folder — and never
 * one per descendant. So an open note inside that folder disappears with no
 * frame that ever says its id, and the id comparison this used to run declared
 * the frame somebody else's business. The pane stayed open on a note that was
 * gone, holding a draft and an armed autosave whose only possible outcome was a
 * 404, and the operator's unsaved words left with it.
 *
 * Both directions are pinned below, and so is the thing neither must ever do:
 * discard text the host does not already have.
 */

const OPEN = "n-note";
const FOLDER = "n-folder";

/** A tree with the open note living inside a folder. */
const withNote: FsNode[] = [
  {
    id: FOLDER,
    name: "Campaigns",
    kind: "folder",
    parentId: null,
    updatedAt: 1,
    createdBy: { kind: "operator" },
    updatedBy: { kind: "operator" },
  },
  {
    id: OPEN,
    name: "Launch.md",
    kind: "file",
    parentId: FOLDER,
    updatedAt: 1,
    createdBy: { kind: "operator" },
    updatedBy: { kind: "operator" },
  },
];

/** The same tree after the folder — and everything under it — was deleted. */
const withoutFolder: FsNode[] = [];

const frame = (nodeId: string, change: string): WorkspaceEvent => ({
  tick: 1,
  nodeId,
  change,
});

/** The open note, being edited, with a paragraph the host has never seen. */
const dirty = { draft: "half-written paragraph", saved: "" };

describe("planOpenNote", () => {
  it("rescues the draft when the open note itself is deleted", () => {
    const plan = planOpenNote({
      openId: OPEN,
      event: frame(OPEN, "removed"),
      tree: withoutFolder,
      mode: "edit",
      ...dirty,
    });

    expect(plan).toEqual({ kind: "vanished", rescue: "half-written paragraph" });
  });

  it("rescues the draft when an ANCESTOR folder is deleted", () => {
    // The regression. The frame names the folder and says nothing about the
    // open note, so the old id comparison returned early — and the note was
    // already out of the tree by the time it did.
    const plan = planOpenNote({
      openId: OPEN,
      event: frame(FOLDER, "removed"),
      tree: withoutFolder,
      mode: "edit",
      ...dirty,
    });

    expect(plan).toEqual({ kind: "vanished", rescue: "half-written paragraph" });
  });

  it("never asks for a re-read of a note that is gone", () => {
    // The one thing a vanished note must not produce. `reload` is the only
    // plan that goes back to the host for this id, and against a deleted node
    // it can only 404 — which is what left an error where a note used to be.
    for (const named of [OPEN, FOLDER]) {
      const plan = planOpenNote({
        openId: OPEN,
        event: frame(named, "removed"),
        tree: withoutFolder,
        mode: "read",
        draft: null,
        saved: "body",
      });
      expect(plan.kind).toBe("vanished");
    }
  });

  it("closes quietly when the buffer holds nothing the host lacks", () => {
    // Opening a note in Edit seeds the buffer from a fresh read, so a note
    // merely opened and then deleted elsewhere has nothing to hand back. A
    // banner offering the operator their own unchanged text is noise.
    const plan = planOpenNote({
      openId: OPEN,
      event: frame(FOLDER, "removed"),
      tree: withoutFolder,
      mode: "edit",
      draft: "body",
      saved: "body",
    });

    expect(plan).toEqual({ kind: "vanished", rescue: null });
  });

  it("leaves a surviving open note alone when the frame is somebody else's", () => {
    const plan = planOpenNote({
      openId: OPEN,
      event: frame("n-elsewhere", "updated"),
      tree: withNote,
      mode: "read",
      draft: null,
      saved: "body",
    });

    expect(plan).toEqual({ kind: "leave" });
  });

  it("re-reads the open note when the frame names it and nobody is typing", () => {
    const plan = planOpenNote({
      openId: OPEN,
      event: frame(OPEN, "updated"),
      tree: withNote,
      mode: "read",
      draft: null,
      saved: "body",
    });

    expect(plan).toEqual({ kind: "reload" });
  });

  it("refuses to re-read over an in-progress edit", () => {
    // Rule 2 of the effect, kept: replacing the body under a dirty buffer would
    // discard typing that no refetch can get back.
    const plan = planOpenNote({
      openId: OPEN,
      event: frame(OPEN, "updated"),
      tree: withNote,
      mode: "edit",
      ...dirty,
    });

    expect(plan).toEqual({ kind: "leave" });
  });

  it("does not close the pane because the tree refetch failed", () => {
    // `null` is "this read answered nothing", not "the note is gone". Closing
    // on it would cost the operator their place every time the host hiccuped.
    const plan = planOpenNote({
      openId: OPEN,
      event: frame("n-elsewhere", "removed"),
      tree: null,
      mode: "edit",
      ...dirty,
    });

    expect(plan).toEqual({ kind: "leave" });
  });

  it("still trusts a removal frame naming the open note when the refetch failed", () => {
    // The one case the frame settles and the tree cannot: it says outright that
    // this id is gone, so the armed autosave behind the pane has to be stopped
    // even though no refreshed list arrived to confirm it.
    const plan = planOpenNote({
      openId: OPEN,
      event: frame(OPEN, "removed"),
      tree: null,
      mode: "edit",
      ...dirty,
    });

    expect(plan).toEqual({ kind: "vanished", rescue: "half-written paragraph" });
  });

  it("has nothing to decide when no note is open", () => {
    const plan = planOpenNote({
      openId: null,
      event: frame(OPEN, "removed"),
      tree: withoutFolder,
      mode: "read",
      draft: null,
      saved: null,
    });

    expect(plan).toEqual({ kind: "leave" });
  });
});
