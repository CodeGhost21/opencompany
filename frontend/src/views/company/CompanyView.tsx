// The Company page (issue #1141): the teammates first, the structure behind a
// toggle.
//
// # Why this exists
//
// Everything on this page already existed and none of it was reachable. `#/team`
// rendered the teammate card grid and `#/team/<agentId>` opened a full detail
// sub-page — but `team` was routable *without a nav entry*, so no operator ever
// arrived at either. Meanwhile the one nav entry that leads here, Company,
// opened the org chart: a tree of desks, which is the company's filing system
// rather than the company's people.
//
// So this page leads with the people and keeps the desks one click away. It is
// the same shape Workflows uses for its Cards/List index (issue #1110) — one
// question with two answers, rendered as a switch — and it is deliberately not
// two nav entries: a roster and an org chart are two views of one company, and
// listing both would make an operator choose between them before they know
// which one answers their question.
//
// # Why the chart is not simply deleted
//
// The chart is the only way in to desk creation, desk membership and the lead
// (issue #311, which restored exactly that after #302 closed it). Cards lead;
// the hierarchy stays reachable.

import { useEffect, useState } from "react";
import { LayoutGrid, Network } from "lucide-react";

import type { OpenCompanyClient } from "@/api/client";
import { Button } from "@/components/ui/button";
import { OrgChartView } from "@/views/company/OrgChartView";
import { TeamView } from "@/views/TeamView";

/** Which half of the company this page is showing. */
export type CompanyMode = "cards" | "chart";

/** Where the cards-or-chart preference is remembered. */
const MODE_KEY = "oc.company.mode";

/**
 * The remembered mode, defaulting to cards — the teammates are the lead.
 *
 * Every access is guarded: `localStorage` throws outright in a browser with
 * site data blocked, and a preference is never worth failing a render over.
 * Same contract as the Workflows index's own remembered mode.
 */
function readMode(): CompanyMode {
  try {
    return window.localStorage.getItem(MODE_KEY) === "chart" ? "chart" : "cards";
  } catch {
    return "cards";
  }
}

/** Remembers the mode. Best-effort, for the same reason. */
function writeMode(mode: CompanyMode): void {
  try {
    window.localStorage.setItem(MODE_KEY, mode);
  } catch {
    // A preference that cannot be saved is not an error worth surfacing.
  }
}

interface Props {
  client: OpenCompanyClient;
  company: string | null;
  /**
   * A desk to bring into view — the second segment of `#/company/<deskId>`,
   * which chat's member pane links to (issue #485).
   *
   * **Its presence forces the chart**, whatever the remembered preference says.
   * A desk address is an org-chart address: honouring it in cards mode would
   * land the operator on a page that cannot show the desk they just named, and
   * silently at that.
   *
   * It also *sticks* for the rest of the visit — see the effect below — so
   * stepping from `#/company/<deskId>` to bare `#/company` stays on the chart
   * rather than swapping the page out from under the operator mid-task. What it
   * never does is rewrite the stored preference: a link followed once out of
   * chat should not decide what Company opens as tomorrow.
   */
  focusDeskId?: string | null;
  /**
   * Open a teammate, or return to this page with `null`.
   *
   * The roster's detail page is `#/team/<agentId>`, an address of its own
   * (issue #264) rather than a second segment of `#/company` — so this page
   * never renders it, and always hands the roster `sub={null}`.
   */
  onOpenAgent: (agentId: string | null) => void;
  /** Bumped when first-run setup staffs the company, so the roster re-reads. */
  refreshKey?: number;
  /** Reopen first-run setup, so skipping it is not a dead end. */
  onRunSetup?: () => void;
}

export function CompanyView({
  client,
  company,
  focusDeskId,
  onOpenAgent,
  refreshKey,
  onRunSetup,
}: Props) {
  const [chosen, setChosen] = useState<CompanyMode>(readMode);
  const mode: CompanyMode = focusDeskId ? "chart" : chosen;

  // Arriving at a desk makes the chart this visit's mode, in memory only. The
  // render above already forces it for the frame the link lands on; this is
  // what keeps it once the desk leaves the hash — and keeps the chart *mounted*
  // across that step, which is what the arrival ring's own state depends on.
  useEffect(() => {
    if (focusDeskId) setChosen("chart");
  }, [focusDeskId]);

  const toolbar = (
    <ModeToggle
      mode={mode}
      onMode={(next) => {
        setChosen(next);
        writeMode(next);
      }}
    />
  );

  return mode === "chart" ? (
    <OrgChartView
      client={client}
      company={company}
      focusDeskId={focusDeskId}
      toolbar={toolbar}
    />
  ) : (
    <TeamView
      client={client}
      company={company}
      sub={null}
      onOpenAgent={onOpenAgent}
      refreshKey={refreshKey}
      onRunSetup={onRunSetup}
      toolbar={toolbar}
    />
  );
}

/**
 * Cards ⇄ Org chart.
 *
 * Segmented rather than two loose buttons, and the same segmented control the
 * Workflows index uses, because the pair is one question with two answers and
 * reads as a switch.
 */
function ModeToggle({
  mode,
  onMode,
}: {
  mode: CompanyMode;
  onMode: (mode: CompanyMode) => void;
}) {
  return (
    <div className="flex items-center gap-1 rounded-lg border p-0.5">
      {(
        [
          { value: "cards", label: "Cards", Icon: LayoutGrid },
          { value: "chart", label: "Org chart", Icon: Network },
        ] as const
      ).map(({ value, label, Icon }) => (
        <Button
          key={value}
          size="sm"
          variant={mode === value ? "secondary" : "ghost"}
          className="h-7 px-2"
          onClick={() => onMode(value)}
          aria-pressed={mode === value}
          data-testid={`company-mode-${value}`}
        >
          <Icon className="mr-1.5 size-3.5" />
          {label}
        </Button>
      ))}
    </div>
  );
}
