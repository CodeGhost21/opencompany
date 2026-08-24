// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { FeedbackResponse } from "@/api/types";
import { FeedbackForm } from "@/components/feedback-form";

const PREVIEW: FeedbackResponse = {
  item_id: "preview",
  destination: "local",
  filed: false,
  blocked: false,
  preview_body: "**Category:** wrong-output\n\nThe total was wrong.",
  deduped: false,
};

const FILED: FeedbackResponse = {
  item_id: "filed",
  destination: "github",
  filed: true,
  blocked: false,
  deduped: false,
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  act(() => {
    root = createRoot(container);
  });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function settle(): Promise<void> {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

describe("FeedbackForm", () => {
  it("requires preview before it sends the inspected report", async () => {
    const feedback = vi.fn().mockResolvedValueOnce(PREVIEW).mockResolvedValueOnce(FILED);
    act(() => {
      root.render(
        createElement(FeedbackForm, {
          client: { feedback } as unknown as OpenCompanyClient,
          company: "acme",
          onDone: vi.fn(),
          showCancel: false,
        }),
      );
    });

    const note = container.querySelector<HTMLTextAreaElement>("#feedback-note");
    expect(note).not.toBeNull();
    act(() => {
      const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set;
      setter?.call(note, "The total was wrong.");
      note?.dispatchEvent(new Event("input", { bubbles: true }));
    });

    expect(
      [...container.querySelectorAll<HTMLButtonElement>("button")].find(
        (button) => button.textContent === "Send",
      ),
    ).toBeUndefined();
    act(() => {
      [...container.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent === "Preview")
        ?.click();
    });
    await settle();

    expect(feedback).toHaveBeenCalledWith(
      { category: "wrong-output", note: "The total was wrong.", preview: true },
      "acme",
    );
    expect(container.textContent).toContain("This is exactly what would be shared");

    act(() => {
      [...container.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent === "Send")
        ?.click();
    });
    await settle();

    expect(feedback).toHaveBeenLastCalledWith(
      {
        category: "wrong-output",
        note: "The total was wrong.",
        preview: false,
        item_id: "preview",
      },
      "acme",
    );
    expect(container.textContent).toContain("Shared — thanks!");
  });
});
