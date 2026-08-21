// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { type FsNode, OPERATOR_ORIGIN } from "@/api/workspace";
import { ConnectionScopeProvider } from "@/connections/ConnectionContext";
import { WorkspaceView } from "@/views/WorkspaceView";

/**
 * The two marked folders in one tree, and the fact that they are marked
 * differently (issues #1377 and #1465).
 *
 * Both rules land on the same three lines of `TreeRow`: the row's muting, the
 * glyph after the name, and the `…` menu. Read on its own each rule is obvious;
 * read together they are the exact inverse of each other, and the inversion is
 * what a careless merge flattens:
 *
 * * `derived/` is "an agent writes this, you do not", so #1377 takes **Rename**
 *   and **Move to…** off its menu — the host refuses both, and a control whose
 *   only outcome is an error toast is worse than no control.
 * * `secrets/` is "you write this, an agent does not", so #1465 **keeps** the
 *   whole menu. `Move to…` is the only control in the console that changes a
 *   note's audience, and the warning #1465 exists to show is reached through
 *   that item and nowhere else.
 *
 * So a resolution that muted or short-menued both folders from one predicate
 * would leave every other test in this repo passing while #1465's warning
 * became unreachable. That is the thing this file pins, and it is why the two
 * rules are asserted against one tree rather than one apiece.
 */

/**
 * A whole `FsNode`, not a partial one.
 *
 * The omissions this replaces were survivable rather than correct: `fetchTree`
 * normalizes a wire node (`parentId ?? null`, `createdBy ?? OPERATOR_ORIGIN`),
 * and the fixture reaches the view through it, so a missing `parentId` became
 * `null` and a missing `createdBy` became the operator before `TreeRow` read
 * either. Spelling them out anyway costs nothing and stops the fixture relying
 * on a defaulting step that belongs to a different test's subject.
 */
function node(over: {
  id: string;
  name: string;
  kind: "folder" | "file";
  parentId?: string;
}): FsNode {
  return {
    parentId: null,
    updatedAt: 1,
    createdBy: OPERATOR_ORIGIN,
    updatedBy: OPERATOR_ORIGIN,
    ...over,
  };
}

/** A fake host: `get` answers the tree read, `patch` the rename/move call. */
function client(tree: FsNode[], patch = vi.fn()): OpenCompanyClient {
  return {
    scopeFor: () => "/api/v1/company/acme",
    get: vi.fn().mockResolvedValue(tree),
    patch,
  } as unknown as OpenCompanyClient;
}

/**
 * One `derived/` file, one `secrets/` file, one ordinary note — plus the two
 * lookalike roots that are ordinary shared content host-side. Marking either of
 * those would promise a rule the host does not enforce.
 */
const TREE = [
  node({ id: "d", name: "derived", kind: "folder" }),
  node({ id: "goals", name: "GOALS.md", kind: "file", parentId: "d" }),
  node({ id: "s", name: "secrets", kind: "folder" }),
  node({ id: "keys", name: "Stripe keys.md", kind: "file", parentId: "s" }),
  node({ id: "p", name: "Playbooks", kind: "folder" }),
  node({ id: "run", name: "Runbook.md", kind: "file", parentId: "p" }),
  node({ id: "dish", name: "derived-notes", kind: "folder" }),
  node({ id: "sold", name: "secrets-old", kind: "folder" }),
];

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

async function render(patch = vi.fn()) {
  await act(async () => {
    root.render(
      createElement(ConnectionScopeProvider, {
        scope: { connection: "c1", company: "acme" },
        children: createElement(WorkspaceView, { client: client(TREE, patch), company: "acme" }),
      }),
    );
  });
  await act(async () => {});
}

// Each tree row is exactly one `div.group` (see `TreeRow`), so matching on that
// class picks the row rather than an ancestor that merely contains the text.
function row(name: string): HTMLElement {
  const found = Array.from(container.querySelectorAll("div.group")).find(
    (d) => d.querySelector("span.truncate")?.textContent?.trim() === name,
  );
  if (!found) throw new Error(`no tree row for “${name}” in:\n${container.innerHTML}`);
  return found as HTMLElement;
}

/** Open a row's `…` Actions menu and read back its items. */
async function menuItemsFor(name: string): Promise<string[]> {
  const trigger = row(name).querySelector('[aria-label="Actions"]');
  if (!trigger) throw new Error(`no Actions button for “${name}”`);
  await act(async () => {
    (trigger as HTMLElement).click();
  });
  // The menu portals onto `document.body`, not into `container`.
  return Array.from(document.querySelectorAll('[role="menuitem"]')).map((el) =>
    (el.textContent ?? "").trim(),
  );
}

