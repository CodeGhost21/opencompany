// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { FsNode } from "@/api/workspace";
import { moveAudienceWarning, renameAudienceWarning } from "@/lib/workspace";
import { MoveAudienceConfirm } from "@/views/workspace/MoveAudienceConfirm";

/**
 * Issue #1465, the half that matters most.
 *
 * A marker on a folder is passive. `Move to…` is the one control in the console
 * that changes a note's *audience*, and it changed it in both directions on a
 * single click with a plain "moved" toast either way: moving into `secrets/`
 * revoked every agent's access, moving out granted it. Neither was announced.
 *
 * The two directions are not the same event, so the copy is asserted per
 * direction and not merely "a warning appeared".
 */

function node(partial: Partial<FsNode> & { id: string; name: string }): FsNode {
  return {
    kind: "file",
    parentId: null,
    updatedAt: 0,
    ...partial,
  } as FsNode;
}

const TREE: FsNode[] = [
  node({ id: "s", name: "secrets", kind: "folder" }),
  node({ id: "keys", name: "Stripe keys.md", parentId: "s" }),
  node({ id: "sub", name: "vendors", kind: "folder", parentId: "s" }),
  node({ id: "deep", name: "Twilio.md", parentId: "sub" }),
  node({ id: "plays", name: "Playbooks", kind: "folder" }),
  node({ id: "runbook", name: "Runbook.md", parentId: "plays" }),
  node({ id: "prod", name: "Product", kind: "folder" }),
];

const NOTE = (id: string) => TREE.find((n) => n.id === id)!;

describe("a move that changes who can read the note", () => {
  it("warns on the way in", () => {
    const warning = moveAudienceWarning(TREE, NOTE("runbook"), "s");
    expect(warning?.change).toBe("hidden");
    expect(warning?.title).toBe("Agents will no longer be able to read this note.");
    // The name is in the sentence, because a move dialog opened from a row menu
    // is the only place the operator sees which note this is about.
    expect(warning?.body).toContain("Runbook");
    expect(warning?.confirmLabel).toBe("Move into secrets");
  });

  it("warns on the way out, and says to check it first", () => {
    const warning = moveAudienceWarning(TREE, NOTE("keys"), "plays");
    expect(warning?.change).toBe("exposed");
    expect(warning?.title).toBe("Agents will be able to read this note.");
    // The dangerous direction: a note the agents have read cannot be un-read,
    // so the sentence has to ask for the check before the click, not after.
    expect(warning?.body).toContain("Check it holds no credentials first.");
    expect(warning?.confirmLabel).toBe("Move out of secrets");
  });

  it("warns on a move out to the workspace root", () => {
    // The root is agent-visible, and `destId === null` is the code path a
    // direction check written on the destination *node* would miss entirely.
    expect(moveAudienceWarning(TREE, NOTE("keys"), null)?.change).toBe("exposed");
  });

  it("counts a nested destination as inside", () => {
    // `secrets/vendors/` is as hidden as `secrets/` — the host's rule is the
    // root segment, so a check written on the destination's own name would say
    // "vendors" is ordinary and warn about nothing.
    expect(moveAudienceWarning(TREE, NOTE("runbook"), "sub")?.change).toBe("hidden");
    // And a move *within* `secrets/` changes nothing, so it must not warn.
    expect(moveAudienceWarning(TREE, NOTE("keys"), "sub")).toBeNull();
  });

  it("says nothing about a move that changes nothing", () => {
    // Nearly every move. A warning shown on those is one nobody reads on the
    // move that matters.
    expect(moveAudienceWarning(TREE, NOTE("runbook"), "prod")).toBeNull();
    expect(moveAudienceWarning(TREE, NOTE("runbook"), null)).toBeNull();
  });

  it("says a folder move takes everything in it", () => {
    // A folder's move carries every note under it in the same call, so "this
    // note" would understate a move of thirty.
    const warning = moveAudienceWarning(TREE, NOTE("plays"), "s");
    expect(warning?.body).toContain("this folder and everything in it");
  });

  it("says folder in the title too, not only in the body", () => {
    // The title is the line that gets read. A heading promising one note above
    // a paragraph describing a subtree is the wrong half to be vague in.
    expect(moveAudienceWarning(TREE, NOTE("plays"), "s")?.title).toBe(
      "Agents will no longer be able to read this folder.",
    );
    expect(moveAudienceWarning(TREE, NOTE("sub"), null)?.title).toBe(
      "Agents will be able to read this folder.",
    );
    // And a note still says note.
    expect(moveAudienceWarning(TREE, NOTE("runbook"), "s")?.title).toBe(
      "Agents will no longer be able to read this note.",
    );
    expect(moveAudienceWarning(TREE, NOTE("keys"), "plays")?.title).toBe(
      "Agents will be able to read this note.",
    );
  });
});

