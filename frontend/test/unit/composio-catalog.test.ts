import { describe, expect, it } from "vitest";

import type { ComposioToolkitEntry } from "@/api/composio";
import {
  CURATED_TOOLKITS,
  availableCategories,
  buildProviderRows,
  catalogWarning,
  filterByCategory,
  filterProviderRows,
  mapComposioCategory,
  permissionHint,
  providerLabel,
  toolkitLabel,
  visibleProviderRows,
} from "@/lib/composio-catalog";

/** A catalog entry, defaulting every field the backend may leave unpublished. */
function entry(slug: string, over: Partial<ComposioToolkitEntry> = {}): ComposioToolkitEntry {
  return { slug, name: "", description: "", logo: null, categories: [], ...over };
}

/** Slug-only entries — a manifest list, a fallback, or a pre-catalog backend. */
function slugs(list: readonly string[]): ComposioToolkitEntry[] {
  return list.map((s) => entry(s));
}

/** A hundred-provider catalog — the shape the backend actually returns. */
function hundredEntries(): ComposioToolkitEntry[] {
  return Array.from({ length: 100 }, (_, i) =>
    entry(`provider${String(i).padStart(3, "0")}`, { categories: ["productivity"] }),
  );
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

  it("splits a compound slug into words", () => {
    // `Capsule_crm` reads as a database column, not a product. Composio slugs
    // are routinely compound, so this is the common case, not the exotic one.
    expect(toolkitLabel("capsule_crm")).toBe("Capsule Crm");
    expect(toolkitLabel("digital-ocean")).toBe("Digital Ocean");
  });
});

describe("providerLabel", () => {
  it("prefers the backend's published name over local typography", () => {
    // The backend is describing the provider it will actually connect you to,
    // and it learns about a rename before this repo does.
    expect(providerLabel(entry("googlecalendar", { name: "Google Calendar (Workspace)" }))).toBe(
      "Google Calendar (Workspace)",
    );
  });

  it("falls back to the local label when the backend published none", () => {
    expect(providerLabel(entry("googlecalendar"))).toBe("Google Calendar");
    expect(providerLabel(entry("googlecalendar", { name: "   " }))).toBe("Google Calendar");
  });
});

describe("mapComposioCategory", () => {
  it("buckets from Composio's own free-form category strings", () => {
    // The point of the whole design: these strings come off the wire, so a
    // provider Composio adds tomorrow buckets itself with no edit here.
    expect(mapComposioCategory(["messaging"])).toBe("Chat");
    expect(mapComposioCategory(["developer-tools"])).toBe("Platform");
    expect(mapComposioCategory(["project management"])).toBe("Productivity");
    expect(mapComposioCategory(["marketing"])).toBe("Social");
  });

  it("is case-insensitive and reads any of several categories", () => {
    expect(mapComposioCategory(["Sales", "CRM"])).toBe("Platform");
  });

  it("returns null when nothing matches, so the caller can guess", () => {
    // Null rather than a default bucket: the caller has a slug/name heuristic
    // that is strictly better than "Tools & Automation", and swallowing the
    // miss here would hide it.
    expect(mapComposioCategory(["quantum-widgets"])).toBeNull();
    expect(mapComposioCategory([])).toBeNull();
  });

  it("keeps the buckets its OpenHuman twin produces", () => {
    // The drift guard. This function is a second copy of
    // `mapComposioCategory` in `app/src/components/composio/toolkitMeta.tsx`
    // in tinyhumansai/openhuman, and nothing mechanical can compare the two
    // across repositories — edit one and both consoles keep looking correct in
    // isolation while bucketing the same provider differently.
    //
    // So the substring table is pinned case-by-case here. It is not testing
    // that the code does what the code does: it is the diff a divergence has to
    // survive. Anyone editing the table has to edit this list too, and that is
    // the moment they are told a twin exists.
    const table: Array<[string, string | null]> = [
      // Chat wins over everything, including a string that also reads social.
      ["chat", "Chat"],
      ["messaging", "Chat"],
      ["communication", "Chat"],
      // Social is tested BEFORE productivity, so `marketing` is Social even
      // though a marketing tool is arguably productivity.
      ["social", "Social"],
      ["marketing", "Social"],
      ["productivity", "Productivity"],
      ["document", "Productivity"],
      ["calendar", "Productivity"],
      ["scheduling", "Productivity"],
      ["project management", "Productivity"],
      ["project-management", "Productivity"],
      ["note", "Productivity"],
      ["task", "Productivity"],
      ["storage", "Productivity"],
      ["email", "Productivity"],
      ["crm", "Platform"],
      ["developer", "Platform"],
      ["devtool", "Platform"],
      ["analytics", "Platform"],
      ["payment", "Platform"],
      ["finance", "Platform"],
      ["database", "Platform"],
      ["cloud", "Platform"],
      // Not in the table on either side.
      ["quantum-widgets", null],
    ];
    for (const [category, expected] of table) {
      expect(mapComposioCategory([category]), `category ${category}`).toBe(expected);
    }
  });

  it("orders its buckets so the first hit wins, not the last", () => {
    // Both copies test Chat, then Social, then Productivity, then Platform,
    // and return on the first hit. An entry carrying several categories
    // therefore depends on that order — reordering the branches on one side
    // only is the subtlest way the twins can drift.
    expect(mapComposioCategory(["email", "messaging"])).toBe("Chat");
    expect(mapComposioCategory(["crm", "marketing"])).toBe("Social");
    expect(mapComposioCategory(["analytics", "calendar"])).toBe("Productivity");
  });
});

