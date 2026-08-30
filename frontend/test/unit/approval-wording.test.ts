import { describe, expect, it } from "vitest";

import { approvedByRuntimeLine, approvedLine, staleDecisionLine } from "@/lib/approval-wording";

/**
 * Issue #561: the confirmation an operator reads after approving.
 *
 * Approving does not resume a suspended call — the host re-dispatches the teammate,
 * and since #469 it does that once per turn, when the LAST decision that turn
 * parked lands. The console used to say "the teammate is completing the action"
 * for every click, including the three out of four that release nothing. What
 * these pin is that the sentence follows the host's count.
 */
describe("the line an approve leaves behind", () => {
  it("says the teammate is picking it up only when this decision released the turn", () => {
    expect(approvedLine(0)).toBe("Approved — the teammate is picking it up now");
    expect(approvedLine(0, "send an email")).toBe(
      "Approved — the teammate is picking it up now: send an email",
    );
  });

  it("names what is still owed when the turn is still blocked", () => {
    expect(approvedLine(1)).toBe(
      "Approved — waiting on 1 more sign-off before the teammate continues",
    );
    expect(approvedLine(3)).toBe(
      "Approved — waiting on 3 more sign-offs before the teammate continues",
    );
  });

  it("claims nothing about what happens next when the host does not say", () => {
    // A host predating the field. Silence is honest; the optimistic guess is
    // the thing this issue is about.
    expect(approvedLine(undefined)).toBe("Approved — recorded");
    expect(approvedLine(undefined, "run a shell command")).toBe(
      "Approved — recorded: run a shell command",
    );
  });

  it("never says 'the teammate' for work the runtime performs itself", () => {
    // Issue #395: a paused workflow gate or a cold-recipient report has no
    // teammate to re-dispatch, and naming one is the same small lie.
    for (const line of [
      approvedByRuntimeLine(0),
      approvedByRuntimeLine(2),
      approvedByRuntimeLine(undefined),
    ]) {
      expect(line).not.toContain("teammate");
    }
    expect(approvedByRuntimeLine(0)).toBe("Approved — carrying it out now");
    expect(approvedByRuntimeLine(2)).toBe(
      "Approved — waiting on 2 more sign-offs before it runs",
    );
  });

  it("never claims completion — approving is not doing", () => {
    for (const still of [0, 1, 5, undefined] as const) {
      for (const line of [approvedLine(still), approvedByRuntimeLine(still)]) {
        expect(line).not.toMatch(/completing|completed|done/i);
      }
    }
  });
});

/**
 * Issue #1449: the line for a click that was never the decision.
 *
 * A resolve can succeed as a *request* and still not be the operator's verdict
 * — a card past its deadline is default-denied by the host, which answers 200
 * and mints nothing. The console had no way to tell that apart from a real
 * approve, so it printed the green success line over work the host had just
 * refused. These pin that each end state gets its own sentence, and that the
 * two the console must not guess about stay silent.
 */
describe("the line a click that decided nothing leaves behind", () => {
  it("says a late click was declined automatically, and that nothing was sent", () => {
    const line = staleDecisionLine("expired");
    expect(line).toContain("deadline");
    expect(line).toContain("declined automatically");
    expect(line).toContain("Nothing was sent");
    expect(staleDecisionLine("expired", "email finance@acme.test")).toContain(
      ": email finance@acme.test",
    );
  });

  it("claims only that nothing changed when the queue was already empty", () => {
    // The host sees an empty parked set and cannot tell a sweep from another
    // operator from another tab. Saying "declined automatically" here would be
    // #1449 pointing the other way — telling somebody nothing happened about a
    // payment a colleague approved a second earlier and which really is going out.
    const line = staleDecisionLine("already_resolved");
    expect(line).toContain("already settled");
    expect(line).not.toMatch(/deadline|declined|expired/i);
  });

  it("stays silent on a real decision, and on a host too old to say", () => {
    // `null` is what lets the caller's own confirmation stand. Guessing in
    // either direction is the defect.
    expect(staleDecisionLine("settled")).toBeNull();
    expect(staleDecisionLine(undefined)).toBeNull();
    expect(staleDecisionLine("settled", "send an email")).toBeNull();
  });

  it("never congratulates the operator for a decision they did not make", () => {
    for (const outcome of ["expired", "already_resolved"] as const) {
      const line = staleDecisionLine(outcome);
      expect(line).not.toMatch(/approved|carrying it out|picking it up/i);
    }
  });
});
