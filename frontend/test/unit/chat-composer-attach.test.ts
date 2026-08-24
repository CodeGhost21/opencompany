// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AttachmentDto } from "@/api/types";
import { MessageComposer } from "@/views/chat/MessageComposer";

/**
 * Issue #1682: the composer's paperclip, wired at last.
 *
 * Born disabled and connected to nothing in the #361 console rebuild. These
 * pin that it is present and enabled where attaching is offered, that picking a
 * file uploads it and shows a chip, that the chip is removable, and that a send
 * threads the staged reference onto `onSend` and then clears it.
 */

const reference: AttachmentDto = {
  nodeId: "node-1",
  name: "diagram.png",
  mime: "image/png",
  size: 2048,
};

let container: HTMLDivElement;
let root: Root;
let sent: ReturnType<typeof vi.fn>;
let upload: ReturnType<typeof vi.fn>;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  sent = vi.fn();
  upload = vi.fn(async () => reference);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render(withUpload = true) {
  await act(async () => {
    root.render(
      createElement(MessageComposer, {
        placeholder: "Message engineering",
        onSend: sent,
        uploadAttachment: withUpload ? upload : undefined,
      }),
    );
  });
}

function paperclip() {
  return container.querySelector('[aria-label="Attach a file"]') as HTMLButtonElement | null;
}

async function pick(file: File) {
  const input = container.querySelector('input[type="file"]') as HTMLInputElement;
  Object.defineProperty(input, "files", { value: [file], configurable: true });
  await act(async () => {
    input.dispatchEvent(new Event("change", { bubbles: true }));
  });
}

async function type(text: string) {
  const textarea = container.querySelector("textarea") as HTMLTextAreaElement;
  const setValue = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set;
  await act(async () => {
    setValue?.call(textarea, text);
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

describe("composer paperclip (issue #1682)", () => {
  it("shows an enabled paperclip when attaching is offered", async () => {
    await render();
    const button = paperclip();
    expect(button).not.toBeNull();
    expect(button!.disabled).toBe(false);
  });

  it("omits the paperclip entirely when no upload handler is given", async () => {
    await render(false);
    expect(paperclip()).toBeNull();
  });

  it("uploads a picked file and shows a removable chip", async () => {
    await render();
    await pick(new File([new Uint8Array([1, 2, 3])], "diagram.png", { type: "image/png" }));

    expect(upload).toHaveBeenCalledTimes(1);
    // The chip names the stored file and offers a remove control.
    expect(container.textContent).toContain("diagram.png");
    const remove = container.querySelector('[aria-label="Remove diagram.png"]') as HTMLButtonElement;
    expect(remove).not.toBeNull();

    await act(async () => remove.click());
    expect(container.querySelector('[aria-label="Remove diagram.png"]')).toBeNull();
  });

  it("threads the staged attachment onto the send, then clears it", async () => {
    await render();
    await pick(new File([new Uint8Array([1, 2, 3])], "diagram.png", { type: "image/png" }));
    await type("here is the diagram");

    await act(async () => {
      (container.querySelector('[aria-label="Send"]') as HTMLButtonElement).click();
    });

    expect(sent).toHaveBeenLastCalledWith("here is the diagram", undefined, [reference]);
    // The chip is gone after send — a stale attachment must not ride the next
    // message.
    expect(container.textContent).not.toContain("diagram.png");
  });
});
