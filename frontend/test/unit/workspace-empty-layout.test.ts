// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { ConnectionScopeProvider } from "@/connections/ConnectionContext";
import { WorkspaceView } from "@/views/WorkspaceView";

/**
 * Issues #1380, #1481 and #1386.
 *
 * #1380: two contradictory empty states. The explorer had a `nodes.length === 0`
 * branch saying "This workspace is empty. Create a note to start." — dead code,
 * since `ensure_workspace_scaffold` lays down `Agents/` and `secrets/` on every
 * boot — while the note pane said "No note open / Pick a note from the
 * explorer". And the two maintenance buttons were enabled whatever the tree
 * held.
 *
 * #1481: neither message said what the tree *is*. The premise — this workspace
 * is shared with the company's agents, who read what is written here and write
 * back into it — lived only in code comments in three files, none rendered.
 *
 * #1386: the two-pane layout is already single-pane below `md` on main. This
 * pins that so it cannot regress silently; it is not rebuilt here.
 */

function node(over: {
  id: string;
  name: string;
  kind: "folder" | "file";
  parentId?: string;
}) {
  return { ...over, updatedAt: 1 };
}

/** Exactly what a freshly provisioned company boots with. */
const SCAFFOLD = [
  node({ id: "agents", name: "Agents", kind: "folder" }),
  node({ id: "secrets", name: "secrets", kind: "folder" }),
  node({ id: "readme", name: "README.md", kind: "file", parentId: "secrets" }),
];

let container: HTMLDivElement;
let root: Root;

function host(tree: ReturnType<typeof node>[]): OpenCompanyClient {
  return {
    scopeFor: () => "/api/v1/company/acme",
    get: vi.fn().mockResolvedValue(tree),
  } as unknown as OpenCompanyClient;
}

beforeEach(() => {
  (
    globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true;
  Element.prototype.scrollIntoView = vi.fn();
  localStorage.clear();
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render(tree: ReturnType<typeof node>[]) {
  await act(async () => {
    root.render(
      createElement(ConnectionScopeProvider, {
        scope: { connection: "c1", company: "acme" },
        children: createElement(WorkspaceView, {
          client: host(tree),
          company: "acme",
        }),
      }),
    );
  });
}

describe("a workspace nobody has written in yet explains itself (issue #1481)", () => {
  it("says the tree is shared with the company's agents", async () => {
    await render(SCAFFOLD);
    const pane = container.querySelector(
      '[data-testid="workspace-empty-first-run"]',
    );

    expect(pane).not.toBeNull();
    const text = pane?.textContent ?? "";
    expect(text).toContain("agents");
    // The premise lived only in code comments: that they read what you write,
    // and write back into the same tree.
    expect(text.toLowerCase()).toContain("what you write");
    expect(text).toContain("derived/");
  });

  it("offers a folder as well as a note, since filing is the first decision", async () => {
    await render(SCAFFOLD);
    const labels = Array.from(container.querySelectorAll("button")).map((b) =>
      b.textContent?.trim(),
    );

    expect(labels).toContain("New note");
    expect(labels).toContain("New folder");
  });

  it("does not tell the operator to pick a note from an explorer of scaffold rows", async () => {
    await render(SCAFFOLD);
    expect(container.textContent).not.toContain(
      "Pick a note from the explorer",
    );
  });
});

describe("a workspace with notes in it keeps the terse copy (issue #1380)", () => {
  it("says No note open once a person has written something", async () => {
    await render([
      ...SCAFFOLD,
      node({ id: "n1", name: "Plan.md", kind: "file" }),
    ]);

    expect(
      container.querySelector('[data-testid="workspace-empty-no-selection"]'),
    ).not.toBeNull();
    expect(container.textContent).toContain("No note open");
    expect(container.textContent).toContain("Pick a note from the explorer");
  });
});

describe("the explorer no longer contradicts the pane (issue #1380)", () => {
  it("posts no empty-workspace message of its own", async () => {
    await render([]);

    // The dead branch's exact sentence.
    expect(container.textContent).not.toContain("This workspace is empty");
  });

  it("disables tidy and repair on a tree with nothing to act on", async () => {
    await render([]);

    expect(
      (
        container.querySelector(
          '[data-testid="workspace-sweep"]',
        ) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
    expect(
      (
        container.querySelector(
          '[data-testid="workspace-repair"]',
        ) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
  });

  it("leaves them enabled once the tree holds anything", async () => {
    await render(SCAFFOLD);

    expect(
      (
        container.querySelector(
          '[data-testid="workspace-sweep"]',
        ) as HTMLButtonElement
      ).disabled,
    ).toBe(false);
  });
});

describe("the narrow-width layout shows one pane at a time (issue #1386)", () => {
  /**
   * Already implemented on main — this locks it. The classes are the contract:
   * the explorer is `md:flex` plus a `showExplorer`-driven flex/hidden, and the
   * note section is its mirror image, so below `md` exactly one is displayed
   * while both are at `md` and up.
   */
  it("makes the two panes mutually exclusive below md", async () => {
    await render(SCAFFOLD);

    const explorer = container.querySelector("aside");
    const note = container.querySelector("section");
    expect(explorer?.className).toContain("md:flex");
    expect(explorer?.className).toContain("flex");
    expect(note?.className).toContain("hidden md:flex");
  });

  it("flips which pane is shown when the explorer is toggled", async () => {
    await render(SCAFFOLD);

    await act(async () => {
      (
        container.querySelector(
          '[aria-label="Toggle explorer"]',
        ) as HTMLButtonElement
      ).click();
    });

    const explorer = container.querySelector("aside");
    const note = container.querySelector("section");
    expect(explorer?.className).toContain("hidden");
    expect(note?.className).not.toContain("hidden md:flex");
  });

  it("keeps a back affordance in the empty pane for the narrow case", async () => {
    await render(SCAFFOLD);
    const pane = container.querySelector(
      '[data-testid="workspace-empty-first-run"]',
    );

    expect(
      pane?.querySelector('[aria-label="Toggle explorer"]'),
    ).not.toBeNull();
    expect(pane?.querySelector(".md\\:hidden")).not.toBeNull();
  });
});
