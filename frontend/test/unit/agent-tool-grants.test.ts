import { describe, expect, it } from "vitest";

import { companyCovers, grantCeiling, isEditable, parseToolGlobs, toolGlobsDiffer } from "@/lib/agent";
import type { AgentDetailDto } from "@/api/types";

/**
 * The Tools card became an editor, and everything it decides before sending a
 * `PATCH` is decided here.
 *
 * The card used to be a read-only report: it showed what a teammate asked for,
 * what the company allowed, and — struck through — what had been dropped
 * between the two, with no way to act on any of it. The host has accepted a
 * `tools` key on the agent patch for admins the whole time. These are the
 * derivations that stand between an operator's typing and that write, and each
 * fails silently rather than loudly when it is wrong: a mis-split glob is
 * stored happily and confers nothing, and a coverage hint that lies sends
 * somebody to edit the wrong list.
 */

function agent(over: Partial<AgentDetailDto> = {}): AgentDetailDto {
  return {
    id: "jamie",
    name: "Jamie",
    role: "Growth",
    description: "Runs paid acquisition.",
    source: "overlay",
    editable: ["name", "role", "description", "instructions", "tools"],
    isOrchestrator: false,
    tools: { requested: [], companyAllow: ["*"], deskAllow: [], effective: ["*"] },
    desks: [],
    inboxEnabled: false,
    ...over,
  };
}

describe("parseToolGlobs", () => {
  it("splits on commas and whitespace alike", () => {
    // Both spellings are what people type after reading a `company.toml`.
    // Splitting on commas alone would store a grant literally named `docs.*,`,
    // which matches nothing and reports as "asked for but not granted".
    expect(parseToolGlobs("docs.*, files.*")).toEqual(["docs.*", "files.*"]);
    expect(parseToolGlobs("docs.*  files.*")).toEqual(["docs.*", "files.*"]);
    expect(parseToolGlobs("docs.*,files.*\nsearch")).toEqual(["docs.*", "files.*", "search"]);
  });

  it("collapses duplicates and keeps first-seen order", () => {
    expect(parseToolGlobs("search, docs.*, search")).toEqual(["search", "docs.*"]);
  });

  it("reads a blank field as an empty list, which is the standard grant", () => {
    // The inversion the card warns about: `[]` means "everything the company
    // allows", so this must not be confused with a parse failure.
    expect(parseToolGlobs("   ")).toEqual([]);
    expect(parseToolGlobs(",, ,")).toEqual([]);
  });
});

describe("toolGlobsDiffer", () => {
  it("ignores re-ordering and re-spacing", () => {
    expect(toolGlobsDiffer(["a", "b"], parseToolGlobs("b, a"))).toBe(false);
  });

  it("notices an addition, a removal, and a clear", () => {
    expect(toolGlobsDiffer(["a"], ["a", "b"])).toBe(true);
    expect(toolGlobsDiffer(["a", "b"], ["a"])).toBe(true);
    expect(toolGlobsDiffer(["a"], [])).toBe(true);
  });

  it("treats a duplicate stored entry as the same grant as its single entry", () => {
    // The stored list is never re-parsed, so a manifest may hold `["search",
    // "search"]` while the editor's parsed view collapses it to `["search"]`.
    // Comparing raw lengths would make the card claim a change and rewrite the
    // stored list even though the grant set is identical.
    expect(toolGlobsDiffer(["search", "search"], ["search"])).toBe(false);
    expect(toolGlobsDiffer(["search", "docs.*", "search"], ["search", "docs.*"])).toBe(false);
  });
});

