// @vitest-environment jsdom

import { createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { inlineCode } from "@/lib/inline-code";

/**
 * A ledger's `purpose` is authored as Markdown and was printed as plain text,
 * so every console surface that showed one showed its backticks: the Work
 * page's subtitle, its disclosure, and every row of Manage lists.
 *
 * The strings this has to get right are the real ones — the `tasks` purpose is
 * the sentence the operator meets first on the console's most-visited screen.
 */
let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (
    globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

async function render(text: string) {
  await act(async () =>
    root.render(createElement("p", null, inlineCode(text))),
  );
}

describe("inlineCode", () => {
  it("renders a code span as a <code>, not as backticks", async () => {
    await render("written through the board — not with `record_entry`.");

    const code = container.querySelectorAll("code");
    expect(code).toHaveLength(1);
    expect(code[0].textContent).toBe("record_entry");
    expect(container.textContent).not.toContain("`");
    expect(container.textContent).toBe(
      "written through the board — not with record_entry.",
    );
  });

  it("handles several spans in one sentence", async () => {
    // The `writtenBy` string on the tasks ledger, which carries three.
    await render(
      "the board — `spawn_task` to open a card, `assign_task` to hand it over. `record_entry` does not write it",
    );

    expect(
      Array.from(container.querySelectorAll("code")).map((c) => c.textContent),
    ).toEqual(["spawn_task", "assign_task", "record_entry"]);
  });

  it("passes an ordinary sentence straight through", async () => {
    await render("Every candidate in flight, and where each one sits.");

    expect(container.querySelectorAll("code")).toHaveLength(0);
    expect(container.textContent).toBe(
      "Every candidate in flight, and where each one sits.",
    );
  });

  it("leaves an unpaired backtick as the character it is", async () => {
    // Better a literal backtick than swallowing the rest of the sentence into
    // a code span that was never closed.
    await render("a stray ` and then some words");

    expect(container.querySelectorAll("code")).toHaveLength(0);
    expect(container.textContent).toBe("a stray ` and then some words");
  });
});
