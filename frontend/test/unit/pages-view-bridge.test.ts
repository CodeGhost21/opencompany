// @vitest-environment jsdom

import { MessageChannel, MessagePort } from "node:worker_threads";
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { PageManifestDto } from "@/api/types";
import { PagesView } from "@/views/PagesView";

/**
 * `PagesView`'s postMessage bridge is the actual security boundary between an
 * agent-authored page and the console's real GraphQL endpoint
 * (docs/spec/runtime/pages.md §6): the console transfers one half of a
 * `MessageChannel` to the loaded iframe document and only ever answers
 * `oc:graphql` requests that arrive on the other half. The port is
 * document-bound — a document the page navigates itself to never receives it —
 * so possession of the port, not a window-message `source` check, is what
 * tells the console this request really came from its own embedded page. This
 * is the one piece of that view worth a unit test — everything else is either
 * a plain fetch-and-render list or the iframe element itself, which needs a
 * real browser to say anything about.
 */

// jsdom does not implement MessageChannel/MessagePort (jsdom#2738), and the
// bridge depends on one. Node's implementation runs on the same event loop as
// the test, which is exactly what lets a test post on the page's half of the
// channel and observe the console's reply on that same half.
Object.assign(globalThis, { MessageChannel, MessagePort });

const PAGE: PageManifestDto = {
  slug: "metrics",
  title: "Metrics",
  description: "The daily numbers.",
  icon: "chart",
  navVisible: true,
};

