// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { ConnectionScopeProvider } from "@/connections/ConnectionContext";
import { WorkspaceView } from "@/views/WorkspaceView";

/**
 * `writeFile` (`PUT …/workspace/file/{id}`) is what the host's
 * `workspace().write` guard sits behind — `derived/` and a handful of other
 * paths refuse it outright. `lib/workspace-save-buffer.ts` unit-tests the
 * buffer's own bookkeeping in isolation; nothing before this proved that a
 * refused write actually reaches the operator's eye through the mounted
 * editor rather than reading as a silent "Saved".
 */

function node(over: { id: string; name: string; kind: "folder" | "file"; parentId?: string }) {
  return { ...over, updatedAt: 1 };
}

const TREE = [node({ id: "note", name: "Plan.md", kind: "file" })];

function clientWith(put: () => Promise<unknown>): OpenCompanyClient {
  return {
    scopeFor: () => "/api/v1/company/acme",
    get: vi.fn((path: string) => {
      if (path.endsWith("/workspace")) return Promise.resolve(TREE);
      if (path.includes("/workspace/file/")) {
        return Promise.resolve({
          id: "note",
          name: "Plan.md",
          content: "original text",
          updatedAt: 1,
          backlinks: [],
        });
      }
      return Promise.reject(new Error(`unexpected GET ${path}`));
    }),
    put,
    listTeam: vi.fn(() => Promise.resolve([])),
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;

async function show(client: OpenCompanyClient) {
  await act(async () => {
    root.render(
      createElement(ConnectionScopeProvider, {
        scope: { connection: "c1", company: "acme" },
        children: createElement(WorkspaceView, { client, company: "acme" }),
      }),
    );
  });
}

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
  localStorage.clear();
});

describe("a write the host refuses shows an honest error, never a false Saved", () => {
  it("says the note was not saved once the host refuses the PUT", async () => {
    const put = vi.fn(() => Promise.reject(new Error("write refused: read-only path")));
    const client = clientWith(put);
    await show(client);

    await act(async () => {
      const row = Array.from(container.querySelectorAll("div.group")).find((d) =>
        d.textContent?.includes("Plan"),
      ) as HTMLElement;
      (row.querySelector("button") as HTMLButtonElement).click();
    });
    await act(async () => {
      (Array.from(container.querySelectorAll("button")).find(
        (b) => b.textContent?.trim() === "Edit",
      ) as HTMLButtonElement).click();
    });
    const editor = container.querySelector('[data-testid="workspace-editor"]') as HTMLTextAreaElement;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set;
      setter?.call(editor, "original text, plus an edit");
      editor.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => {
      editor.dispatchEvent(new FocusEvent("focusout", { bubbles: true }));
      await new Promise((r) => setTimeout(r, 0));
      await Promise.resolve();
    });

    expect(put).toHaveBeenCalled();
    const status = container.querySelector('[data-testid="workspace-save-state"]');
    expect(status?.textContent).toContain("Not saved");
    expect(status?.textContent).not.toBe("Saved");
  });
});
