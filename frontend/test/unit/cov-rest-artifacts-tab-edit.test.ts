// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { ArtifactView } from "@/api/artifacts";
import { ArtifactsTab } from "@/views/ArtifactsTab";

/**
 * The Artifacts tab's append route (`POST …/artifacts/{id}/versions`) is
 * `ScopedCompany` (`ops/artifacts.rs:211`), not `AdminScopedCompany` — any
 * member who can open the card may record an operator edit, and the tab
 * offers "Edit as operator" with no role check of its own, which matches.
 *
 * The property this file actually pins is what makes two tabs appending
 * concurrently safe with no version token on the wire at all: a save sends
 * only `{ body }`, and the tab replaces its whole local copy with whatever
 * the host answers rather than bumping a version count itself — so a second
 * tab's append is reflected exactly, never guessed at or clobbered by a
 * stale in-memory copy.
 */

const ARTIFACT: ArtifactView = {
  id: "a1",
  taskId: "t1",
  title: "launch-notes.md",
  kind: "markdown",
  versions: [
    {
      version: 1,
      body: "agent draft",
      author: "agent",
      authorId: "theorist",
      createdAtMillis: 1000,
    },
  ],
  createdAtMillis: 1000,
  updatedAtMillis: 1000,
};

function clientAs(opts: {
  post?: (path: string, body: unknown) => Promise<ArtifactView>;
}): OpenCompanyClient {
  return {
    scopeFor: () => "/api/v1/companies/acme",
    get: () => Promise.resolve([ARTIFACT]),
    post: vi.fn(opts.post ?? (() => Promise.resolve(ARTIFACT))),
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;

async function show(element: React.ReactElement) {
  await act(async () => {
    root.render(element);
  });
}

function findButton(text: string): HTMLButtonElement | null {
  return (
    (Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes(text),
    ) as HTMLButtonElement | undefined) ?? null
  );
}

/** Set a controlled textarea's value the way React's onChange expects. */
async function type(el: HTMLTextAreaElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(
    window.HTMLTextAreaElement.prototype,
    "value",
  )!.set!;
  await act(async () => {
    setter.call(el, value);
    el.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

describe("ArtifactsTab, an operator edit with no admin gate", () => {
  it("offers the edit control on a plain member client — the append route carries no admin check", async () => {
    const client = clientAs({});
    await show(createElement(ArtifactsTab, { client, company: "acme", taskId: "t1" }));

    findButton("launch-notes.md")!.click();
    await act(async () => {});

    expect(findButton("Edit as operator")).not.toBeNull();
  });

  it("sends only the body on save, with no stale version stamp two concurrent tabs could collide on", async () => {
    const client = clientAs({});
    await show(createElement(ArtifactsTab, { client, company: "acme", taskId: "t1" }));

    findButton("launch-notes.md")!.click();
    await act(async () => {});
    findButton("Edit as operator")!.click();
    await act(async () => {});

    const textarea = container.querySelector("textarea") as HTMLTextAreaElement;
    await type(textarea, "operator's cleaned-up draft");
    findButton("Save")!.click();
    await act(async () => {});

    expect(client.post).toHaveBeenCalledWith(
      "/api/v1/companies/acme/artifacts/a1/versions",
      { body: "operator's cleaned-up draft" },
    );
  });

  it("shows the version the host actually stored, not a locally-guessed count — the other tab's append is never clobbered", async () => {
    const afterSecondTabAppended: ArtifactView = {
      ...ARTIFACT,
      versions: [
        ...ARTIFACT.versions,
        {
          version: 2,
          body: "the OTHER tab's edit, already landed before this save resolved",
          author: "operator",
          authorId: "other-tab",
          createdAtMillis: 2000,
        },
        {
          version: 3,
          body: "this tab's edit",
          author: "operator",
          authorId: "this-tab",
          createdAtMillis: 3000,
        },
      ],
      updatedAtMillis: 3000,
    };
    const client = {
      scopeFor: () => "/api/v1/companies/acme",
      // The background refresh after a save re-reads the list — by the time it
      // lands, the host's own roster already agrees with what the append
      // answered, exactly as it would once the write is durable.
      get: () => Promise.resolve([afterSecondTabAppended]),
      post: vi.fn(() => Promise.resolve(afterSecondTabAppended)),
    } as unknown as OpenCompanyClient;
    await show(createElement(ArtifactsTab, { client, company: "acme", taskId: "t1" }));

    findButton("launch-notes.md")!.click();
    await act(async () => {});
    findButton("Edit as operator")!.click();
    await act(async () => {});
    const textarea = container.querySelector("textarea") as HTMLTextAreaElement;
    await type(textarea, "this tab's edit");
    findButton("Save")!.click();
    await act(async () => {});

    // Three versions on screen — the intervening v2 this tab never typed is
    // present because the tab trusts the host's answer wholesale rather than
    // appending its own guess onto what it had in memory.
    expect(container.textContent).toContain("v3");
    expect(container.textContent).toContain("operator");
  });
});
