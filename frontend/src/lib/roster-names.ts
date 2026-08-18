// One shared way to turn a roster id into the name a human recognizes it by
// (issue #973). #931 hit this once already: an operator-added teammate's id is
// a minted internal string (a ULID for the eight teammates created before
// #686 started minting a readable slug), and printing it told a reader
// nothing. #939 fixed the two connections surfaces by having the host pair
// each id with a name server-side (`RosterAgent`, in `@/api/types`) — but that
// only works where the host is already walking the roster to answer the
// request. A surface that only has the roster listing (`GET …/team`) and a
// bare id to look up in it — the workspace tree chief among them, since the id
// is also that teammate's real folder name — needs the client-side twin: build
// one lookup from the roster read, then resolve every id through it. Route any
// new id-bearing surface through this rather than growing a fifth per-surface
// fix.

import type { RosterAgent } from "@/api/types";

/** id -> display name, built from a roster listing. */
export type RosterNames = ReadonlyMap<string, string>;

/** Build the lookup {@link rosterDisplayName} needs, from a roster read. */
export function rosterNameMap(agents: readonly Pick<RosterAgent, "id" | "name">[]): RosterNames {
  return new Map(agents.map((a) => [a.id, a.name]));
}

/**
 * Resolve a roster id to its display name.
 *
 * Falls back to the id itself — never a blank label — when the roster has
 * nothing to say about it: an id the roster does not carry (the roster has not
 * loaded yet, or the id names something else entirely) or an entry whose name
 * is genuinely empty.
 */
export function rosterDisplayName(id: string, names: RosterNames): string {
  return names.get(id) || id;
}
