// @vitest-environment jsdom

import { beforeEach, describe, expect, it } from "vitest";

import type { LocalScope } from "@/connections/types";
import { readOverviewVisit, writeOverviewVisit } from "@/lib/overview-visit";

const ACME: LocalScope = { connection: "local-a", company: "acme" };
const OTHER_CONNECTION: LocalScope = { connection: "local-b", company: "acme" };

describe("the operator overview visit boundary (#1321)", () => {
  beforeEach(() => window.localStorage.clear());

  it("survives a reload for the same connection and company", () => {
    writeOverviewVisit(ACME, 1_700_000_000_000);

    expect(readOverviewVisit(ACME)).toBe(1_700_000_000_000);
  });

  it("does not share a browser-local boundary between connections", () => {
    writeOverviewVisit(ACME, 1_700_000_000_000);

    expect(readOverviewVisit(OTHER_CONNECTION)).toBeNull();
  });

  it("ignores malformed stored values rather than inventing a boundary", () => {
    window.localStorage.setItem("oc.overview.last-visit:local-a::acme", "yesterday");

    expect(readOverviewVisit(ACME)).toBeNull();
  });
});