/**
 * The other control that crosses the same boundary (issue #1465, review).
 *
 * The host decides agent visibility from the *first path segment*
 * (`is_agent_hidden_path`), so renaming the root `secrets/` folder rewrites
 * that segment for its whole subtree — and the host allows it: a `PATCH
 * …/workspace/<id>` with `{"name":"vault"}` answers 200, verified against a
 * running host. Only `derived/` is guarded on that route. So the rename needed
 * the same panel the move got, or the console's promise was one click from
 * quietly ending.
 */
describe("a rename that changes who can read the note", () => {
  it("warns when the secrets root is renamed away", () => {
    const warning = renameAudienceWarning(TREE, NOTE("s"), "vault");
    expect(warning?.change).toBe("exposed");
    expect(warning?.title).toBe("Agents will be able to read this folder.");
    expect(warning?.body).toContain("vault");
    expect(warning?.body).toContain("Check it holds no credentials first.");
    expect(warning?.confirmLabel).toBe("Rename out of secrets");
  });

  it("warns when the secrets root is *moved* under another folder", () => {
    // The same segment rewrite by the other route: `secrets/` nested under
    // `Playbooks/` becomes `Playbooks/secrets/...`, whose first segment is no
    // longer `secrets`, so the host stops hiding it. `moveAudienceWarning`
    // already caught this — it compares the node's own ancestry to the
    // destination's — but nothing pinned it, and it is half the boundary.
    const warning = moveAudienceWarning(TREE, NOTE("s"), "plays");
    expect(warning?.change).toBe("exposed");
    expect(warning?.title).toBe("Agents will be able to read this folder.");
  });

  it("warns when an ordinary root folder is renamed into the boundary", () => {
    const warning = renameAudienceWarning(TREE, NOTE("plays"), "secrets");
    expect(warning?.change).toBe("hidden");
    expect(warning?.confirmLabel).toBe("Rename into secrets");
  });

  it("cannot be stepped around by capitalising it", () => {
    // The host compares the segment case-insensitively, so `Secrets` hides just
    // as `secrets` does — and renaming `secrets` to `SECRETS` changes nothing,
    // so it must not warn.
    expect(renameAudienceWarning(TREE, NOTE("plays"), "Secrets")?.change).toBe("hidden");
    expect(renameAudienceWarning(TREE, NOTE("s"), "SECRETS")).toBeNull();
    expect(renameAudienceWarning(TREE, NOTE("s"), "  secrets  ")).toBeNull();
  });

  it("says nothing about a rename that cannot move the boundary", () => {
    // A nested node's first segment belongs to an ancestor, so no rename of it
    // can cross the line — including one that names it `secrets`.
    expect(renameAudienceWarning(TREE, NOTE("keys"), "Stripe keys (old).md")).toBeNull();
    expect(renameAudienceWarning(TREE, NOTE("sub"), "secrets")).toBeNull();
    expect(renameAudienceWarning(TREE, NOTE("runbook"), "secrets")).toBeNull();
    // And an ordinary root renamed to another ordinary name.
    expect(renameAudienceWarning(TREE, NOTE("plays"), "Runbooks")).toBeNull();
    // `secrets-old` is ordinary content on both sides of the rename.
    expect(renameAudienceWarning(TREE, NOTE("plays"), "secrets-old")).toBeNull();
  });
});

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function render(warning: NonNullable<ReturnType<typeof moveAudienceWarning>>, onConfirm = () => {}) {
  act(() =>
    root.render(
      createElement(MoveAudienceConfirm, { warning, onCancel: () => {}, onConfirm }),
    ),
  );
}

describe("the confirmation the move dialog shows instead of committing", () => {
  it("shows the whole sentence, not a tooltip", () => {
    const warning = moveAudienceWarning(TREE, NOTE("keys"), "plays")!;
    render(warning);
    const panel = container.querySelector('[data-testid="workspace-move-audience"]');
    expect(panel?.textContent).toContain(warning.title);
    expect(panel?.textContent).toContain(warning.body);
  });

  it("marks the outward move as the destructive one", () => {
    render(moveAudienceWarning(TREE, NOTE("keys"), "plays")!);
    expect(container.querySelector('[data-audience-change="exposed"]')).not.toBeNull();
    // The button repeats the direction: it is the last thing read before the
    // click, and a bare "Move" there would undo the paragraph above it.
    const confirm = container.querySelector('[data-testid="workspace-move-audience-confirm"]');
    expect(confirm?.textContent).toBe("Move out of secrets");
  });

  it("does not commit until the confirm is clicked", () => {
    const onConfirm = vi.fn();
    render(moveAudienceWarning(TREE, NOTE("runbook"), "s")!, onConfirm);
    expect(onConfirm).not.toHaveBeenCalled();
    const confirm = container.querySelector<HTMLButtonElement>(
      '[data-testid="workspace-move-audience-confirm"]',
    );
    expect(confirm?.textContent).toBe("Move into secrets");
    act(() => confirm?.click());
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });
});
