import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { defaultDesks, GENERAL_CHANNEL, isGeneralChannel, type Desk } from "@/lib/desks";
import type { TeamMember } from "@/lib/team";
import { buildChannels, channelIdForThread, channelMembers } from "@/views/chat/model";

/**
 * The built-in `#general` channel (issue #1743).
 *
 * The defect it closes is narrow and easy to miss: `#general` existed only in
 * `defaultDesks()`, the **fallback** set used when the host exposes no desks at
 * all. So the moment a company had real desks — which is every shipped company
 * — the company-wide line vanished from the rail, and there was nowhere to
 * address everyone.
 *
 * Three properties are pinned here, and each is a requirement rather than a
 * rendering detail:
 *
 * 1. it is present whatever the host's desk list says, including when that list
 *    is long and real;
 * 2. its membership is the roster, derived on every render, with nothing
 *    written anywhere — so a teammate added a moment ago is in it;
 * 3. it is not a desk, and carries no desk affordance, because it never reaches
 *    the surfaces that offer them.
 */

function member(over: Partial<TeamMember> & Pick<TeamMember, "id" | "name">): TeamMember {
  return {
    role: "Engineer",
    description: "",
    tone: "sky",
    avatar: "green",
    inboxEnabled: false,
    effectiveTools: [],
    desks: [],
    ...over,
  };
}

const ROSTER: TeamMember[] = [
  member({ id: "ceo", name: "Ada", role: "Chief", isOrchestrator: true }),
  member({ id: "eng", name: "Blake", role: "Engineer" }),
];

const DESKS: Desk[] = [
  { id: "engineering", channel: "engineering", name: "Engineering", blurb: "", members: ["eng"] },
  { id: "growth", channel: "growth", name: "Growth", blurb: "", members: ["ceo"] },
];

function channels(members: TeamMember[], desks: Desk[]) {
  return buildChannels(members, desks, {}).find((s) => s.id === "channels")!.channels;
}

