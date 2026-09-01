import { describe, expect, it } from "vitest";

import { isPolicyStatus, type PolicyStatus } from "@/api/policy";

/**
 * A `/policy` response that is not a policy must not be rendered.
 *
 * This is a crash fence, and the crash it fences is not hypothetical. The
 * autonomy pill is mounted for the entire life of the console on every view
 * (`window-title-bar.tsx`), its first act is `status.tiers.find(...)`, and there
 * is no error boundary anywhere in `src/` — so a 200 carrying the wrong shape
 * threw during render and React unmounted the whole tree. The symptom is a
 * blank white console on every page with no way back but a reload; the console
 * E2E lane found it as `TypeError: Cannot read properties of undefined (reading
 * 'find')` and thirty-plus specs sitting on their timeouts against an empty
 * document.
 *
 * The predicate lives beside the request rather than inside it because
 * "renderable" is a reader's question, not the transport's:
 * `useApprovalDeadline` reads `approvalTtlHours` alone and documents that an
 * older host omits it, so a transport that insisted on the tier list would
 * break a hook that is deliberately lenient. The readers that put a policy ON
 * SCREEN gate on this instead — `useAutonomy` for the title row, and the
 * settings page's `load` and `apply`.
 */

const WELL_FORMED: PolicyStatus = {
  mode: "supervised",
  alwaysApprove: ["shell"],
  autoApproveUnderUsd: null,
  approvalTtlHours: 24,
  manifestMode: "supervised",
  manifestAlwaysApprove: ["shell"],
  manifestAutoApproveUnderUsd: null,
  manifestApprovalTtlHours: null,
  overridden: false,
  takesEffect: "on the next turn",
  tiers: [
    { value: "readonly", label: "Read-only", description: "Looks, changes nothing." },
    { value: "supervised", label: "Supervised", description: "Asks before acting." },
  ],
};

/**
 * Every body seen in place of a policy. `[]` is first because it is what a
 * catch-all route fulfils with, and so the one that took the E2E lane down.
 */
const MALFORMED: [string, unknown][] = [
  ["an empty array, which is what a catch-all route answers with", []],
  ["an array of policies rather than one", [WELL_FORMED]],
  ["null", null],
  ["undefined", undefined],
  ["a bare string", "supervised"],
  ["an object with no tiers", { ...WELL_FORMED, tiers: undefined }],
  ["an object whose tiers are not a list", { ...WELL_FORMED, tiers: "supervised,full" }],
  ["an object with no mode", { ...WELL_FORMED, mode: undefined }],
  ["an object whose alwaysApprove is not a list", { ...WELL_FORMED, alwaysApprove: "shell" }],
  ["an error envelope answered with a 200", { error: "not_found", message: "no policy here" }],
];

describe("isPolicyStatus", () => {
  for (const [what, body] of MALFORMED) {
    it(`refuses ${what}`, () => {
      expect(isPolicyStatus(body)).toBe(false);
    });
  }

  it("accepts a well-formed policy", () => {
    expect(isPolicyStatus(WELL_FORMED)).toBe(true);
  });

  it("keeps every optional field optional, so an older host still passes", () => {
    // `knownTools`, `setBy` and `setAtMillis` are all documented as absent on
    // some hosts, and `approvalTtlHours` is absent on hosts that predate it.
    // The fence checks the three fields the render paths dereference and
    // nothing else; it must not become a schema a host is obliged to grow into.
    expect(isPolicyStatus({ mode: "auto", alwaysApprove: [], tiers: [] })).toBe(true);
  });

  it("does not require the tiers to be populated", () => {
    // A host that offers no selectable tiers renders a pill that states the
    // mode and opens an empty menu. That is a thin answer, not a broken one.
    expect(isPolicyStatus({ ...WELL_FORMED, tiers: [] })).toBe(true);
  });
});
