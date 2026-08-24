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
let del: ReturnType<typeof vi.fn>;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  sent = vi.fn();
  upload = vi.fn(async () => reference);
  del = vi.fn();
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
        deleteAttachment: del,
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

    // The trailing `undefined` is the mentions arg — no mention directory is
    // loaded here, so the composer passes it absent.
    expect(sent).toHaveBeenLastCalledWith("here is the diagram", undefined, [reference], undefined);
    // The chip is gone after send — a stale attachment must not ride the next
    // message.
    expect(container.textContent).not.toContain("diagram.png");
  });

  // Codex review finding on #1682: an upload lands on the server the instant
  // it succeeds, before the operator has sent anything. Removing, replacing
  // or abandoning it used to just drop the local reference and leave the
  // node behind, orphaned against the workspace quota forever.
  describe("cleaning up a staged upload that never sends", () => {
    it("deletes the stored node when the operator clicks Remove", async () => {
      await render();
      await pick(new File([new Uint8Array([1, 2, 3])], "diagram.png", { type: "image/png" }));

      await act(async () => {
        (
          container.querySelector('[aria-label="Remove diagram.png"]') as HTMLButtonElement
        ).click();
      });

      expect(del).toHaveBeenCalledExactlyOnceWith("node-1");
    });

    it("deletes the replaced upload, not the new one, when a fresh pick supersedes it", async () => {
      const second: AttachmentDto = {
        nodeId: "node-2",
        name: "photo.png",
        mime: "image/png",
        size: 4096,
      };
      upload = vi
        .fn()
        .mockResolvedValueOnce(reference)
        .mockResolvedValueOnce(second);
      await render();
      await pick(new File([new Uint8Array([1, 2, 3])], "diagram.png", { type: "image/png" }));
      await pick(new File([new Uint8Array([4, 5, 6])], "photo.png", { type: "image/png" }));

      expect(del).toHaveBeenCalledExactlyOnceWith("node-1");
      // The new chip is the one that survives.
      expect(container.textContent).toContain("photo.png");
      expect(container.textContent).not.toContain("diagram.png");
    });

    it("deletes a still-pending attachment when the composer unmounts", async () => {
      await render();
      await pick(new File([new Uint8Array([1, 2, 3])], "diagram.png", { type: "image/png" }));

      await act(async () => root.unmount());

      expect(del).toHaveBeenCalledExactlyOnceWith("node-1");
    });

    it("does NOT delete the attachment a send just claimed", async () => {
      await render();
      await pick(new File([new Uint8Array([1, 2, 3])], "diagram.png", { type: "image/png" }));
      await type("here is the diagram");

      await act(async () => {
        (container.querySelector('[aria-label="Send"]') as HTMLButtonElement).click();
      });

      expect(del).not.toHaveBeenCalled();
    });

    // Codex review, round 2: `onSend` is fire-and-forget from the composer's
    // side, and acceptance is asynchronous — the request can still fail
    // after the chip is already gone. `onSend` may report the outcome by
    // returning a promise; an explicit `false` means the host definitely
    // never saw it, so the attachment it carried must not stay orphaned.
    it("deletes the attachment when onSend reports the send definitely did not journal", async () => {
      sent = vi.fn().mockResolvedValue(false);
      await render();
      await pick(new File([new Uint8Array([1, 2, 3])], "diagram.png", { type: "image/png" }));
      await type("here is the diagram");

      await act(async () => {
        (container.querySelector('[aria-label="Send"]') as HTMLButtonElement).click();
      });
      // Let the returned promise's `.then` continuation run.
      await act(async () => {
        await Promise.resolve();
      });

      expect(del).toHaveBeenCalledExactlyOnceWith("node-1");
    });

    it("does not delete when onSend reports the send DID journal", async () => {
      sent = vi.fn().mockResolvedValue(true);
      await render();
      await pick(new File([new Uint8Array([1, 2, 3])], "diagram.png", { type: "image/png" }));
      await type("here is the diagram");

      await act(async () => {
        (container.querySelector('[aria-label="Send"]') as HTMLButtonElement).click();
      });
      await act(async () => {
        await Promise.resolve();
      });

      expect(del).not.toHaveBeenCalled();
    });

    // Codex review, round 4: `false` and "unknown" are not the same thing.
    // `ChatView.send` returns `undefined` on a thrown request — a network
    // drop, a timeout — because `accept_chat_turn` journals the message
    // before the turn's cycle is spawned onto its own task, so a failure
    // from deep inside that task reaches the same `catch` a pre-journal
    // refusal does. Treating `undefined` as "not sent" would delete a node a
    // delivered message might still reference. Only an explicit `false` may
    // ever trigger a delete.
    it("does NOT delete when onSend reports an ambiguous (undefined) outcome", async () => {
      sent = vi.fn().mockResolvedValue(undefined);
      await render();
      await pick(new File([new Uint8Array([1, 2, 3])], "diagram.png", { type: "image/png" }));
      await type("here is the diagram");

      await act(async () => {
        (container.querySelector('[aria-label="Send"]') as HTMLButtonElement).click();
      });
      await act(async () => {
        await Promise.resolve();
      });

      expect(del).not.toHaveBeenCalled();
    });

    // Codex review finding, round 2: unmounting used to check only the
    // synchronous `pending` state. An upload still in flight at unmount time
    // has no chip yet — the unmount cleanup sees nothing pending — and if it
    // then lands on a dead component, nothing was left to free the node it
    // just charged against the quota.
    it("deletes an upload that lands after the composer has already unmounted", async () => {
      let resolveUpload!: (value: AttachmentDto) => void;
      upload = vi.fn(
        () =>
          new Promise<AttachmentDto>((resolve) => {
            resolveUpload = resolve;
          }),
      );
      await render();

      const input = container.querySelector('input[type="file"]') as HTMLInputElement;
      const file = new File([new Uint8Array([1, 2, 3])], "diagram.png", { type: "image/png" });
      Object.defineProperty(input, "files", { value: [file], configurable: true });
      act(() => {
        input.dispatchEvent(new Event("change", { bubbles: true }));
      });

      // Navigate away while the upload is still pending — no chip exists yet
      // for the ordinary unmount cleanup to see.
      await act(async () => root.unmount());
      expect(del).not.toHaveBeenCalled();

      await act(async () => {
        resolveUpload(reference);
        await Promise.resolve();
      });

      expect(del).toHaveBeenCalledExactlyOnceWith("node-1");
    });
  });
});
