import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { NAV_SECTIONS } from "@/components/sidebar-navigation";
import { TOUR } from "@/tour/steps";
import {
  CONNECTION_PAGES,
  connectionsHref,
  DEFAULT_CONNECTION_PAGE,
  isConnectionPage,
  resolveConnectionPage,
} from "@/views/connection-pages";

const here = dirname(fileURLToPath(import.meta.url));
const read = (rel: string) => readFileSync(resolve(here, "../../src", rel), "utf8");

/**
 * The Connections section: Apps and MCP Servers, out of the Settings rail.
 *
 * The counterpart to `settings-navigation.test.ts`, which holds the rail they
 * left. What is worth pinning here is the part a reviewer cannot see by reading
 * either file alone — that the section's two pages are exactly the two that
 * moved, that its rail is built the way the other two rails are, and that the
 * one stop in the guided tour whose title is "Connect your tools" points at the
 * nav row that now leads there.
 */
describe("the Connections section", () => {
  it("carries exactly the two pages that left the Settings rail", () => {
    expect(CONNECTION_PAGES.map((page) => page.id)).toEqual(["apps", "mcp"]);
  });

  it("does not take the three credential forms with them", () => {
    // Inference, Hosting and Search stayed in Settings on purpose: a credential
    // form belongs beside the one thing it unlocks. Asserted from this side as
    // well as from `settings-navigation.test.ts`, because "Connections grew a
    // fourth page" and "Settings lost a page" are the same mistake seen from
    // two directions, and only one of the two files would fail.
    const ids = CONNECTION_PAGES.map((page) => page.id as string);
    for (const stayed of ["inference", "hosting", "search"]) {
      expect(ids, `${stayed} belongs beside what it unlocks`).not.toContain(stayed);
    }
  });

  it("leads with Apps, so the section is never an empty frame", () => {
    expect(DEFAULT_CONNECTION_PAGE).toBe("apps");
    expect(resolveConnectionPage(null)).toBe("apps");
    expect(resolveConnectionPage("not-a-page")).toBe("apps");
    expect(resolveConnectionPage("mcp")).toBe("mcp");
  });

  it("distinguishes its page ids from unknown sub-hashes", () => {
    expect(isConnectionPage("apps")).toBe(true);
    expect(isConnectionPage("oauth")).toBe(false);
    expect(isConnectionPage(null)).toBe(false);
  });

  it("mints hashes under its own section, not under Settings", () => {
    // The typed helper exists so a link cannot outlive the page it names —
    // the defect `#/settings/connections` was for four releases.
    expect(connectionsHref("apps")).toBe("#/connections/apps");
    expect(connectionsHref("mcp")).toBe("#/connections/mcp");
  });

  it("gives every page a label and a hint, so the rail says what each is for", () => {
    for (const page of CONNECTION_PAGES) {
      expect(page.label, page.id).toBeTruthy();
      expect(page.hint, page.id).toBeTruthy();
      // A hint is what the rail shows under the label; repeating the label
      // there tells an operator nothing they cannot already see.
      expect(page.hint, page.id).not.toBe(page.label);
    }
  });

  it("renames the accounts page to Apps everywhere it is named", () => {
    // Three places, and they have to agree: the rail's row, the rail's hint,
    // and the page's own `h1`. A rename that reaches two of the three leaves an
    // operator clicking "Apps" and landing on a page headed "OAuth".
    expect(CONNECTION_PAGES.find((p) => p.id === "apps")?.label).toBe("Apps");
    expect(read("views/OAuthView.tsx")).toContain('title="Apps"');
    expect(read("views/OAuthView.tsx")).not.toContain('title="OAuth"');
  });

  it("draws its sub-navigation in the sidebar rather than a rail in the page", () => {
    // This section shipped with a `w-60` rail inside the content area, modelled
    // on Finance's. Sub-navigation lives in the sidebar now, under the
    // section's own row, so all four sections use one pattern — and the
    // content pane keeps the 240px the rail was charging it.
    const section = read("views/connections/ConnectionsSection.tsx");
    expect(section).not.toMatch(/<nav[\s>]/);
    expect(section).not.toContain("w-60");

    // Where it went, asserted from the rendered sidebar rather than from source
    // text: the rows are what an operator clicks, and a `toContain` over a nav
    // table is satisfied by a commented-out row that renders nothing (#1311).
    const connections = NAV_SECTIONS.find((s) => s.view === "connections")!;
    expect(connections.children?.map((child) => [child.label, child.sub])).toEqual([
      ["Apps", "apps"],
      ["MCP Servers", "mcp"],
    ]);
  });

  it("still renders Composio on the Apps page rather than splitting it out", () => {
    // `ComposioSection` looks self-contained, but `ProvidersSection` reads its
    // credential state to decide what every provider tile renders — the
    // credential is the engine the provider list runs on. Splitting them would
    // separate a credential from what it unlocks, which is the one thing this
    // whole section is arranged around not doing.
    // Matched at the JSX element boundary rather than as a substring. A bare
    // `toContain("<ComposioSection")` is satisfied by `<ComposioSectionAnything`,
    // so renaming the element past it proved nothing — verified by mutating it
    // to `<ComposioSectionRemoved`, which passed.
    const oauth = read("views/OAuthView.tsx");
    expect(oauth).toMatch(/<ComposioSection[\s/>]/);
    expect(oauth).toMatch(/<ProvidersSection[\s/>]/);
  });
});

describe("the guided tour's Connect-your-tools stop", () => {
  const stop = TOUR.find((s) => s.title === "Connect your tools");

  it("exists", () => {
    expect(stop).toBeDefined();
  });

  it("spotlights the nav row that actually leads to the tools", () => {
    // It used to navigate to `{ view: "settings", sub: "oauth" }` and spotlight
    // `nav-settings` — so a step titled "Connect your tools" pointed an operator
    // at a gear. The row exists now, so the step names the same surface the
    // sidebar does.
    expect(stop!.view).toBe("connections");
    expect(stop!.sub).toBe("apps");
    expect(stop!.target).toBe('[data-tour="nav-connections"]');
  });

  it("targets an anchor the sidebar actually renders", () => {
    // A tour step whose anchor never mounts degrades to a *skipped* step —
    // silently — so the tour goes on teaching half the product and nothing
    // reports it. That is why this is asserted rather than left to the browser.
    //
    // Asserted against the nav table itself rather than against the shell's
    // source text. The source form had to be anchored to the start of a line to
    // stop a **commented-out** row satisfying it (`// { view: "connections", …
    // }` still holds the substring, and a commented row renders no anchor at
    // all — issue #1311). Reading the table removes the trap by construction:
    // a commented row is not a member of it.
    const row = NAV_SECTIONS.find((section) => section.view === "connections");
    expect(row?.label).toBe("Connections");
    expect(stop!.target).toBe(`[data-tour="nav-${row!.view}"]`);
  });
});
