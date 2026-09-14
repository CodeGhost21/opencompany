import { describe, expect, it } from "vitest";

import { parseSearchQuery } from "@/search/query";
import {
  agentResults,
  channelResults,
  fileResults,
  formatWhen,
  messageResults,
} from "@/search/sources";
import type { Desk } from "@/lib/desks";
import type { TeamMember } from "@/lib/team";
import { OPERATOR_ORIGIN, type SearchHit } from "@/api/workspace";

/**
 * A whole `SearchHit`, rather than a cast of the three fields under test.
 *
 * `typecheck:unit` compiles this file against a different tsconfig from
 * `typecheck`, and a partial cast passes one and fails the other — which is
 * how a test file ends up green locally and red in CI.
 */
function hit(over: Pick<SearchHit, "id" | "name" | "path" | "matched"> & Partial<SearchHit>): SearchHit {
  return {
    kind: "file",
    parentId: null,
    updatedAt: 0,
    createdBy: OPERATOR_ORIGIN,
    updatedBy: OPERATOR_ORIGIN,
    ...over,
  };
}

const desks = [
  { id: "general", channel: "general", name: "General", blurb: "Everything else." },
  {
    id: "autumn_launch",
    channel: "autumn-launch",
    name: "Autumn launch",
    blurb: "Shipping the autumn range.",
  },
] as Desk[];

function member(over: Partial<TeamMember> & { id: string; name: string }): TeamMember {
  return {
    role: "",
    description: "",
    tone: "blue",
    avatar: over.id,
    inboxEnabled: false,
    effectiveTools: [],
    desks: [],
    ...over,
  } as TeamMember;
}

const members = [
  member({ id: "priya", name: "Priya", role: "Ecommerce Merchandiser" }),
  member({ id: "nadia", name: "Nadia", role: "Packaging Buyer" }),
];

describe("channels as results", () => {
  it("matches the name you would type and the purpose you might remember", () => {
    expect(channelResults(desks, parseSearchQuery("autumn")).map((r) => r.title)).toEqual([
      "#autumn-launch",
    ]);
    // Nobody remembers the slug; they remember what the channel was for.
    expect(channelResults(desks, parseSearchQuery("shipping")).map((r) => r.title)).toEqual([
      "#autumn-launch",
    ]);
  });

  it("says which of the two things Enter does", () => {
    // In a fused palette both are reachable from one row, so the row says it.
    const [go] = channelResults(desks, parseSearchQuery("#general"));
    expect(go.action).toBe("Go to #general");
  });

  it("links to the address the hash router already routes", () => {
    const [hit] = channelResults(desks, parseSearchQuery("autumn"));
    expect(hit.href).toBe("#/chat/autumn_launch");
  });

  it("offsets the highlight past the # the title adds", () => {
    const [hit] = channelResults(desks, parseSearchQuery("general"));
    const [range] = hit.titleMatches;
    expect(hit.title.slice(range[0], range[1])).toBe("general");
  });

  it("stays out of the way when the scope names people", () => {
    expect(channelResults(desks, parseSearchQuery("@priya"))).toEqual([]);
  });
});

describe("agents as results", () => {
  it("matches a name or the role you would describe them by", () => {
    expect(agentResults(members, parseSearchQuery("priya")).map((r) => r.title)).toEqual(["Priya"]);
    expect(agentResults(members, parseSearchQuery("packaging")).map((r) => r.title)).toEqual([
      "Nadia",
    ]);
  });

  it("answers a half-typed @ scope, which is what opens the picker", () => {
    expect(agentResults(members, parseSearchQuery("@pri")).map((r) => r.title)).toEqual(["Priya"]);
  });

  it("links to the console-local DM id the router routes", () => {
    // Deliberately `dmChannelId`, not `dmThreadId` — the latter is the host
    // thread a DM is addressed on, and the two differ for a teammate whose id
    // spells General.
    const [hit] = agentResults(members, parseSearchQuery("priya"));
    expect(hit.href).toBe("#/chat/dm%3Apriya");
  });

  it("stays out of the way when the scope names channels", () => {
    expect(agentResults(members, parseSearchQuery("#general"))).toEqual([]);
  });
});