describe("companyCovers", () => {
  it("treats the catch-all as covering the ordinary families", () => {
    expect(companyCovers(["*"], "docs.*")).toBe(true);
    expect(companyCovers(["*"], "workspace.read")).toBe(true);
    expect(companyCovers(["*"], "workspace.write")).toBe(true);
  });

  it("does not let the catch-all cover the explicit opt-in namespaces", () => {
    // The host's `allow_covers` rejects these under a bare `*`, and the hint
    // must agree or it would promise a grant that never lands. A dotted
    // descendant ask is as much an opt-in as the bare namespace — it must not
    // fall through to the generic matcher, where the wildcard would cover it.
    expect(companyCovers(["*"], "search")).toBe(false);
    expect(companyCovers(["*"], "search.web")).toBe(false);
    expect(companyCovers(["*"], "media")).toBe(false);
    expect(companyCovers(["*"], "media.image")).toBe(false);
    expect(companyCovers(["*"], "composio")).toBe(false);
    expect(companyCovers(["*"], "composio.gmail")).toBe(false);
    expect(companyCovers(["*"], "chargebee")).toBe(false);
    expect(companyCovers(["*"], "chargebee.read")).toBe(false);
    expect(companyCovers(["*"], "hosting")).toBe(false);
    expect(companyCovers(["*"], "hosting.deploy")).toBe(false);
    expect(companyCovers(["*"], "paypal")).toBe(false);
    expect(companyCovers(["*"], "paypal.wallet")).toBe(false);
    expect(companyCovers(["*"], "mcp:*")).toBe(false);
    expect(companyCovers(["*"], "mcp:notion")).toBe(false);
  });

  it("covers an explicit opt-in only from a grant that names it", () => {
    expect(companyCovers(["search"], "search")).toBe(true);
    expect(companyCovers(["search.*"], "search.web")).toBe(true);
    expect(companyCovers(["media"], "media")).toBe(true);
    expect(companyCovers(["media.*"], "media")).toBe(true);
    expect(companyCovers(["composio"], "composio")).toBe(true);
    expect(companyCovers(["chargebee"], "chargebee.read")).toBe(true);
    expect(companyCovers(["hosting"], "hosting.deploy")).toBe(true);
    expect(companyCovers(["paypal.wallet"], "paypal")).toBe(true);
    expect(companyCovers(["mcp:*"], "mcp:notion")).toBe(true);
    expect(companyCovers(["mcp:notion"], "mcp:notion")).toBe(true);
    // …but a *different* namespace does not confer it.
    expect(companyCovers(["media.generation"], "composio")).toBe(false);
    // …while the opt-in predicate accepts any sub-grant of the namespace, so
    // `search.web` does confer a bare `search` request — unlike the generic
    // matcher, where `docs.read` would not confer `docs`.
    expect(companyCovers(["search.web"], "search")).toBe(true);
    // …and the bare namespace grant covers its dotted descendants, matching
    // `grants_search_explicit` (which `search` and `search.web` both satisfy).
    expect(companyCovers(["search"], "search.web")).toBe(true);
    expect(companyCovers(["media"], "media.image")).toBe(true);
    expect(companyCovers(["composio"], "composio.gmail")).toBe(true);
    expect(companyCovers(["chargebee"], "chargebee.read")).toBe(true);
    expect(companyCovers(["hosting.*"], "hosting.deploy")).toBe(true);
    expect(companyCovers(["paypal"], "paypal.wallet")).toBe(true);
  });

  it("covers a sub-grant from a starred namespace", () => {
    expect(companyCovers(["workspace.*"], "workspace.read")).toBe(true);
  });

  it("does not cover a bare namespace from its starred form", () => {
    // The asymmetry that makes manifests list `"workspace", "workspace.*"` as
    // two entries. It reads like a bug and is the host's actual rule, so the
    // hint has to have it too or it would promise a grant that never lands.
    expect(companyCovers(["workspace.*"], "workspace")).toBe(false);
  });

  it("stops a prefix that does not end on a separator", () => {
    // `documentation.read` is not a `docs` grant, however much of the string
    // lines up — and an unstarred grant matches only itself.
    expect(companyCovers(["docs*"], "documentation.read")).toBe(false);
    // And an unstarred grant matches only itself, sub-grants included — the
    // same rule that makes a manifest list `"workspace", "workspace.*"`. The
    // opt-in namespaces are the exception, exercised in the test above.
    expect(companyCovers(["docs"], "docs.read")).toBe(false);
  });

  it("reports an uncovered ask, which is the whole warning", () => {
    // `*` covers the ordinary families but not the opt-ins, so a company
    // allowing it still has to name `search` before a teammate can hold it.
    expect(companyCovers(["*", "media"], "search")).toBe(false);
    expect(companyCovers(["docs.*", "files.*"], "search")).toBe(false);
    expect(companyCovers([], "docs.*")).toBe(false);
  });
});

describe("grantCeiling", () => {
  it("is the company allow-list when no desk states a ceiling", () => {
    // `deskAllow` empty means no desk narrows anything — the same
    // empty-is-not-nothing trap `requested` carries, not "this desk grants no
    // tools". The company list is the whole gate.
    expect(
      grantCeiling({
        requested: [],
        companyAllow: ["*", "media"],
        deskAllow: [],
        effective: ["*", "media"],
      }),
    ).toEqual(["*", "media"]);
  });

  it("is the desk allowance when a desk states a ceiling", () => {
    // The marketing agency's creative desk omits `media` while the company
    // allows it, so the desk allowance (already company-narrowed on the host)
    // is the gate an editor draft has to clear.
    expect(
      grantCeiling({
        requested: [],
        companyAllow: ["*", "media"],
        deskAllow: ["*"],
        effective: ["*"],
      }),
    ).toEqual(["*"]);
  });

  it("lets the desk ceiling narrow what an editor warns about", () => {
    // The whole point of the preview: a grant the company allows but the desk
    // omits is stored happily and then dropped immediately after saving, so the
    // live hint must flag it while typing. `media` is company-allowed but not
    // on the creative desk, so `companyCovers` against the desk allowance says
    // it will not apply — exactly what `willNotApply` renders.
    const ceiling = grantCeiling({
      requested: [],
      companyAllow: ["*", "media"],
      deskAllow: ["*"],
      effective: ["*"],
    });
    expect(companyCovers(ceiling, "media")).toBe(false);
    expect(companyCovers(ceiling, "docs.*")).toBe(true);
  });
});

describe("isEditable", () => {
  it("accepts `tools`, which the host lists but no form field carries", () => {
    expect(isEditable(agent(), "tools")).toBe(true);
    // A member gets every other key and not this one — the host gates `tools`
    // on admin because an empty list is a potential widening.
    expect(isEditable(agent({ editable: ["name", "role", "description"] }), "tools")).toBe(false);
  });
});
