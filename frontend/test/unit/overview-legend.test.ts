// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { KnowledgeGraphLegend } from "@/views/overview/kg/KnowledgeGraph";

let host: HTMLElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => root.render(createElement(KnowledgeGraphLegend)));
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

function item(label: string): HTMLElement {
  const el = [...host.querySelectorAll("span")].find((candidate) => candidate.textContent === label);
  expect(el, `the legend must include ${label}`).toBeDefined();
  return el!;
}

describe("the overview graph legend", () => {
  it("gives stages a colour distinct from both workflows and AI teammates", () => {
    const workflowIcon = item("Workflow").querySelector("svg");
    const stageIcon = item("Stage").querySelector("svg");
    const employeeIcon = item("AI teammate").querySelector("svg");

    expect(workflowIcon?.style.color).toBe("var(--brain-2)");
    expect(employeeIcon?.style.color).toBe("var(--accent)");
    expect(stageIcon?.style.color).toBe("var(--stage)");
    expect(stageIcon?.style.color).not.toBe(workflowIcon?.style.color);
    expect(stageIcon?.style.color).not.toBe(employeeIcon?.style.color);
    // `--stage` is an identity hue, not a status one: stages are declared
    // workflow structure, and `--ok` would paint unexecuted stages with the
    // product's "finished cleanly" colour (docs/brand/README.md).
    expect(stageIcon?.style.color).not.toBe("var(--ok)");
  });

  it("wraps legend items instead of letting a narrow graph crop them", () => {
    const legend = host.querySelector('[aria-label="Graph legend"]');
    expect(legend?.className).toContain("flex-wrap");
    expect(legend?.className).toContain("max-w-full");
    expect(item("Workflow").className).toContain("whitespace-nowrap");
    expect(item("Stage").className).toContain("whitespace-nowrap");
  });
});
