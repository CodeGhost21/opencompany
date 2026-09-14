// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "@/api/types";
import type { OpenCompanyClient } from "@/api/client";
import type { PageManifestDto } from "@/api/types";
import { PagesView } from "@/views/PagesView";

/**
 * The Pages list read itself failing — `client.listPages` rejecting, not the
 * bridged GraphQL request `cov-rest-pages-view.test.ts` covers.
 *
 * The view holds three load states and the failing one has to be reached: a
 * host that cannot serve pages must say so and offer a retry that really
 * re-reads, rather than leaving the four skeleton rows up forever or falling
 * through to the empty-state copy, which reads as "this company has no pages"
 * — a different, false claim.
 */

function page(over: Partial<PageManifestDto> = {}): PageManifestDto {
  return { slug: "metrics", title: "Metrics", navVisible: true, ...over };
}

let container: HTMLDivElement;
let root: Root;

async function show(client: OpenCompanyClient) {
  await act(async () => {
    root.render(createElement(PagesView, { client, company: "acme" }));
  });
}

function text(): string {
  return container.textContent ?? "";
}

function button(label: string): HTMLButtonElement | undefined {
  return Array.from(container.querySelectorAll("button")).find((b) => b.textContent?.includes(label));
}

function listItems(): HTMLElement[] {
  return Array.from(container.querySelectorAll('[data-testid="pages-list-item"]'));
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

describe("PagesView, a page-list read the host never answers", () => {
  it("names the failure and offers a retry, rather than an empty list or a permanent skeleton", async () => {
    const client = {
      listPages: () => Promise.reject(new ApiError(0, "network_error", "cannot reach the company host")),
      pageUrl: (slug: string) => `/api/v1/company/pages/${slug}`,
    } as unknown as OpenCompanyClient;

    await show(client);

    expect(text()).toContain("Pages unavailable");
    expect(text()).toContain("cannot reach the company host");
    expect(text()).not.toContain("No pages");
    expect(listItems()).toHaveLength(0);
    expect(button("Try again")).toBeDefined();
  });

  it("says a refused read was refused, in the host's own words", async () => {
    const client = {
      listPages: () => Promise.reject(new ApiError(403, "forbidden", "only an admin can do that", true)),
      pageUrl: (slug: string) => `/api/v1/company/pages/${slug}`,
    } as unknown as OpenCompanyClient;

    await show(client);

    expect(text()).toContain("Pages unavailable");
    expect(text()).toContain("only an admin can do that");
  });

  it("recovers the real list once the retry lands", async () => {
    const listPages = vi
      .fn()
      .mockRejectedValueOnce(new ApiError(500, "server_error", "pages index is rebuilding", true))
      .mockResolvedValueOnce([page()]);
    const client = {
      listPages,
      pageUrl: (slug: string) => `/api/v1/company/pages/${slug}`,
      graphqlRequest: () => Promise.resolve({ data: {}, errors: undefined }),
    } as unknown as OpenCompanyClient;

    await show(client);
    expect(text()).toContain("pages index is rebuilding");

    await act(async () => {
      button("Try again")?.click();
    });

    expect(text()).not.toContain("Pages unavailable");
    expect(listItems().map((el) => el.textContent).join(" ")).toContain("Metrics");
    expect(listPages).toHaveBeenCalledTimes(2);
  });
});