describe("buildProviderRows", () => {
  it("renders every provider the host sent, whatever it is", () => {
    expect(buildProviderRows(hundredEntries(), [], {})).toHaveLength(100);
  });

  it("carries the backend's display metadata onto the tile", () => {
    // The regression test for #600 on the console side. Every field here was on
    // the wire and thrown away by the host, which is why the panel could only
    // ever be a flat list of slugs.
    const [row] = buildProviderRows(
      [
        entry("hubspot", {
          name: "HubSpot",
          description: "CRM and marketing automation.",
          logo: "https://logos.composio.dev/api/hubspot",
          categories: ["crm"],
        }),
      ],
      [],
      {},
    );
    expect(row).toMatchObject({
      slug: "hubspot",
      label: "HubSpot",
      description: "CRM and marketing automation.",
      logoUrl: "https://logos.composio.dev/api/hubspot",
      category: "Platform",
    });
  });

  it("derives a logo for an entry that published none", () => {
    // Slug-only entries still get branded tiles. The URL is a guess, so the
    // tile handles a 404 — but a guess that usually works beats a grid of grey
    // squares.
    expect(buildProviderRows(slugs(["notion"]), [], {})[0].logoUrl).toBe(
      "https://logos.composio.dev/api/notion",
    );
  });

  it("guesses a category from the slug when the backend published none", () => {
    // The fallback path — a manifest allowlist, a degraded fallback, or a
    // backend predating the dynamic catalog. Without it every such tile would
    // land in one bucket and the chips would be useless exactly where the
    // catalog is thinnest.
    const rows = buildProviderRows(slugs(["slack", "gmail", "github", "quantumwidgets"]), [], {});
    const by = Object.fromEntries(rows.map((r) => [r.slug, r.category]));
    expect(by).toEqual({
      slack: "Chat",
      gmail: "Productivity",
      github: "Platform",
      quantumwidgets: "Tools & Automation",
    });
  });

  it("never drops a provider whose category Composio has just invented", () => {
    // The property worth pinning explicitly rather than inferring from the two
    // tests above: Composio can add a category string tomorrow that neither
    // `mapComposioCategory` nor the keyword heuristic knows, and that provider
    // must still get a tile. "Tools & Automation" is the floor, and it is a
    // real bucket that `availableCategories` will offer — not a hole a provider
    // can fall through and silently leave the grid.
    const rows = buildProviderRows([entry("newthing", { categories: ["quantum-widgets"] })], [], {});
    expect(rows.map((r) => r.slug)).toEqual(["newthing"]);
    expect(rows[0].category).toBe("Tools & Automation");
    expect(availableCategories(rows)).toContain("Tools & Automation");
    expect(visibleProviderRows(rows, "All", "")).toHaveLength(1);
  });

  it("puts connected first, then the common ones, then the tail alphabetically", () => {
    const rows = buildProviderRows(slugs(["zendesk", "gmail", "airtable", "github", "notion"]), [], {
      notion: true,
    });
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
    const rows = buildProviderRows(slugs(["gmail", "GMAIL", "", "  "]), ["hubspot", "gmail"], {});
    expect(rows.map((r) => r.slug)).toEqual(["gmail", "hubspot"]);
  });

  it("marks connection state case-insensitively", () => {
    const rows = buildProviderRows(slugs(["Gmail"]), [], { gmail: true });
    expect(rows[0]).toMatchObject({ slug: "gmail", connected: true, curated: true });
  });
});

