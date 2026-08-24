// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { InboxView } from "@/views/InboxView";

/**
 * The inbox's empty state has no teammate id to deep-link to. Its action must
 * therefore reach the Company roster, which can open each teammate's detail
 * page and its inbox switch (issue #1331).
 */
let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function client(): OpenCompanyClient {
  return {
    listInboxes: vi.fn().mockResolvedValue([]),
  } as unknown as OpenCompanyClient;
}

describe("the empty inbox state", () => {
  it("links operators to the Company roster instead of the retired Team page", async () => {
    await act(async () => {
      root.render(createElement(InboxView, { client: client(), company: "acme" }));
    });
    await act(async () => {});

    const link = [...container.querySelectorAll<HTMLAnchorElement>("a")].find(
      (anchor) => anchor.textContent === "Company page",
    );
    expect(link?.getAttribute("href")).toBe("#/company");
    expect(container.textContent).toContain("open a teammate to flip on the inbox toggle");
    expect(container.textContent).not.toContain("Team page");
  });
});
