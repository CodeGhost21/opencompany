// What the operator typed, as something the sources can act on.
//
// The whole of this file is pure: a string in, a `SearchQuery` out, no React,
// no client, no DOM. That is deliberate and it is the point — the grammar is
// the part of search that is easy to get subtly wrong and expensive to debug
// through a modal, so it is testable on its own terms (`search-query.test.ts`)
// rather than only through the component that renders it.
//
// ## The grammar
//
// Three shapes, and the third is the one that makes search feel like Slack:
//
// - `priya`      — a bare term. Everything is searched: channels, agents, files.
// - `@priya`     — a person. Narrows the picker to agents while it is being
//                  typed, so the operator can pick one.
// - `@priya box` — a person **and** a term. Now the term is searched *inside*
//                  that agent's DM, which is the whole reason a scope exists.
//
// `#` does the same for a channel. The prefix is not stripped and forgotten:
// it becomes {@link SearchScope}, which the caller turns into the one
// conversation to read.
//
// ## What is deliberately not here
//
// No `in:`/`from:`/`before:` filter vocabulary yet. Slack's is large and every
// one of those needs a host route to answer against; shipping the two prefixes
// that the console can genuinely resolve beats shipping eight that mostly
// return "not supported".

/** Which kind of thing a scope prefix names. */
export type ScopeKind = "person" | "channel" | "file";

/** The prefix character that opens each scope. */
export const SCOPE_PREFIX: Record<ScopeKind, string> = {
  person: "@",
  channel: "#",
  file: "/",
};

/**
 * Whether what follows the prefix can be narrowed further by a term.
 *
 * `@priya box` and `#autumn box` mean "that conversation, this word" — the
 * scope names a place with things inside it. `/` names the things themselves,
 * so everything after it is one file query: `/autumn brief` is looking for a
 * file called something like "autumn brief", not for "brief" inside a file
 * called "autumn".
 */
const SCOPE_TAKES_TERM: Record<ScopeKind, boolean> = {
  person: true,
  channel: true,
  file: false,
};

/** A scope the operator has begun, whether or not they have finished naming it. */
export interface SearchScope {
  kind: ScopeKind;
  /**
   * What was typed after the prefix, lowercased and trimmed.
   *
   * This is a *name being typed*, not a resolved id: `@pri` is a legitimate
   * intermediate state and the picker's job is to offer Priya for it. The
   * caller resolves it against the roster; this module never guesses.
   */
  name: string;
}

/** A parsed query, ready for the sources to answer. */
export interface SearchQuery {
  /** Exactly what was typed, untouched. Kept so the input can be restored. */
  raw: string;
  /**
   * The scope, when one was opened with `@` or `#`.
   *
   * Present as soon as the prefix is typed — `@` alone parses to a person
   * scope with an empty name, which is what makes the picker open the moment
   * the key is pressed rather than after the first letter.
   */
  scope: SearchScope | null;
  /**
   * What to actually match on, lowercased and trimmed.
   *
   * With a scope, this is only the part *after* the scope's name: in
   * `@priya box`, the term is `box`. Empty means "nothing to match yet",
   * which is a different state from a term that matched nothing.
   */
  term: string;
  /** Whether anything at all was typed. */
  isEmpty: boolean;
}

/**
 * The scope prefixes, longest first, so a future two-character prefix cannot be
 * shadowed by a one-character one that happens to share its head.
 */
const PREFIXES: readonly (readonly [string, ScopeKind])[] = [
  ["@", "person"],
  ["#", "channel"],
  ["/", "file"],
];

/**
 * Parses what the operator typed.
 *
 * Never throws and never returns null: every string is a valid query, because
 * a search box that rejects input has nothing useful to say instead. A string
 * of only whitespace is `isEmpty`, which is the caller's cue to show the
 * resting state rather than "no results".
 */
export function parseSearchQuery(raw: string): SearchQuery {
  const trimmed = raw.trim();
  if (trimmed === "") {
    return { raw, scope: null, term: "", isEmpty: true };
  }

  const found = PREFIXES.find(([prefix]) => trimmed.startsWith(prefix));
  if (!found) {
    return { raw, scope: null, term: trimmed.toLowerCase(), isEmpty: false };
  }

  const [prefix, kind] = found;
  const rest = trimmed.slice(prefix.length);

  // A scope that takes no term swallows the rest whole — see `SCOPE_TAKES_TERM`.
  if (!SCOPE_TAKES_TERM[kind]) {
    return {
      raw,
      scope: { kind, name: rest.trim().toLowerCase() },
      term: "",
      isEmpty: false,
    };
  }

  // The scope's name runs to the first space; everything after it is the term
  // to search *within* that scope. `@priya autumn box` scopes to Priya and
  // searches "autumn box", so a multi-word term survives the split.
  const gap = rest.indexOf(" ");
  const name = gap === -1 ? rest : rest.slice(0, gap);
  const term = gap === -1 ? "" : rest.slice(gap + 1).trim();

  return {
    raw,
    scope: { kind, name: name.trim().toLowerCase() },
    term: term.toLowerCase(),
    isEmpty: false,
  };
}

/**
 * Whether the scope names something specific enough to resolve.
 *
 * `@` on its own opens the picker but resolves to nobody, and a caller that
 * fetched a conversation for it would be fetching an arbitrary one.
 */
export function scopeIsNamed(query: SearchQuery): boolean {
  return query.scope !== null && query.scope.name.length > 0;
}

/**
 * Whether this query asks for messages inside one conversation.
 *
 * Both halves are required: a named scope says *where*, and a term says *what*.
 * `@priya` alone is the operator choosing a person, not yet a search — offering
 * them every message Priya ever sent is not what they asked for.
 */
export function isScopedMessageSearch(query: SearchQuery): boolean {
  return scopeIsNamed(query) && query.term.length > 0;
}

/**
 * Rewrites a query with its scope resolved to a chosen name.
 *
 * What pressing Enter on a picked agent does: `@pri` becomes `@priya `, with
 * the trailing space that starts the term. Returned as a string rather than a
 * `SearchQuery` because it goes back into the input the operator is typing in.
 */
export function withScopeName(query: SearchQuery, name: string): string {
  const kind = query.scope?.kind ?? "person";
  return `${SCOPE_PREFIX[kind]}${name} `;
}
