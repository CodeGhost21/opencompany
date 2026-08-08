import { describe, expect, it } from "vitest";

import {
  CURATED_TOOLKITS,
  PREVIEW_COUNT,
  buildProviderRows,
  catalogWarning,
  filterProviderRows,
  toolkitLabel,
  visibleProviderRows,
} from "@/lib/composio-catalog";

/** A hundred-slug catalog — the shape the backend actually returns. */
function hundredSlugs(): string[] {
  return Array.from({ length: 100 }, (_, i) => `provider${String(i).padStart(3, "0")}`);
}

describe("toolkitLabel", () => {
  it("reads a known slug as its product name", () => {
    expect(toolkitLabel("googlecalendar")).toBe("Google Calendar");
    expect(toolkitLabel("github")).toBe("GitHub");
  });

  it("still renders a slug it has never seen", () => {
    // The list is the backend's; only its typography is ours. A slug missing
    // from the label table must never be dropped or blanked — that would put a
    // local table back in charge of what the operator can reach, which is the
    // bug #397 is about.
    expect(toolkitLabel("obscureprovider")).toBe("Obscureprovider");
    expect(toolkitLabel("  MixedCase  ")).toBe("Mixedcase");
  });
});

describe("buildProviderRows", () => {
  it("renders every slug the host sent, whatever it is", () => {
    const rows = buildProviderRows(hundredSlugs(), [], {});
    expect(rows).toHaveLength(100);
  });

  it("puts connected first, then the common ones, then the tail alphabetically", () => {
    const rows = buildProviderRows(
      ["zendesk", "gmail", "airtable", "github", "notion"],
      [],
      { notion: true },
    );
    expect(rows.map((r) => r.slug)).toEqual([
      // connected first — an operator scanning the panel is checking what is live
      "notion",
      // then curated, in curated order
      "gmail",
      "github",
      // then the tail, alphabetically by label
      "airtable",
      "zendesk",
    ]);
  });

  it("keeps session-connected extras and collapses duplicates and blanks", () => {
    const rows = buildProviderRows(["gmail", "GMAIL", "", "  "], ["hubspot", "gmail"], {});
    expect(rows.map((r) => r.slug)).toEqual(["gmail", "hubspot"]);
  });

  it("marks connection state case-insensitively", () => {
    const rows = buildProviderRows(["Gmail"], [], { gmail: true });
    expect(rows[0]).toMatchObject({ slug: "gmail", connected: true, curated: true });
  });
});

describe("filterProviderRows", () => {
  const rows = buildProviderRows(["googlecalendar", "gmail", "hubspot", "zendesk"], [], {});

  it("matches on the product name an operator would type", () => {
    expect(filterProviderRows(rows, "google cal").map((r) => r.slug)).toEqual(["googlecalendar"]);
  });

  it("matches on the Composio slug too", () => {
    // Both halves matter: one operator knows the product, the other knows
    // Composio, and neither should come up empty.
    expect(filterProviderRows(rows, "hubspot").map((r) => r.slug)).toEqual(["hubspot"]);
  });

  it("is case-insensitive and returns nothing for a genuine miss", () => {
    expect(filterProviderRows(rows, "ZENDESK").map((r) => r.slug)).toEqual(["zendesk"]);
    expect(filterProviderRows(rows, "nothing-like-this")).toEqual([]);
  });

  it("an empty query narrows nothing", () => {
    expect(filterProviderRows(rows, "   ")).toHaveLength(rows.length);
  });
});

describe("visibleProviderRows", () => {
  const rows = buildProviderRows(hundredSlugs(), [], {});

  it("collapses a hundred rows to a preview and reports what is held back", () => {
    // The reason this exists: the host now serves the real catalog, and
    // rendering a hundred unfiltered rows would trade one broken page for
    // another.
    const { visible, hidden } = visibleProviderRows(rows, "", false);
    expect(visible).toHaveLength(PREVIEW_COUNT);
    expect(hidden).toBe(100 - PREVIEW_COUNT);
  });

  it("shows everything once the operator expands", () => {
    const { visible, hidden } = visibleProviderRows(rows, "", true);
    expect(visible).toHaveLength(100);
    expect(hidden).toBe(0);
  });

  it("a search reaches past the preview — the tail must be discoverable", () => {
    // provider0NN — 10 matches, all beyond the twelve-row preview for most of
    // them. A search that only looked at the preview would make the long tail
    // unreachable by name, leaving the slug field as the only route to it.
    const { visible, hidden } = visibleProviderRows(rows, "provider09", false);
    expect(visible.map((r) => r.slug)).toEqual([
      "provider090",
      "provider091",
      "provider092",
      "provider093",
      "provider094",
      "provider095",
      "provider096",
      "provider097",
      "provider098",
      "provider099",
    ]);
    expect(hidden).toBe(0);
  });

  it("a short list is never collapsed", () => {
    const short = buildProviderRows(CURATED_TOOLKITS, [], {});
    const { visible, hidden } = visibleProviderRows(short, "", false);
    expect(visible).toHaveLength(short.length);
    expect(hidden).toBe(0);
  });
});

describe("catalogWarning", () => {
  it("warns the operator when the list is a fallback, and says why", () => {
    // The console half of the host's honesty contract. A fallback rendered
    // identically to a fetched catalog would make the host's marking worthless:
    // eight providers would look like the whole set.
    const warning = catalogWarning({
      catalogSource: "fallback",
      catalogNotice: "Composio's provider catalog could not be fetched (connection refused).",
    });
    expect(warning).toContain("connection refused");
  });

  it("still warns when the host sent no reason", () => {
    const warning = catalogWarning({ catalogSource: "fallback", catalogNotice: null });
    expect(warning).toContain("may be incomplete");
  });

  it("stays silent for a real catalog", () => {
    expect(catalogWarning({ catalogSource: "backend", catalogNotice: null })).toBeNull();
  });

  it("stays silent for a company's own allowlist", () => {
    // `manifest` is not a degradation — the company chose that list, and telling
    // it the list "may be incomplete" would be false.
    expect(catalogWarning({ catalogSource: "manifest", catalogNotice: null })).toBeNull();
  });
});
