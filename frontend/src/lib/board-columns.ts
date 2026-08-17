// The board's columns, read off the `tasks` ledger rather than kept here.
//
// # Why this replaced a literal list
//
// `TASK_COLUMNS` was a hand-maintained copy of the host's `BOARD_COLUMNS`, and
// its own comment admitted what that cost: *"a Rust test cannot see the TS
// list, so a column added on one side and not the other keeps this green."* A
// column that existed only here was one the host's write boundary refused; a
// column that existed only there was one the board silently never rendered, so
// its cards vanished with no error — the exact disappearance the column
// vocabulary was introduced to prevent.
//
// The host now declares columns once (`src/ledger/board.rs`), builds the
// `tasks` ledger's statuses from that table, and sends each one's `label` on the
// wire. So the console asks. A column added on the host appears here on the next
// read, correctly labelled, with no console release and nothing to keep in step.

import type { OpenCompanyClient } from "@/api/client";
import { listLedgers, type LedgerStatus, type LedgerSummary } from "@/api/ledgers";

/** The `tasks` ledger's slug — the board, as the ledger surface names it. */
export const BOARD_LEDGER = "tasks";

export interface TaskColumn {
  id: string;
  label: string;
  /** Whether a card here is finished. Only `done` is. */
  closed: boolean;
}

/**
 * A readable label for a status the host sent no label for.
 *
 * Used for the ledgers a company declares, whose statuses are already written
 * to be read (`open`, `at_risk`, `kept`), and as the last resort for a stored
 * card carrying a column this build has never heard of. It is deliberately not
 * used for the board: `in_progress` humanises to "In progress" by luck and
 * `todo` becomes "Todo", which is why the host sends the real labels.
 */
export function humanizeStatus(id: string): string {
  const words = id.trim().replace(/[_-]+/g, " ").trim();
  if (!words) return id;
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/** A ledger status as a column, preferring the host's label. */
export function columnOf(status: LedgerStatus): TaskColumn {
  return {
    id: status.name,
    label: status.label?.trim() || humanizeStatus(status.name),
    closed: status.closed === true,
  };
}

/**
 * Every column a ledger declares, in declaration order.
 *
 * Declaration order is board order: the host's table is written left to right,
 * and a console that sorted these itself would put Done next to To-do the first
 * time somebody added a column.
 */
export function columnsOf(ledger: LedgerSummary): TaskColumn[] {
  return ledger.statuses.map(columnOf);
}

/**
 * The label for one column id, given the columns currently known.
 *
 * Falls back to humanising rather than to the raw id, so a card whose column
 * this build does not know still reads as words. Never invents a mapping: the
 * host's label wins whenever there is one.
 */
export function labelFor(columns: TaskColumn[], id: string): string {
  return columns.find((column) => column.id === id)?.label ?? humanizeStatus(id);
}

/**
 * The board's columns, fetched once per company.
 *
 * Returns `[]` until the read lands. Callers render labels through
 * {@link labelFor}, which humanises in the meantime — so a board that has not
 * yet heard from the host shows "In progress" rather than an empty header or a
 * flash of `in_progress`.
 */
export async function fetchBoardColumns(
  client: OpenCompanyClient,
  company: string,
): Promise<TaskColumn[]> {
  const list = await listLedgers(client, company);
  const board = list.ledgers.find((held) => held.slug === BOARD_LEDGER);
  return board ? columnsOf(board) : [];
}