describe("messages inside one conversation", () => {
  const context = {
    channelId: "dm:priya",
    label: "Priya",
    nameFor: (author: string) => (author === "priya" ? "Priya" : "You"),
  };
  const messages = [
    { id: "m1", channel: "dm:priya", author: "priya", text: "The autumn box ships friday", atMillis: 1, mine: false },
    { id: "m2", channel: "dm:priya", author: "operator", text: "nothing relevant here", atMillis: 2, mine: true },
  ];

  it("returns only the messages that match, with the author on the row", () => {
    const hits = messageResults(messages, parseSearchQuery("@priya box"), context);
    expect(hits).toHaveLength(1);
    expect(hits[0].title).toBe("Priya");
    expect(hits[0].excerpt).toContain("autumn box");
  });

  it("answers nothing until there is a term, so picking a person is not a search", () => {
    // `@priya` alone is choosing somebody. Every message they ever sent is not
    // what was asked for.
    expect(messageResults(messages, parseSearchQuery("@priya"), context)).toEqual([]);
  });

  it("says where the message lives and where opening it goes", () => {
    const [hit] = messageResults(messages, parseSearchQuery("@priya box"), context);
    expect(hit.subtitle).toContain("Priya");
    expect(hit.action).toBe("Open in Priya");
    // Deep link to the line itself, in the `h`-prefixed CONSOLE id form that
    // `MessageRow` renders into `data-message-id`. The bare host id the history
    // route answers with matches no row, so the link would land on the channel
    // and then find nothing to scroll to.
    expect(hit.href).toBe("#/chat/dm%3Apriya?m=hm1");
  });
});

describe("workspace files as results", () => {
  const hits = [
    hit({ id: "n1", name: "Autumn brief.md", path: "briefs/Autumn brief.md", matched: "name" }),
    hit({ id: "n2", name: "Notes.md", path: "Notes.md", matched: "content", excerpt: "the autumn box" }),
  ];

  it("puts a name match above a body match", () => {
    const results = fileResults(hits, parseSearchQuery("autumn"));
    expect(results[0].score).toBeGreaterThan(results[1].score);
  });

  it("shows the path, and the body excerpt where there is one", () => {
    const [named, bodied] = fileResults(hits, parseSearchQuery("autumn"));
    expect(named.subtitle).toBe("briefs/Autumn brief.md");
    expect(bodied.excerpt).toContain("autumn box");
  });

  it("stays out of the way of a scoped search", () => {
    // `@priya box` is a question about a conversation, not about files.
    expect(fileResults(hits, parseSearchQuery("@priya box"))).toEqual([]);
  });
});

describe("when a message was sent", () => {
  const noon = new Date("2026-09-10T12:00:00Z").getTime();

  it("gives a time today, a weekday this week, and a date beyond that", () => {
    expect(formatWhen(noon, noon + 60_000)).toMatch(/\d/);
    expect(formatWhen(noon, noon + 3 * 24 * 3600_000)).toMatch(/day$/);
    expect(formatWhen(noon, noon + 30 * 24 * 3600_000)).toMatch(/\w/);
  });
});

describe("the file scope", () => {
  const hits = [
    hit({ id: "n1", name: "Autumn brief.md", path: "briefs/Autumn brief.md", matched: "name" }),
  ];

  it("answers a / scope, where a bare term and an @ scope would not", () => {
    expect(fileResults(hits, parseSearchQuery("/autumn"))).toHaveLength(1);
    expect(fileResults(hits, parseSearchQuery("@priya box"))).toEqual([]);
  });

  it("keeps channels and agents out of a file search", () => {
    expect(channelResults(desks, parseSearchQuery("/general"))).toEqual([]);
    expect(agentResults(members, parseSearchQuery("/priya"))).toEqual([]);
  });
});

