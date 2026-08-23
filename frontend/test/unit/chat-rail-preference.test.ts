// @vitest-environment jsdom

import { beforeEach, describe, expect, it } from "vitest";

import { scopedKey } from "@/connections/types";
import { readChannelRailCollapsed, writeChannelRailCollapsed } from "@/lib/chat-rail";

const SCOPE = { connection: "conn-a", company: "acme" };
const SAME_COMPANY_ELSEWHERE = { connection: "conn-b", company: "acme" };

beforeEach(() => {
  window.localStorage.clear();
});

describe("Chat's compact channel rail preference (issue #1340)", () => {
  it("defaults to the expanded rail", () => {
    expect(readChannelRailCollapsed(SCOPE)).toBe(false);
  });

  it("round-trips the operator's choice", () => {
    writeChannelRailCollapsed(SCOPE, true);
    expect(readChannelRailCollapsed(SCOPE)).toBe(true);

    writeChannelRailCollapsed(SCOPE, false);
    expect(readChannelRailCollapsed(SCOPE)).toBe(false);
  });

  it("keeps connections serving the same company separate", () => {
    writeChannelRailCollapsed(SCOPE, true);

    expect(readChannelRailCollapsed(SAME_COMPANY_ELSEWHERE)).toBe(false);
    expect(window.localStorage.getItem(scopedKey("oc.chat.channel-rail", SCOPE))).toBe("collapsed");
  });
});
