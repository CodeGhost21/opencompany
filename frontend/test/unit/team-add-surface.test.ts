// The Add-teammate dialog's branch, and the derivation the reduced one depends
// on (issue #1989).
//
// The branch is the thing this redesign lives or dies on, and its failure is
// silent in one direction: answer `form` on a company whose copilot works and
// the dialog looks exactly as it did before, so nothing anywhere reports that
// the reduction never shipped. A rendered test can only prove the cases
// somebody thought to render; the decision is a pure function so every input
// can be.

import { describe, expect, it } from "vitest";

import type { CognitionPath } from "@/api/inference";
import {
  addTeammateSurface,
  describedTeammateFields,
  roleFromDescription,
} from "@/lib/team-add-surface";

/** Every cognition path the host can report, so a new one is never silently untested. */
const PATHS: CognitionPath[] = ["harness", "hosted", "sidecar", "echo", "custom", "test"];

describe("addTeammateSurface", () => {
  it("shows the reduced dialog for every cognition path that is not the offline brain", () => {
    for (const cognition of PATHS.filter((p) => p !== "echo")) {
      expect(
        addTeammateSurface({ cognition, roleUnderivable: false }),
        `cognition=${cognition} can draft, so the dialog must be the reduced one`,
      ).toBe("describe");
    }
  });

  it("shows the reduced dialog while the cognition read has not landed", () => {
    // `null` is both "in flight" and "this host has no /inference route". Issue
    // #753 leaves the copilot ENABLED in that case rather than refusing because
    // it could not confirm, and this follows it: guessing `describe` wrong is
    // corrected out loud on the detail page, where guessing `form` wrong is
    // corrected by nothing at all.
    expect(addTeammateSurface({ cognition: null, roleUnderivable: false })).toBe("describe");
  });

  it("shows the full form on the offline brain", () => {
    // The operator's decision on #1988, applied here: the can't-draft path keeps
    // today's form, so a company with no model is never locked out of writing a
    // description that nothing downstream could draft for it.
    expect(addTeammateSurface({ cognition: "echo", roleUnderivable: false })).toBe("form");
  });

  it("hands over the full form once the sentence yielded no role", () => {
    // The reduced dialog's one dead end. A blank role must never be written, so
    // the operator gets every field rather than a Create that cannot work.
    expect(addTeammateSurface({ cognition: "harness", roleUnderivable: true })).toBe("form");
  });

  it("keeps the full form on the offline brain even before any Create", () => {
    // Both reasons at once must not cancel out.
    expect(addTeammateSurface({ cognition: "echo", roleUnderivable: true })).toBe("form");
  });
});

describe("roleFromDescription", () => {
  it("stores the operator's sentence entire, without its full stop", () => {
    // The whole sentence, not a clause of it. "Runs paid acquisition" was what
    // the clause split produced here, and it drops the half of the job the
    // operator bothered to write down.
    expect(roleFromDescription("Runs paid acquisition and reports on ROAS.")).toBe(
      "Runs paid acquisition and reports on ROAS",
    );
    expect(roleFromDescription("Runs paid acquisition, and reports on ROAS.")).toBe(
      "Runs paid acquisition, and reports on ROAS",
    );
    expect(roleFromDescription("Writes the weekly digest; emails it on Monday.")).toBe(
      "Writes the weekly digest; emails it on Monday",
    );
  });

  it("keeps the job when the sentence opens with when rather than what", () => {
    // Verified live before this fix: the first clause is where English puts the
    // *when*, so this stored the role "Every Monday" and the whole job — the
    // thing the roster card, the persona line and the delegation Team block all
    // read — was thrown away. No split can tell an adverbial from a job title.
    const role = roleFromDescription(
      "Every Monday, reconciles the ad spend against the invoices.",
    );
    expect(role).toBe("Every Monday, reconciles the ad spend against the invoices");
    expect(role).toContain("reconciles the ad spend");
  });

  it("reads a description with no ASCII clause break at all", () => {
    // The regression that made this urgent, reproduced end to end against a
    // live host: no comma, no semicolon, so the clause split found nothing to
    // cut on and the 60-character cap minted
    // "Runs wholesale outreach to boutique retailers and keeps the…" as a
    // permanent job title, ellipsis included.
    const role = roleFromDescription(
      "Runs wholesale outreach to boutique retailers and keeps the stockist pipeline warm.",
    );
    expect(role).toBe(
      "Runs wholesale outreach to boutique retailers and keeps the stockist pipeline warm",
    );
    expect(role).not.toContain("…");
  });

  it("reads a CJK description, which the ASCII clause split could not", () => {
    // `，。、；！？` were absent from the old character class, so a Chinese or
    // Japanese description never split and became a 60-character cut. It is not
    // split now either — it is simply kept whole, which is the right answer for
    // every script at once rather than one more class of punctuation to miss.
    expect(roleFromDescription("负责批发外联，维护零售商渠道。")).toBe(
      "负责批发外联，维护零售商渠道",
    );
  });

  it("reads a description that opens with punctuation", () => {
    // `split(sep, 1)` takes the FIRST element, not the first non-empty one, so
    // a leading delimiter derived "" from a sentence with a plain job in it and
    // bounced the operator to the full form for nothing.
    expect(roleFromDescription("\n\nKeeps the stockist pipeline warm")).toBe(
      "Keeps the stockist pipeline warm",
    );
    expect(roleFromDescription("— Keeps the stockist pipeline warm")).toBe(
      "— Keeps the stockist pipeline warm",
    );
  });

  it("capitalises and collapses whitespace", () => {
    expect(roleFromDescription("  growth   marketer  ")).toBe("Growth marketer");
    // Not "Owns": a newline is whitespace, not a clause boundary.
    expect(roleFromDescription("owns\nthe backlog")).toBe("Owns the backlog");
  });

  it("refuses to truncate, answering empty instead", () => {
    // The heart of this function. A truncated role is worse than no role: no
    // role is a question the operator gets asked, and a truncated one is a
    // permanent record they were never shown. Nothing that comes back may be a
    // piece of what went in.
    const long = "a".repeat(200);
    expect(roleFromDescription(long)).toBe("");
    // And the boundary is a real bound, not a cut point.
    expect(roleFromDescription("b".repeat(120))).toHaveLength(120);
    expect(roleFromDescription("b".repeat(121))).toBe("");
    // Two sentences about a job are a description, not a role.
    expect(
      roleFromDescription(
        "Runs wholesale outreach to boutique retailers and keeps the stockist pipeline " +
          "warm. Reports on reorder rates every month, by account.",
      ),
    ).toBe("");
  });

  it("never answers with an ellipsis, for any input", () => {
    // The teeth on the rule above: `…` in a stored role can only have come from
    // this function cutting one.
    for (const description of [
      "a".repeat(59),
      "a".repeat(200),
      "Runs paid acquisition and reports on ROAS.",
      "Every Monday, reconciles the ad spend against the invoices.",
      "负责批发外联，维护零售商渠道。",
    ]) {
      expect(roleFromDescription(description)).not.toContain("…");
    }
  });

  it("answers empty when the sentence has nothing usable", () => {
    // The caller MUST treat this as "no role derived" and hand over the form. A
    // blank role breaks the teammate's own system prompt (`persona_prompt`
    // interpolates it unguarded), empties the orchestrator's Team block, and
    // switches off the copilot on the very page the create redirects to.
    expect(roleFromDescription("")).toBe("");
    expect(roleFromDescription("   ")).toBe("");
    expect(roleFromDescription(".,;")).toBe("");
    expect(roleFromDescription("\n\n")).toBe("");
    // Emoji are not a job title. The old implementation stored "🎉🎉" as one —
    // its own doc said it returned "" here and it did not.
    expect(roleFromDescription("🎉🎉")).toBe("");
  });
});

