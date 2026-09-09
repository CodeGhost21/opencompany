// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { MessageAttachments } from "@/views/chat/MessageAttachments";
import type { AttachmentDto } from "@/api/types";

/**
 * `ChatView`'s `resolveAttachmentUrl` wraps `fetchBlobUrl` against the node id
 * a chat line stored — the node named there can be gone by the time someone
 * clicks it (deleted from the workspace after the line was written). The
 * download button's own `catch` sets `downloadError`; before this it was an
 * unhandled rejection with the button just looking like it did nothing.
 */

const ATTACHMENT: AttachmentDto = {
  nodeId: "node-1",
  name: "report.pdf",
  mime: "application/pdf",
  size: 1024,
};

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
  vi.restoreAllMocks();
});

function downloadButton(): HTMLButtonElement {
  return container.querySelector("button") as HTMLButtonElement;
}

describe("MessageAttachments, a node deleted from the workspace after the line was written (FAIL)", () => {
  it("shows an honest error instead of silently doing nothing", async () => {
    const resolveUrl = vi.fn(async () => {
      throw new Error("This file is no longer in the workspace.");
    });
    await act(async () => {
      root.render(createElement(MessageAttachments, { attachments: [ATTACHMENT], resolveUrl }));
    });

    await act(async () => {
      downloadButton().click();
    });

    const alert = container.querySelector('[role="alert"]');
    expect(alert).not.toBeNull();
    expect(alert?.textContent).toContain("This file is no longer in the workspace.");
    // Not stuck showing the spinner either.
    expect(downloadButton().disabled).toBe(false);
  });

  it("falls back to a generic message for a non-Error rejection", async () => {
    const resolveUrl = vi.fn(async () => {
      throw { status: 404 };
    });
    await act(async () => {
      root.render(createElement(MessageAttachments, { attachments: [ATTACHMENT], resolveUrl }));
    });

    await act(async () => {
      downloadButton().click();
    });

    expect(container.querySelector('[role="alert"]')?.textContent).toBe(
      "Couldn't download this file.",
    );
  });
});

describe("MessageAttachments, no resolver bound (client cannot reach the blob route)", () => {
  it("renders the chip inert rather than offering a download that can only fail", async () => {
    await act(async () => {
      root.render(createElement(MessageAttachments, { attachments: [ATTACHMENT] }));
    });

    expect(downloadButton().disabled).toBe(true);
  });
});
