// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { useHashView } from "@/hooks/use-hash-view";
import { isSettingsPage } from "@/views/settings-pages";

const VIEWS = ["overview", "settings"] as const;

// This is the Settings branch of app-shell.tsx's route rewrite. Settings has a
// closed page table, so unlike entity-addressed views it can repair an unknown
// segment before the section renders its default page.
const REWRITE = (head: string, sub: string | null): [string, string | null] | null =>
  head === "settings" && sub !== null && !isSettingsPage(sub) ? ["settings", "general"] : null;

describe("Settings hash routes (issue #1422)", () => {
  let container: HTMLDivElement;
  let root: Root;
  let seen: [string, string | null];

  function Probe() {
    const [view, sub] = useHashView<string>(VIEWS as readonly string[], "overview", REWRITE);
    seen = [view, sub];
    return null;
  }

  async function visit(hash: string) {
    window.history.replaceState(null, "", hash);
    await act(async () => {
      root.render(createElement(Probe));
    });
  }

  beforeEach(() => {
    (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it("replaces an unknown Settings sub-hash with General", async () => {
    await visit("#/settings/nonsense");

    expect(seen).toEqual(["settings", "general"]);
    expect(window.location.hash).toBe("#/settings/general");
  });

  it("leaves a real Settings sub-hash alone", async () => {
    await visit("#/settings/people");

    expect(seen).toEqual(["settings", "people"]);
    expect(window.location.hash).toBe("#/settings/people");
  });
});
