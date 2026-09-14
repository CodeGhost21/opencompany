// @vitest-environment jsdom

import { MessageChannel, MessagePort } from "node:worker_threads";
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { PageManifestDto } from "@/api/types";
import { PagesView } from "@/views/PagesView";

/**
 * Two gaps in `PagesView`, unrelated to the postMessage-bridge property
 * `pages-view-bridge.test.ts` already pins.
 *
 * PAGE-001: `page.toml`'s `nav_visible = false` is meant to keep a page off
 * the sidebar and reachable only by a direct address — nothing in this file
 * gives it one, so the property actually enforceable here is the other half:
 * it must never be discoverable through the sidebar list, nor become the
 * view's own default selection.
 *
 * PAGE-002: every `oc:graphql` request crosses the bridge under the
 * operator's own full session (`server/graphql/mod.rs`) with no per-page
 * scope narrowing — a backend fact this file cannot test. What it can pin is
 * the FAIL half: a request the host refuses must answer the waiting page with
 * an honest `errors` array over the same port, never silence that leaves its
 * promise hanging.
 */

Object.assign(globalThis, { MessageChannel, MessagePort });

function page(over: Partial<PageManifestDto> = {}): PageManifestDto {
  return { slug: "metrics", title: "Metrics", navVisible: true, ...over };
}

function clientWith(pages: PageManifestDto[], graphqlRequest?: OpenCompanyClient["graphqlRequest"]): OpenCompanyClient {
  return {
    listPages: () => Promise.resolve(pages),
    pageUrl: (slug: string) => `/api/v1/company/pages/${slug}`,
    graphqlRequest: graphqlRequest ?? (() => Promise.resolve({ data: {}, errors: undefined })),
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;

async function show(client: OpenCompanyClient, company = "acme") {
  await act(async () => {
    root.render(createElement(PagesView, { client, company }));
  });
}

function listItems(): HTMLElement[] {
  return Array.from(container.querySelectorAll('[data-testid="pages-list-item"]'));
}

function iframe(): HTMLIFrameElement | null {
  return container.querySelector("iframe");
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

describe("a nav_visible:false page is never surfaced through the console's own nav", () => {
  it("lists only the nav-visible page in the sidebar, even when the host lists the hidden one first", async () => {
    const client = clientWith([
      page({ slug: "internal-audit", title: "Internal Audit", navVisible: false }),
      page({ slug: "metrics", title: "Metrics", navVisible: true }),
    ]);
    await show(client);

    const titles = listItems().map((el) => el.textContent);
    expect(titles.some((t) => t?.includes("Metrics"))).toBe(true);
    expect(titles.some((t) => t?.includes("Internal Audit"))).toBe(false);
  });

  it("never defaults the open document onto the hidden page, even though the host listed it first", async () => {
    const client = clientWith([
      page({ slug: "internal-audit", title: "Internal Audit", navVisible: false }),
      page({ slug: "metrics", title: "Metrics", navVisible: true }),
    ]);
    await show(client);

    expect(iframe()?.getAttribute("src")).toBe("/api/v1/company/pages/metrics");
  });
});

describe("a rejected bridged GraphQL request answers the page honestly", () => {
  it("posts an errors array back over the port instead of leaving the request hanging", async () => {
    const graphqlRequest = vi.fn().mockRejectedValue(new Error("company host refused the request"));
    const client = clientWith([page()], graphqlRequest);
    await show(client);

    const frame = iframe()!;
    const contentWindow = frame.contentWindow as Window;
    const postMessage = vi.spyOn(contentWindow, "postMessage").mockImplementation(() => {});
    await act(async () => {
      frame.dispatchEvent(new Event("load"));
    });
    const init = postMessage.mock.calls.find(
      ([msg]) => (msg as { type?: string })?.type === "oc:init",
    ) as unknown as [message: { capability?: string }, origin: string, transfer: MessagePort[]];
    const capability = init[0].capability!;
    const pagePort = init[2][0];

    const reply = new Promise((resolve) => {
      pagePort.addEventListener("message", (e) => resolve((e as { data?: unknown }).data), {
        once: true,
      });
    });
    await act(async () => {
      pagePort.postMessage({ type: "oc:graphql", id: "q1", query: "{ ping }", capability });
      await new Promise((r) => setTimeout(r, 0));
      await Promise.resolve();
      await Promise.resolve();
    });

    const data = (await reply) as { type: string; id: string; errors?: { message: string }[] };
    expect(data.type).toBe("oc:graphql:result");
    expect(data.id).toBe("q1");
    expect(data.errors?.[0]?.message).toContain("company host refused the request");
  });
});
