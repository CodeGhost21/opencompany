import { useCallback, useEffect, useState, type ReactNode } from "react";
import { ChevronRight, Cpu, Mail, Pencil, Server, Sparkles, Users, Wrench } from "lucide-react";
import { toast } from "sonner";

import type { OpenCompanyClient } from "@/api/client";
import { setInboxEnabled } from "@/api/inbox";
import { listTasks } from "@/api/tasks";
import { isDesktopRuntime } from "@/api/transport";
import {
  cachedAcpModels,
  ensureAcpModels,
  type AcpHarnessModel,
} from "@/api/transport/desktop";
import { ApiError, type AgentDetailDto, type EditAgentInput, type HarnessDto } from "@/api/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import {
  agentEdits,
  draftFrom,
  draftIsValid,
  harnessEdit,
  harnessOptionLabel,
  isEditable,
  modelEdit,
  resolvedHarnessKind,
  summarizeGrants,
  tierLabel,
  type AgentDraft,
  type AgentFieldKey,
} from "@/lib/agent";
import { fetchBoardColumns } from "@/lib/board-columns";
import { roleSubtitle, toneFor } from "@/lib/team";
import { workloadByAssignee, type Workload } from "@/lib/team-workload";
import { cn } from "@/lib/utils";
import { Avatar } from "@/views/chat/Avatar";
import { AgentFields } from "@/views/team/AgentFields";

type Load = "loading" | "ready" | "missing" | "unsupported" | "error";

/**
 * The Harness select's value for "use the company default" (issue #1245's
 * harness-picker follow-up). Not `""`: an empty string is Base UI Select's own
 * placeholder/unset sentinel, so a real option needs a value of its own — the
 * boundary to `harnessEdit`'s `""`-means-default contract is translated at
 * the two points that cross it, `onEdit` and `saveHarnessAndModel`.
 */
const HARNESS_DEFAULT = "__default__";

/**
 * The model select's value for "leave it to the harness".
 *
 * A sentinel for the same reason [`HARNESS_DEFAULT`] is: `""` is Base UI
 * Select's own unset marker, so a real option needs a value of its own. It is
 * translated back to `""` — which `modelEdit` reads as "clear the override" —
 * at the single point that crosses the boundary, the select's `onValueChange`.
 */
const MODEL_HARNESS_DEFAULT = "__harness_default__";

/**
 * Why a detail read failed, in the operator's terms rather than the wire's.
 *
 * A `404` from `GET …/team/{agentId}` is **two different facts**, and the status
 * cannot tell them apart: a host that predates this route 404s the path it does
 * not serve, and a host that serves it 404s a teammate that is gone. Saying "no
 * such teammate" to the first sends an operator looking for a deletion that
 * never happened; saying "this host is too old" to the second hides a real
 * removal behind a version complaint.
 *
 * The roster settles it, but only if the right question is asked. "Did `GET
 * …/team` answer?" is not enough — the roster route is the *older* one, so an
 * out-of-date host answers it perfectly. The question that separates the two is
 * whether the roster still **contains this agent**:
 *
 * | `GET …/team` | outcome |
 * |---|---|
 * | lists this agent | the host has the roster but not the detail route → `unsupported` |
 * | omits this agent | the host serves both and the teammate is gone → `missing` |
 * | fails too | nothing is reachable; do not guess → `error` |
 *
 * Anything that is not a `404` — a transport failure, a `500` — is `error`. It
 * used to fall into `unsupported`, which told an operator their host was too old
 * when their network had simply dropped.
 */
async function classifyFailure(
  error: unknown,
  roster: () => Promise<{ id: string }[]>,
  agentId: string,
): Promise<Exclude<Load, "loading" | "ready">> {
  if (!(error instanceof ApiError) || error.status !== 404) return "error";
  const members = await roster().catch(() => null);
  if (members === null) return "error";
  return members.some((member) => member.id === agentId) ? "unsupported" : "missing";
}

