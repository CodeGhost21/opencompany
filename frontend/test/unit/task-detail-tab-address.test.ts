import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

const here = dirname(fileURLToPath(import.meta.url));
const read = (rel: string) => readFileSync(resolve(here, "../../src", rel), "utf8");

/**
 * The task detail's tab is addressed (issue #1399), so the screen and its URL
 * must never disagree about which tab is open. The tab resolution itself is
 * covered as a pure function in `task-links.test.ts`; what this file pins is
 * the wiring that makes the screen re-read the address.
 */
describe("Task detail tab address (issue #1399)", () => {
  const view = read("views/TaskDetailView.tsx");

  it("re-reads the address on a lineage hop, not only on a focus change", () => {
    // A lineage hop keeps this component instance mounted and writes a plain
    // `#/tasks/<id>`: the focus goes empty, so without `taskId` in the deps the
    // effect would not run and the previous card's tab would survive a URL
    // that claims the default.
    expect(view).toContain("}, [focusKey, taskId]);");
  });

  it("resets rather than returning early when the address names no tab", () => {
    expect(view).not.toContain("if (!hasFocus(");
    expect(view).toContain("setTab(tabForFocus(focus));");
  });

  it("writes every tab selection back into the address", () => {
    expect(view).toContain("if (isTaskTab(selected)) onTabChange?.(selected);");
  });
});