describe("the built-in #general channel", () => {
  it("is the first channel in a company that has real desks", () => {
    const rail = channels(ROSTER, DESKS);
    expect(rail.map((c) => c.name)).toEqual([GENERAL_CHANNEL, "engineering", "growth"]);
    expect(rail[0].kind).toBe("channel");
  });

  it("replaces the static fallback's main row rather than sitting beside it", () => {
    const rail = channels(ROSTER, defaultDesks());
    expect(rail.filter((c) => c.name === GENERAL_CHANNEL)).toHaveLength(1);
    // And the one that survived is the derived channel, not the members-less
    // fallback row.
    expect(rail[0].memberIds).toEqual(["ceo", "eng"]);
  });

  it("steps aside for a blueprint desk that authored the id `general`", () => {
    // The host grandfathers this: `is_general_channel` is guarded on
    // `!record.desk_exists`, so such a desk keeps its lead and its writes and
    // `responder_for` routes to that lead. Adding the built-in one beside it
    // put two `#general` rows in the rail folding onto one transcript — the
    // host's `is_general_chat` treats `main` and `general` as one conversation
    // — while a send could pick either responder.
    const authored: Desk[] = [
      { id: "general", channel: "general", name: "Ops lead", blurb: "The line", members: ["ceo"] },
      ...DESKS,
    ];
    const rail = channels(ROSTER, authored);
    expect(rail.filter((c) => isGeneralChannel(c.id))).toHaveLength(1);
    expect(rail.map((c) => c.id)).toEqual(["general", "engineering", "growth"]);
    // And it is the real desk that survived: its lead, its blurb, its members.
    expect(rail[0].voice).toBe("Ops lead");
    expect(rail[0].memberIds).toEqual(["ceo"]);
  });

  it("keeps a blueprint desk that authored the id `main` instead of hiding it", () => {
    // The reverse failure: this desk was filtered out of the rail, so the
    // built-in channel took the slot and named the orchestrator as who answers
    // — while the host still routed `main` to this desk's lead, because
    // `responder_for` checks desks first. The UI both hid a real desk and
    // misstated the responder.
    const authored: Desk[] = [
      { id: "main", channel: "general", name: "Front office", blurb: "The line", members: ["eng"] },
      ...DESKS,
    ];
    const rail = channels(ROSTER, authored);
    expect(rail.filter((c) => isGeneralChannel(c.id))).toHaveLength(1);
    expect(rail.map((c) => c.id)).toEqual(["main", "engineering", "growth"]);
    expect(rail[0].voice).toBe("Front office");
    expect(rail[0].memberIds).toEqual(["eng"]);
    // Not the derived channel's claim about who picks up an unmentioned message.
    expect(rail[0].purpose).toBe("The line");
  });

  it("has no fabricated general desk left in the fallback set to be confused for one", () => {
    // The rule above is "a desk claims the line". That is only a fact about the
    // company if the console has stopped inventing such a desk itself.
    expect(defaultDesks().some((d) => isGeneralChannel(d.id))).toBe(false);
  });

  it("is present on a company with no desks at all", () => {
    expect(channels(ROSTER, [])[0].name).toBe(GENERAL_CHANNEL);
  });

  it("holds the whole roster, derived — a teammate added later is in it", () => {
    const before = channels(ROSTER, DESKS)[0];
    expect(channelMembers(before, ROSTER)!.map((m) => m.id)).toEqual(["ceo", "eng"]);

    const grown = [...ROSTER, member({ id: "designer", name: "Cass", role: "Designer" })];
    const after = channels(grown, DESKS)[0];
    expect(channelMembers(after, grown)!.map((m) => m.id)).toEqual(["ceo", "eng", "designer"]);

    // Nothing about the desk list changed to make that true.
    expect(DESKS.map((d) => d.members)).toEqual([["eng"], ["ceo"]]);
  });

  it("names the orchestrator as who picks up an unmentioned message", () => {
    expect(channels(ROSTER, DESKS)[0].purpose).toBe(
      "Everyone's here. Ada picks up anything you don't @-mention.",
    );
  });

  it("makes no claim about who answers when the host does not say", () => {
    const silent = ROSTER.map((m) => ({ ...m, isOrchestrator: undefined }));
    expect(channels(silent, DESKS)[0].purpose).toBe(
      "Everyone's here — the whole company on one line",
    );
  });

  it("carries no overlay membership, so no surface can offer a remove", () => {
    // `overlayMembers` is what the console reads to decide a member is
    // removable. A desk row has it; the built-in channel has no such concept —
    // the `Channel` shape it produces carries only ids.
    const general = channels(ROSTER, DESKS)[0];
    expect(Object.keys(general).sort()).toEqual(
      ["id", "kind", "memberIds", "name", "purpose", "voice"].sort(),
    );
  });

  it("is absent from the desk list every desk affordance is built from", () => {
    // The host does not list it under `GET .../desks`, so the org chart, the
    // assignee picker and the desk counts never see it. This asserts the
    // console does not put it back: `buildChannels` composes it for the rail
    // and returns a `Channel`, never a `Desk`.
    expect(DESKS.some((d) => isGeneralChannel(d.id))).toBe(false);
  });
});

describe("resolving a host thread to the general channel", () => {
  it("folds every spelling the host journals it under", () => {
    for (const spelling of ["", "main", "General", "general", "  MAIN  "]) {
      expect(channelIdForThread(spelling, DESKS, ROSTER)).toBe("main");
    }
  });

  it("leaves a real desk thread alone", () => {
    expect(channelIdForThread("engineering", DESKS, ROSTER)).toBe("engineering");
  });

  it("still resolves a teammate DM", () => {
    expect(channelIdForThread("eng", DESKS, ROSTER)).toBe("dm:eng");
  });

  it("lets a blueprint desk that authored a general id keep its own thread", () => {
    const authored: Desk[] = [
      { id: "general", channel: "general", name: "General", blurb: "", members: ["ceo"] },
    ];
    expect(channelIdForThread("general", authored, ROSTER)).toBe("general");
  });
});

