import { ArrowRight, Building2, Plus, RotateCcw } from "lucide-react";

import type { CompanyStatus } from "@/api/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { CREATE_UNAVAILABLE_NOTE } from "@/components/create-company-dialog";
import { StatusPill } from "@/components/status-pill";
import { ThemeToggle } from "@/components/theme-toggle";

interface Props {
  companies: CompanyStatus[];
  onPick: (id: string) => void;
  /**
   * Start the New-company flow (issue #1807). Optional so a caller that cannot
   * offer it (single-company mode) simply omits the header button.
   */
  onCreate?: () => void;
  /** Start the reset (archive + start clean) flow for one company. */
  onReset?: (company: CompanyStatus) => void;
  /**
   * Whether this console can actually provision. When false the create/reset
   * controls render disabled with an honest note rather than 401ing after the
   * click (the #1401 dishonest-button lesson) — never silently hidden.
   */
  canCreate?: boolean;
}

/** Multi-company hosts: choose which company to operate. */
export function CompanyPicker({ companies, onPick, onCreate, onReset, canCreate }: Props) {
  return (
    <div className="min-h-svh bg-background">
      <header className="flex items-center justify-between border-b px-6 py-4">
        <div className="flex items-center gap-2">
          <div className="flex size-7 items-center justify-center rounded-md bg-primary text-primary-foreground">
            <Building2 className="size-4" />
          </div>
          <span className="text-sm font-semibold">OpenCompany</span>
        </div>
        <ThemeToggle />
      </header>

      <main className="mx-auto w-full max-w-4xl px-6 py-10">
        <div className="mb-6 flex flex-wrap items-end justify-between gap-3">
          <div className="space-y-1">
            <h1 className="text-2xl font-semibold tracking-tight">Your companies</h1>
            <p className="text-sm text-muted-foreground">Choose a company to operate.</p>
          </div>
          {onCreate && (
            <div className="flex flex-col items-end gap-1">
              <Button
                onClick={onCreate}
                disabled={!canCreate}
                title={canCreate ? undefined : CREATE_UNAVAILABLE_NOTE}
                data-testid="picker-new-company"
              >
                <Plus className="size-4" /> New company
              </Button>
              {!canCreate && (
                <p className="max-w-xs text-right text-2xs text-muted-foreground">
                  {CREATE_UNAVAILABLE_NOTE}
                </p>
              )}
            </div>
          )}
        </div>

        <div className="grid gap-4 sm:grid-cols-2">
          {companies.map((c) => (
            <Card
              key={c.id}
              role="button"
              tabIndex={0}
              onClick={() => onPick(c.id)}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  onPick(c.id);
                }
              }}
              className="group cursor-pointer p-5 transition-colors hover:border-primary/40 hover:bg-accent/40"
            >
              <div className="flex items-start gap-3">
                <div className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-muted">
                  <Building2 className="size-5" />
                </div>
                <div className="min-w-0 flex-1">
                  <p className="truncate font-medium">{c.name}</p>
                  <div className="mt-2 flex flex-wrap items-center gap-2">
                    <StatusPill lifecycle={c.lifecycle} emergencyPaused={c.emergency_paused} />
                    {c.pending_approvals > 0 && (
                      <Badge variant="secondary">{c.pending_approvals} to approve</Badge>
                    )}
                  </div>
                  {onReset && canCreate && c.lifecycle !== "archived" && (
                    <Button
                      variant="ghost"
                      size="sm"
                      className="mt-3 h-7 px-2 text-xs text-muted-foreground hover:text-destructive"
                      // The card is a button; keep the reset click from also
                      // opening the company under it.
                      onClick={(e) => {
                        e.stopPropagation();
                        onReset(c);
                      }}
                      data-testid={`picker-reset-${c.id}`}
                    >
                      <RotateCcw className="size-3.5" /> Reset
                    </Button>
                  )}
                </div>
                <ArrowRight className="size-4 shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5" />
              </div>
            </Card>
          ))}
        </div>
      </main>
    </div>
  );
}