/**
 * One agent, opened (issue #264).
 *
 * Before this the Team card was a dead end: a name, a role, and a destructive
 * Remove. None of what an agent *is* was reachable once it existed, so an
 * operator could not read the instructions it was defined with, could not see
 * which tools it may use or which desks it belongs to, and could not change any
 * of it. This is the screen that answers those questions, and edits the ones
 * the host says are editable.
 *
 * ## Read-only is a fact about the agent, not a state of this screen
 *
 * A **manifest** teammate is declared in the company's version-controlled
 * `company.toml`. Its fields are shown read-only, with the reason next to them:
 * the console does not rewrite the blueprint, so the edit belongs in the file.
 * An **overlay** teammate was added here and is edited here.
 *
 * Which is which comes from the host's own `editable` list rather than from a
 * rule this file re-implements. A console that decided for itself would
 * eventually offer a field the host refuses, and the operator would meet the
 * disagreement as a failed save instead of as a field that will not take an
 * edit.
 */
export function AgentDetailView({
  client,
  company,
  agentId,
  onBack,
}: {
  client: OpenCompanyClient;
  company: string | null;
  agentId: string;
  onBack: () => void;
}) {
  const [load, setLoad] = useState<Load>("loading");
  const [agent, setAgent] = useState<AgentDetailDto | null>(null);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<AgentDraft>({ name: "", role: "", description: "" });
  const [saving, setSaving] = useState(false);
  /**
   * What this teammate is on and carrying (issue #1141), or `null` when the
   * board could not be read — in which case the header states neither rather
   * than an invented "idle · 0 open".
   */
  const [workload, setWorkload] = useState<Workload | null>(null);
  /** An inbox write is in flight; the switch is held until the host answers. */
  const [inboxSaving, setInboxSaving] = useState(false);
  /**
   * The Harness & Model editor (issue #1245's harness-picker follow-up). Its
   * own small state, separate from `draft`/`editing`: both fields are
   * admin-only, and neither is part of the name/role/description group the
   * Instructions card edits together. One toggle covers both — they're saved
   * together, since a model override is only ever meaningful relative to
   * whichever harness this same save leaves the teammate on.
   */
  const [editingHarness, setEditingHarness] = useState(false);
  const [harnessDraft, setHarnessDraft] = useState(HARNESS_DEFAULT);
  const [modelDraft, setModelDraft] = useState("");
  const [savingHarness, setSavingHarness] = useState(false);
  /**
   * The company's declared harnesses, for the picker's options. Best-effort
   * and silent on failure, like `PolicySettings`' own `wiredTools`: an older
   * host without `GET {scope}/harnesses` still opens a teammate, the picker
   * just has nothing to offer beyond the free-text model field it already had.
   */
  const [harnesses, setHarnesses] = useState<HarnessDto[]>([]);

  const boot = useCallback(async () => {
    setLoad("loading");
    try {
      const detail = await client.getAgent(agentId, company);
      setAgent(detail);
      setDraft(draftFrom(detail));
      setLoad("ready");
    } catch (error) {
      setAgent(null);
      setLoad(await classifyFailure(error, () => client.listTeam(company), agentId));
    }
  }, [client, company, agentId]);

  useEffect(() => {
    void boot();
  }, [boot]);

  /**
   * The board, read for this one teammate — the same derivation the Company
   * cards use, from the same two reads (`lib/team-workload.ts`).
   *
   * Best-effort and never blocking: a host with no `…/tasks` route still opens
   * a teammate, it just cannot say what they are on.
   */
  useEffect(() => {
    let live = true;
    if (!company) {
      setWorkload(null);
      return;
    }
    void (async () => {
      const [tasks, columns] = await Promise.all([
        listTasks(client, company).catch(() => null),
        fetchBoardColumns(client, company).catch(() => null),
      ]);
      if (!live) return;
      // Empty columns is a host whose ledger list carries no board — an absence,
      // not a vocabulary. Same rule as the roster's cards.
      setWorkload(
        tasks && columns?.length
          ? (workloadByAssignee(tasks, columns).get(agentId) ?? { open: 0, status: "idle" })
          : null,
      );
    })();
    return () => {
      live = false;
    };
  }, [client, company, agentId]);

  /**
   * The Harness picker's options (issue #1245's harness-picker follow-up).
   * Read once per (client, company) rather than per edit, so opening the
   * editor is instant. Silent on failure — see the state's own docs.
   */
  useEffect(() => {
    let live = true;
    void client
      .listHarnesses(company)
      .then((next) => {
        if (live) setHarnesses(next);
      })
      .catch(() => {
        if (live) setHarnesses([]);
      });
    return () => {
      live = false;
    };
  }, [client, company]);

  /**
   * Give this teammate an inbox, or take it away (issue #1190).
   *
   * Moved here from the roster card, where it was the only control that wrote
   * to the host and sat one mis-click away while scanning thirteen cards. This
   * page already *reported* inbox state as a badge and offered no way to change
   * it; the read and the write live together now.
   *
   * Optimistic, then reverted on failure — the switch must never be left
   * claiming a state the host refused. Keyed on the roster agent id, which is
   * the `InboxStore` key the Inbox page reads and the ingest webhook files mail
   * under; nothing is persisted client-side.
   */
  async function toggleInbox(next: boolean) {
    if (!agent || inboxSaving) return;
    // Scoped to the teammate this call is *about*. This screen does not remount
    // when the hash names a different agent — it re-reads into the same state —
    // so a slow write for A that fails after the operator has stepped to B would
    // otherwise roll back B's switch, for a request B never made.
    const apply = (enabled: boolean) =>
      setAgent((held) => (held?.id === agentId ? { ...held, inboxEnabled: enabled } : held));
    apply(next);
    // One write in flight at a time. Two quick taps otherwise race, and the
    // host's last-writer-wins can settle on the opposite of what the switch shows.
    setInboxSaving(true);
    try {
      await setInboxEnabled(client, company, agentId, next);
    } catch (error) {
      apply(!next);
      toast.error(
        error instanceof ApiError && error.status === 404
          ? "This host doesn't offer teammate inboxes yet."
          : error instanceof Error
            ? error.message
            : "Couldn't change the inbox.",
      );
    } finally {
      setInboxSaving(false);
    }
  }

  async function save() {
    if (!agent) return;
    const edits = agentEdits(agent, draft);
    if (!edits) {
      setEditing(false);
      return;
    }
    setSaving(true);
    try {
      const updated = await client.updateAgent(agentId, edits, company);
      setAgent(updated);
      setDraft(draftFrom(updated));
      setEditing(false);
      toast.success("Teammate updated.");
    } catch (error) {
      toast.error(
        error instanceof ApiError && error.status === 409
          ? error.message
          : error instanceof Error
            ? error.message
            : "Couldn't save this teammate.",
      );
    } finally {
      setSaving(false);
    }
  }

  /**
   * Save the harness binding, the model override, or both (issue #1245's
   * harness-picker follow-up) — one `PATCH`, so the host's cross-field check
   * (a model only means anything on the harness this same save leaves the
   * teammate on) validates against the *new* binding, not a stale one from
   * before this edit. Either field left unchanged is simply omitted, the same
   * partial-save contract `agentEdits`/`save` follow above; a blank model
   * draft still clears with `null` rather than being refused.
   */
  async function saveHarnessAndModel() {
    if (!agent) return;
    const harness = harnessEdit(agent.harness, harnessDraft === HARNESS_DEFAULT ? "" : harnessDraft);
    const model = modelEdit(agent.model, modelDraft);
    if (harness === undefined && model === undefined) {
      setEditingHarness(false);
      return;
    }
    const edits: EditAgentInput = {};
    if (harness !== undefined) edits.harness = harness;
    if (model !== undefined) edits.model = model;

    setSavingHarness(true);
    try {
      const updated = await client.updateAgent(agentId, edits, company);
      setAgent(updated);
      setEditingHarness(false);
      toast.success("Harness updated.");
    } catch (error) {
      toast.error(
        error instanceof ApiError && (error.status === 403 || error.status === 400)
          ? error.message
          : error instanceof Error
            ? error.message
            : "Couldn't save this teammate's harness.",
      );
    } finally {
      setSavingHarness(false);
    }
  }

  return (
    <div className="flex-1 overflow-y-auto">
      <div className="mx-auto w-full max-w-3xl space-y-6 px-4 py-6">
        {/*
          A breadcrumb rather than a Back button (issue #1141). Back said where
          the operator had been; this says where they *are* — one teammate,
          inside the company — which is the question a linked page has to answer,
          and this page is linked from the org chart, the chat member pane and
          every "Not on a desk" chip. Arriving from any of those, "Back to team"
          named a page they had never seen.

          The Edit affordance sits on the same row for the same reason. It
          already existed, buried in the Instructions card halfway down, so a
          teammate read as a read-only record; the page's one editing action
          belongs where a page's actions go.
        */}
        <div className="flex flex-wrap items-center justify-between gap-2">
          <nav aria-label="Breadcrumb" data-testid="agent-breadcrumb">
            <ol className="flex flex-wrap items-center gap-1 text-sm">
              <li>
                <Button
                  variant="ghost"
                  size="sm"
                  className="-ml-2 h-7 px-2 text-muted-foreground"
                  onClick={onBack}
                  data-testid="agent-breadcrumb-company"
                >
                  Company
                </Button>
              </li>
              <li aria-hidden className="text-muted-foreground">
                <ChevronRight className="size-3.5" />
              </li>
              <li aria-current="page" className="min-w-0 truncate font-medium">
                {/* Named as soon as there is a name, and "Teammate" until then.
                    A crumb that appeared only once the read landed would move
                    the Edit button across the row as the page settled. */}
                {agent ? (agent.name?.trim() || agent.role) : "Teammate"}
              </li>
            </ol>
          </nav>
          {load === "ready" && agent && !editing && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => setEditing(true)}
              // Disabled with the reason, never absent. A manifest teammate is
              // declared in version control and the host says so through its
              // own `editable` list — an operator looking for the edit needs to
              // find out *why* there isn't one, not to conclude the console
              // forgot to build it.
              disabled={agent.editable.length === 0}
              title={
                agent.editable.length === 0
                  ? "This teammate is declared in your company blueprint (company.toml), so its name, role and instructions are edited there."
                  : undefined
              }
              data-testid="agent-edit"
            >
              <Pencil className="size-4" /> Edit
            </Button>
          )}
        </div>

        {load === "loading" && <Skeleton className="h-64 rounded-xl" />}

        {load === "missing" && (
          <EmptyState
            title="This teammate is no longer on the roster."
            body="It may have been removed. Go back to the team to see who is here now."
          />
        )}

        {load === "unsupported" && (
          <EmptyState
            title="This host can't open a teammate yet."
            body="Opening a teammate needs a newer host. The roster still works."
          />
        )}

        {load === "error" && (
          <EmptyState
            title="Couldn't load this teammate."
            body="The company host didn't answer. Try again in a moment."
          />
        )}

        {load === "ready" && agent && (
          <>
            <Identity agent={agent} />
            <FactLine agent={agent} workload={workload} />

            {/* The Edit action used to render here. It is on the page header
                now (issue #1141) — one editing action, in the place a page's
                actions live, rather than halfway down inside one of its cards. */}
            <Section
              title="Instructions"
              subtitle="What this teammate was defined to do. It frames every turn they take."
            >
              {editing ? (
                <div className="grid gap-4">
                  <AgentFields
                    idPrefix="agent-edit"
                    draft={draft}
                    onChange={(key: AgentFieldKey, value) =>
                      setDraft((d) => ({ ...d, [key]: value }))
                    }
                    readOnly={(key) => !isEditable(agent, key)}
                  />
                  <div className="flex justify-end gap-2">
                    <Button
                      variant="ghost"
                      onClick={() => {
                        setDraft(draftFrom(agent));
                        setEditing(false);
                      }}
                    >
                      Cancel
                    </Button>
                    <Button
                      onClick={() => void save()}
                      disabled={saving || !draftIsValid(agent, draft)}
                      data-testid="agent-save"
                    >
                      Save
                    </Button>
                  </div>
                </div>
              ) : (
                <>
                  <p
                    className="whitespace-pre-wrap text-sm text-muted-foreground"
                    data-testid="agent-description"
                  >
                    {agent.description?.trim() ||
                      "No instructions were written for this teammate."}
                  </p>
                  {agent.editable.length === 0 && (
                    <p className="text-xs text-muted-foreground" data-testid="agent-readonly-note">
                      This teammate is part of your company blueprint, so its name, role and
                      instructions are set in company.toml. Its daily budget can still be changed
                      from its card on the Company page.
                    </p>
                  )}
                </>
              )}
            </Section>

            <Tools agent={agent} />
            <HarnessAndModel
              agent={agent}
              harnesses={harnesses}
              editing={editingHarness}
              harnessDraft={harnessDraft}
              modelDraft={modelDraft}
              saving={savingHarness}
              onEdit={() => {
                setHarnessDraft(agent.harness ?? HARNESS_DEFAULT);
                setModelDraft(agent.model ?? "");
                setEditingHarness(true);
              }}
              onHarnessChange={setHarnessDraft}
              onModelChange={setModelDraft}
              onCancel={() => setEditingHarness(false)}
              onSave={() => void saveHarnessAndModel()}
            />
            <Inbox
              agent={agent}
              busy={inboxSaving}
              onToggle={(next) => void toggleInbox(next)}
            />
            <Desks agent={agent} />
          </>
        )}
      </div>
    </div>
  );
}

