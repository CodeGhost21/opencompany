// The Add-teammate dialog's branch, and what the reduced one writes once the
// host has designed it (issue #1989).
//
// The branch is the thing this redesign lives or dies on, and its failure is
// silent in one direction: answer `form` on a company whose copilot works and
// the dialog looks exactly as it did before, so nothing anywhere reports that
// the reduction never shipped. A rendered test can only prove the cases
// somebody thought to render; the decision is a pure function so every input
// can be.
//
// `roleFromDescription` used to live here with a suite of its own. It is gone,
// and the tests that pinned its behaviour went with it, because the behaviour
// was the defect: it took the first clause of the operator's sentence and cut
// it at sixty characters with an ellipsis, and the result was **stored** as the
// teammate's permanent job title. The role, the mandate and the persona now
// come from one host-side design pass, and what is left to test on this side is
// the rule that nothing part-designed is ever written.

import { describe, expect, it } from "vitest";

import type { TeammateDesign } from "@/api/agent-copilot";
import type { CognitionPath } from "@/api/inference";
import {
  addTeammateSurface,
  carriedDescribe,
  describeBlocked,
  designedTeammateFields,
  heldFields,
} from "@/lib/team-add-surface";

/** Every cognition path the host can report, so a new one is never silently untested. */
const PATHS: CognitionPath[] = ["harness", "hosted", "sidecar", "echo", "custom", "test"];

/** A design as the host returns one when the pass worked. */
const DESIGNED: TeammateDesign = {
  role: "Wholesale Account Manager",
  description: "Owns the stockist pipeline and the terms behind it.",
  instructions: "Check terms against the price list before quoting. Escalate anything under 40% margin.",
  source: "model",
};

describe("addTeammateSurface", () => {
  it("shows the reduced dialog for every non-echo path a host has not ruled out", () => {
    // `designsProfiles` absent is an older host that does not report the
    // capability. The cognition label alone is all there is to go on there, and
    // this is the behaviour that shipped.
    for (const cognition of PATHS.filter((p) => p !== "echo")) {
      expect(
        addTeammateSurface({ cognition, designRefused: false }),
        `cognition=${cognition} is not ruled out, so the dialog must be the reduced one`,
      ).toBe("describe");
    }
  });

  it("shows the full form on any path where the host says no design pass can run", () => {
    // The bug this input exists for. The console used to answer this question
    // itself, as `cognition !== "echo"`, and it is wrong for three of the six:
    // `profile_drafter()` is built from `workflow_harness_deps`, assigned in
    // exactly one place — the embedded harness arm of `RuntimeBuilder::build` —
    // so a `hosted`, `sidecar` or `custom` company has no drafter either. Every
    // create on one of them was a sentence typed, a Create pressed, a model
    // call waited on that could only answer `no_model`, and the full form
    // anyway.
    for (const cognition of PATHS) {
      expect(
        addTeammateSurface({ cognition, designsProfiles: false, designRefused: false }),
        `cognition=${cognition} cannot design, so the reduced dialog is a dead end`,
      ).toBe("form");
    }
  });

  it("keeps the reduced dialog when the host says a design pass CAN run", () => {
    for (const cognition of PATHS.filter((p) => p !== "echo")) {
      expect(addTeammateSurface({ cognition, designsProfiles: true, designRefused: false })).toBe(
        "describe",
      );
    }
    // `echo` still wins: there is no model on that path at all, and that is
    // true from the label alone.
    expect(addTeammateSurface({ cognition: "echo", designsProfiles: true, designRefused: false })).toBe(
      "form",
    );
  });

  it("treats an unreported capability as unknown, not as a refusal", () => {
    // `undefined` is an older host; `null` is a check still in flight. Both are
    // read the way an unsettled `cognition` is read — offer the reduced dialog
    // and meet the refusal honestly if one comes — because guessing `form` here
    // is the silent wrong answer this module's header argues about.
    for (const designsProfiles of [undefined, null] as const) {
      expect(
        addTeammateSurface({ cognition: "hosted", designsProfiles, designRefused: false }),
      ).toBe("describe");
      expect(addTeammateSurface({ cognition: null, designsProfiles, designRefused: false })).toBe(
        "describe",
      );
    }
  });

  it("shows the reduced dialog while the cognition read has not landed", () => {
    // `null` is both "in flight" and "this host has no /inference route". Issue
    // #753 leaves the copilot ENABLED in that case rather than refusing because
    // it could not confirm, and this follows it: guessing `describe` wrong is
    // corrected out loud on the detail page, where guessing `form` wrong is
    // corrected by nothing at all.
    expect(addTeammateSurface({ cognition: null, designRefused: false })).toBe("describe");
  });

  it("shows the full form on the offline brain", () => {
    // The operator's decision on #1988, applied here: the can't-draft path keeps
    // today's form, so a company with no model is never locked out of writing a
    // description that nothing downstream could draft for it.
    expect(addTeammateSurface({ cognition: "echo", designRefused: false })).toBe("form");
  });

  it("hands over the full form once a design pass has refused", () => {
    // The reduced dialog's one dead end, and it never writes anything: a
    // teammate is created only from a design the host returned whole.
    expect(addTeammateSurface({ cognition: "harness", designRefused: true })).toBe("form");
  });

  it("keeps the full form on the offline brain even before any Create", () => {
    // Both reasons at once must not cancel out.
    expect(addTeammateSurface({ cognition: "echo", designRefused: true })).toBe("form");
  });
});

