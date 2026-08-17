// The Ledgers section: the company's own record, whatever shape it takes.
//
// # Nothing here knows what a ledger holds
//
// Every read carries the ledger's own `fields`, `statuses` and `sections`, and
// the form and the table below are built from that. So a ledger a teammate
// declared this morning — a hiring pipeline, a customer promise, an experiment
// — renders correctly this afternoon with no console release. A screen that
// hard-coded the goals columns would have made "declare your own axis" a
// promise the UI quietly broke.
//
// # Two asymmetries this screen has to show, not hide
//
// **Only a person deletes.** Teammates open, amend and close rows; the delete
// control exists here and nowhere an agent can reach. Closing is offered first
// and framed as the ordinary way to be finished with something, because a
// closed row keeps its reason and a deleted one keeps nothing.
//
// **The task board writes differently, not less.** The board IS the `tasks`
// ledger and renders here as columns, because its statuses are a lifecycle and
// that is what a lifecycle looks like. But entering a column *fires work* — the
// dispatch edge, the planning pass, the settle — so a drop there goes through
// `patchTask` rather than through `record_entry`, and there is no compose box:
// a card is opened by a person or an agent through the board's own paths, and a
// form here would produce a save the host refuses. Clicking a card leaves for
// `#/tasks/<id>`, which carries the timeline, plan, discussion and attempts
// this screen does not try to reproduce.

import { useCallback, useEffect, useMemo, useState } from "react";
import {
  AlertTriangle,
  BookText,
  CheckCircle2,
  FileText,
  Columns3,
  Loader2,
  Lock,
  Rows3,
  Plus,
  RefreshCw,
  Search,
  Trash2,
} from "lucide-react";
import { toast } from "sonner";

import type { OpenCompanyClient } from "@/api/client";
import { patchTask } from "@/api/tasks";
import { CreateTaskDialog } from "@/views/CreateTaskDialog";
import { LedgerBoard } from "@/views/LedgerBoard";
import { BOARD_LEDGER, columnsOf, labelFor } from "@/lib/board-columns";
import {
  byline,
  composableFields,
  defineLedger,
  deleteEntry,
  isClosingStatus,
  isWritable,
  listLedgers,
  readLedger,
  recordEntry,
  renderedLedger,
  retireLedger,
  statusField,
  statusNeedsReason,
  type LedgerEntry,
  type LedgerRead,
  type LedgerSummary,
} from "@/api/ledgers";
import { Markdown } from "@/components/markdown";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";

interface Props {
  client: OpenCompanyClient;
  company: string | null;
  /** The ledger named in `#/ledgers/<slug>`, when the address carries one. */
  sub?: string | null;
  /** Navigates to `#/ledgers/<slug>`, so a ledger survives a refresh. */
  onOpenLedger?: (slug: string | null) => void;
  /**
   * Opens a task card's detail screen (`#/tasks/<id>`).
   *
   * The board is the `tasks` ledger and renders here, but a *card* carries far
   * more than a ledger row — a timeline, a plan brief, a discussion, attempts, a
   * workflow proposal, steer controls. Reproducing any of that here would be a
   * worse second copy, so the board's cards link out to the screen that already
   * does it. Absent when this view is rendered with nowhere to link to, in which
   * case a card opens the ordinary amend form.
   */
  onOpenCard?: (id: string) => void;
}

/** A row is either being opened fresh or amended; the form differs only in id. */
interface Composing {
  /** The row this edits, or empty for a new one. */
  id: string;
  fields: Record<string, string>;
  status: string;
  /** True when the form was opened by "Close" rather than "Record". */
  closing: boolean;
}