/** Name, role, id, and the two facts that classify an agent. */
function Identity({ agent }: { agent: AgentDetailDto }) {
  const display = agent.name?.trim() || agent.role;
  // #1208, on the page a teammate *is*. `display` already falls back to the
  // role, and a manifest-declared agent has no `name` at all, so the line under
  // the title was the title again on every teammate in every shipped company.
  const subtitle = roleSubtitle(display, agent.role);
  const tone = toneFor(agent.id || display);
  return (
    <div className="flex items-start gap-4">
      {/* The header of the page a teammate *is* — the one screen that should
          never be the one showing letters (issue #1181). 56px. */}
      <Avatar
        name={display}
        tone={tone}
        className="size-14 rounded-xl text-base"
        data-testid="agent-avatar"
      />
      <div className="min-w-0 flex-1 space-y-2">
        <div>
          <h2 className="truncate text-2xl font-semibold tracking-tight" data-testid="agent-name">
            {display}
          </h2>
          {subtitle && (
            <p className="truncate text-sm text-muted-foreground" data-testid="agent-role">
              {subtitle}
            </p>
          )}
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Badge variant="secondary" className="gap-1" data-testid="agent-tier">
            <Sparkles className="size-3" /> {tierLabel(agent)}
          </Badge>
          <Badge variant="outline" data-testid="agent-source">
            {agent.source === "manifest" ? "Company blueprint" : "Added here"}
          </Badge>
          {agent.inboxEnabled && (
            <Badge variant="outline" className="gap-1">
              <Mail className="size-3" /> Inbox
            </Badge>
          )}
          <span className="font-mono text-xs text-muted-foreground" data-testid="agent-id">
            {agent.id}
          </span>
        </div>
      </div>
    </div>
  );
}