/**
 * The things review found could be offered but not reached, or ranked but not
 * ordered (#2245).
 */
describe("results that have to survive the round trip", () => {
  const multiWord = [member({ id: "user_researcher", name: "User Researcher", role: "Research" })];

  it("hands back a one-word token for an agent whose name has a space", () => {
    // `@User Researcher ` would re-parse as the scope `user` and the term
    // `researcher`, searching a different agent's DM for a word nobody typed.
    const [hit] = agentResults(multiWord, parseSearchQuery("@user"));
    expect(hit.scopeName).toBe("user_researcher");
    expect(parseSearchQuery(`@${hit.scopeName} box`).scope).toEqual({
      kind: "person",
      name: "user_researcher",
    });
  });

  it("hands back the channel's own slug, not the # the row draws", () => {
    const [hit] = channelResults(desks, parseSearchQuery("#autumn"));
    expect(hit.scopeName).toBe("autumn-launch");
  });
});

describe("ordering that the limit would otherwise decide", () => {
  const context = {
    channelId: "dm:priya",
    label: "Priya",
    nameFor: () => "Priya",
  };

  it("breaks an equal-score tie by recency, so the limit keeps the newest", () => {
    // History answers oldest-first, and a common word scores identically on
    // every hit — so without the second key the six oldest fill the list.
    const messages = Array.from({ length: 8 }, (_, index) => ({
      id: `m${index}`,
      channel: "dm:priya",
      author: "priya",
      text: "yes",
      atMillis: index + 1,
      mine: false,
    }));
    const hits = messageResults(messages, parseSearchQuery("@priya yes"), context);
    expect(hits).toHaveLength(6);
    expect(hits[0].id).toBe("message:m7");
  });

  it("ranks files before applying the limit, so a name match is not dropped", () => {
    const many = [
      ...Array.from({ length: 6 }, (_, index) =>
        hit({ id: `body${index}`, name: `Note ${index}.md`, path: `Note ${index}.md`, matched: "content" }),
      ),
      hit({ id: "named", name: "Autumn.md", path: "Autumn.md", matched: "name" }),
    ];
    const results = fileResults(many, parseSearchQuery("autumn"));
    expect(results[0].id, "the name match should lead, not be sliced away").toBe("file:named");
  });
});

describe("links that have somewhere to land", () => {
  const context = { channelId: "dm:priya", label: "Priya", nameFor: () => "Priya" };

  it("names the parent thread for a folded reply", () => {
    // A reply renders in `ThreadPanel` and not in the timeline at all, so the
    // address has to open the panel before there is a line to scroll to.
    const reply = {
      id: "r1",
      parentId: "p1",
      channel: "dm:priya",
      author: "priya",
      text: "the candle ships friday",
      atMillis: 1,
      mine: false,
    };
    const [result] = messageResults([reply], parseSearchQuery("@priya candle"), context);
    expect(result.href).toContain("thread=hp1");
    expect(result.href).toContain("m=hr1");
  });
});

describe("when a message was sent, by the calendar", () => {
  it("does not call yesterday evening today because it is inside 24 hours", () => {
    const yesterdayEvening = new Date(2026, 8, 9, 23, 0).getTime();
    const thisMorning = new Date(2026, 8, 10, 10, 0).getTime();
    // 11 hours ago, and still not today.
    expect(formatWhen(yesterdayEvening, thisMorning)).not.toMatch(/^\d{1,2}[:.]/);
  });

  it("gives a bare time for a message from earlier the same day", () => {
    const earlier = new Date(2026, 8, 10, 9, 0).getTime();
    const later = new Date(2026, 8, 10, 17, 0).getTime();
    expect(formatWhen(earlier, later)).toMatch(/\d/);
  });
});
