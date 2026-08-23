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

  it("distinguishes Settings page ids from unknown sub-hashes", () => {
    expect(isSettingsPage("general")).toBe(true);
    expect(isSettingsPage("nonsense")).toBe(false);
    expect(isSettingsPage(null)).toBe(false);
  });

  it("renders linkable rows and gives narrow-screen navigation its missing context", () => {
    const settings = read("views/SettingsView.tsx");

    expect(settings.match(/href=\{`#\/settings\/\$\{item\.id\}`\}/g)).toHaveLength(2);
    expect(settings).toContain("title={item.hint}");
    expect(settings).toContain("{activePage.hint}");
    expect(settings).toContain('className="text-2xl font-semibold tracking-tight lg:sr-only"');
  });
});
