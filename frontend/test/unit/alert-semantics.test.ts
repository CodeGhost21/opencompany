// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { Alert } from "@/components/ui/alert";

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

async function render(variant?: "default" | "destructive") {
  await act(async () => {
    root.render(createElement(Alert, { variant }, "Notice"));
  });
}

describe("Alert announcement semantics", () => {
  it("does not announce a standing informational notice assertively", async () => {
    await render();

    expect(container.querySelector("[data-slot=alert]")?.getAttribute("role")).not.toBe("alert");
  });

  // Polite, not absent. A default alert is often mounted in response to
  // something the operator did — the feedback form's success notice, the setup
  // wizard's last-teammate warning — and with no role at all a screen reader
  // would give no sign that the action produced anything.
  it("keeps a default alert in a polite live region", async () => {
    await render();

    expect(container.querySelector("[data-slot=alert]")?.getAttribute("role")).toBe("status");
  });

  it("keeps destructive alerts assertive", async () => {
    await render("destructive");

    expect(container.querySelector("[data-slot=alert]")?.getAttribute("role")).toBe("alert");
  });
});
