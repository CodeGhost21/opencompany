import { describe, expect, it } from "vitest";

import type { TeamMember } from "@/lib/team";
import { buildChannels, directMessageForId } from "@/views/room/model";

function member(id: string, name: string): TeamMember {
  return {
    id,
    name,
    role: "Engineer",
    description: "",
    tone: "sky",
    avatar: "green",
    inboxEnabled: false,
    effectiveTools: [],
    desks: [],
  };
}

describe("direct-message channels", () => {
  it("shows every agent, conversations first and newest of those on top", () => {
    const ada = member("ada", "Ada");
    const ben = member("ben", "Ben");
    const cy = member("cy", "Cy");

    const dms = buildChannels([ada, ben, cy], [], {
      "dm:ada": [{ id: "a", from: "you", text: "Earlier", at: 10 }],
      "dm:ben": [
        { id: "b1", from: "you", text: "Old", at: 5 },
        { id: "b2", from: "company", text: "Newest", at: 20 },
      ],
    }).find((section) => section.id === "dms")?.channels;

    // Ben and Ada have transcripts, so they lead in recency order. Cy has
    // none and is listed anyway — the section is the roster, not an inbox — and
    // ties with every other untouched row at `latestMessageAt` 0, which falls
    // through to the name.
    expect(dms?.map((channel) => channel.id)).toEqual(["dm:ben", "dm:ada", "dm:cy"]);
  });

  it("resolves an unused DM by its stable id for the picker and saved links", () => {
    const ada = member("ada", "Ada");
    expect(directMessageForId([ada], "dm:ada")?.name).toBe("Ada");
  });
});
