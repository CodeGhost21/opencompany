// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { ConnectionScopeProvider } from "@/connections/ConnectionContext";
import { WorkspaceView } from "@/views/WorkspaceView";

/**
 * PR #1498 review (CodeRabbit, thread 3829432151): the repair dialog's
 * residual rows are supposed to say where each one lives (issue #1469's own
 * requirement — "each one below says what it needs from you"). `pathOf` walks
 * zero folders for a root-level residual (`parentId === null`), so the joined
 * `where` string came out empty, and `{where && (...)}` read that as "say
 * nothing" — the one row with the shortest possible answer ("Workspace root")
 * was the one that went silent.
 */

const ROOT_DUPE_A = "root-dupe-a";
const ROOT_DUPE_B = "root-dupe-b";
const NESTED_DUPE = "nested-dupe";
const FOLDER = "notes";

const TREE = [
  { id: ROOT_DUPE_A, name: "Notes.md", kind: "file", parentId: null, updatedAt: 1 },
  { id: ROOT_DUPE_B, name: "Notes", kind: "folder", parentId: null, updatedAt: 1 },
  { id: FOLDER, name: "Notes", kind: "folder", parentId: null, updatedAt: 1 },
  { id: NESTED_DUPE, name: "Draft.md", kind: "file", parentId: FOLDER, updatedAt: 1 },
];

/** One root-level residual pair, plus a nested one as a same-row control. */
const RESIDUAL_ONLY = {
  residuals: [
    { id: ROOT_DUPE_A, name: "Notes.md", parentId: null, cause: "fileSharesTheName" },
    { id: NESTED_DUPE, name: "Draft.md", parentId: FOLDER, cause: "fileSharesTheName" },
  ],
};

function host() {
  return {
    scopeFor: () => "/api/v1/company/acme",
    get: vi.fn().mockResolvedValue(TREE),
    post: vi.fn().mockResolvedValue(RESIDUAL_ONLY),
    patch: vi.fn(),
    del: vi.fn(),
  };
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
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

async function openRepair() {
  const client = host();
  await act(async () => {
    root.render(
      createElement(ConnectionScopeProvider, {
        scope: { connection: "c1", company: "acme" },
        children: createElement(WorkspaceView, {
          client: client as unknown as OpenCompanyClient,
          company: "acme",
        }),
      }),
    );
  });
  await act(async () => {
    (container.querySelector('[data-testid="workspace-repair"]') as HTMLButtonElement).click();
  });
}

function rows(): HTMLElement[] {
  return Array.from(
    document.querySelectorAll('[data-testid="workspace-repair-residual"]'),
  ) as HTMLElement[];
}

describe("every residual row says where it lives, root-level included (#1498 review)", () => {
  it("labels a root-level residual 'Workspace root' instead of omitting the location", async () => {
    await openRepair();
    const [rootRow] = rows();

    expect(rootRow.textContent).toContain("Notes.md");
    expect(rootRow.textContent).toContain("in Workspace root");
  });

  it("still says the real folder name for a nested residual", async () => {
    await openRepair();
    const [, nestedRow] = rows();

    expect(nestedRow.textContent).toContain("in Notes");
    expect(nestedRow.textContent).not.toContain("Workspace root");
  });
});