describe("availableCategories", () => {
  it("offers only buckets that have something in them", () => {
    // A chip that reliably yields an empty grid teaches the operator to
    // distrust the whole filter row.
    const rows = buildProviderRows(slugs(["slack", "gmail"]), [], {});
    expect(availableCategories(rows)).toEqual(["All", "Chat", "Productivity"]);
  });

  it("always leads with All, in a fixed order", () => {
    // Fixed rather than derived: a filter row that reshuffles when a company
    // connects a provider moves under the operator's cursor.
    const rows = buildProviderRows(slugs(["github", "slack"]), [], {});
    expect(availableCategories(rows)).toEqual(["All", "Chat", "Platform"]);
  });
});

describe("filterProviderRows", () => {
  const rows = buildProviderRows(
    [
      entry("googlecalendar"),
      entry("gmail"),
      entry("hubspot"),
      entry("zendesk"),
      entry("stripe", { name: "Stripe", description: "Payments, invoices and subscriptions." }),
    ],
    [],
    {},
  );

  it("matches on the product name an operator would type", () => {
    expect(filterProviderRows(rows, "google cal").map((r) => r.slug)).toEqual(["googlecalendar"]);
  });

  it("matches on the Composio slug too", () => {
    // Both halves matter: one operator knows the product, the other knows
    // Composio, and neither should come up empty.
    expect(filterProviderRows(rows, "hubspot").map((r) => r.slug)).toEqual(["hubspot"]);
  });

  it("matches on the description — what a provider does, not what it is called", () => {
    // New with #600: there was no description on the wire to search before.
    // An operator who does not know Stripe by name can still reach it.
    expect(filterProviderRows(rows, "invoices").map((r) => r.slug)).toEqual(["stripe"]);
  });

  it("is case-insensitive and returns nothing for a genuine miss", () => {
    expect(filterProviderRows(rows, "ZENDESK").map((r) => r.slug)).toEqual(["zendesk"]);
    expect(filterProviderRows(rows, "nothing-like-this")).toEqual([]);
  });

  it("an empty query narrows nothing", () => {
    expect(filterProviderRows(rows, "   ")).toHaveLength(rows.length);
  });
});

describe("filterByCategory", () => {
  const rows = buildProviderRows(slugs(["slack", "gmail", "github"]), [], {});

  it("narrows to one bucket", () => {
    expect(filterByCategory(rows, "Chat").map((r) => r.slug)).toEqual(["slack"]);
  });

  it("All is the identity", () => {
    expect(filterByCategory(rows, "All")).toHaveLength(rows.length);
  });
});

describe("visibleProviderRows", () => {
  it("shows the whole catalog — there is no preview cut any more", () => {
    // The heart of #600. The old helper collapsed to twelve rows behind a "Show
    // all 123 providers" button; the cut was a workaround for a flat list being
    // unreadable, not a feature. A grid shows all of them, so a test that
    // asserted a preview would now be pinning the bug.
    expect(visibleProviderRows(buildProviderRows(hundredEntries(), [], {}), "All", "")).toHaveLength(
      100,
    );
  });

  it("composes the category filter and the search with AND", () => {
    // Search must not silently clear the chip: that throws away half of what
    // the operator already told us.
    const rows = buildProviderRows(
      [entry("slack", { categories: ["messaging"] }), entry("gmail", { categories: ["email"] })],
      [],
      {},
    );
    expect(visibleProviderRows(rows, "Chat", "slack").map((r) => r.slug)).toEqual(["slack"]);
    expect(visibleProviderRows(rows, "Chat", "gmail")).toEqual([]);
  });

  it("a search reaches the whole tail, not just the top of it", () => {
    // provider0NN — 10 matches, all deep in the list. A search that only looked
    // at a preview would make the long tail unreachable by name, leaving the
    // slug field as the only route to it.
    const rows = buildProviderRows(hundredEntries(), [], {});
    expect(visibleProviderRows(rows, "All", "provider09").map((r) => r.slug)).toEqual([
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
  });

  it("a short list renders whole", () => {
    const short = buildProviderRows(slugs(CURATED_TOOLKITS), [], {});
    expect(visibleProviderRows(short, "All", "")).toHaveLength(short.length);
  });
});

describe("permissionHint", () => {
  it("describes the shape of the access, not a scope list", () => {
    // Composio decides the real scopes at consent time and does not publish
    // them here. Enumerating them would be a claim this console cannot back.
    expect(permissionHint("Chat")).toContain("communication");
    expect(permissionHint("Tools & Automation")).toBe("Connected account data");
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