export function LedgersView({
  client,
  company,
  sub,
  onOpenLedger,
  onOpenCard,
}: Props) {
  const [ledgers, setLedgers] = useState<LedgerSummary[]>([]);
  const [faults, setFaults] = useState<string[]>([]);
  const [remaining, setRemaining] = useState(0);
  const [selected, setSelected] = useState<string | null>(sub ?? null);
  const [read, setRead] = useState<LedgerRead | null>(null);
  const [loading, setLoading] = useState(true);
  const [reading, setReading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [statusFilter, setStatusFilter] = useState("all");
  const [composing, setComposing] = useState<Composing | null>(null);
  const [saving, setSaving] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState<LedgerEntry | null>(null);
  const [declaring, setDeclaring] = useState(false);
  const [rendered, setRendered] = useState<string | null>(null);
  /**
   * Columns or rows.
   *
   * Board by default for anything with a status, because that is what a
   * ledger's statuses *are* — a lifecycle, read left to right. The list is for
   * reading a row's whole contents, which is what a ledger with long prose
   * fields is actually for; the board truncates by construction.
   */
  const [mode, setMode] = useState<"board" | "list">("board");
  /**
   * The board's own create dialog, for the native `tasks` ledger only.
   *
   * Every other ledger takes `record_entry`, which this screen's compose form
   * already is. The board does not — entering a column fires work — so opening
   * a card keeps its own `POST …/tasks` path and its own dialog, which is the
   * one lifted out of the deleted board screen rather than rewritten here.
   */
  const [creatingCard, setCreatingCard] = useState(false);

  const ledger = useMemo(
    () => ledgers.find((held) => held.slug === selected) ?? null,
    [ledgers, selected],
  );

  const refreshList = useCallback(async () => {
    if (!company) return;
    try {
      const list = await listLedgers(client, company);
      setLedgers(list.ledgers);
      setFaults(list.faults ?? []);
      setRemaining(list.remaining);
      setError(null);
      // Land somewhere real: an address naming a ledger that has since been
      // retired should show the first one rather than an empty screen with no
      // explanation.
      setSelected((current) => {
        if (current && list.ledgers.some((held) => held.slug === current)) {
          return current;
        }
        return list.ledgers[0]?.slug ?? null;
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [client, company]);

  useEffect(() => {
    void refreshList();
  }, [refreshList]);

  const refreshRead = useCallback(async () => {
    if (!company || !selected) {
      setRead(null);
      return;
    }
    setReading(true);
    try {
      const next = await readLedger(client, company, selected, {
        q: query.trim() || undefined,
        status: statusFilter === "all" ? undefined : statusFilter,
        limit: 100,
      });
      setRead(next);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setReading(false);
    }
  }, [client, company, selected, query, statusFilter]);

  useEffect(() => {
    void refreshRead();
  }, [refreshRead]);

  // The status filter is per ledger, so switching ledgers must clear it —
  // otherwise the new ledger reads as empty under a filter it does not declare.
  useEffect(() => {
    setStatusFilter("all");
    setRendered(null);
  }, [selected]);

  const openLedger = (slug: string) => {
    setSelected(slug);
    onOpenLedger?.(slug);
  };

  const save = async () => {
    if (!company || !ledger || !composing) return;
    const id = composing.id.trim();
    if (!id) {
      toast.error("A row needs an id.");
      return;
    }
    setSaving(true);
    try {
      await recordEntry(client, company, ledger.slug, {
        id,
        fields: composing.fields,
        status: composing.status || undefined,
      });
      setComposing(null);
      await Promise.all([refreshRead(), refreshList()]);
      toast.success(`Recorded ${id}.`);
    } catch (e) {
      // The host's refusals are written to be read — an unknown status names
      // the real ones, a silent close names the missing reason — so they are
      // shown verbatim rather than replaced with a generic failure.
      toast.error(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  /**
   * Moves a row into `status` — the drop half of the board.
   *
   * Two writes, one gesture, and the split is the whole reason the board can
   * live here at all. A native ledger's rows are the task board's, and entering
   * a column *fires work*: the dispatch edge, the planning pass, the settle. So
   * that write goes through `patchTask`, exactly as dragging on the old board
   * did. Every other ledger takes the ordinary `record_entry` merge.
   *
   * A status that demands a reason does not write at all — it opens the compose
   * form pre-filled instead, because the host refuses a silent close and asking
   * first is the same rule met before it bites.
   *
   * **Optimistic, and reverted out loud.** The card moves under the pointer and
   * snaps back if the host refuses, and the message then carries the whole
   * story: which card, where it was going, and the host's own words for why it
   * would not go. This screen validates no status itself — the declaration is
   * the host's — so the reason for a refusal only ever exists in the response,
   * and a card that silently returned to where it started is the failure issue
   * #334 was reported as.
   */
  const move = async (entry: LedgerEntry, status: string) => {
    if (!company || !ledger || entry.status === status) return;
    if (statusNeedsReason(ledger, status) && !entry.fields.reason?.trim()) {
      setComposing({
        id: entry.id,
        fields: { ...entry.fields },
        status,
        closing: true,
      });
      return;
    }
    const was = entry.status;
    const restage = (to: string) =>
      setRead((current) =>
        current
          ? {
              ...current,
              entries: current.entries.map((held) =>
                held.id === entry.id
                  ? { ...held, status: to, closed: isClosingStatus(ledger, to) }
                  : held,
              ),
            }
          : current,
      );
    restage(status);
    try {
      if (ledger.source === "native") {
        await patchTask(client, company, entry.id, { column: status });
      } else {
        await recordEntry(client, company, ledger.slug, {
          id: entry.id,
          status,
        });
      }
      await Promise.all([refreshRead(), refreshList()]);
    } catch (e) {
      restage(was);
      const label = labelFor(columnsOf(ledger), status);
      toast.error(`Could not move "${entry.title || entry.id}" to ${label}.`, {
        description: e instanceof Error ? e.message : "the host refused the move",
      });
    }
  };

  const remove = async () => {
    if (!company || !ledger || !confirmDelete) return;
    try {
      await deleteEntry(client, company, ledger.slug, confirmDelete.id);
      setConfirmDelete(null);
      await Promise.all([refreshRead(), refreshList()]);
      toast.success(`Deleted ${confirmDelete.id}.`);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    }
  };

  const retire = async (slug: string) => {
    if (!company) return;
    try {
      await retireLedger(client, company, slug);
      await refreshList();
      toast.success(`Retired ${slug}. Its rows were kept.`);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    }
  };

  const showRendered = async () => {
    if (!company || !ledger) return;
    try {
      setRendered(await renderedLedger(client, company, ledger.slug));
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    }
  };

  if (!company) {
    return (
      <div className="p-6 text-sm text-muted-foreground">
        Pick a company to see its ledgers.
      </div>
    );
  }

  if (loading) {
    return (
      <div className="space-y-3 p-6">
        <Skeleton className="h-8 w-48" />
        <Skeleton className="h-40 w-full" />
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col gap-4 p-6">
      <header className="flex flex-wrap items-center gap-3">
        <div className="flex-1 min-w-[16rem]">
          <h1 className="text-xl font-semibold">Ledgers</h1>
          <p className="text-sm text-muted-foreground">
            What this company records and can look up again. Every ledger writes
            a file into <code>derived/</code> in the workspace, which nothing
            edits by hand — the rows here are the source.
          </p>
        </div>
        <Button
          variant="outline"
          size="sm"
          onClick={() => void refreshList().then(() => refreshRead())}
        >
          <RefreshCw className="mr-2 size-4" />
          Refresh
        </Button>
        <Button
          size="sm"
          onClick={() => setDeclaring(true)}
          disabled={remaining <= 0}
          title={
            remaining <= 0
              ? "This company is at the ledger cap. Retire one nothing reads first."
              : undefined
          }
        >
          <Plus className="mr-2 size-4" />
          New ledger
        </Button>
      </header>

      {error && (
        <Alert variant="destructive">
          <AlertTriangle className="size-4" />
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      )}

      {faults.length > 0 && (
        <Alert>
          <AlertTriangle className="size-4" />
          <AlertDescription>
            <p className="font-medium">
              Some declarations could not be loaded:
            </p>
            <ul className="mt-1 list-disc pl-4">
              {faults.map((fault) => (
                <li key={fault}>{fault}</li>
              ))}
            </ul>
          </AlertDescription>
        </Alert>
      )}

      <div className="flex min-h-0 flex-1 gap-4">
        <nav className="w-64 shrink-0 space-y-1 overflow-y-auto">
          {ledgers.map((held) => (
            <button
              key={held.slug}
              type="button"
              onClick={() => openLedger(held.slug)}
              className={cn(
                "flex w-full flex-col gap-0.5 rounded-md border px-3 py-2 text-left text-sm",
                held.slug === selected
                  ? "border-primary bg-accent"
                  : "border-transparent hover:bg-accent/50",
              )}
            >
              <span className="flex items-center gap-2 font-medium">
                <BookText className="size-4 shrink-0" />
                {held.title}
                {!isWritable(held) && (
                  <Lock
                    className="size-3 text-muted-foreground"
                    aria-label="written elsewhere"
                  />
                )}
              </span>
              <span className="text-xs text-muted-foreground">
                {held.open} open · {held.closed} closed
              </span>
            </button>
          ))}
        </nav>

        <section className="flex min-h-0 min-w-0 flex-1 flex-col gap-3">
          {!ledger ? (
            <p className="text-sm text-muted-foreground">
              This company has no ledgers yet.
            </p>
          ) : (
            <>
              <div className="space-y-1">
                <h2 className="text-lg font-medium">{ledger.title}</h2>
                <p className="text-sm text-muted-foreground">
                  {ledger.purpose}
                </p>
                <p className="text-xs text-muted-foreground">
                  Renders into <code>{ledger.derived}</code>
                </p>
              </div>

              {!isWritable(ledger) && (
                <Alert>
                  <Lock className="size-4" />
                  <AlertDescription>
                    Rows here are opened elsewhere: {ledger.writtenBy}. You can
                    still move one between columns — that goes through the board,
                    which is what makes it start work.
                  </AlertDescription>
                </Alert>
              )}

              <div className="flex flex-wrap items-center gap-2">
                <div className="relative min-w-[12rem] flex-1">
                  <Search className="pointer-events-none absolute left-2 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
                  <Input
                    className="pl-8"
                    placeholder="Search every field"
                    value={query}
                    onChange={(e) => setQuery(e.target.value)}
                  />
                </div>
                <Select
                  value={statusFilter}
                  onValueChange={(value) => setStatusFilter(value ?? "all")}
                >
                  <SelectTrigger className="w-[12rem]">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="all">Every status</SelectItem>
                    {ledger.statuses.map((status) => (
                      <SelectItem key={status.name} value={status.name}>
                        {status.name}
                        {status.closed ? " (closed)" : ""}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => setMode(mode === "board" ? "list" : "board")}
                  title={
                    mode === "board"
                      ? "Show every field on each row"
                      : "Group rows by status"
                  }
                >
                  {mode === "board" ? (
                    <Rows3 className="mr-2 size-4" />
                  ) : (
                    <Columns3 className="mr-2 size-4" />
                  )}
                  {mode === "board" ? "List" : "Board"}
                </Button>
                <Button variant="outline" size="sm" onClick={() => void showRendered()}>
                  <FileText className="mr-2 size-4" />
                  Rendered file
                </Button>
                {isWritable(ledger) ? (
                  <Button
                    size="sm"
                    onClick={() =>
                      setComposing({
                        id: "",
                        fields: {},
                        status: ledger.statuses[0]?.name ?? "",
                        closing: false,
                      })
                    }
                  >
                    <Plus className="mr-2 size-4" />
                    Record
                  </Button>
                ) : (
                  ledger.slug === BOARD_LEDGER && (
                    <Button size="sm" onClick={() => setCreatingCard(true)}>
                      <Plus className="mr-2 size-4" />
                      Add task
                    </Button>
                  )
                )}
                {!ledger.builtin && (
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => void retire(ledger.slug)}
                  >
                    Retire
                  </Button>
                )}
              </div>

              {reading && (
                <p className="flex items-center gap-2 text-sm text-muted-foreground">
                  <Loader2 className="size-4 animate-spin" />
                  Reading…
                </p>
              )}

              {read && read.entries.length === 0 && !reading && (
                <p className="text-sm text-muted-foreground">
                  Nothing recorded here yet.
                </p>
              )}

              {mode === "board" && read ? (
                <BoardMode
                  ledger={ledger}
                  entries={read.entries}
                  onMove={(entry, status) => void move(entry, status)}
                  // A drop on dead board pixels. Saying so is the whole fix:
                  // silence here is indistinguishable from a frozen app.
                  onMiss={() => toast.error("Drop the card on a column to move it.")}
                  onOpen={(entry) =>
                    ledger.source === "native" && onOpenCard
                      ? onOpenCard(entry.id)
                      : setComposing({
                          id: entry.id,
                          fields: { ...entry.fields },
                          status: entry.status,
                          closing: false,
                        })
                  }
                />
              ) : (
                <div className="min-h-0 flex-1 space-y-2 overflow-y-auto">
                  {read?.entries.map((entry) => (
                    <EntryCard
                      key={entry.id}
                      entry={entry}
                      ledger={ledger}
                      onOpen={
                        ledger.source === "native" && onOpenCard
                          ? () => onOpenCard(entry.id)
                          : undefined
                      }
                      onAmend={() =>
                        setComposing({
                          id: entry.id,
                          fields: { ...entry.fields },
                          status: entry.status,
                          closing: false,
                        })
                      }
                      onClose={() =>
                        setComposing({
                          id: entry.id,
                          fields: { ...entry.fields },
                          status:
                            ledger.statuses.find((s) => s.closed)?.name ?? "",
                          closing: true,
                        })
                      }
                      onDelete={() => setConfirmDelete(entry)}
                    />
                  ))}
                </div>
              )}

              {read && read.matched > read.entries.length && (
                <p className="text-xs text-muted-foreground">
                  Showing {read.entries.length} of {read.matched}. Narrow with
                  the search or the status filter — the rest are not gone.
                </p>
              )}

              {read?.faults?.map((fault) => (
                <Alert key={fault}>
                  <AlertTriangle className="size-4" />
                  <AlertDescription>{fault}</AlertDescription>
                </Alert>
              ))}
            </>
          )}
        </section>
      </div>

      {composing && ledger && (
        <ComposeDialog
          ledger={ledger}
          composing={composing}
          saving={saving}
          onChange={setComposing}
          onCancel={() => setComposing(null)}
          onSave={() => void save()}
        />
      )}

      {confirmDelete && (
        <Dialog open onOpenChange={() => setConfirmDelete(null)}>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>Delete {confirmDelete.id}?</DialogTitle>
              <DialogDescription>
                This removes the row and everything ever recorded against it.
                Nothing can bring it back, and no teammate can do this — they
                close a row instead, which keeps the reason. Close it rather
                than delete it unless it should never have existed.
              </DialogDescription>
            </DialogHeader>
            <DialogFooter>
              <Button variant="ghost" onClick={() => setConfirmDelete(null)}>
                Cancel
              </Button>
              <Button variant="destructive" onClick={() => void remove()}>
                <Trash2 className="mr-2 size-4" />
                Delete permanently
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      )}

      {declaring && (
        <DeclareDialog
          remaining={remaining}
          onCancel={() => setDeclaring(false)}
          onDeclare={async (declaration) => {
            if (!company) return;
            try {
              const created = await defineLedger(client, company, declaration);
              setDeclaring(false);
              await refreshList();
              openLedger(created.slug);
              toast.success(`Declared ${created.slug}.`);
            } catch (e) {
              toast.error(e instanceof Error ? e.message : String(e));
            }
          }}
        />
      )}

      {ledger && (
        <CreateTaskDialog
          open={creatingCard}
          onClose={() => setCreatingCard(false)}
          onCreated={() => {
            setCreatingCard(false);
            void refreshRead();
            void refreshList();
          }}
          client={client}
          company={company}
        />
      )}

      {rendered !== null && ledger && (
        <Dialog open onOpenChange={() => setRendered(null)}>
          <DialogContent className="max-h-[80vh] max-w-3xl overflow-y-auto">
            <DialogHeader>
              <DialogTitle>{ledger.derived}</DialogTitle>
              <DialogDescription>
                Written by the runtime on every write to this ledger. Editing it
                in the workspace is refused — the next write would erase the
                edit without saying so.
              </DialogDescription>
            </DialogHeader>
            <Markdown>{rendered}</Markdown>
          </DialogContent>
        </Dialog>
      )}
    </div>
  );
}

/**
 * A ledger as columns, over the shared board.
 *
 * Everything about the *gesture* — the columns, the counts, the edge scroll, the
 * miss, the gutter — is [`LedgerBoard`](./LedgerBoard)'s, which the task board
 * uses too. What is here is only what a *ledger row* looks like, which is the
 * one thing the two boards genuinely disagree about: a task card carries a
 * priority, an assignee and a plan badge off its `Task` record, and a ledger row
 * carries whatever fields its declaration named.
 *
 * This is the second attempt. The first re-implemented the board inline and
 * lost all three of issue #334's fixes on the way, which is exactly the failure
 * a shared component exists to prevent.
 */
function BoardMode({
  ledger,
  entries,
  onMove,
  onMiss,
  onOpen,
}: {
  ledger: LedgerSummary;
  entries: LedgerEntry[];
  onMove: (entry: LedgerEntry, status: string) => void;
  onMiss: () => void;
  onOpen: (entry: LedgerEntry) => void;
}) {
  return (
    <LedgerBoard
      columns={columnsOf(ledger)}
      rows={entries}
      statusOf={(entry) => entry.status}
      onMove={onMove}
      onMiss={onMiss}
      renderCard={(entry, dragging) => (
        <button
          type="button"
          onClick={() => onOpen(entry)}
          className={cn(
            "w-full cursor-grab rounded-lg border bg-card p-3 text-left shadow-sm transition-shadow hover:shadow active:cursor-grabbing",
            dragging && "opacity-50",
          )}
        >
          <span className="line-clamp-2 text-sm font-medium leading-snug">
            {entry.title || entry.id}
          </span>
          {/* One line beneath the title, not every field: a board is scanned,
              and a card carrying all of them is a list with extra steps. The
              list mode is where a row is read.

              `data-owner` is set only when the ledger declares an owner field
              AND this row filled it in — so "unassigned" is machine-checkable
              rather than inferred from the absence of a span, which the id
              fallback would otherwise occupy. */}
          <span
            className="mt-1 block truncate text-xs text-muted-foreground"
            {...(ownerOf(entry, ledger)
              ? { "data-owner": ownerOf(entry, ledger) }
              : {})}
          >
            {ownerOf(entry, ledger) || entry.id}
          </span>
        </button>
      )}
    />
  );
}

/**
 * Who owns a row, if the ledger declares an owner field and the row filled it
 * in. Empty otherwise — the card then falls back to its id.
 *
 * Its **id**, and not the first prose field. A board card has to be
 * identifiable at a glance so somebody can say which one they mean, and a
 * truncated sentence of detail identifies nothing.
 */
function ownerOf(entry: LedgerEntry, ledger: LedgerSummary): string {
  const owner = ledger.fields.find((field) => field.role === "owner");
  return (owner ? entry.fields[owner.name]?.trim() : "") ?? "";
}

function EntryCard({
  entry,
  ledger,
  onOpen,
  onAmend,
  onClose,
  onDelete,
}: {
  entry: LedgerEntry;
  ledger: LedgerSummary;
  /** Leaves for the row's own screen, when it has one. Only the board does. */
  onOpen?: () => void;
  onAmend: () => void;
  onClose: () => void;
  onDelete: () => void;
}) {
  const writable = isWritable(ledger);
  return (
    <Card className={cn(entry.closed && "opacity-75")}>
      <CardContent className="space-y-2 p-4">
        <div className="flex flex-wrap items-start gap-2">
          <code className="text-xs text-muted-foreground">{entry.id}</code>
          {entry.status && (
            <Badge variant={entry.closed ? "secondary" : "default"}>
              {entry.closed && <CheckCircle2 className="mr-1 size-3" />}
              {entry.status}
            </Badge>
          )}
          {onOpen ? (
            <button
              type="button"
              className="flex-1 text-left font-medium hover:underline"
              onClick={onOpen}
            >
              {entry.title}
            </button>
          ) : (
            <span className="flex-1 font-medium">{entry.title}</span>
          )}
        </div>

        <dl className="grid gap-x-4 gap-y-1 text-sm sm:grid-cols-[10rem_1fr]">
          {ledger.fields
            .filter((field) => field.role !== "id" && field.role !== "title")
            .map((field) => {
              const value = entry.fields[field.name];
              if (!value) return null;
              return (
                <div key={field.name} className="contents">
                  <dt className="text-muted-foreground">{field.name}</dt>
                  <dd className="whitespace-pre-wrap">{value}</dd>
                </div>
              );
            })}
        </dl>

        <p className="text-xs text-muted-foreground">
          Last recorded by {byline(entry.updatedBy)} · {entry.events}{" "}
          {entry.events === 1 ? "write" : "writes"}
        </p>

        {writable && (
          <div className="flex flex-wrap gap-2">
            <Button variant="outline" size="sm" onClick={onAmend}>
              Amend
            </Button>
            {!entry.closed && ledger.statuses.some((s) => s.closed) && (
              <Button variant="outline" size="sm" onClick={onClose}>
                Close
              </Button>
            )}
            {/* Last, and quiet. Closing is the ordinary way to be finished
                with a row; this is the one that keeps nothing. */}
            <Button
              variant="ghost"
              size="sm"
              className="text-destructive"
              onClick={onDelete}
            >
              <Trash2 className="size-4" />
            </Button>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function ComposeDialog({
  ledger,
  composing,
  saving,
  onChange,
  onCancel,
  onSave,
}: {
  ledger: LedgerSummary;
  composing: Composing;
  saving: boolean;
  onChange: (next: Composing) => void;
  onCancel: () => void;
  onSave: () => void;
}) {
  const status = statusField(ledger);
  // Asked for here rather than let the save fail: the host refuses a silent
  // close, and meeting that rule in the form is the same rule met earlier.
  const needsReason = statusNeedsReason(ledger, composing.status);
  const reasonMissing = needsReason && !composing.fields.reason?.trim();

  return (
    <Dialog open onOpenChange={onCancel}>
      <DialogContent className="max-h-[85vh] max-w-2xl overflow-y-auto">
        <DialogHeader>
          <DialogTitle>
            {composing.closing
              ? `Close ${composing.id}`
              : composing.id
                ? `Amend ${composing.id}`
                : `New row on ${ledger.title}`}
          </DialogTitle>
          <DialogDescription>
            {composing.id
              ? "Only what you change is written; everything else on the row is left alone."
              : "Give it a short, readable id — it is how anybody names this row later."}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="space-y-1">
            <Label htmlFor="ledger-entry-id">id</Label>
            <Input
              id="ledger-entry-id"
              value={composing.id}
              disabled={Boolean(composing.id) && composing.closing}
              placeholder="vendor-slip"
              onChange={(e) => onChange({ ...composing, id: e.target.value })}
            />
          </div>

          {status && (
            <div className="space-y-1">
              <Label>{status.name}</Label>
              <Select
                value={composing.status}
                onValueChange={(value) =>
                  onChange({ ...composing, status: value ?? "" })
                }
              >
                <SelectTrigger>
                  <SelectValue placeholder="Pick a status" />
                </SelectTrigger>
                <SelectContent>
                  {ledger.statuses.map((declared) => (
                    <SelectItem key={declared.name} value={declared.name}>
                      {declared.name}
                      {declared.closed ? " (closes the row)" : ""}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          )}

          {composableFields(ledger).map((field) => (
            <div key={field.name} className="space-y-1">
              <Label htmlFor={`ledger-field-${field.name}`}>
                {field.name}
                {field.required && " *"}
                {field.name === "reason" && needsReason && " — required to close"}
              </Label>
              {field.role === "prose" ? (
                <Textarea
                  id={`ledger-field-${field.name}`}
                  rows={3}
                  value={composing.fields[field.name] ?? ""}
                  onChange={(e) =>
                    onChange({
                      ...composing,
                      fields: {
                        ...composing.fields,
                        [field.name]: e.target.value,
                      },
                    })
                  }
                />
              ) : (
                <Input
                  id={`ledger-field-${field.name}`}
                  value={composing.fields[field.name] ?? ""}
                  onChange={(e) =>
                    onChange({
                      ...composing,
                      fields: {
                        ...composing.fields,
                        [field.name]: e.target.value,
                      },
                    })
                  }
                />
              )}
              {field.description && (
                <p className="text-xs text-muted-foreground">
                  {field.description}
                </p>
              )}
            </div>
          ))}

          {reasonMissing && (
            <Alert>
              <AlertTriangle className="size-4" />
              <AlertDescription>
                Closing into <code>{composing.status}</code> needs a reason. A
                row that does not say why it closed is worth nothing to whoever
                reads it next.
              </AlertDescription>
            </Alert>
          )}
        </div>

        <DialogFooter>
          <Button variant="ghost" onClick={onCancel}>
            Cancel
          </Button>
          <Button onClick={onSave} disabled={saving || reasonMissing}>
            {saving && <Loader2 className="mr-2 size-4 animate-spin" />}
            {composing.closing ? "Close row" : "Record"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/**
 * Declaring a ledger by hand.
 *
 * A JSON editor rather than a wizard, deliberately: the declaration is small,
 * the field roles matter, and a wizard that produced a subset of what a
 * teammate's `define_ledger` can produce would leave the console unable to
 * express a ledger it can display. The starting document is a working example,
 * because the commonest mistake is not a syntax error — it is a ledger with no
 * closing status, which can never say why anything ended.
 */
function DeclareDialog({
  remaining,
  onCancel,
  onDeclare,
}: {
  remaining: number;
  onCancel: () => void;
  onDeclare: (declaration: unknown) => Promise<void>;
}) {
  const [text, setText] = useState(TEMPLATE);
  const [invalid, setInvalid] = useState<string | null>(null);

  const submit = async () => {
    let parsed: unknown;
    try {
      parsed = JSON.parse(text);
    } catch (e) {
      setInvalid(e instanceof Error ? e.message : String(e));
      return;
    }
    setInvalid(null);
    await onDeclare(parsed);
  };

  return (
    <Dialog open onOpenChange={onCancel}>
      <DialogContent className="max-h-[85vh] max-w-2xl overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Declare a ledger</DialogTitle>
          <DialogDescription>
            An axis this company will need to look up again. {remaining} more
            can be declared. Mark the statuses that end a row{" "}
            <code>closed</code>, and set <code>needs_reason</code> on those —
            a row that closes without saying why is worth nothing later.
          </DialogDescription>
        </DialogHeader>
        <Textarea
          rows={20}
          className="font-mono text-xs"
          value={text}
          onChange={(e) => setText(e.target.value)}
        />
        {invalid && (
          <Alert variant="destructive">
            <AlertTriangle className="size-4" />
            <AlertDescription>{invalid}</AlertDescription>
          </Alert>
        )}
        <DialogFooter>
          <Button variant="ghost" onClick={onCancel}>
            Cancel
          </Button>
          <Button onClick={() => void submit()}>Declare</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

const TEMPLATE = JSON.stringify(
  {
    slug: "customer-promises",
    title: "Customer promises",
    purpose:
      "What we have told a customer we would do, and whether we did it. Read it before promising anything else to the same account.",
    fields: [
      { name: "id", role: "id" },
      { name: "promise", role: "title", required: true },
      { name: "status", role: "status", required: true },
      { name: "customer", role: "owner" },
      { name: "due", role: "date" },
      { name: "detail", role: "prose" },
      { name: "reason", role: "prose" },
    ],
    statuses: [
      { name: "open" },
      { name: "kept", closed: true, needs_reason: true },
      { name: "broken", closed: true, needs_reason: true },
    ],
    sections: [
      {
        heading: "Outstanding",
        blurb: "Promised and not yet met. Most recently updated first.",
        statuses: ["open"],
        order: "recent",
      },
      {
        heading: "Settled",
        blurb: "Kept or broken, each with the reason.",
        statuses: ["kept", "broken"],
      },
    ],
    checks: ["required-field", "known-status", "closed-needs-reason"],
  },
  null,
  2,
);