describe("describeBlocked", () => {
  it("names the missing half, and nothing when both are there", () => {
    expect(describeBlocked({ name: "", description: "Runs ads." })).toBe("A name is required.");
    expect(describeBlocked({ name: "  ", description: "Runs ads." })).toBe("A name is required.");
    expect(describeBlocked({ name: "Nova", description: "  " })).toBe("Say what they should do.");
    expect(describeBlocked({ name: "Nova", description: "Runs ads." })).toBeNull();
  });
});

describe("designedTeammateFields", () => {
  const described = { name: "  Sable  ", description: "  Runs wholesale outreach.  " };

  it("writes the operator's name and the model's three fields", () => {
    // The split is what divides them: the name is the one thing no pass can
    // produce — it is how the teammate is addressed on every card and beside
    // every message — and the other three are the model's, together, from the
    // one sentence.
    expect(designedTeammateFields(described, DESIGNED)).toEqual({
      name: "Sable",
      role: "Wholesale Account Manager",
      description: "Owns the stockist pipeline and the terms behind it.",
      instructions:
        "Check terms against the price list before quoting. Escalate anything under 40% margin.",
    });
  });

  it("refuses a design that refused", () => {
    // All four host refusals look the same from here: no fields, so nothing to
    // write. The dialog shows the full form and the host's own reason.
    for (const reason of ["no_model", "model_unreachable", "unreadable", "budget_exhausted"] as const) {
      expect(
        designedTeammateFields(described, { source: "unavailable", reason }),
        `a ${reason} refusal must not produce a teammate`,
      ).toBeNull();
    }
  });

  it("refuses a design missing any one of the three fields", () => {
    // All-or-nothing, and this is the assertion that says why: a teammate with
    // a real mandate and no role is exactly what shipped before, and on screen
    // it looks finished. There is no partial success to salvage here.
    for (const missing of ["role", "description", "instructions"] as const) {
      const partial: TeammateDesign = { ...DESIGNED, [missing]: "" };
      expect(
        designedTeammateFields(described, partial),
        `a design with no ${missing} must not be written`,
      ).toBeNull();
      const blank: TeammateDesign = { ...DESIGNED, [missing]: "   " };
      expect(designedTeammateFields(described, blank)).toBeNull();
    }
  });

  it("refuses a teammate with no name, which nothing can derive", () => {
    // The id is slugged from the name host-side (`mint_agent_id`), and no pass
    // produces one: a model asked for a person's name either invents a person
    // or restates the job.
    expect(designedTeammateFields({ name: "   ", description: "Runs ads." }, DESIGNED)).toBeNull();
  });

  it("refuses a role carrying an ellipsis, whatever produced it", () => {
    // The teeth on the whole change. A `…` in a stored role can only be a
    // sentence someone cut, and this is the last place that can decline to
    // write one — the host refuses to truncate a role, and this asserts it a
    // second time on the side the operator would meet it.
    expect(
      designedTeammateFields(described, {
        ...DESIGNED,
        role: "Runs wholesale outreach to boutique retailers and keeps the…",
      }),
    ).toBeNull();
  });

  it("never returns a partial or blank-role teammate, for any design", () => {
    const designs: TeammateDesign[] = [
      DESIGNED,
      { source: "model" },
      { source: "model", role: "Ops" },
      { source: "model", role: "Ops", description: "Owns ops." },
      { source: "unavailable", reason: "no_model" },
      { source: "model", role: " ", description: "x", instructions: "y" },
    ];
    for (const design of designs) {
      const fields = designedTeammateFields({ name: "Nova", description: "Runs ads." }, design);
      if (fields) {
        expect(fields.role.trim()).not.toBe("");
        expect(fields.description.trim()).not.toBe("");
        expect(fields.instructions.trim()).not.toBe("");
        expect(fields.role).not.toContain("…");
      }
    }
  });
});

