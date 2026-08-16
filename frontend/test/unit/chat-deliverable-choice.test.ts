import { describe, expect, it } from "vitest";

import type { ChannelKind } from "@/views/chat/model";
import { offersDeliverableChoice } from "@/views/chat/model";

/**
 * Issue #845: which composers offer "Do it once" / "Build me the workflow".
 *
 * #580 shipped the control on channel composers only. Nothing downstream was
 * ever scoped to channels — `client.chat` carries `deliverable` off the payload
 * whatever thread it came from, and the chat route reads it the same way — so
 * the asymmetry lived entirely in the caller. A DM asking for a workflow had no
 * way to say so: it went as a `once` card, was dispatched to a desk agent
 * holding no workflow-authoring tool, and came back as a refusal. Reported
 * verbatim on staging as "The only workflow tools I have are read-only".
 */
describe("offersDeliverableChoice", () => {
  it("offers the choice on a DM — the gap this closes", () => {
    expect(offersDeliverableChoice("dm")).toBe(true);
  });

  it("still offers it on a channel, unchanged from #580", () => {
    expect(offersDeliverableChoice("channel")).toBe(true);
  });

  /**
   * The rule is total over `ChannelKind` rather than an inline
   * `kind === "channel"`, so a new kind is a decision someone makes in the
   * function instead of a control that silently fails to appear. This pins that
   * every kind the type admits has an answer.
   */
  it("answers for every channel kind", () => {
    const kinds: ChannelKind[] = ["channel", "dm"];
    for (const kind of kinds) {
      expect(typeof offersDeliverableChoice(kind)).toBe("boolean");
    }
  });
});
