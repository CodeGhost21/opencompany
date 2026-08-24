// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AttachmentDto } from "@/api/types";
import { MessageAttachments } from "@/views/chat/MessageAttachments";

/**
 * Issue #1682: how an attachment renders in the transcript. v1 is a download
 * chip for every file and an inline preview for a non-SVG image. SVG is
 * download-only — it is an XML document whose script would execute, so the blob
 * route serves it as an attachment and the console never inlines it.
 */

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  // jsdom does not implement the object-URL lifecycle the component revokes;
  // patch just the two static methods, leaving the `URL` constructor intact.
  URL.createObjectURL = vi.fn(() => "blob:mock");
  URL.revokeObjectURL = vi.fn();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

async function render(attachments: AttachmentDto[], resolveUrl?: (id: string) => Promise<string>) {
  await act(async () => {
    root.render(createElement(MessageAttachments, { attachments, resolveUrl }));
  });
  // Flush the preview-fetch effect's microtasks.
  await act(async () => {
    await Promise.resolve();
  });
}

const png: AttachmentDto = { nodeId: "n1", name: "chart.png", mime: "image/png", size: 4096 };
const svg: AttachmentDto = { nodeId: "n2", name: "logo.svg", mime: "image/svg+xml", size: 512 };
const pdf: AttachmentDto = { nodeId: "n3", name: "report.pdf", mime: "application/pdf", size: 8192 };

describe("MessageAttachments (issue #1682)", () => {
  it("renders a download chip with the file's name and size", async () => {
    await render([pdf]);
    expect(container.textContent).toContain("report.pdf");
    expect(container.textContent).toContain("8.0 KB");
    expect(container.querySelector('[title="Download report.pdf"]')).not.toBeNull();
  });

  it("previews a non-SVG image inline, fetched through the resolver", async () => {
    const resolveUrl = vi.fn(async () => "blob:the-image");
    await render([png], resolveUrl);
    expect(resolveUrl).toHaveBeenCalledWith("n1");
    const img = container.querySelector("img");
    expect(img).not.toBeNull();
    expect(img!.getAttribute("src")).toBe("blob:the-image");
  });

  it("never inlines an SVG — download-only", async () => {
    const resolveUrl = vi.fn(async () => "blob:the-svg");
    await render([svg], resolveUrl);
    // The chip is there; the inline preview is not.
    expect(container.textContent).toContain("logo.svg");
    expect(container.querySelector("img")).toBeNull();
  });

  it("clicking the chip resolves the bytes for download", async () => {
    const resolveUrl = vi.fn(async () => "blob:the-pdf");
    await render([pdf], resolveUrl);
    await act(async () => {
      (container.querySelector('[title="Download report.pdf"]') as HTMLButtonElement).click();
    });
    expect(resolveUrl).toHaveBeenCalledWith("n3");
  });
});