describe("carriedDescribe", () => {
  const described = { name: "Nova", description: "Runs wholesale outreach." };
  const empty = { name: "", description: "" };

  it("carries the reduced dialog's two values into an untouched form", () => {
    // The bug: `/inference` answers `echo` after the operator has already
    // started typing into the reduced dialog, the surface flips, and the full
    // form's separate state is empty. Nothing errored and nothing can be
    // retried — it reads as the console eating the input.
    expect(carriedDescribe(described, empty)).toEqual({
      name: "Nova",
      description: "Runs wholesale outreach.",
    });
  });

  it("trims what it carries, the way the form's own writes are trimmed", () => {
    expect(carriedDescribe({ name: "  Nova  ", description: " Runs ads. " }, empty)).toEqual({
      name: "Nova",
      description: "Runs ads.",
    });
  });

  it("carries whichever half was typed", () => {
    expect(carriedDescribe({ name: "Nova", description: "" }, empty)).toEqual({
      name: "Nova",
      description: "",
    });
    expect(carriedDescribe({ name: "", description: "Runs ads." }, empty)).toEqual({
      name: "",
      description: "Runs ads.",
    });
  });

  it("carries nothing when nothing was typed", () => {
    // The ordinary `echo` open: the full form is simply what the dialog is, and
    // there is no earlier shape to carry from.
    expect(carriedDescribe(empty, empty)).toBeNull();
    expect(carriedDescribe({ name: "  ", description: "\n " }, empty)).toBeNull();
  });

  it("never overwrites a form the operator has touched", () => {
    // This is what makes it safe to run on every render rather than once, and
    // safe on the hand-over path where `handOver` has already carried the same
    // two values. A second flip must not put the sentence back over an edit.
    expect(carriedDescribe(described, { name: "Atlas", description: "" })).toBeNull();
    expect(carriedDescribe(described, { name: "", description: "Owns stockists." })).toBeNull();
    expect(
      carriedDescribe(described, { name: "Atlas", description: "Owns stockists." }),
    ).toBeNull();
  });
});

describe("heldFields", () => {
  const fields = {
    name: "Sable",
    role: "Wholesale Account Manager",
    description: "Owns the stockist pipeline.",
    instructions: "Check terms before quoting.",
  };
  const held = { name: "Sable", description: "Runs wholesale outreach.", fields };

  it("spends a design the host already answered for exactly this box", () => {
    // The point of holding one: a write that 5xx'd must be retriable without
    // buying the same design a second time.
    expect(
      heldFields(held, { name: "Sable", description: "Runs wholesale outreach." }),
    ).toBe(fields);
    // The dialog trims before it sends, so the held copy is compared trimmed.
    expect(
      heldFields(held, { name: "  Sable ", description: " Runs wholesale outreach.  " }),
    ).toBe(fields);
  });

  it("drops it the moment either field is edited", () => {
    // A design belongs to the sentence it was written from. Reusing it against
    // a different one would store an answer to a question nobody asked.
    expect(heldFields(held, { name: "Atlas", description: "Runs wholesale outreach." })).toBeNull();
    expect(heldFields(held, { name: "Sable", description: "Runs paid acquisition." })).toBeNull();
  });

  it("holds nothing when there is nothing held", () => {
    expect(heldFields(null, { name: "Sable", description: "Runs wholesale outreach." })).toBeNull();
  });
});