/**
 * The three running facts about a teammate, on one line (issue #1141): what
 * they are on, how much is on them, and what today has cost.
 *
 * Every part is omitted independently when its source is silent, and none is
 * defaulted. A host that cannot answer the board draws no status and no count —
 * not "idle · 0 open", which is a claim — and an uncapped teammate draws no
 * spend line, because absence *is* the uncapped signal on the wire and `$0.00`
 * would read as a teammate capped at nothing.
 */
function FactLine({
  agent,
  workload,
}: {
  agent: AgentDetailDto;
  workload: Workload | null;
}) {
  const capped = agent.budgetUsdDaily !== undefined;
  if (!workload && !capped) return null;
  const working = workload?.status === "working";
  return (
    <div
      className="flex flex-wrap items-center gap-x-4 gap-y-2 text-sm text-muted-foreground"
      data-testid="agent-facts"
    >
      {workload && (
        <>
          <span className="flex items-center gap-1.5">
            <span
              className={cn(
                "size-2 shrink-0 rounded-full",
                working ? "bg-status-running" : "bg-status-idle",
              )}
              aria-hidden
            />
            <span
              className={cn(
                "font-medium",
                working ? "text-status-running-text" : "text-status-idle-text",
              )}
              data-testid="agent-status"
            >
              {working ? "Working" : "Idle"}
            </span>
          </span>
          <span data-testid="agent-tasks">
            {workload.open === 1 ? "1 open task" : `${workload.open} open tasks`}
          </span>
        </>
      )}
      {capped && (
        <span data-testid="agent-spend">
          Today ${(agent.spentTodayUsd ?? 0).toFixed(2)} of $
          {(agent.budgetUsdDaily ?? 0).toFixed(2)}
        </span>
      )}
    </div>
  );
}

