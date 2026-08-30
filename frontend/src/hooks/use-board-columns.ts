// The board's columns as a hook, so a view renders them without owning a copy.
//
// One read per company, on mount. The columns are a vocabulary, not live data:
// they change when the host ships a new one, which is a deploy, not a poll. So
// this deliberately does not refresh on a tick — a board re-reading its own
// column list every few seconds would be spending requests to learn something
// that cannot have changed.

import { useEffect, useState } from "react";

import type { OpenCompanyClient } from "@/api/client";
import { fetchBoardColumns, type TaskColumn } from "@/lib/board-columns";

/**
 * The `tasks` ledger's columns, or `[]` until the read lands.
 *
 * An empty list is the honest answer while loading **and** when the host cannot
 * be reached: callers label through `labelFor`, which humanises, so nothing
 * renders a raw wire word either way. A view that must show columns before the
 * read returns — the board itself — should treat `[]` as "not yet" rather than
 * as "this company has no columns", which is not a state that exists.
 */
export function useBoardColumns(
  client: OpenCompanyClient,
  company: string | null,
): TaskColumn[] {
  const [columns, setColumns] = useState<TaskColumn[]>([]);

  useEffect(() => {
    if (!company) {
      setColumns([]);
      return;
    }
    let live = true;
    void fetchBoardColumns(client, company)
      .then((next) => {
        if (live) setColumns(next);
      })
      .catch(() => {
        // Swallowed on purpose. A failed column read must not surface as an
        // error banner over a board whose cards loaded fine — the labels
        // degrade to humanised ids and everything else still works.
        if (live) setColumns([]);
      });
    return () => {
      live = false;
    };
  }, [client, company]);

  return columns;
}
