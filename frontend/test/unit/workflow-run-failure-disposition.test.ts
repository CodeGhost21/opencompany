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
