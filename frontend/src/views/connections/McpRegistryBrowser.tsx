import { useEffect, useRef, useState } from "react";

import type { OpenCompanyClient } from "@/api/client";
import {
  searchMcpRegistry,
  type McpCatalogueEntry,
} from "@/api/mcp-registry";
import { registryOutage, type McpRegistryOutage } from "@/lib/mcp-registry";

/**
 * The directory half of the server search.
 *
 * Discover used to be a panel of its own, nested under the add form, under the
 * server list, inside the same card — and it searched only the directory. So the
 * first thing it could tell an operator was that they could install something
 * they already had. It is a hook now, and its rows land in the one list beside
 * the company's own under a heading that says which is which: an operator types
 * a name to find out whether this company has that server, and the honest answer
 * is either a server they already have or one they could install.
 *
 * ## It cannot take the server list down with it
 *
 * Every failure is classified here and none is rethrown. The directories are two
 * network hops away and either can be down; the host itself may be built without
 * the `mcp` feature, in which case these routes answer `404 not_wired`. Both are
 * an empty half with a reason, while the company's own servers go on rendering.
 * A dead directory is not a broken page.
 */

/** How many directory rows one search asks for. */
const PAGE_SIZE = 10;

/** How long a keystroke waits before it costs a directory call. */
const DEBOUNCE_MS = 350;

export type DirectoryHalf =
  /** Nothing typed. The directory is not called at all, so opening costs nothing. */
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "outage"; outage: McpRegistryOutage }
  | {
      kind: "ready";
      entries: McpCatalogueEntry[];
      page: number;
      totalPages: number;
    };

export function useMcpDirectorySearch(
  client: OpenCompanyClient,
  company: string | null,
  query: string,
): DirectoryHalf {
  const [half, setHalf] = useState<DirectoryHalf>({ kind: "idle" });
  /**
   * Only the newest search may write its answer.
   *
   * Two searches issued quickly can land out of order, and the older answer
   * arriving last would paint results for a query the operator has already
   * replaced. Bumped on a scope change too, so an answer for the company just
   * left cannot repaint this list.
   */
  const generation = useRef(0);

  useEffect(() => {
    const term = query.trim();
    generation.current += 1;
    if (term === "") {
      setHalf({ kind: "idle" });
      return;
    }
    const mine = generation.current;
    setHalf({ kind: "loading" });
    const timer = window.setTimeout(() => {
      void (async () => {
        try {
          const found = await searchMcpRegistry(client, company, {
            q: term,
            page: 1,
            pageSize: PAGE_SIZE,
          });
          if (generation.current !== mine) return;
          setHalf({
            kind: "ready",
            entries: found.servers,
            page: found.page,
            totalPages: found.totalPages,
          });
        } catch (err) {
          if (generation.current !== mine) return;
          setHalf({ kind: "outage", outage: registryOutage(err) });
        }
      })();
    }, DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [client, company, query]);

  return half;
}