describe("isGeneralChannel", () => {
  it("matches exactly the four spellings the host folds", () => {
    for (const yes of ["", "main", "Main", "MAIN", "general", "General", " general "]) {
      expect(isGeneralChannel(yes)).toBe(true);
    }
    for (const no of ["engineering", "generals", "main-line", "dm:main"]) {
      expect(isGeneralChannel(no)).toBe(false);
    }
  });
});

/**
 * The two desk affordances `ChatView` derives from a channel, and why neither
 * may reach the built-in one.
 *
 * A full `ChatView` render needs the whole client and every hook, so this uses
 * the source-contract idiom `chat-rail-focus.test.ts` established for exactly
 * that case: pin the wiring the behaviour rests on. The behaviour itself is
 * verified in a browser — see the PR.
 *
 * Both gates matter because `#general` is the one channel that carries
 * `memberIds` **without** being a desk. Every previous non-desk channel (a DM,
 * a static fallback desk) was excluded by having no `memberIds` at all, so both
 * tests read as "has membership ⇒ is a desk" — an inference that is true of
 * every channel except this one.
 */
describe("ChatView offers no desk affordance on the built-in channel", () => {
  const here = dirname(fileURLToPath(import.meta.url));
  const chatView = readFileSync(resolve(here, "../../src/views/ChatView.tsx"), "utf8");
  // Collapsed so an assertion pins the wiring rather than the line wrapping
  // Prettier happens to choose for it.
  const source = chatView.replace(/\s+/g, " ");

  it("decides by the desk list, not by the id's spelling", () => {
    // Both affordances hang off this one predicate. Asking the desk list is
    // what makes the built-in channel excluded for the right reason — and what
    // keeps a blueprint desk that authored a General id from being hidden with
    // it, since the host grandfathers that desk and the org chart holds it.
    expect(source).toContain(
      'const activeIsDesk = active.kind === "channel" && (desks ?? []).some((d) => d.id === active.id);',
    );
  });

  it("does not badge anyone its lead", () => {
    // `memberIds[0]` is the roster's first row here, not a hierarchy.
    expect(source).toContain("activeIsDesk ? active.memberIds?.[0] : undefined }");
  });

  it("does not offer the org-chart link that would open on a desk that does not exist", () => {
    expect(source).toContain("onManageDesk={ activeIsDesk && active.memberIds");
  });

  it("no longer needs the id predicate at all", () => {
    expect(source).toContain('import { defaultDesks, type Desk } from "@/lib/desks";');
    expect(source).not.toContain("isGeneralChannel(active.id)");
  });
});

/**
 * The shell's host-thread → channel map, and the line #1743 had to change.
 *
 * `channelMap` used to seed `map[MAIN_THREAD_ID] = desks[0].id`: with no
 * `#general` channel to land in, the company's main line was parked on
 * whichever desk sorted first so it would at least be somewhere findable.
 * That is now wrong in a way a browser makes obvious — an unaddressed message
 * and its reply rendered inside `#engineering`, with an unread badge, while
 * the host's own history for that desk was empty.
 *
 * The map is module-private to `app-shell.tsx`, so this pins the wiring the
 * same way the `ChatView` block above does.
 */
describe("the shell maps the main line to #general, not to the first desk", () => {
  const here2 = dirname(fileURLToPath(import.meta.url));
  const shell = readFileSync(resolve(here2, "../../src/components/app-shell.tsx"), "utf8").replace(
    /\s+/g,
    " ",
  );

  it("no longer parks the main line on the first desk", () => {
    expect(shell).not.toContain("map[MAIN_THREAD_ID] = desks[0].id");
  });

  it("maps every spelling the host journals the general line under", () => {
    expect(shell).toContain(
      'for (const spelling of ["", MAIN_THREAD_ID, "General", GENERAL_CHANNEL]) { map[spelling] = MAIN_THREAD_ID; }',
    );
  });

  it("lands an unaddressed system line in #general rather than the first desk", () => {
    expect(shell).toContain("setFirstDeskChannelId(MAIN_THREAD_ID);");
  });

  it("names #general as a rehydration target, since it is in no desk list", () => {
    expect(shell).toContain("{ channelId: MAIN_THREAD_ID, threadId: MAIN_THREAD_ID },");
  });
});
