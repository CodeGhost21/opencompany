import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import {
  isSettingsPage,
  SETTINGS_PAGE_GROUPS,
  SETTINGS_PAGES,
} from "@/views/settings-pages";
import { SETTINGS_NAMED_BY } from "./support/routed-views";

const here = dirname(fileURLToPath(import.meta.url));
const read = (rel: string) => readFileSync(resolve(here, "../../src", rel), "utf8");

describe("Settings navigation (issue #1468)", () => {
  it("keeps the Integrations group, which the Connections move did not empty", () => {
    // Apps and MCP Servers left this rail for `#/connections`. Inference,
    // Hosting and Search stayed, deliberately: each is a credential form
    // beside the one thing it unlocks, which is the argument the file makes
    // twice. Pinned by id rather than by count so that "the group survived" and
    // "it survived with the right three" are the same assertion.
    const integrations = SETTINGS_PAGES.filter((page) => page.group === "integrations");
    expect(integrations.map((page) => page.id)).toEqual(["inference", "hosting", "search"]);
  });

  it("no longer carries the two pages that became the Connections section", () => {
    // Widened to `string` for the same reason the Brain assertion below is:
    // once these ids are gone from the table they are not in the union, so a
    // narrow comparison is a type error rather than the assertion being made.
    const ids = SETTINGS_PAGES.map((page) => page.id as string);
    expect(ids).not.toContain("oauth");
    expect(ids).not.toContain("mcp");
  });

  it("groups every settings page exactly once", () => {
    expect(SETTINGS_PAGE_GROUPS.map((group) => group.label)).toEqual([
      "Identity & lifecycle",
      "Integrations",
      "Capability",
      "Spend",
    ]);
    expect(SETTINGS_PAGE_GROUPS.flatMap((group) => SETTINGS_PAGES.filter((page) => page.group === group.id)))
      .toEqual(SETTINGS_PAGES);
  });

  it("names Approvals in the General hint", () => {
    expect(SETTINGS_PAGES.find((page) => page.id === "general")?.hint).toContain("Approvals");
  });

  it("does not carry the memory browser, which is a nav row now", () => {
    // Brain left this rail for `#/brain`. Settings is where an operator
    // *changes* how the company is configured; what the company remembers is
    // something they read, and reading it should not cost three clicks. The old
    // address still resolves — `console-routes.test.ts` pins that
    // `#/settings/brain` rewrites onto the row.
    // Widened to `string` deliberately: once `brain` is gone from the table its
    // id is not in the union, so a narrow comparison is a type error rather
    // than the assertion this test is making — which is about the DATA, and has
    // to keep holding if someone puts the page back.
    expect(SETTINGS_PAGES.map((page) => page.id as string)).not.toContain("brain");
  });

  it("distinguishes Settings page ids from unknown sub-hashes", () => {
    expect(isSettingsPage("general")).toBe(true);
    expect(isSettingsPage("nonsense")).toBe(false);
    expect(isSettingsPage(null)).toBe(false);
  });

  it("renders linkable rows and gives narrow-screen navigation its missing context", () => {
    const section = read("views/SettingsSection.tsx");
    // The settings sub-pages (one view per SETTINGS_PAGES id). Devices and
    // Connections became pages of their own elsewhere in the redesign, and Apps
    // (the third-party accounts) and MCP Servers then left this rail entirely
    // for the Connections section — `connections-navigation.test.ts` holds them
    // to the same rules there. The list tracks the ids settings-pages.ts
    // actually declares.
    // Derived from `SETTINGS_NAMED_BY`, not restated. A hand-written list is a
    // list that quietly stops covering the newest page: Observatory joined this
    // rail and the hard-coded seven still passed, testing six-sevenths of it and
    // saying nothing. `SETTINGS_NAMED_BY` is a `Record<SettingsPage, …>`, so a
    // page with no entry is a compile error and this sweep cannot fall behind.
    const settingsPages = SETTINGS_PAGES.map(({ id }) =>
      read(`views/${SETTINGS_NAMED_BY[id]}`),
    );
    expect(settingsPages.length).toBe(SETTINGS_PAGES.length);

    expect(section.match(/href=\{`#\/settings\/\$\{item\.id\}`\}/g)).toHaveLength(2);
    expect(section).toContain("title={item.hint}");
    expect(section).toContain("{activePage.hint}");
    // And no longer renders the memory browser: that page is routed by the
    // shell now, not by this rail.
    expect(section).not.toContain("MemoryView");
    // Every settings page draws a visible title, and draws it the one way the
    // console has (issue #1763). It used to be a hand-rolled
    // `text-2xl font-semibold tracking-tight` on each of them; the type scale
    // lives in `PageHeader` now, so what is worth pinning here is that each
    // page still *has* a header rather than what size it sets.
    for (const page of settingsPages) {
      expect(page).toContain("<PageHeader");
      expect(page).not.toContain('hidden title=');
    }
    // General included. It used to hide its own title above `lg` on the
    // reasoning that the rail beside it already says "Settings" (issue #1221);
    // #1763 makes it visible at every width, because every one of its siblings
    // above sits beside that same rail and shows one.
    expect(read("views/SettingsView.tsx")).toContain(
      '<PageHeader title="General settings" width="3xl" />',
    );
  });
});