/**
 * The tool grants, resolved.
 *
 * Three facts, because the difference between them is the whole reason this
 * section exists. What the agent holds. Whether it holds it because it asked or
 * because it asked for nothing and inherited the company's grant. And what it
 * asked for and did not get, which is the line an operator checking a tool
 * change is actually looking for and which no surface showed before.
 */
function Tools({ agent }: { agent: AgentDetailDto }) {
  const summary = summarizeGrants(agent.tools);
  return (
    <Section
      title="Tools"
      subtitle={
        summary.standardGrant
          ? "This teammate lists no tools of its own, so it holds everything the company allows."
          : "What this teammate asked for, narrowed by what the company allows."
      }
    >
      {summary.effective.length === 0 ? (
        <p className="text-sm text-muted-foreground" data-testid="agent-tools-empty">
          {/* Both ways of holding nothing land here, and they are not the same
              fact. An agent that asked for nothing under a company that allows
              nothing has been refused nothing. */}
          {summary.standardGrant
            ? "This teammate has no tools, because the company allows none."
            : "This teammate has no tools. Nothing it asked for is covered by the company tool list."}
        </p>
      ) : (
        <div className="flex flex-wrap gap-2" data-testid="agent-tools">
          {summary.effective.map((glob) => (
            <Badge key={glob} variant="secondary" className="gap-1 font-mono text-xs">
              <Wrench className="size-3" /> {glob}
            </Badge>
          ))}
        </div>
      )}
      {summary.dropped.length > 0 && (
        <div className="space-y-1" data-testid="agent-tools-dropped">
          <p className="text-xs text-muted-foreground">
            Asked for but not granted, because the company tool list does not cover it:
          </p>
          <div className="flex flex-wrap gap-2">
            {summary.dropped.map((glob) => (
              <Badge key={glob} variant="outline" className="font-mono text-xs line-through">
                {glob}
              </Badge>
            ))}
          </div>
        </div>
      )}
      <p className="text-xs text-muted-foreground">
        Company tool list: {agent.tools.companyAllow.join(", ") || "nothing allowed"}
      </p>
    </Section>
  );
}

