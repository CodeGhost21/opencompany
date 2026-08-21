// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { TeamMemberDto } from "@/api/types";
import { ConnectionScopeProvider } from "@/connections/ConnectionContext";
import { WorkspaceView } from "@/views/WorkspaceView";

/**
 * Issue #1459: a truncated tree name was unrecoverable.
 *
 * The row is `truncate` inside a fixed 256px pane, indented 12px per level, so
 * a depth-5 note has about 22 characters of room — and the seeded tree
 * ellipsises six rows out of the box. `title` was set for exactly one case, a
 * roster folder showing a *substituted* name, and `undefined` for everything
 * else. No wrap, no row scroll, no resize handle: the only way to read a name
 * was to open a row you could not identify.
 */

function member(id: string, name: string): TeamMemberDto {
  return { id, name, role: name };
}

const LONG =
  "A note with a deliberately very long file name that will not fit.md";

function host(team: TeamMemberDto[] = []): OpenCompanyClient {
  return {
    scopeFor: () => "/api/v1/company/acme",
    get: vi.fn().mockResolvedValue([
      { id: "n1", name: LONG, kind: "file", updatedAt: 1 },
      { id: "agents", name: "Agents", kind: "folder", updatedAt: 1 },
      {
        id: "roster",
        name: "01JQZY8T7K",
        kind: "folder",
        parentId: "agents",
        updatedAt: 1,
      },
    ]),
    listTeam: vi.fn().mockResolvedValue(team),
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

async function render(team: TeamMemberDto[] = []) {
  await act(async () => {
    root.render(
      createElement(ConnectionScopeProvider, {
        scope: { connection: "c1", company: "acme" },
        children: createElement(WorkspaceView, {
          client: host(team),
          company: "acme",
        }),
      }),
    );
  });
}

function names(): HTMLElement[] {
  return Array.from(
    container.querySelectorAll('[data-testid="workspace-tree-name"]'),
  ) as HTMLElement[];
}

describe("a clipped tree name can still be read (issue #1459)", () => {
  it("carries its full name on every row, not just on a substituted roster one", async () => {
    await render();

    const note = names().find((n) => n.textContent?.startsWith("A note with"));
    expect(note).toBeDefined();
    // The defect: `title` was `undefined` for every row but one.
    expect(note?.getAttribute("title")).toBe(
      "A note with a deliberately very long file name that will not fit",
    );
  });

  it("wires the name to a tooltip so a pointer meets it too", async () => {
    await render();
    for (const name of names()) {
      expect(name.getAttribute("data-slot")).toBe("tooltip-trigger");
    }
  });

  it("keeps the roster folder's raw id reachable, as #973 left it", async () => {
    await render([member("01JQZY8T7K", "Nadia")]);

    const folder = names().find((n) => n.textContent === "Nadia");
    expect(folder).toBeDefined();
    // The substitution shows the name; the id is still one hover away.
    expect(folder?.getAttribute("title")).toBe("01JQZY8T7K");
  });
});
