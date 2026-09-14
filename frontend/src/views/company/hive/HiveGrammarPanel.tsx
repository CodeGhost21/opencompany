import { useEffect, useMemo, useRef, useState } from "react";
import { AlertTriangle, Loader2 } from "lucide-react";

import type { OpenCompanyClient } from "@/api/client";
import type { DeskHiveDeclared, DeskHiveDto } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import {
  derivedQuorum,
  derivedTurnBudget,
  eligibleSupporters,
  effectiveQuorum,
  grammarProblems,
  GATEABLE_KINDS,
  type MoveKind,
} from "@/lib/hive/grammar";
import { DerivedNumber } from "@/views/company/hive/DerivedNumber";
import { MovesMatrix } from "@/views/company/hive/MovesMatrix";

/**
 * Install or replace a desk's move grammar.
 *
 * Until this existed the table could only be changed by editing `company.toml`
 * and redeploying, which meant the one knob that turns a room of agreeing agents
 * back into a deliberation was unreachable from the product that runs them.
 *
 * # Everything here mirrors a refusal the host will make
 *
 * The checks below are the console's copy of `hive_problems`, run before the
 * round trip so an error lands next to the control that caused it rather than as
 * a sentence at the bottom of a form. The **host is still the authority**: its
 * own refusal comes back on the `PUT` and is rendered verbatim, because a
 * paraphrase of a validation message is a second account of a rule that already
 * has one.
 *
 * The one asymmetry worth knowing: the host judges the table against the desk's
 * **effective** roster — overlay additions and retirements included — so it can
 * legitimately refuse a table that `company.toml` would have accepted. That is
 * correct, and it is why its message names the desk.
 */
