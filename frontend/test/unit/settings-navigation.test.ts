import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import {
  isSettingsPage,
  SETTINGS_PAGE_GROUPS,
  SETTINGS_PAGES,
} from "@/views/settings-pages";

const here = dirname(fileURLToPath(import.meta.url));
const read = (rel: string) => readFileSync(resolve(here, "../../src", rel), "utf8");

describe("Settings navigation (issue #1468)", () => {
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

  it("puts the memory browser in the capability section", () => {
    expect(SETTINGS_PAGES.find((page) => page.id === "brain")).toMatchObject({
      label: "Brain",
      hint: "What your company remembers",
      group: "capability",
    });
  });

  it("distinguishes Settings page ids from unknown sub-hashes", () => {
    expect(isSettingsPage("general")).toBe(true);
    expect(isSettingsPage("nonsense")).toBe(false);
    expect(isSettingsPage(null)).toBe(false);
  });

  it("renders linkable rows and gives narrow-screen navigation its missing context", () => {
    const section = read("views/SettingsSection.tsx");
    // The settings sub-pages (one view per SETTINGS_PAGES id). Devices and
    // Connections became pages of their own elsewhere in the redesign, so the
    // list tracks the ids settings-pages.ts actually declares.
    const settingsPages = [
      "SettingsView.tsx",
      "PeopleView.tsx",
      "OAuthView.tsx",
      "McpServersView.tsx",
      "InferenceView.tsx",
      "HostingView.tsx",
      "SearchView.tsx",
      "SkillsView.tsx",
      "UsageView.tsx",
    ].map((page) => read(`views/${page}`));

    expect(section.match(/href=\{`#\/settings\/\$\{item\.id\}`\}/g)).toHaveLength(2);
    expect(section).toContain("title={item.hint}");
    expect(section).toContain("{activePage.hint}");
    expect(section).toContain('{page === "brain" && <MemoryView client={client} company={company} />}');
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
