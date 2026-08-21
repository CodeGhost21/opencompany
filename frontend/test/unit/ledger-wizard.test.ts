// The declare wizard's pure logic: presets assemble a valid spec, a status
// that needs a reason always gets somewhere to put one, and slugs never
// collide with what the company already has.

import { describe, expect, it } from "vitest";

import {
  buildLedgerSpec,
  FIELD_PRESETS,
  slugify,
  STAGE_PRESETS,
  stagePreset,
  summarize,
  WIZARD_CHECKS,
  type WizardDraft,
} from "@/lib/ledger-wizard";

function draft(overrides: Partial<WizardDraft> = {}): WizardDraft {
  return {
    purpose: "What we told a customer we would do.",
    title: "Customer promises",
    slug: "customer-promises",
    statuses: stagePreset("open-closed")!.statuses,
    fields: [],
    ...overrides,
  };
}

describe("slugify", () => {
  it("lowercases and hyphenates", () => {
    expect(slugify("Customer promises")).toBe("customer-promises");
  });

  it("collapses runs of unusable characters and trims separators", () => {
    expect(slugify("  Weekly   digest!!  ")).toBe("weekly-digest");
  });

  it("returns empty for a title with nothing slug-safe", () => {
    expect(slugify("???")).toBe("");
  });

  it("disambiguates a collision", () => {
    expect(slugify("Goals", ["goals"])).toBe("goals-2");
    expect(slugify("Goals", ["goals", "goals-2"])).toBe("goals-3");
  });

  it("stays within the 48-character slug ceiling even after a suffix", () => {
    const long = "a".repeat(60);
    const slug = slugify(long, [long.slice(0, 48)]);
    expect(slug.length).toBeLessThanOrEqual(48);
    expect(slug.endsWith("-2")).toBe(true);
  });
});

describe("buildLedgerSpec", () => {
  it("always prepends id, title and status fields", () => {
    const spec = buildLedgerSpec(draft());
    expect(spec.fields.slice(0, 3)).toEqual([
      { name: "id", role: "id" },
      { name: "title", role: "title", required: true },
      { name: "status", role: "status", required: true },
    ]);
  });

  it("auto-adds a reason field exactly when a status needs one", () => {
    const withReason = buildLedgerSpec(draft());
    expect(withReason.fields.some((f) => f.name === "reason" && f.role === "prose")).toBe(true);

    const withoutReason = buildLedgerSpec(
      draft({ statuses: stagePreset("todo-in-progress-done")!.statuses }),
    );
    expect(withoutReason.fields.some((f) => f.name === "reason")).toBe(false);
  });

  it("appends chosen preset and custom fields after the required three", () => {
    const owner = FIELD_PRESETS.find((p) => p.id === "owner")!.field;
    const custom = { name: "vertical", role: "prose" as const, description: "Which market" };
    const spec = buildLedgerSpec(draft({ fields: [owner, custom] }));
    expect(spec.fields).toEqual([
      { name: "id", role: "id" },
      { name: "title", role: "title", required: true },
      { name: "status", role: "status", required: true },
      { name: "owner", role: "owner" },
      { name: "vertical", role: "prose", description: "Which market" },
      { name: "reason", role: "prose" },
    ]);
  });

  it("keeps statuses in snake_case needs_reason, matching the wire shape", () => {
    const spec = buildLedgerSpec(draft());
    expect(spec.statuses).toEqual([
      { name: "open", closed: false, needs_reason: false },
      { name: "closed", closed: true, needs_reason: true },
    ]);
  });

  it("groups open statuses into Outstanding and closed ones into Settled", () => {
    const spec = buildLedgerSpec(draft());
    expect(spec.sections.map((s) => s.heading)).toEqual(["Outstanding", "Settled"]);
    expect(spec.sections[0].statuses).toEqual(["open"]);
    expect(spec.sections[1].statuses).toEqual(["closed"]);
  });

  it("omits a section with nothing to group", () => {
    const spec = buildLedgerSpec(
      draft({ statuses: [{ name: "active", closed: false, needs_reason: false }] }),
    );
    expect(spec.sections.map((s) => s.heading)).toEqual(["Outstanding"]);
  });

  it("fixes checks to the three the engine defines", () => {
    const spec = buildLedgerSpec(draft());
    expect(spec.checks).toEqual(WIZARD_CHECKS);
  });

  it("passes slug/title/purpose through unchanged", () => {
    const spec = buildLedgerSpec(draft());
    expect(spec.slug).toBe("customer-promises");
    expect(spec.title).toBe("Customer promises");
    expect(spec.purpose).toBe("What we told a customer we would do.");
  });
});

describe("presets", () => {
  it("ships exactly the two stage presets plus custom is left to the caller", () => {
    expect(STAGE_PRESETS.map((p) => p.id)).toEqual(["todo-in-progress-done", "open-closed"]);
  });

  it("the todo/in-progress/done preset does not demand a reason", () => {
    const preset = stagePreset("todo-in-progress-done")!;
    expect(preset.statuses.every((s) => !s.needs_reason)).toBe(true);
    expect(preset.statuses.some((s) => s.closed)).toBe(true);
  });

  it("the open/closed preset demands a reason on close", () => {
    const preset = stagePreset("open-closed")!;
    const closing = preset.statuses.find((s) => s.closed)!;
    expect(closing.needs_reason).toBe(true);
  });
});

describe("summarize", () => {
  it("reads the same draft buildLedgerSpec posts", () => {
    const text = summarize(draft({ fields: [{ name: "owner", role: "owner" }] }));
    expect(text).toContain("Customer promises");
    expect(text).toContain("open → closed");
    expect(text).toContain("owner");
  });
});
