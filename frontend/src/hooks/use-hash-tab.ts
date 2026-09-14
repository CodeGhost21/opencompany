// The active sub-tab of a page, riding the current hash's query suffix
// (`#/connections/mcp?tab=json`).
//
// The value-carrying sibling of `useHashFlag`, and for the same reason: a page's
// sub-tab is not a `view`/`sub` address of its own — it is a slice of one page —
// but it still has to survive a refresh, be linkable, and answer the Back
// button. `useHashView`'s segment parsing strips everything from `?` onward
// (`readSegments`), so this rides alongside the router without the router ever
// seeing it, exactly as `?new` does.
//
// It is the shape OpenHuman's Connections page already uses (`?tab=<name>`), so
// a link pasted between the two consoles reads the same way.

import { useCallback, useEffect, useState } from "react";

function readTab<T extends string>(key: string, tabs: readonly T[], fallback: T): T {
  const [, query = ""] = window.location.hash.split("?");
  const raw = new URLSearchParams(query).get(key);
  // Validated against the page's own list rather than trusted: a hand-edited or
  // stale `?tab=` must land on a real tab, not render an empty page.
  return tabs.includes(raw as T) ? (raw as T) : fallback;
}

/**
 * `tabs` is the page's own list and `fallback` the one a bare address lands on.
 *
 * The setter writes a genuine history entry rather than replacing, so Back
 * returns to the tab you came from — the behaviour `useHashFlag` chose for the
 * same reason, and what a deep-linkable tab is for.
 *
 * The default tab **clears** the key instead of writing it. `#/connections/mcp`
 * and `#/connections/mcp?tab=connections` are the same place, and only one of
 * them should be the address you copy out of the bar.
 */
export function useHashTab<T extends string>(
  tabs: readonly T[],
  fallback: T,
  key = "tab",
): [T, (next: T) => void] {
  const [active, setActive] = useState(() => readTab(key, tabs, fallback));

  useEffect(() => {
    const onHashChange = () => setActive(readTab(key, tabs, fallback));
    window.addEventListener("hashchange", onHashChange);
    return () => window.removeEventListener("hashchange", onHashChange);
    // `tabs` is a module-level constant at every call site; listing it by
    // identity would re-subscribe on every render for an array that never
    // changes. `join` is the value the effect actually depends on.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, fallback, tabs.join(",")]);

  const set = useCallback(
    (next: T) => {
      const [path, query = ""] = window.location.hash.replace(/^#/, "").split("?");
      const params = new URLSearchParams(query);
      // Every other key is left standing — `?host=` rides the address across
      // navigations (`use-host-route.ts`), and dropping it here would strand
      // the console rendering one host under an address naming none.
      if (next === fallback) params.delete(key);
      else params.set(key, next);
      const qs = params.toString().replace(/=(?=&|$)/g, "");
      const nextHash = `#${path}${qs ? `?${qs}` : ""}`;
      if (nextHash !== window.location.hash) window.location.hash = nextHash;
      setActive(next);
    },
    [key, fallback],
  );

  return [active, set];
}
