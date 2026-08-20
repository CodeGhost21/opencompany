// A boolean that rides the current hash's query suffix (`#/ledgers/goals?new`),
// for UI state that should close on the browser Back button without being a
// distinct `view`/`sub` address of its own (issue #1284: the declare wizard,
// opened in place over whatever list was already on screen).
//
// `useHashView`'s own segment parsing already strips everything from `?`
// onward (`readSegments` in `use-hash-view.ts`), so this coexists with the
// ordinary view/sub routing untouched — the router never sees the flag, and
// this hook never touches the path.

import { useCallback, useEffect, useState } from "react";

function currentlySet(flag: string): boolean {
  const [, query = ""] = window.location.hash.split("?");
  return new URLSearchParams(query).has(flag);
}

/**
 * `flag` names the query key. Returns whether it is currently set, and a
 * setter that pushes (or removes) it from the current hash — a genuine
 * history entry, which is what makes the browser's Back button close it
 * without any extra wiring: popping that entry is a `hashchange` this hook
 * already listens for.
 */
export function useHashFlag(flag: string): [boolean, (on: boolean) => void] {
  const [on, setOn] = useState(() => currentlySet(flag));

  useEffect(() => {
    const onHashChange = () => setOn(currentlySet(flag));
    window.addEventListener("hashchange", onHashChange);
    return () => window.removeEventListener("hashchange", onHashChange);
  }, [flag]);

  const set = useCallback(
    (next: boolean) => {
      const [path, query = ""] = window.location.hash.replace(/^#/, "").split("?");
      const params = new URLSearchParams(query);
      if (next) params.set(flag, "");
      else params.delete(flag);
      // Bare `?new`, not `?new=`: `URLSearchParams` always serialises a
      // value, even an empty one, and the trailing `=` on every value-less
      // flag reads as visual noise in an address bar meant to stay readable.
      const qs = params.toString().replace(/=(?=&|$)/g, "");
      const nextHash = `#${path}${qs ? `?${qs}` : ""}`;
      if (nextHash !== window.location.hash) {
        // A plain assignment, not `history.replaceState`: this is meant to be
        // a real entry the Back button pops, which is the entire point.
        window.location.hash = nextHash;
      }
      setOn(next);
    },
    [flag],
  );

  return [on, set];
}
