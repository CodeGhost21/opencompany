// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { ConnectionScopeProvider } from "@/connections/ConnectionContext";
import { WorkspaceView } from "@/views/WorkspaceView";

/**
 * PR #1498 review (CodeRabbit, thread 3829432141): `RepairDialog.onReveal` is
 * documented "Show a residual in the tree. Never writes — it expands and
 * scrolls." — but a *file* residual fell through to the ordinary `open()`
 * route, and `open()` starts with `await flush()`. If the editor held a
 * staged draft when the operator clicked "show me" on a file residual, that
 * draft got written to the host as a side effect of a control promised as
 * reveal-only.
 *
 * This pins the fix at the one place that could actually observe it: with a
 * *different* note open and dirty, revealing a file residual must not touch
 * the network, and the dirty note's draft must still read as unsaved
 * afterward — not "saving" or "saved" out from under the operator.
 */

const ENG = "eng";
const SPECS_FOLDER = "specs-folder";
const SPECS_NOTE = "specs-note";
const OPEN_NOTE = "note-1";

const TREE = [
  { id: ENG, name: "Engineering", kind: "folder", updatedAt: 1 },
  { id: SPECS_FOLDER, name: "Specs", kind: "folder", parentId: ENG, updatedAt: 1 },
  { id: SPECS_NOTE, name: "Specs", kind: "file", parentId: ENG, updatedAt: 1 },
  { id: OPEN_NOTE, name: "Notes.md", kind: "file", updatedAt: 1 },
];

/** The repair preview: a residual-only outcome naming both the folder and the file above. */
const RESIDUAL_ONLY = {
  residuals: [
    { id: SPECS_FOLDER, name: "Specs", parentId: ENG, cause: "fileSharesTheName" },
    { id: SPECS_NOTE, name: "Specs", parentId: ENG, cause: "fileSharesTheName" },
  ],
};

/** The one host each test drives, so its write methods can be asserted on. */
function host() {
  return {
    scopeFor: () => "/api/v1/company/acme",
    get: vi.fn(async (path: string) => {
      if (path.includes("/workspace/file/")) {
        return {
          id: OPEN_NOTE,
          name: "Notes.md",
          content: "saved body",
          backlinks: [],
          updatedAt: 1,
        };
      }
      return TREE;
    }),
    post: vi.fn().mockResolvedValue(RESIDUAL_ONLY),
    patch: vi.fn(),
    del: vi.fn(),
    // The write route a residual reveal must never reach (`writeFile` calls
    // `client.put`) — asserted on directly rather than inferred from state.
    put: vi.fn().mockResolvedValue({ updatedAt: 2 }),
  };
}

let client: ReturnType<typeof host>;
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

function button(label: string): HTMLButtonElement {
  const match = Array.from(container.querySelectorAll("button")).find(
    (b) => b.textContent?.trim() === label,
  );
  expect(match, `no button labeled "${label}"`).toBeTruthy();
  return match as HTMLButtonElement;
}

function saveStateAttr(): string | null {
  return document.querySelector('[data-testid="workspace-save-state"]')?.getAttribute("data-state") ?? null;
}

function residualRows(): HTMLElement[] {
  return Array.from(
    document.querySelectorAll('[data-testid="workspace-repair-residual"]'),
  ) as HTMLElement[];
}

/**
 * Open `Notes.md`, switch it to Edit, and type a paragraph the host has never
 * seen — the exact "staged draft" precondition the finding names — then open
 * the repair dialog on top of it without letting the autosave debounce (800ms,
 * real timers here) ever fire.
 */
async function openRepairWithADirtyNoteBehindIt() {
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

  await act(async () => {
    button("Notes").click();
  });

  await act(async () => {
    (
      Array.from(document.querySelectorAll('[role="tab"]')).find(
        (t) => t.textContent?.trim() === "Edit",
      ) as HTMLElement
    ).click();
  });

  const editor = document.querySelector('[data-testid="workspace-editor"]') as HTMLTextAreaElement;
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set;
    setter?.call(editor, "a paragraph the host has never seen");
    editor.dispatchEvent(new Event("input", { bubbles: true }));
  });
  expect(saveStateAttr()).toBe("dirty");

  await act(async () => {
    (container.querySelector('[data-testid="workspace-repair"]') as HTMLButtonElement).click();
  });
}

describe("a residual reveal never flushes a dirty draft (PR #1498 review)", () => {
  it("does not call the write route when the residual is a file", async () => {
    await openRepairWithADirtyNoteBehindIt();
    const rows = residualRows();
    expect(rows).toHaveLength(2);

    // rows()[1] is SPECS_NOTE — the file residual, and the one `open()` used
    // to swallow. rows()[0] (the folder) already went through `revealFolder`,
    // which never wrote.
    await act(async () => {
      rows[1].click();
    });

    expect(client.put).not.toHaveBeenCalled();
    expect(client.patch).not.toHaveBeenCalled();
    expect(client.del).not.toHaveBeenCalled();
  });

  it("leaves the open note's draft reading as unsaved, not saving or saved", async () => {
    await openRepairWithADirtyNoteBehindIt();
    const rows = residualRows();

    await act(async () => {
      rows[1].click();
    });

    // If the reveal had gone through `open()` -> `flush()`, this would read
    // "saving" (or "saved", once the mocked `put` resolved) instead — the
    // operator's still-open, still-unsaved note would look like it had been
    // written out from under them by a click that named a different file.
    expect(saveStateAttr()).toBe("dirty");
  });

  it("still does the reveal: closes the dialog and does not disturb the open note", async () => {
    await openRepairWithADirtyNoteBehindIt();
    const rows = residualRows();

    await act(async () => {
      rows[1].click();
    });

    expect(document.querySelector('[data-testid="workspace-repair-residual"]')).toBeNull();
    // The open note is still Notes.md, still in Edit, with the same unwritten
    // text — the reveal changed the tree's selection, not the open pane.
    const editor = document.querySelector('[data-testid="workspace-editor"]') as HTMLTextAreaElement;
    expect(editor).not.toBeNull();
    expect(editor.value).toBe("a paragraph the host has never seen");
  });
});
