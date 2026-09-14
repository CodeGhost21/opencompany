// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { DropZone } from "@/views/memory/DropZone";

/**
 * MEM-003's backend half (`memory_ingest.rs`) accepts and discards every
 * write while the null engine is bound — the host has no lever to refuse an
 * ingest into a sink that keeps nothing. The console's own lever is
 * `discarding`, and `DropZone` is where it has to hold: a raw drag-and-drop
 * bypasses the two buttons' `disabled` prop entirely, so the guard inside
 * `onDrop` is the only thing standing between an operator's drop and a write
 * that would confer nothing while looking like it worked.
 */

function dataTransferWith(files: File[]): DataTransfer {
  return {
    items: files.map((file) => ({
      kind: "file",
      webkitGetAsEntry: () => ({
        isFile: true,
        isDirectory: false,
        name: file.name,
        file: (cb: (f: File) => void) => cb(file),
      }),
    })),
    getData: () => "",
    files,
  } as unknown as DataTransfer;
}

let container: HTMLDivElement;
let root: Root;

async function show(client: OpenCompanyClient, discarding: boolean) {
  await act(async () => {
    root.render(
      createElement(DropZone, {
        client,
        company: "acme",
        discarding,
        onIngested: () => {},
      }),
    );
  });
}

function dropzone(): HTMLElement {
  return container.querySelector('[data-testid="memory-dropzone"]') as HTMLElement;
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

describe("a drop onto a discarding engine ingests nothing", () => {
  it("refuses a raw drop, which the two disabled buttons cannot stop", async () => {
    const postForm = vi.fn(() => Promise.resolve({ items: [] }));
    const client = {
      scopeFor: () => "/api/v1/company/acme",
      postForm,
      post: vi.fn(),
    } as unknown as OpenCompanyClient;
    await show(client, true);

    const file = new File(["hello"], "note.txt", { type: "text/plain" });
    await act(async () => {
      const event = new Event("drop", { bubbles: true, cancelable: true }) as unknown as Event & {
        dataTransfer: DataTransfer;
        preventDefault: () => void;
      };
      Object.defineProperty(event, "dataTransfer", { value: dataTransferWith([file]) });
      dropzone().dispatchEvent(event);
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(postForm).not.toHaveBeenCalled();
  });

  it("ingests normally once the engine actually retains what is dropped", async () => {
    const postForm = vi.fn(() => Promise.resolve({ items: [{ source: "note.txt", status: "stored" }] }));
    const client = {
      scopeFor: () => "/api/v1/company/acme",
      postForm,
      post: vi.fn(),
    } as unknown as OpenCompanyClient;
    await show(client, false);

    const file = new File(["hello"], "note.txt", { type: "text/plain" });
    await act(async () => {
      const event = new Event("drop", { bubbles: true, cancelable: true }) as unknown as Event & {
        dataTransfer: DataTransfer;
      };
      Object.defineProperty(event, "dataTransfer", { value: dataTransferWith([file]) });
      dropzone().dispatchEvent(event);
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(postForm).toHaveBeenCalled();
  });
});
