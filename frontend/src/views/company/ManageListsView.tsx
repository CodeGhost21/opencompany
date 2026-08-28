// Manage Lists (issue #1284): where a list is retired, and — for an operator
// who wants the full roster rather than the title switcher's menu — declared.
//
// Lives inside Work (`#/ledgers/manage`, a reserved segment `LedgersView`
// checks for — see `MANAGE_SEGMENT`), not under Company. The first cut of
// this screen put it there, on the reasoning that it was "parallel to Manage
// Desks" — that analogy was wrong: desks are company *structure*, so
// managing them belongs on the Company page; lists are work records, and the
// operator reaches this screen almost entirely from the Work switcher. A
// route that lived under Company while being opened from Work meant every
// visit crossed a section boundary and came back (Work → Company → Work),
// which is what made the whole flow feel arbitrary. One parent now: Work.
//
// Retire is here — ported from the old per-list toolbar before issue #1284 —
// so a list's own screen (`LedgersView`) stays about its rows and never about
// whether the list itself continues to exist. Declaring a list, by contrast,
// no longer has to happen here: the switcher's own "New list" opens the
// wizard in place over whatever list was already on screen. This page's
// "New list" stays too, for browsing the full roster before adding one.

import { useEffect, useState } from "react";
import { AlertTriangle, ArrowLeft, Lock, Plus } from "lucide-react";
import { toast } from "sonner";

import { inlineCode } from "@/lib/inline-code";
import type { OpenCompanyClient } from "@/api/client";
import { retireLedger, type LedgerSummary } from "@/api/ledgers";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { withHostParam } from "@/hooks/use-host-route";
import type { LedgerNav } from "@/hooks/use-ledger-nav";
import { RESERVED_SEGMENTS } from "@/views/LedgersView";
import { BOARD_LEDGER } from "@/lib/board-columns";
import { DeclareListWizard } from "@/views/company/DeclareListWizard";

interface Props {
  client: OpenCompanyClient;
  company: string | null;
  ledgerNav: LedgerNav;
  onBack: () => void;
}

