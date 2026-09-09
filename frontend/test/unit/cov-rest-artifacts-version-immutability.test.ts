// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { ArtifactView } from "@/api/artifacts";
import { ArtifactsTab } from "@/views/ArtifactsTab";

/**
 * The Artifacts tab has no role gate at all — `POST …/artifacts/{id}/versions`
 * is `ScopedCompany` (any signed-in teammate), not admin-only — so the control
 * this axis actually turns on is version, not role: `PATCH`-a-version does not
 * exist, and the one control that could rewrite history —
 * "Edit as operator" — must appear only on the *latest* revision of a
 * text/markdown artifact and never on an older one, however it is reached.
 */

const ARTIFACT: ArtifactView = {
  id: "art-1",
  taskId: "task-1",
  title: "Launch brief",
  kind: "markdown",
  createdAtMillis: 1,
  updatedAtMillis: 2,
  versions: [
    { version: 1, body: "First draft.", author: "agent", authorId: "writer", createdAtMillis: 1 },
    { version: 2, body: "Second draft.", author: "agent", authorId: "writer", createdAtMillis: 2 },
  ],
};

function clientWith(artifact: ArtifactView): OpenCompanyClient {
  return {
    scopeFor: () => "/api/v1/company/acme",
    get: vi.fn(() => Promise.resolve([artifact])),
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;

async function show(artifact: ArtifactView, openVersion?: number) {
  const client = clientWith(artifact);
  await act(async () => {
    root.render(
      createElement(ArtifactsTab, {
        client,
        company: "acme",
        taskId: "task-1",
        openArtifactId: artifact.id,
        openVersion,
      }),
    );
  });
}

function editButton(): HTMLElement | null {
  return Array.from(container.querySelectorAll("button")).find(
    (b) => b.textContent?.trim() === "Edit as operator",
  ) ?? null;
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

describe("the artifact version surface offers Edit only on the latest revision", () => {
  it("offers no edit control while an older version is shown", async () => {
    await show(ARTIFACT, 1);

    // Deep-linked to v1, so this must show v1, not silently jump to latest.
    expect(container.textContent).toContain("First draft.");
  });

  it("withholds the edit control from a past revision, even though the kind is editable", async () => {
    await show(ARTIFACT, 1);
    expect(editButton()).toBeNull();
  });

  it("offers the edit control on the latest revision of the same artifact", async () => {
    await show(ARTIFACT, 2);
    expect(editButton()).not.toBeNull();
  });
});
