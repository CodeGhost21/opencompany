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
// **The task board is read-only here.** It is listed as a ledger so this screen
// is the whole record and not most of it, but its rows fire dispatch and
// planning passes, so they are written on the board. The card says so and
// offers no compose box, rather than offering one whose save the host refuses.

import { useCallback, useEffect, useMemo, useState } from "react";
import {
  AlertTriangle,
  BookText,
  CheckCircle2,
  FileText,
  Loader2,
  Lock,
  Plus,
  RefreshCw,
  Search,
  Trash2,
} from "lucide-react";
import { toast } from "sonner";

import type { OpenCompanyClient } from "@/api/client";
import {
  byline,
  composableFields,
  defineLedger,
  deleteEntry,
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

export function LedgersView({ client, company, sub, onOpenLedger }: Props) {
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

        <section className="min-w-0 flex-1 space-y-3 overflow-y-auto">
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
                    Read-only here. {ledger.writtenBy}
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
                <Button variant="outline" size="sm" onClick={() => void showRendered()}>
                  <FileText className="mr-2 size-4" />
                  Rendered file
                </Button>
                {isWritable(ledger) && (
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
                <div className="space-y-2">
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

function EntryCard({
  entry,
  ledger,
  onAmend,
  onClose,
  onDelete,
}: {
  entry: LedgerEntry;
  ledger: LedgerSummary;
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
          <span className="flex-1 font-medium">{entry.title}</span>
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