/**
 * This teammate's harness binding and its own model override (issue #1245's
 * harness-picker follow-up).
 *
 * One section, one edit control, for both: they are saved together (see
 * `saveHarnessAndModel`'s own docs on why), and the model field's very
 * relevance depends on which harness is selected — showing it in a card of
 * its own would let an operator set a model with no harness in view to judge
 * whether it does anything.
 *
 * Admin-only, same as `tools` (the "cost/scope decision" reasoning), and
 * `agent.editable` says so per actor — a member sees the values with no edit
 * affordance, the same shape `agent.editable.length === 0` already gives a
 * manifest teammate above.
 */
function HarnessAndModel({
  agent,
  harnesses,
  editing,
  harnessDraft,
  modelDraft,
  saving,
  onEdit,
  onHarnessChange,
  onModelChange,
  onCancel,
  onSave,
}: {
  agent: AgentDetailDto;
  harnesses: HarnessDto[];
  editing: boolean;
  harnessDraft: string;
  modelDraft: string;
  saving: boolean;
  onEdit: () => void;
  onHarnessChange: (value: string) => void;
  onModelChange: (value: string) => void;
  onCancel: () => void;
  onSave: () => void;
}) {
  const editable = agent.editable.includes("harness") || agent.editable.includes("model");
  const declaredKind = resolvedHarnessKind(harnesses, agent.harness);
  const draftKind = resolvedHarnessKind(
    harnesses,
    harnessDraft === HARNESS_DEFAULT ? undefined : harnessDraft,
  );
  const defaultHarness = harnesses.find((h) => h.default);

  /**
   * The models the drafted harness advertises.
   *
   * Fetched when the editor opens on an ACP harness, and again whenever the
   * operator picks a different one — the lists are per harness and share no
   * ids, so carrying claude's over to codex would offer models it will
   * silently refuse. Cached in the transport, so switching back and forth
   * spawns nothing after the first look.
   */
  const [models, setModels] = useState<AcpHarnessModel[]>([]);
  const draftHarnessId = harnessDraft === HARNESS_DEFAULT ? defaultHarness?.id : harnessDraft;

  useEffect(() => {
    if (!editing || draftKind !== "acp" || !draftHarnessId) {
      setModels([]);
      return;
    }
    let live = true;
    setModels(cachedAcpModels(draftHarnessId));
    void ensureAcpModels(draftHarnessId).then((found) => {
      if (live) setModels(found);
    });
    return () => {
      live = false;
    };
  }, [editing, draftKind, draftHarnessId]);

  const unlistedModel =
    modelDraft && !models.some((m) => m.value === modelDraft) ? modelDraft : undefined;
  /**
   * What the harness would use if this teammate pins nothing — the entry the
   * adapter itself reports as current, not a guess. Absent when the adapter
   * names none (`claude-agent-acp` leads its list with a synthetic `default`
   * instead), in which case the option stays unqualified rather than
   * inventing an answer.
   */
  const currentModel = models.find((m) => m.current);
  const modelLabel = () => {
    if (!modelDraft) return "Whatever the harness defaults to";
    const found = models.find((m) => m.value === modelDraft);
    return found ? (found.name ?? found.value) : modelDraft;
  };

  // `Select.Value` cannot read a label off its matching `SelectItem` here —
  // `SelectContent` (and every item in it) is portal-rendered only while the
  // popup is open, so a trigger that has never been opened has nothing to
  // read from and falls back to the raw value (issue #1245's harness-picker
  // follow-up shipped with exactly that: the trigger read the literal
  // `__default__` sentinel until this closed-form label was added). Passing
  // the render-function form sidesteps the mount-order dependency entirely.
  const harnessLabel = (value: string) =>
    value === HARNESS_DEFAULT
      ? `Company default${defaultHarness ? ` (${harnessOptionLabel(defaultHarness)})` : ""}`
      : (() => {
          const found = harnesses.find((h) => h.id === value);
          return found ? harnessOptionLabel(found) : value;
        })();

  return (
    <Section
      title="Harness & model"
      subtitle="Which coding engine this teammate runs on, and — on an ACP harness (an operator's own coding CLI) — which model to pin it to."
      action={
        editable && !editing ? (
          <Button variant="ghost" size="sm" onClick={onEdit} data-testid="agent-harness-edit">
            <Pencil className="size-4" />
          </Button>
        ) : undefined
      }
    >
      {editing ? (
        <div className="space-y-3">
          <Select value={harnessDraft} onValueChange={(value) => onHarnessChange(value ?? HARNESS_DEFAULT)}>
            <SelectTrigger className="w-full" data-testid="agent-harness-select">
              <SelectValue>{harnessLabel}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={HARNESS_DEFAULT}>
                Company default{defaultHarness ? ` (${harnessOptionLabel(defaultHarness)})` : ""}
              </SelectItem>
              {harnesses.map((harness) => (
                <SelectItem key={harness.id} value={harness.id}>
                  {harnessOptionLabel(harness)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          {draftKind === "acp" ? (
            models.length > 0 ? (
              <>
                <Select
                  value={modelDraft === "" ? MODEL_HARNESS_DEFAULT : modelDraft}
                  onValueChange={(value) =>
                    onModelChange(!value || value === MODEL_HARNESS_DEFAULT ? "" : value)
                  }
                >
                  <SelectTrigger className="w-full" data-testid="agent-model-select">
                    <SelectValue>{modelLabel}</SelectValue>
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value={MODEL_HARNESS_DEFAULT}>
                      Whatever the harness defaults to
                      {/* Named rather than left abstract: leaving this alone
                          is the common choice, and an operator should not
                          have to guess what they are choosing. The adapter
                          reports which entry is current, so this tracks the
                          CLI's own default instead of asserting one. */}
                      {currentModel && (
                        <span className="text-muted-foreground">
                          {" "}
                          — {currentModel.name ?? currentModel.value}
                        </span>
                      )}
                    </SelectItem>
                    {models.map((model) => (
                      <SelectItem key={model.value} value={model.value}>
                        {model.name ?? model.value}
                        {model.description && (
                          <span className="text-muted-foreground"> — {model.description}</span>
                        )}
                      </SelectItem>
                    ))}
                    {/* A value the harness no longer advertises is still
                        offered, so opening the editor cannot silently drop a
                        pin somebody set deliberately. The list moves when the
                        CLI updates; the teammate's setting should not. */}
                    {unlistedModel && (
                      <SelectItem value={unlistedModel}>
                        {unlistedModel}
                        <span className="text-muted-foreground"> — no longer offered</span>
                      </SelectItem>
                    )}
                  </SelectContent>
                </Select>
                <p className="text-xs text-muted-foreground">
                  Read from the harness itself, so these are the models it will actually
                  accept.
                </p>
              </>
            ) : (
              // No list to offer. Free text rather than an empty dropdown:
              // "nothing cached yet" (a browser, or a harness never probed)
              // is not the same as "this harness has no models", and an empty
              // picker would assert the second.
              <>
                <Input
                  value={modelDraft}
                  onChange={(event) => onModelChange(event.target.value)}
                  placeholder="Leave blank to use the harness's own default"
                  data-testid="agent-model-input"
                />
                <p className="text-xs text-muted-foreground">
                  {isDesktopRuntime()
                    ? "This harness hasn't reported its models yet — open Settings › External harnesses to check it."
                    : "Open the desktop app to pick from the models this harness offers."}
                </p>
              </>
            )
          ) : (
            <p className="text-xs text-muted-foreground">
              A model override only applies on an ACP harness — pick one above to set one.
            </p>
          )}
          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={onCancel} disabled={saving}>
              Cancel
            </Button>
            <Button onClick={onSave} disabled={saving} data-testid="agent-harness-save">
              Save
            </Button>
          </div>
        </div>
      ) : (
        <div className="flex flex-wrap items-center gap-2">
          <Badge variant="secondary" className="gap-1 font-mono text-xs" data-testid="agent-harness">
            <Server className="size-3" />
            {agent.harness ?? (defaultHarness ? `${defaultHarness.id} (default)` : "default harness")}
          </Badge>
          {agent.model ? (
            <Badge variant="secondary" className="gap-1 font-mono text-xs" data-testid="agent-model">
              <Cpu className="size-3" /> {agent.model}
            </Badge>
          ) : (
            <span className="text-sm text-muted-foreground" data-testid="agent-model-empty">
              {declaredKind === "acp"
                ? "No model override set — uses the harness's own default."
                : "No model override (this harness has no ACP transport to steer)."}
            </span>
          )}
        </div>
      )}
    </Section>
  );
}

/**
 * Whether mail addressed to this teammate lands anywhere (issue #1190).
 *
 * A per-teammate setting, on the teammate's own page — not a switch in a grid
 * of cards, which is what it was. The subtitle says what turning it on actually
 * does, because "Inbox" alone does not: an inbox is an address the outside
 * world can reach, which is a different kind of decision from the rest of this
 * screen and worth one sentence.
 */
function Inbox({
  agent,
  busy,
  onToggle,
}: {
  agent: AgentDetailDto;
  /** A write is in flight — the switch is held rather than allowed to race. */
  busy: boolean;
  onToggle: (next: boolean) => void;
}) {
  return (
    <Section
      title="Inbox"
      subtitle="Give this teammate an address of its own, so mail routed to it arrives here rather than nowhere."
    >
      <label className="flex cursor-pointer items-center justify-between gap-3">
        <span className="flex items-center gap-2 text-sm">
          <Mail className="size-4 text-muted-foreground" />
          {agent.inboxEnabled ? "This teammate has an inbox." : "This teammate has no inbox."}
        </span>
        <Switch
          checked={agent.inboxEnabled}
          disabled={busy}
          onCheckedChange={onToggle}
          aria-label="Give this teammate an inbox"
          data-testid="agent-inbox-toggle"
        />
      </label>
    </Section>
  );
}

/** Desk membership, with the lead named. */
function Desks({ agent }: { agent: AgentDetailDto }) {
  return (
    <Section
      title="Desks"
      subtitle="The desks this teammate works on. A desk hands its work to its lead first."
    >
      {agent.desks.length === 0 ? (
        <p className="text-sm text-muted-foreground" data-testid="agent-desks-empty">
          This teammate is not on any desk.
        </p>
      ) : (
        <div className="flex flex-wrap gap-2" data-testid="agent-desks">
          {agent.desks.map((desk) => (
            <Badge key={desk.id} variant="secondary" className="gap-1">
              <Users className="size-3" /> {desk.name}
              {desk.lead && <span className="text-xs opacity-70">(lead)</span>}
            </Badge>
          ))}
        </div>
      )}
    </Section>
  );
}

function Section({
  title,
  subtitle,
  action,
  children,
}: {
  title: string;
  subtitle: string;
  action?: ReactNode;
  children: ReactNode;
}) {
  return (
    <Card>
      <CardContent className="space-y-3 py-4">
        <div className="flex items-start justify-between gap-3">
          <div className="space-y-1">
            <h3 className="font-medium">{title}</h3>
            <p className="text-xs text-muted-foreground">{subtitle}</p>
          </div>
          {action}
        </div>
        {children}
      </CardContent>
    </Card>
  );
}

function EmptyState({ title, body }: { title: string; body: string }) {
  return (
    <Card>
      <CardContent className="space-y-1 py-8 text-center">
        <p className="font-medium">{title}</p>
        <p className="text-sm text-muted-foreground">{body}</p>
      </CardContent>
    </Card>
  );
}