export function ManageListsView({ client, company, ledgerNav, onBack }: Props) {
  const [declaring, setDeclaring] = useState(false);
  const [confirmRetire, setConfirmRetire] = useState<LedgerSummary | null>(
    null,
  );
  const [retiring, setRetiring] = useState(false);

  const { ledgers, faults, remaining, loading, refresh } = ledgerNav;

  // `app-shell.tsx`'s `useLedgerNav` reads once on mount/company-change and
  // is otherwise only refreshed by an action taken *through* it (declaring or
  // retiring here). This is the settings page for the set of lists that
  // exist, so opening it should always show the current set — including a
  // list a teammate's `define_ledger` tool call declared, or another operator
  // retired, since this page was last open.
  useEffect(() => {
    void refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  if (!company) {
    return (
      <div className="p-6 text-sm text-muted-foreground">
        Pick a company to manage its lists.
      </div>
    );
  }
  const board = ledgers.find((l) => l.slug === BOARD_LEDGER);
  const rest = ledgers.filter((l) => l.slug !== BOARD_LEDGER);
  const ordered = board ? [board, ...rest] : rest;

  const retire = async (target: LedgerSummary) => {
    setRetiring(true);
    try {
      await retireLedger(client, company, target.slug);
      setConfirmRetire(null);
      await refresh();
      toast.success(`Retired ${target.title}. Its rows were kept.`);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    } finally {
      setRetiring(false);
    }
  };

  return (
    <div className="flex h-full min-h-0 flex-col gap-4 p-6">
      <header className="space-y-2">
        {/* `history.back()`, not a fixed destination (issue #1284): this
            screen is reached from wherever a list's own switcher was open,
            not from one canonical parent, so the way back has to be
            wherever the operator actually came from. */}
        <Button
          variant="ghost"
          size="sm"
          className="w-fit"
          onClick={onBack}
          data-testid="lists-back"
        >
          <ArrowLeft className="mr-2 size-4" />
          Back
        </Button>
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div>
            <h1 className="text-xl font-semibold">Manage lists</h1>
            <p className="text-sm text-muted-foreground">
              Every list this company tracks — its own board, plus whatever else
              it records. Reach any of them from the switcher on its own title;
              retire one here.
            </p>
          </div>
          <Button
            size="sm"
            onClick={() => setDeclaring(true)}
            disabled={remaining <= 0}
            title={
              remaining <= 0
                ? "This company is at the list cap. Retire one nothing reads first."
                : undefined
            }
          >
            <Plus className="mr-2 size-4" />
            New list
          </Button>
        </div>
      </header>

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

      <div className="min-h-0 flex-1 space-y-2 overflow-y-auto">
        {loading && ledgers.length === 0 ? (
          <div className="space-y-2">
            <Skeleton className="h-16 w-full" />
            <Skeleton className="h-16 w-full" />
          </div>
        ) : (
          ordered.map((held) => (
            <Card key={held.slug}>
              <CardContent className="flex flex-wrap items-center justify-between gap-3 py-2">
                <a
                  href={withHostParam(`ledgers/${held.slug}`)}
                  data-testid={`managed-ledger-${held.slug}`}
                  className="min-w-[16rem] flex-1 rounded-md outline-none hover:text-primary focus-visible:ring-2 focus-visible:ring-ring"
                >
                  <div className="flex flex-wrap items-center gap-2 text-sm font-medium">
                    {held.title}
                    {held.source === "native" && (
                      <Lock
                        className="size-3 text-muted-foreground"
                        aria-label="written elsewhere"
                      />
                    )}
                    <span className="rounded-full bg-muted px-2 py-0.5 text-xs font-medium tabular-nums text-foreground">
                      {held.open} open · {held.closed} closed
                    </span>
                  </div>
                  <p className="mt-0.5 max-w-prose text-xs text-muted-foreground">
                    {inlineCode(held.purpose)}
                  </p>
                </a>
                {held.builtin ? (
                  <Badge variant="secondary" title="Built into every company">
                    Built in
                  </Badge>
                ) : (
                  <AlertDialog
                    open={confirmRetire?.slug === held.slug}
                    onOpenChange={(open) =>
                      setConfirmRetire(open ? held : null)
                    }
                  >
                    <AlertDialogTrigger
                      render={
                        <Button
                          variant="destructive"
                          size="sm"
                        >
                          Retire
                        </Button>
                      }
                    />
                    <AlertDialogContent>
                      <AlertDialogHeader>
                        <AlertDialogTitle>
                          Retire “{held.title}”?
                        </AlertDialogTitle>
                        <AlertDialogDescription>
                          This list leaves the switcher's menu and its{" "}
                          <code>{held.derived}</code> file stops being
                          rewritten. Its rows are kept, but nothing in the
                          console lists them afterward — re-declaring{" "}
                          <code>{held.slug}</code> is the only way back.
                        </AlertDialogDescription>
                      </AlertDialogHeader>
                      <AlertDialogFooter>
                        <AlertDialogCancel>Keep it</AlertDialogCancel>
                        <AlertDialogAction
                          onClick={() => void retire(held)}
                          disabled={retiring}
                          className="bg-destructive text-white hover:bg-destructive/90"
                          data-testid="ledger-retire-confirm"
                        >
                          Retire list
                        </AlertDialogAction>
                      </AlertDialogFooter>
                    </AlertDialogContent>
                  </AlertDialog>
                )}
              </CardContent>
            </Card>
          ))
        )}
      </div>

      {declaring && (
        <DeclareListWizard
          client={client}
          company={company}
          existingSlugs={[...ledgers.map((l) => l.slug), ...RESERVED_SEGMENTS]}
          remaining={remaining}
          onCancel={() => setDeclaring(false)}
          onCreated={async (created) => {
            setDeclaring(false);
            await refresh();
            toast.success(`Declared ${created.title}.`);
          }}
        />
      )}
    </div>
  );
}