describe("describedTeammateFields", () => {
  it("derives the role and trims what the operator typed", () => {
    expect(
      describedTeammateFields({
        name: "  Nova  ",
        description: "  Runs paid acquisition, reports on ROAS.  ",
      }),
    ).toEqual({
      name: "Nova",
      role: "Runs paid acquisition, reports on ROAS",
      description: "Runs paid acquisition, reports on ROAS.",
    });
  });

  it("refuses a teammate with no name", () => {
    // The id is slugged from the name host-side (`mint_agent_id`), and nothing
    // in this path can derive one: `DraftableField` excludes `name` on purpose,
    // so there is no model to ask.
    expect(
      describedTeammateFields({ name: "   ", description: "Runs paid acquisition." }),
    ).toBeNull();
  });

  it("refuses a teammate with no description", () => {
    expect(describedTeammateFields({ name: "Nova", description: "  " })).toBeNull();
  });

  it("refuses a teammate whose description yields no role", () => {
    // The case the hand-over exists for: a real name, a non-empty description,
    // and nothing in it a person could read as a job.
    //
    // The first line asserted `.not.toBeNull()` before this change, under this
    // same title — the test encoded the bug and the title described the fix, so
    // reading either one alone told you the opposite of the truth. "🎉🎉" really
    // did create a teammate whose stored role was two party poppers.
    expect(describedTeammateFields({ name: "Nova", description: "🎉🎉" })).toBeNull();
    expect(describedTeammateFields({ name: "Nova", description: "..." })).toBeNull();
    expect(describedTeammateFields({ name: "Nova", description: "!?!" })).toBeNull();
  });

  it("refuses a description too long to be anybody's job title", () => {
    // The other half of the hand-over, and the common one: an ordinary two
    // sentence answer to "what should they do?". It is not truncated into a
    // role — the operator is asked for one.
    expect(
      describedTeammateFields({
        name: "Sable",
        description:
          "Runs wholesale outreach to boutique retailers and keeps the stockist " +
          "pipeline warm. Reports on reorder rates every month, by account.",
      }),
    ).toBeNull();
  });

  it("never returns a blank role", () => {
    // The invariant the whole module exists to hold. Anything that comes back
    // non-null is safe to POST.
    for (const description of [
      "Nova",
      "a",
      "🎉",
      "Runs ads.",
      "  x  ",
      "...",
      "",
      "!?",
      "\n\nRuns ads",
      "负责批发外联。",
      "a".repeat(500),
    ]) {
      const fields = describedTeammateFields({ name: "Nova", description });
      if (fields) {
        expect(fields.role.trim()).not.toBe("");
        // And never a piece of the description: a stored role is either what
        // the operator wrote or it does not exist.
        expect(fields.role).not.toContain("…");
        expect(description).toContain(fields.role.slice(1));
      }
    }
  });
});