describe("the two marked folders, side by side in one tree", () => {
  it("gives each folder its own glyph, and no folder both", async () => {
    await render();
    // A lock is "you may not write this"; an eye-off is "nobody else reads
    // this". Same slot, opposite rules — one glyph each is the whole point.
    expect(row("derived").querySelector('[data-testid="workspace-tree-derived"]')).not.toBeNull();
    expect(row("derived").querySelector('[data-testid="workspace-tree-secret"]')).toBeNull();
    expect(row("secrets").querySelector('[data-testid="workspace-tree-secret"]')).not.toBeNull();
    expect(row("secrets").querySelector('[data-testid="workspace-tree-derived"]')).toBeNull();
  });

  it("mutes both marked folders and neither lookalike", async () => {
    await render();
    for (const name of ["derived", "secrets"]) {
      expect(row(name).className).toContain("text-muted-foreground");
    }
    // `derived-notes/` and `secrets-old/` are ordinary shared content: the rule
    // is a segment rule, not a prefix rule, on both sides.
    for (const name of ["Playbooks", "derived-notes", "secrets-old"]) {
      expect(row(name).className).not.toContain("text-muted-foreground");
      expect(row(name).querySelector('[data-testid="workspace-tree-derived"]')).toBeNull();
      expect(row(name).querySelector('[data-testid="workspace-tree-secret"]')).toBeNull();
    }
  });

  it("takes Rename and Move to… off a derived row, because the host refuses both", async () => {
    await render();
    const items = await menuItemsFor("derived");
    expect(items).toContain("Delete");
    expect(items).not.toContain("Rename");
    expect(items).not.toContain("Move to…");
  });

  it("keeps the whole menu on a secrets row, because Move to… is where the warning lives", async () => {
    await render();
    // The regression this file exists for: #1465's audience warning is raised
    // by `MoveDialog`, and `Move to…` is the only way into it. Short-menuing
    // `secrets/` the way `derived/` is short-menued would delete the warning
    // without touching a line of `MoveDialog` or failing one of its tests.
    const items = await menuItemsFor("secrets");
    expect(items).toContain("Move to…");
    expect(items).toContain("Rename");
    expect(items).toContain("Delete");
  });

  it("leaves an ordinary folder its whole menu", async () => {
    await render();
    const items = await menuItemsFor("Playbooks");
    // The two "New … here" entries are issue #1477's — a folder is a place to
    // put something, and this menu is where that is decided.
    expect(items).toEqual(["New note here", "New folder here", "Rename", "Move to…", "Delete"]);
  });
});

/**
 * The rename half of the boundary (issue #1465, review).
 *
 * The host reads agent visibility off the *first path segment*, and a rename of
 * a root folder rewrites it for the whole subtree. `PATCH …/workspace/<id>` with
 * `{"name":"vault"}` answers 200 against a running host — only `derived/` is
 * guarded on that route — so `secrets/` was one unannounced click from being
 * agent-readable. The move already stops and asks; this pins that the rename
 * does too, at the surface rather than only in the pure function.
 */
describe("renaming the secrets root", () => {
  /** Open the row's `…` menu, click Rename, and type `next` into the field. */
  async function typeRename(rowName: string, next: string) {
    const trigger = row(rowName).querySelector('[aria-label="Actions"]');
    await act(async () => {
      (trigger as HTMLElement).click();
    });
    const item = Array.from(document.querySelectorAll('[role="menuitem"]')).find(
      (el) => el.textContent?.trim() === "Rename",
    );
    await act(async () => {
      (item as HTMLElement).click();
    });
    const input = document.querySelector<HTMLInputElement>("#fs-name")!;
    await act(async () => {
      // React tracks the last value it wrote, so a bare `input.value =` is
      // swallowed as "unchanged"; the native setter is how a test types.
      Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value")!.set!.call(
        input,
        next,
      );
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    const confirm = Array.from(document.querySelectorAll("button")).find(
      (b) => b.textContent?.trim() === "Rename",
    );
    await act(async () => {
      (confirm as HTMLButtonElement).click();
    });
  }

  it("asks before it renames the boundary away, and does not call the host", async () => {
    const patch = vi.fn().mockResolvedValue({});
    await render(patch);
    await typeRename("secrets", "vault");

    const panel = document.querySelector('[data-testid="workspace-move-audience"]');
    expect(panel).not.toBeNull();
    expect(panel?.textContent).toContain("Agents will be able to read this folder.");
    // The whole point: the rename has not happened yet.
    expect(patch).not.toHaveBeenCalled();

    const confirm = document.querySelector<HTMLButtonElement>(
      '[data-testid="workspace-move-audience-confirm"]',
    );
    expect(confirm?.textContent).toBe("Rename out of secrets");
    await act(async () => confirm?.click());
    expect(patch).toHaveBeenCalledTimes(1);
  });

  it("renames an ordinary folder on one click, as before", async () => {
    // The step is added to the renames that change something and to no others.
    const patch = vi.fn().mockResolvedValue({});
    await render(patch);
    await typeRename("Playbooks", "Runbooks");

    expect(document.querySelector('[data-testid="workspace-move-audience"]')).toBeNull();
    expect(patch).toHaveBeenCalledTimes(1);
  });
});
