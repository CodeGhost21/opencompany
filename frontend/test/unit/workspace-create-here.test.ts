// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { ConnectionScopeProvider } from "@/connections/ConnectionContext";
import { WorkspaceView } from "@/views/WorkspaceView";

/**
 * Issue #1477: everything an operator created landed at the root.
 *
 * `createAndOpen`, `createFolder` and the upload all passed `parentId: null`,
 * the New-note and New-folder entry points were toolbar-only, and the name
 * prompt said nothing about a destination. So an operator standing in
 * `Standards/Engineering/` made a note and it appeared at the root, unannounced
 * — and the only way to file it was the Move dialog, which was itself unusable
 * (#1381).
 */

function node(over: {
  id: string;
  name: string;
  kind: "folder" | "file";
  parentId?: string;
}) {
  return { ...over, updatedAt: 1 };
}

const TREE = [
  node({ id: "standards", name: "Standards", kind: "folder" }),
  node({
    id: "eng",
    name: "Engineering",
    kind: "folder",
    parentId: "standards",
  }),
  node({ id: "note", name: "Plan.md", kind: "file", parentId: "eng" }),
  node({ id: "derived", name: "derived", kind: "folder" }),
  node({ id: "goals", name: "GOALS.md", kind: "file", parentId: "derived" }),
];

let container: HTMLDivElement;
let root: Root;
let client: {
  scopeFor: () => string;
  get: ReturnType<typeof vi.fn>;
  post: ReturnType<typeof vi.fn>;
};

function host() {
  return {
    scopeFor: () => "/api/v1/company/acme",
    get: vi.fn(async (path: string) => {
      if (path.includes("/workspace/file/")) {
        return {
          id: "note",
          name: "Plan.md",
          content: "",
          backlinks: [],
          updatedAt: 1,
        };
      }
      return TREE;
    }),
    post: vi.fn(async (_path: string, body: { name: string; kind: string }) =>
      node({
        id: "new",
        name: body.name,
        kind: body.kind as "file" | "folder",
      }),
    ),
  };
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

async function render() {
  client = host();
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
}

function menuItem(label: string): HTMLElement {
  const found = Array.from(document.querySelectorAll('[role="menuitem"]')).find(
    (el) => el.textContent?.trim() === label,
  );
  if (!found)
    throw new Error(`no “${label}” menu item in:\n${document.body.innerHTML}`);
  return found as HTMLElement;
}

function actionsButtonFor(name: string): HTMLButtonElement {
  const row = Array.from(container.querySelectorAll("div.group")).find((d) =>
    d.textContent?.includes(name),
  );
  const found = row?.querySelector('[aria-label="Actions"]');
  if (!found) throw new Error(`no Actions button for “${name}”`);
  return found as HTMLButtonElement;
}

/**
 * Expand down to the seeded note. The tree opens the root's own children, so
 * only the second level needs a click.
 */
async function expandToNote() {
  for (const folder of ["Engineering"]) {
    const row = Array.from(container.querySelectorAll("button")).find(
      (b) => b.textContent?.trim() === folder,
    ) as HTMLButtonElement | undefined;
    if (!row) continue;
    await act(async () => {
      row.click();
    });
  }
}

async function submitName(name: string) {
  const input = document.querySelector("#fs-name") as HTMLInputElement;
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )?.set;
    setter?.call(input, name);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => {
    (
      Array.from(document.querySelectorAll("button")).find(
        (b) => b.textContent?.trim() === "Create",
      ) as HTMLButtonElement
    ).click();
  });
}

/** The parentId the create request actually carried. */
function createdParent(): string | null | undefined {
  const call = client.post.mock.calls.find((c) =>
    (c[0] as string).endsWith("/workspace"),
  );
  return (call?.[1] as { parentId?: string | null } | undefined)?.parentId;
}

describe("a note is created where the operator is (issue #1477)", () => {
  it("lands beside the open note, not at the root", async () => {
    await render();
    await expandToNote();
    await act(async () => {
      (
        Array.from(container.querySelectorAll("button")).find((b) =>
          b.textContent?.includes("Plan"),
        ) as HTMLButtonElement
      ).click();
    });

    await act(async () => {
      (
        container.querySelector('[aria-label="New file"]') as HTMLButtonElement
      ).click();
    });
    // Said before it happens, not discovered afterwards.
    expect(
      document.querySelector('[data-testid="workspace-prompt-dest"]')
        ?.textContent,
    ).toContain("Standards / Engineering");

    await submitName("Runbook");
    // The reported defect: this was always null.
    expect(createdParent()).toBe("eng");
  });

  it("falls back to the root when nothing is open", async () => {
    await render();
    await act(async () => {
      (
        container.querySelector('[aria-label="New file"]') as HTMLButtonElement
      ).click();
    });

    expect(
      document.querySelector('[data-testid="workspace-prompt-dest"]')
        ?.textContent,
    ).toContain("the workspace root");
    await submitName("Runbook");
    expect(createdParent()).toBe(null);
  });

  it("never inherits derived/, where the host refuses every write", async () => {
    await render();
    await act(async () => {
      (
        Array.from(container.querySelectorAll("button")).find((b) =>
          b.textContent?.includes("GOALS"),
        ) as HTMLButtonElement
      ).click();
    });

    await act(async () => {
      (
        container.querySelector('[aria-label="New file"]') as HTMLButtonElement
      ).click();
    });
    await submitName("Runbook");

    // Inheriting the open ledger file's folder would turn every create into an
    // error toast.
    expect(createdParent()).toBe(null);
  });
});

describe("the tree can make a note where you are pointing (issue #1477)", () => {
  it("offers New note here on a folder row and files it there", async () => {
    await render();
    await expandToNote();
    await act(async () => {
      actionsButtonFor("Engineering").click();
    });
    await act(async () => {
      menuItem("New note here").click();
    });

    expect(
      document.querySelector('[data-testid="workspace-prompt-dest"]')
        ?.textContent,
    ).toContain("Standards / Engineering");
    await submitName("Runbook");
    expect(createdParent()).toBe("eng");
  });

  it("offers New folder here too", async () => {
    await render();
    await act(async () => {
      actionsButtonFor("Standards").click();
    });
    await act(async () => {
      menuItem("New folder here").click();
    });
    await submitName("Specs");

    expect(createdParent()).toBe("standards");
    const call = client.post.mock.calls.find((c) =>
      (c[0] as string).endsWith("/workspace"),
    );
    expect((call?.[1] as { kind: string }).kind).toBe("folder");
  });

  it("does not offer it on a note — a note is not a destination", async () => {
    await render();
    await expandToNote();
    await act(async () => {
      actionsButtonFor("Plan").click();
    });

    expect(
      Array.from(document.querySelectorAll('[role="menuitem"]')).map((m) =>
        m.textContent?.trim(),
      ),
    ).not.toContain("New note here");
  });
});
