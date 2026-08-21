// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { ConnectionScopeProvider } from "@/connections/ConnectionContext";
import { WorkspaceView } from "@/views/WorkspaceView";

/**
 * Issue #1382 — the density and consistency nits from the design audit.
 *
 * The header showed a bare relative time beside the title, which read like part
 * of it. The backlinks rail is `xl:flex`, so below about 1280px a note with
 * eleven backlinks and one with none looked identical — the signal vanished
 * rather than degrading. And `setExpanded(new Set())` already existed with
 * nothing invoking it, so a deep tree stayed open behind every reveal with no
 * way back short of collapsing each folder by hand.
 */

function node(over: {
  id: string;
  name: string;
  kind: "folder" | "file";
  parentId?: string;
}) {
  return { ...over, updatedAt: 1 };
}

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

describe("the density nits (issue #1382)", () => {
  it("labels the timestamp rather than leaving a bare relative time by the title", async () => {
    await render([node({ id: "n1", name: "Plan.md", kind: "file" })]);
    await act(async () => {
      (
        Array.from(container.querySelectorAll("button")).find((b) =>
          b.textContent?.includes("Plan"),
        ) as HTMLButtonElement
      ).click();
    });

    expect(
      container.querySelector('[data-testid="workspace-updated"]')?.textContent,
    ).toMatch(/^Edited /);
  });

  it("keeps Collapse all disabled until something is expanded", async () => {
    await render([]);

    expect(
      (
        container.querySelector(
          '[data-testid="workspace-collapse-all"]',
        ) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
  });

  it("collapses every open folder in one press", async () => {
    await render([
      node({ id: "a", name: "Alpha", kind: "folder" }),
      node({ id: "b", name: "Beta", kind: "folder", parentId: "a" }),
    ]);

    // The tree opens the root's own folders on load, so Beta is on screen and
    // the control is live before anything is clicked.
    expect(
      Array.from(container.querySelectorAll("button")).some(
        (btn) => btn.textContent?.trim() === "Beta",
      ),
    ).toBe(true);
    const collapse = container.querySelector(
      '[data-testid="workspace-collapse-all"]',
    ) as HTMLButtonElement;
    expect(collapse.disabled).toBe(false);

    await act(async () => {
      collapse.click();
    });

    // Alpha's child is gone with it, and the control has nothing left to do.
    expect(
      Array.from(container.querySelectorAll("button")).some(
        (btn) => btn.textContent?.trim() === "Beta",
      ),
    ).toBe(false);
    expect(
      (
        container.querySelector(
          '[data-testid="workspace-collapse-all"]',
        ) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
  });
});