export function HiveGrammarPanel({
  client,
  company,
  deskId,
  onClose,
}: {
  client: OpenCompanyClient;
  company?: string | null;
  deskId: string;
  onClose?: () => void;
}) {
  const [state, setState] = useState<DeskHiveDto | null>(null);
  const [draft, setDraft] = useState<DeskHiveDeclared>({});
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // The desk this panel is showing right now, on every render. An `await` on
  // `putDeskHive`/`resetDeskHive` can resolve after the operator has already
  // switched desks; comparing the desk a save/reset was issued for against
  // this ref when it settles is what stops that stale response from
  // overwriting the newly loaded desk's state, error, or `busy` lock.
  const currentDeskId = useRef(deskId);
  currentDeskId.current = deskId;

  useEffect(() => {
    let live = true;
    setState(null);
    setLoadError(null);
    // A save/reset refusal from the desk this panel just left must not go on
    // being shown under the desk it switched to.
    setSaveError(null);
    client
      .getDeskHive(deskId, company)
      .then((dto) => {
        if (!live) return;
        setState(dto);
        setDraft(dto.declared ?? {});
      })
      .catch((e: unknown) => {
        if (!live) return;
        // A host that predates the route is not an error the operator can act
        // on — say what is missing rather than showing a broken form.
        setLoadError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      live = false;
    };
  }, [client, company, deskId]);

  const seats = useMemo(() => {
    const current = state?.seats ?? [];
    const currentIds = new Set(current.map((seat) => seat.agentId));
    // Keep an orphaned authored row visible long enough to remove it. Without
    // this the validator quite correctly rejects the saved grammar, but the
    // only control that could repair it has disappeared with the former seat.
    const stale = Object.keys(draft.moves ?? {})
      .filter((agentId) => !currentIds.has(agentId))
      .map((agentId) => ({
        agentId,
        label: agentId,
        role: "no longer on this desk",
        moves: draft.moves?.[agentId] ?? [],
        governed: true,
      }));
    return [...current, ...stale];
  }, [state, draft.moves]);
  // Validation must use the effective roster, not `seats`: the latter also
  // contains stale authored rows solely so an operator can remove them.
  const memberIds = useMemo(() => state?.seats.map((seat) => seat.agentId) ?? [], [state]);
  const moves = draft.moves ?? {};

  const problems = useMemo(
    () =>
      grammarProblems(
        {
          enabled: draft.enabled,
          quorum: draft.quorum,
          turnBudget: draft.turn_budget,
          dominanceCap: draft.dominance_cap,
          repetitionCap: draft.repetition_cap,
          refutationCap: draft.refutation_cap,
          moves,
        },
        memberIds,
      ),
    [draft, moves, memberIds],
  );

  const eligible = eligibleSupporters(moves, memberIds);
  const quorum = effectiveQuorum(draft.quorum, memberIds.length);

  if (loadError) {
    return (
      <Panel title="Move grammar" onClose={onClose}>
        <p className="text-sm text-muted-foreground">
          This host cannot report a desk&rsquo;s move grammar. {loadError}
        </p>
      </Panel>
    );
  }
  if (!state) {
    return (
      <Panel title="Move grammar" onClose={onClose}>
        <Loader2 aria-hidden className="size-4 animate-spin text-muted-foreground" />
      </Panel>
    );
  }

  const toggle = (agentId: string, kind: MoveKind, next: boolean) => {
    setDraft((prev) => {
      const table = { ...(prev.moves ?? {}) };
      // A departed seat cannot be meaningfully narrowed. Any interaction with
      // its visible stale row removes that invalid authored entry so the
      // operator can install the repaired grammar.
      if (!memberIds.includes(agentId)) {
        delete table[agentId];
        return { ...prev, moves: table };
      }
      // Toggling an ungoverned seat materialises its row: until now the table
      // did not name it, and naming it is what makes the narrowing a decision.
      const current = new Set(table[agentId] ?? GATEABLE_KINDS);
      // An empty list is deliberately fail-open in the runtime, so do not
      // serialize an apparently fully-restricted seat as one.
      if (!next && current.size === 1 && current.has(kind)) return prev;
      if (next) current.add(kind);
      else current.delete(kind);
      table[agentId] = GATEABLE_KINDS.filter((k) => current.has(k));
      return { ...prev, moves: table };
    });
  };

  const save = async () => {
    const issuedFor = deskId;
    setBusy(true);
    setSaveError(null);
    try {
      const dto = await client.putDeskHive(deskId, draft, company);
      if (currentDeskId.current !== issuedFor) return;
      setState(dto);
      setDraft(dto.declared ?? {});
    } catch (e: unknown) {
      if (currentDeskId.current !== issuedFor) return;
      // The host's own sentence, verbatim.
      setSaveError(e instanceof Error ? e.message : String(e));
    } finally {
      if (currentDeskId.current === issuedFor) setBusy(false);
    }
  };

  const reset = async () => {
    const issuedFor = deskId;
    setBusy(true);
    setSaveError(null);
    try {
      const dto = await client.resetDeskHive(deskId, company);
      if (currentDeskId.current !== issuedFor) return;
      setState(dto);
      setDraft(dto.declared ?? {});
    } catch (e: unknown) {
      if (currentDeskId.current !== issuedFor) return;
      setSaveError(e instanceof Error ? e.message : String(e));
    } finally {
      if (currentDeskId.current === issuedFor) setBusy(false);
    }
  };

  return (
    <Panel title="Move grammar" onClose={onClose}>
      <div className="space-y-5">
        {/*
          Below two seats the switch is ABSENT with a sentence, not disabled:
          `enabled: true` cannot conjure a room out of one member, and a control
          claiming otherwise would be a lie the host then quietly ignores.
        */}
        {seats.length < 2 ? (
          <p className="rounded-md border border-border bg-muted/40 p-3 text-sm text-muted-foreground">
            A room needs two seats. This desk answers with a single responder.
          </p>
        ) : (
          <label className="flex items-center justify-between gap-3">
            <span>
              <span className="text-sm font-medium">Answer as a room</span>
              <span className="block text-2xs text-muted-foreground">
                Off keeps the desk on one responder, exactly as it behaved before
                deliberation existed.
              </span>
            </span>
            <Switch
              checked={draft.enabled !== false}
              disabled={busy}
              onCheckedChange={(on: boolean) =>
                setDraft((p) => ({ ...p, enabled: on ? undefined : false }))
              }
            />
          </label>
        )}

        <div className="grid gap-4 sm:grid-cols-2">
          <DerivedNumber
            label="Turn budget"
            hint="Conformity in a room of language models rises with interaction time, so a bigger budget buys correlated error rather than a better answer."
            declared={draft.turn_budget}
            derived={derivedTurnBudget(memberIds.length)}
            formula={`3 × ${memberIds.length} seats`}
            disabled={busy}
            onChange={(next) => setDraft((p) => ({ ...p, turn_budget: next }))}
          />
          <DerivedNumber
            label="Quorum"
            hint="Distinct grounded supporters a topic needs to carry. The default is a majority that still leaves somebody outside it, so a decision never requires the whole room."
            declared={draft.quorum}
            derived={derivedQuorum(memberIds.length)}
            formula={`majority of ${memberIds.length}, one short of all`}
            disabled={busy}
            onChange={(next) => setDraft((p) => ({ ...p, quorum: next }))}
          />
        </div>

        <div>
          <h3 className="text-sm font-medium">Who may make which move</h3>
          <p className="mb-2 text-2xs text-muted-foreground">
            A room whose members may all propose is a room that votes — a proposal
            already counts as its own author&rsquo;s support, so agreement is reached
            without anybody engaging with anybody else&rsquo;s reasoning.
          </p>
          <MovesMatrix
            seats={seats}
            moves={moves}
            onToggle={toggle}
            disabled={busy}
          />
        </div>

        {/*
          The load-bearing check, stated as a sentence rather than a red field:
          a table that lets fewer seats support than the quorum needs describes a
          room that can never decide anything, however much it agrees.
        */}
        <p
          className={cnState(eligible >= quorum)}
          role={eligible >= quorum ? undefined : "alert"}
        >
          {eligible >= quorum
            ? `${eligible} of ${memberIds.length} seats can carry a topic; quorum needs ${quorum}.`
            : `Only ${eligible} of ${memberIds.length} seats can carry a topic, but quorum needs ${quorum} — this room could never decide anything.`}
        </p>

        {problems.length > 0 && (
          <ul className="space-y-1 rounded-md border border-status-failed/40 bg-status-failed-soft p-3 text-xs text-status-failed-text">
            {problems.map((p) => (
              <li key={`${p.field}:${p.message}`}>{p.message}</li>
            ))}
          </ul>
        )}

        {saveError && (
          <p className="flex items-start gap-1.5 rounded-md border border-status-failed/40 bg-status-failed-soft p-3 text-xs text-status-failed-text">
            <AlertTriangle aria-hidden className="mt-0.5 size-3.5 shrink-0" />
            {saveError}
          </p>
        )}

        <div className="flex items-center gap-2">
          <Button type="button" disabled={busy} onClick={save}>
            {busy ? <Loader2 aria-hidden className="size-4 animate-spin" /> : null}
            Install
          </Button>
          {/*
            Only offered when something is installed: "restore the manifest's
            version" is meaningless when the manifest's version is what is
            already in force.
          */}
          {state.source === "overlay" && (
            <Button type="button" variant="ghost" disabled={busy} onClick={reset}>
              Restore the blueprint&rsquo;s
            </Button>
          )}
          <span className="ml-auto text-2xs text-muted-foreground">
            {state.source === "overlay"
              ? "installed from the console"
              : state.source === "manifest"
                ? "from company.toml"
                : "nothing declared"}
          </span>
        </div>

        <p className="text-2xs text-muted-foreground">
          A room already deliberating keeps the grammar it opened with — the change
          takes effect on the next message that opens one.
        </p>
      </div>
    </Panel>
  );
}

/** Full class strings, never assembled — Tailwind scans source text. */
function cnState(ok: boolean): string {
  return ok
    ? "text-2xs text-muted-foreground"
    : "text-2xs font-medium text-status-failed-text";
}

function Panel({
  title,
  onClose,
  children,
}: {
  title: string;
  onClose?: () => void;
  children: React.ReactNode;
}) {
  return (
    <section className="rounded-lg border border-border bg-card p-4">
      <header className="mb-3 flex items-center justify-between">
        <h2 className="text-sm font-semibold">{title}</h2>
        {onClose && (
          <Button type="button" variant="ghost" size="sm" onClick={onClose}>
            Close
          </Button>
        )}
      </header>
      {children}
    </section>
  );
}
