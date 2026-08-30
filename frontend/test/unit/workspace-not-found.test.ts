// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { ConnectionScopeProvider } from "@/connections/ConnectionContext";
import { WorkspaceView } from "@/views/WorkspaceView";

/**
 * Issue #1473: a link to a note that no longer exists answered "No note open /
 * Pick a note from the explorer, or create one."
 *
 * The whole file pane — header, error, skeleton, editor — lived inside
 * `openNode && openNode.kind === "file"`, and `openNode` is a lookup in the
 * *tree*. A `#/workspace/<id>` deep link (a shipped feature, #552's "Open in
 * workspace") to a deleted note therefore skipped the branch entirely and fell
 * through to the idle empty state — blaming the reader for a link they had just
 * followed, while the 404 that had been fetched, parsed and stored was never
 * rendered at all. The same fall-through swallowed the *initial load* of a
 * deep-linked id.
 *
 * These pin the pane answering on `openId` — what was asked for — rather than
 * on tree membership.
 */

/** A host with one note, and a read that 404s for anything else. */
function host(tree: unknown[], readError?: string) {
  return {
    scopeFor: () => "/api/v1/company/acme",
    get: vi.fn(async (path: string) => {
      if (path.includes("/workspace/file/")) {
        if (readError) throw new Error(readError);
        return {
          id: "note-1",
          name: "Plan.md",
          content: "body",
          backlinks: [],
          updatedAt: 1,
        };
      }
      return tree;
    }),
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;

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

async function render(client: OpenCompanyClient, initialNodeId?: string) {
  await act(async () => {
    root.render(
      createElement(ConnectionScopeProvider, {
        scope: { connection: "c1", company: "acme" },
        children: createElement(WorkspaceView, {
          client,
          company: "acme",
          initialNodeId,
        }),
      }),
    );
  });
}

const IDLE = "Pick a note from the explorer";

describe("a link to a note that is gone says so (issue #1473)", () => {
  it("does not answer the idle empty state", async () => {
    await render(host([]), "deleted-note");

    // The reported defect, verbatim.
    expect(container.textContent).not.toContain("No note open");
    expect(container.textContent).not.toContain(IDLE);
    expect(
      container.querySelector('[data-testid="workspace-missing-note"]'),
    ).not.toBeNull();
    expect(container.textContent).toContain("no longer in this workspace");
  });

  it("renders the read error it had already fetched and stored", async () => {
    await render(host([], "workspace node not found"), "deleted-note");

    const shown = container.querySelector(
      '[data-testid="workspace-missing-error"]',
    );
    expect(shown).not.toBeNull();
    expect(shown?.textContent).toContain("not found");
  });

  it("claims nothing about why the note is gone", async () => {
    await render(host([], "workspace node not found"), "deleted-note");
    const text = container.textContent ?? "";

    // The pane cannot tell a deletion from a wrong company from a failed tree
    // read, so it must not name one.
    for (const cause of ["deleted", "expired", "discarded", "removed by"]) {
      expect(text.toLowerCase()).not.toContain(cause);
    }
  });

  it("offers a way onward that clears the stale id", async () => {
    await render(host([]), "deleted-note");

    const back = Array.from(container.querySelectorAll("button")).find(
      (b) => b.textContent?.trim() === "Back to the explorer",
    );
    expect(back).toBeDefined();

    await act(async () => {
      back?.click();
    });

    // Now — and only now — the empty pane is the honest answer. This host has
    // no operator content, so that pane is the first-run one (issue #1481).
    expect(
      container.querySelector('[data-testid="workspace-missing-note"]'),
    ).toBeNull();
    expect(
      container.querySelector('[data-testid="workspace-empty-first-run"]'),
    ).not.toBeNull();
  });
});

describe("the pane answers on what was asked for, not on tree membership (issue #1473)", () => {
  it("keeps the ordinary open-a-note path intact", async () => {
    const tree = [
      { id: "note-1", name: "Plan.md", kind: "file", updatedAt: 1 },
    ];
    await render(host(tree), "note-1");

    expect(
      container.querySelector('[data-testid="workspace-missing-note"]'),
    ).toBeNull();
    expect(
      container.querySelector('[data-testid="workspace-note"]'),
    ).not.toBeNull();
  });

  it("shows a skeleton while a deep-linked id is still loading, not the idle state", async () => {
    // The skeleton lived inside the `openNode` branch too, so a deep link spent
    // the whole tree read looking like an idle pane and then changed its mind.
    let release: (nodes: unknown[]) => void = () => {};
    const pending = new Promise<unknown[]>((resolve) => {
      release = resolve;
    });
    const client = {
      scopeFor: () => "/api/v1/company/acme",
      get: vi.fn(async (path: string) => {
        if (path.includes("/workspace/file/")) throw new Error("not found");
        return pending;
      }),
    } as unknown as OpenCompanyClient;

    await render(client, "deleted-note");

    expect(
      container.querySelector('[data-testid="workspace-missing-loading"]'),
    ).not.toBeNull();
    expect(container.textContent).not.toContain(IDLE);

    await act(async () => {
      release([]);
      await pending;
    });

    expect(
      container.querySelector('[data-testid="workspace-missing-loading"]'),
    ).toBeNull();
    expect(container.textContent).toContain("no longer in this workspace");
  });

  it("shows the empty pane when nothing was asked for at all", async () => {
    await render(
      host([{ id: "note-1", name: "Plan.md", kind: "file", updatedAt: 1 }]),
    );

    expect(
      container.querySelector('[data-testid="workspace-missing-note"]'),
    ).toBeNull();
    expect(container.textContent).toContain("No note open");
  });
});