function clientWith(graphqlRequest: OpenCompanyClient["graphqlRequest"]): OpenCompanyClient {
  return {
    listPages: () => Promise.resolve([PAGE]),
    pageUrl: (slug: string) => `/api/v1/company/pages/${slug}`,
    graphqlRequest,
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;

async function show(client: OpenCompanyClient) {
  await act(async () => {
    root.render(createElement(PagesView, { client, company: "acme" }));
  });
}

function iframe(): HTMLIFrameElement | null {
  return container.querySelector("iframe");
}

/** Fires the iframe `load` handler, minting the current document's bridge. */
function loadFrame(frame: HTMLIFrameElement): void {
  frame.dispatchEvent(new Event("load"));
}

/**
 * Returns the capability and the page-side port the view hands to the loaded
 * iframe document's first `oc:init` message. The mock swallows the transfer,
 * which is what keeps the page-side port usable by the test itself.
 */
function mintBridge(frame: HTMLIFrameElement): { capability: string; port: MessagePort } {
  const contentWindow = frame.contentWindow as Window;
  const postMessage = vi.spyOn(contentWindow, "postMessage").mockImplementation(() => {});
  loadFrame(frame);
  const init = postMessage.mock.calls.find(
    ([msg]) => (msg as { type?: string })?.type === "oc:init",
  );
  const capability = (init?.[0] as { capability: string } | undefined)?.capability;
  const port = (init?.[2] as MessagePort[] | undefined)?.[0];
  if (!capability || !port) {
    throw new Error("oc:init did not carry a capability and a transferred port");
  }
  return { capability, port };
}

/**
 * Resolves with the next message delivered on a port — the reply the console
 * posts back on the page-side port after forwarding a request.
 */
function nextPortMessage(port: MessagePort): Promise<unknown> {
  return new Promise((resolve) => {
    port.addEventListener(
      "message",
      (event) => resolve((event as { data?: unknown }).data),
      { once: true },
    );
  });
}

/**
 * Drains both queues the bridge works across: the macrotask that delivers a
 * posted port message to the console's handler, and the microtasks the
 * handler's `.then`/`.catch` settle on.
 */
async function flush() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
    await Promise.resolve();
    await Promise.resolve();
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

describe("PagesView bridge", () => {
  it("embeds the page in an opaque-origin sandbox (allow-scripts, no allow-same-origin)", async () => {
    // The `sandbox` attribute is the actual isolation boundary: without
    // `allow-same-origin` the frame is opaque-origin and holds no session
    // cookie, so the document-bound port / capability checks in the bridge are
    // meaningful. A regression that drops the attribute — or quietly adds
    // `allow-same-origin` — must be caught by the suite, not shipped.
    const graphqlRequest = vi.fn();
    await show(clientWith(graphqlRequest));

    const frame = iframe();
    expect(frame).not.toBeNull();
    const sandbox = frame!.getAttribute("sandbox");
    expect(sandbox).toContain("allow-scripts");
    expect(sandbox).not.toContain("allow-same-origin");
  });

  it("ignores an oc:graphql message posted to the window rather than the minted port", async () => {
    const graphqlRequest = vi.fn();
    await show(clientWith(graphqlRequest));

    // The bridge listens only on the port it transferred to the loaded
    // document. A window message — the only thing a spoofing frame or tab can
    // reach — must be ignored even if it carries a valid-looking shape.
    window.dispatchEvent(
      new MessageEvent("message", {
        data: { type: "oc:graphql", id: "spoofed", capability: "cap", query: "{ ping }" },
        source: window,
      }),
    );
    await flush();

    expect(graphqlRequest).not.toHaveBeenCalled();
  });

  it("forwards an oc:graphql message received on the minted port and replies on it", async () => {
    const graphqlRequest = vi.fn().mockResolvedValue({ data: { ping: "pong" }, errors: undefined });
    await show(clientWith(graphqlRequest));

    const frame = iframe();
    expect(frame).not.toBeNull();
    const { capability, port } = mintBridge(frame as HTMLIFrameElement);

    const reply = nextPortMessage(port);
    port.postMessage({
      type: "oc:graphql",
      id: "req-1",
      capability,
      query: "{ ping }",
      variables: { a: 1 },
    });

    expect(await reply).toEqual({
      type: "oc:graphql:result",
      id: "req-1",
      data: { ping: "pong" },
      errors: undefined,
    });
    expect(graphqlRequest).toHaveBeenCalledWith("{ ping }", { a: 1 });
  });

  it("ignores a message on the minted port that isn't the oc:graphql shape", async () => {
    const graphqlRequest = vi.fn();
    await show(clientWith(graphqlRequest));

    const frame = iframe();
    expect(frame).not.toBeNull();
    const { port } = mintBridge(frame as HTMLIFrameElement);

    port.postMessage({ type: "some-other-message" });
    await flush();

    expect(graphqlRequest).not.toHaveBeenCalled();
  });

  it("revokes bridge access when the frame navigates itself", async () => {
    const graphqlRequest = vi.fn();
    await show(clientWith(graphqlRequest));

    const frame = iframe();
    expect(frame).not.toBeNull();
    // Mint a bridge for the current document.
    const { capability, port } = mintBridge(frame as HTMLIFrameElement);
    // The page then navigates itself to a new document. Its load must receive
    // no replacement `oc:init`, and the new occupant — which never received a
    // port — can no longer speak through the bridge even if it replays the
    // original document's capability.
    loadFrame(frame as HTMLIFrameElement);

    port.postMessage({
      type: "oc:graphql",
      id: "stale",
      capability,
      query: "{ secrets }",
    });
    await flush();

    expect(graphqlRequest).not.toHaveBeenCalled();
  });

  it("hands the initial iframe document one capability and port via oc:init", async () => {
    const graphqlRequest = vi.fn();
    await show(clientWith(graphqlRequest));
    const frame = iframe();
    expect(frame).not.toBeNull();

    const contentWindow = frame!.contentWindow as Window;
    const postMessage = vi.spyOn(contentWindow, "postMessage").mockImplementation(() => {});
    loadFrame(frame as HTMLIFrameElement);
    loadFrame(frame as HTMLIFrameElement);
    const inits = postMessage.mock.calls.filter(
      ([msg]) => (msg as { type?: string })?.type === "oc:init",
    );
    expect(inits.length).toBe(1);
    const [message, , transfer] = inits[0] as [
      { capability?: string },
      string,
      MessagePort[],
    ];
    expect(message.capability).toBeTruthy();
    expect(transfer[0]).toBeInstanceOf(MessagePort);
    // The second load represents a frame navigation, not a new document the
    // console selected, so it cannot be granted bridge access.
  });
});
