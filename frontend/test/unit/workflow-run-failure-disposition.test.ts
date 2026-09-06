import { describe, expect, it } from "vitest";

import type { RunFailure } from "@/views/workflows/run-failure";
import {
  failureDisposition,
  PRE_EXECUTION_REFUSAL_CODES,
} from "@/views/workflows/run-failure";

const FAILURE: RunFailure = {
  message: "failed",
  fromHost: true,
  sawRunStart: false,
  startedAtMillis: 1_000,
  atMillis: 1_100,
  request: "",
  dryRun: false,
};

describe("failureDisposition", () => {
  it("uses the host's structured code, not merely its envelope, for pre-execution claims", () => {
    expect(failureDisposition({ ...FAILURE, code: "not_wired" })).toBe("refusal-not-wired");
    expect(failureDisposition({ ...FAILURE, code: "engine_failed" })).toBe("cautious");
  });

  it("keeps the known refusal codes together without treating not_wired as inference", () => {
    expect(PRE_EXECUTION_REFUSAL_CODES.has("not_wired")).toBe(true);
    expect(failureDisposition({ ...FAILURE, code: "inference_required" })).toBe(
      "refusal-inference",
    );
  });

  it("treats a paused company as a refusal, never as a run that may have started", () => {
    // B-037: the host's pause gate returns above the runner lookup, so nothing
    // was spawned. Before this arm the code fell through to `cautious`, whose
    // copy sends the operator to a History row that does not exist.
    expect(failureDisposition({ ...FAILURE, code: "lifecycle_conflict" })).toBe(
      "refusal-lifecycle",
    );
    expect(PRE_EXECUTION_REFUSAL_CODES.has("lifecycle_conflict")).toBe(true);
  });

  it("claims a history row only after this console saw the run start", () => {
    expect(failureDisposition({ ...FAILURE, sawRunStart: true })).toBe("journaled");
    expect(failureDisposition(FAILURE)).toBe("cautious");
  });

  it("keeps a failed transport request distinct from a host failure", () => {
    expect(failureDisposition({ ...FAILURE, fromHost: false, sawRunStart: true })).toBe(
      "transport",
    );
  });
});
