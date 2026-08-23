// The body of an approval card, shared by the two surfaces that decide one
// (issue #379): the Approvals page and the inline card in the conversation the
// request came from.
//
// Extracted rather than duplicated because the two must say the *same thing*.
// The whole point of raising a request in the conversation is that it is the
// same request, told in full — what will happen, who is asking, what it is for,
// how long it has waited. Two copies of that would drift, and the half that
// drifted would be the one nobody was reading when it mattered.
//
// What is deliberately NOT here: the action buttons and the deciding state.
// The page's buttons resolve with the default response shape; the inline card's
// resolve detached (#391), because a body-delivered reply plus its SSE echo
// would put one continuation into the channel twice. Same content, different
// verbs — so the verbs stay with their owners.

import { useEffect, useMemo, useState } from "react";
import {
  AtSign,
  ChevronDown,
  ChevronUp,
  CreditCard,
  EyeOff,
  FileSignature,
  FileText,
  Globe,
  KeyRound,
  Mail,
  MessageSquare,
  Repeat,
  RefreshCw,
  Rocket,
  ShieldCheck,
  SquareKanban,
  Workflow,
  type LucideIcon,
} from "lucide-react";

import type { OpenCompanyClient } from "@/api/client";
import { GRANT_DURATIONS, type ApprovalSummary, type GrantScope } from "@/api/types";
import {
  approvalAction,
  approvalDeadline,
  type DeadlineTone,
  money,
  payloadAge,
  payloadLines,
} from "@/lib/language";
import { cn } from "@/lib/utils";

const KIND_ICONS: Record<string, LucideIcon> = {
  "payment.send": CreditCard,
  "subscription.start": Repeat,
  "email.send": Mail,
  "dm.external": MessageSquare,
  "filing.submit": FileText,
  "contract.accept": FileSignature,
  "external.publish": Globe,
  "website.deploy": Rocket,
  "handle.register": AtSign,
  "handle.renew": RefreshCw,
  "key.rotate": KeyRound,
  "workflow.approve": Workflow,
};

/**
 * The host's consequence group in the operator's vocabulary (#1426).
 *
 * `group` is derived from the tool call and its arguments by the host, which
 * is the only layer that can know the actual consequence. `other` is the
 * internal catch-all, while an absent group means an older host: both stay
 * deliberately unmarked so the badge and tint retain their signal.
 *
 * The tints come from the identity palette (`--tone-1` … `--tone-5`), not from
 * `--status-*`. A consequence group is a category, which is what that palette
 * exists for — `docs/brand/README.md` ("Identity is not status") reserves the
 * five status hues for run state and says not to reuse one for anything that
 * is not that status. These badges did: a pending hire approval was painted
 * the green that means "finished cleanly", and spend the red that means
 * "failed". The identity palette deliberately holds no amber, green or red,
 * so a queue of approvals can no longer read as a queue of run outcomes.
 *
 * Six groups over five tones, so `spend` and `hire` share tone 4 — the two
 * that most often move the same money. Sharing is safe here and precedented
 * (`lib/team.ts`, `lib/skills.ts` both fold more names onto these five): the
 * badge always carries its own icon and label, so colour is never the only
 * carrier of the distinction, which is the rule the brand doc actually sets.
 */
const APPROVAL_CONSEQUENCES = {
  spend: { label: "Spends money", iconClass: "bg-tone-4/15 text-tone-4-text" },
  send: { label: "Leaves the company", iconClass: "bg-tone-2/15 text-tone-2-text" },
  sign: { label: "Makes a commitment", iconClass: "bg-tone-1/15 text-tone-1-text" },
  // Not "Goes public". The group spans genuinely external publishing —
  // `repo_publish`'s push to the real remote, `hosting_launch_site`,
  // `hosting_add_domain` — *and* `publish_artifact`, which writes only into the
  // company's own workspace and artifact chain and sends nothing anywhere. A
  // card reading "Goes public" over that hand-off is the misleading label
  // `language.ts` already refuses to print for the same tool. "Publishes work"
  // is true under either reading: it is the step that makes finished work
  // visible past the agent's sandbox, whoever is on the other side of it.
  publish: { label: "Publishes work", iconClass: "bg-tone-3/15 text-tone-3-text" },
  // `hire` and `identity` are separate rows in the taxonomy
  // (`docs/spec/company-brain/approvals.md`) and had been sharing one label,
  // which hid exactly the distinction this change exists to draw. `hire` is an
  // outbound engagement with another company or the firing of a vendor;
  // `identity` is handle registration and renewal, key rotation, delegated
  // signer mint/expand and `composio_authorize` — the company's own name and
  // credentials, not who it does business with.
  hire: { label: "Engages or drops a counterparty", iconClass: "bg-tone-4/15 text-tone-4-text" },
  identity: { label: "Changes its identity or keys", iconClass: "bg-tone-5/15 text-tone-5-text" },
} as const;

type ApprovalConsequence = (typeof APPROVAL_CONSEQUENCES)[keyof typeof APPROVAL_CONSEQUENCES];

/** The marked consequence, or nothing for internal and old-host approvals. */
export function approvalConsequence(group: ApprovalSummary["group"]): ApprovalConsequence | null {
  if (group == null || group === "other") return null;
  return APPROVAL_CONSEQUENCES[group];
}

/**
 * Every distinct consequence a batch of approvals carries, in the order the
 * batch first raises each one.
 *
 * A turn that parks several calls renders as one card with one Approve, so the
 * warning has to survive the consolidation: a batch of three outbound sends
 * still leaves the company, and a mixed batch spends money *and* leaves it.
 * Returning the distinct set rather than a single verdict is what lets the
 * headline stay honest either way — one badge when the batch agrees with
 * itself, one per consequence when it does not.
 *
 * Deduplicated by label, not by group: `hire` and `identity` are separate
 * groups, while `spend` and `hire` share a tint, so the label is the thing an
 * operator actually reads twice.
 */
export function batchConsequences(approvals: ApprovalSummary[]): ApprovalConsequence[] {
  const seen = new Set<string>();
  const out: ApprovalConsequence[] = [];
  for (const a of approvals) {
    const consequence = approvalConsequence(a.group);
    if (!consequence || seen.has(consequence.label)) continue;
    seen.add(consequence.label);
    out.push(consequence);
  }
  return out;
}

/**
 * How much of a payload is shown before it is clamped. Past either bound the
 * block collapses behind a "Show everything" toggle — a queue of approvals has
 * to stay scannable, and a forty-line argument object buries the next card.
 */
const PREVIEW_LINES = 3;
const PREVIEW_VALUE_CHARS = 160;

/** The glyph for an effect kind; a shield for one this console doesn't know. */
export function approvalIcon(kind: string): LucideIcon {
  return KIND_ICONS[kind] ?? ShieldCheck;
}

/**
 * The headline row: the glyph, what will happen, and the amount when there is
 * one. Takes its actions as a slot so each surface supplies its own.
 */
export function ApprovalHeadline({
  approval: a,
  actions,
}: {
  approval: ApprovalSummary;
  actions?: React.ReactNode;
}) {
  const Icon = approvalIcon(a.kind);
  const consequence = approvalConsequence(a.group);
  return (
    <div className="flex items-start gap-4">
      <div
        className={cn(
          "flex size-10 shrink-0 items-center justify-center rounded-lg",
          consequence?.iconClass ?? "bg-muted text-foreground",
        )}
      >
        <Icon className="size-5" />
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
          <p className="font-medium">{approvalAction(a)}</p>
          {consequence && (
            <span className="rounded-full bg-muted px-2 py-0.5 text-2xs font-medium text-foreground">
              {consequence.label}
            </span>
          )}
        </div>
        {a.amount_usd != null && (
          <p className="text-xs font-medium text-muted-foreground">{money(a.amount_usd)}</p>
        )}
        {/*
         * #618: an absent amount normally means "this effect involves no
         * money". When it was withheld it means the opposite could be true, and
         * a member reading a hidden payment as a free action is exactly the
         * misreading the flag exists to prevent.
         */}
        {a.amount_usd == null && a.contents_hidden && (
          <p className="text-xs font-medium text-muted-foreground italic">Amount hidden</p>
        )}
      </div>
      {actions && <div className="flex shrink-0 gap-2">{actions}</div>}
    </div>
  );
}

/**
 * The footer line: who asked, which card it belongs to, how long it has waited,
 * and whatever status the surface wants to append.
 */
export function ApprovalMeta({
  approval: a,
  now,
  askerNames,
  status,
}: {
  approval: ApprovalSummary;
  now: number;
  askerNames: Map<string, string>;
  /** Trailing status text ("Waiting for the teammate…", "Approved"), if any. */
  status?: React.ReactNode;
}) {
  const taskId = a.task?.link === "task" ? a.task.id : null;
  // An id the roster does not know still beats no attribution at all — the
  // operator can at least tell two askers apart.
  const asker = a.agent ? (askerNames.get(a.agent) ?? a.agent) : null;
  // #1024, computed once: the age, and whether this card should say it loudly.
  const age = payloadAge(a, now);
  // #1403, likewise: the deadline's words and how loudly to say them. Computed
  // unconditionally and read only inside the guard below, so a host that
  // reports no deadline still renders nothing.
  const deadline = approvalDeadline(a.expires_at_millis ?? 0, now);

  return (
    <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
      {asker && (
        <>
          <span>
            Asked by <span className="font-medium text-foreground">{asker}</span>
          </span>
          <span aria-hidden>·</span>
        </>
      )}
      {taskId && (
        <>
          <a
            href={`#/tasks/${encodeURIComponent(taskId)}`}
            className="flex w-fit items-center gap-1 rounded-full bg-accent px-2 py-0.5 font-medium text-accent-foreground transition-opacity hover:opacity-80"
          >
            <SquareKanban className="size-3 shrink-0" />
            Open the card
          </a>
          <span aria-hidden>·</span>
        </>
      )}
      {/*
       * #1024. The same integer means two different things depending on where it
       * sits. In this footer, between "Asked by Maya" and "Open the card", a bare
       * "5d ago" reads as QUEUE LATENCY — how long the operator's backlog has held
       * this — a fact about the queue. What decides an outbound send is that the
       * PAYLOAD is five days old, a fact about the content. A digest built from
       * 13 Aug mailed as "Weekly Digest — 18 Aug" the moment a backlog was cleared,
       * and the report says why nobody caught it: "from the operator's side it
       * looked like a routine send." The signal was not missing — it was
       * unlabelled, and dressed as routing metadata.
       *
       * Wording and emphasis both come from `payloadAge`, so they are testable as
       * a string rather than only as rendered output.
       */}
      <span className={age.emphasise ? "font-medium text-foreground" : undefined}>
        {age.text}
      </span>
      {/* The deadline (#971), beside how old the payload is — the two halves of
          "is this still worth deciding?".

          Rendered only when the host reports one. An absent
          `expires_at_millis` means the host does not have deadlines, NOT that
          this card has none, so the console shows nothing rather than
          computing a deadline nothing would enforce: an operator who acted on
          an invented "in 3h" would be refused.

          Wording and tone both come from `approvalDeadline` (#1403), so what
          this says is testable as a string rather than only as rendered output
          — the same split `payloadAge` above already uses. The tone is not
          decoration: this line is the only thing on the card that says the
          decision will be taken *for* the operator if they keep scrolling, and
          it used to say it in the same grey as everything else. Amber is what
          the rest of the console already means by "parked until a person acts"
          (`--status-blocked`), and the passed state borrows the failed token
          because a deadline that ran out is a terminal no. */}
      {typeof a.expires_at_millis === "number" && (
        <>
          <span aria-hidden>·</span>
          <span className={deadlineToneClass(deadline.tone)}>{deadline.text}</span>
        </>
      )}
      {status && (
        <>
          <span aria-hidden>·</span>
          <span className="text-foreground">{status}</span>
        </>
      )}
    </div>
  );
}

/**
 * How an approval's deadline is typeset, by tone (#1403).
 *
 * Weight as well as colour in both loud arms, so the distinction survives
 * greyscale and the colour-vision deficiencies red/amber is worst for. `normal`
 * returns nothing at all and inherits the meta line's muted grey, which keeps
 * the quiet case exactly as it shipped — the emphasis is only worth anything if
 * most cards do not have it.
 */
function deadlineToneClass(tone: DeadlineTone): string | undefined {
  if (tone === "passed") return "font-medium text-status-failed-text";
  if (tone === "soon") return "font-medium text-status-blocked-text";
  return undefined;
}

/**
 * The tool call's own arguments, verbatim (#372) — `null` when the effect
 * carries none, so a caller can skip the block entirely.
 *
 * Monospace and wrapping rather than truncating: a shell command cut off
 * mid-flag is exactly as un-decidable as no command at all, and `break-all` is
 * what keeps a long unbroken path or URL inside the card. Everything here was
 * redacted and bounded by the host, so `[redacted]` is a value the console
 * renders — never one it computes.
 */
export function ApprovalPayload({ approval }: { approval: ApprovalSummary }) {
  const lines = useMemo(() => payloadLines(approval), [approval]);
  const [expanded, setExpanded] = useState(false);

  // Withheld by role (#618) — say so. Returning `null` here would be the one
  // wrong answer: it is what an approval with no arguments renders as, so a
  // member would read a hidden payment as an ordinary empty card. The point of
  // the flag is that "nothing to show" and "not shown to you" must not look
  // alike.
  if (approval.contents_hidden) {
    return (
      <div className="flex items-center gap-2 rounded-lg border border-dashed bg-muted/40 px-3 py-2 text-xs text-muted-foreground">
        <EyeOff className="size-3.5 shrink-0" />
        <span>
          Details hidden by your role. An admin can see what this approval will do and decide it.
        </span>
      </div>
    );
  }

  if (lines.length === 0) return null;

  const clampable =
    lines.length > PREVIEW_LINES || lines.some((l) => l.value.length > PREVIEW_VALUE_CHARS);
  const shown = expanded || !clampable ? lines : lines.slice(0, PREVIEW_LINES);

  return (
    <div className="rounded-lg border bg-muted/40 px-3 py-2">
      <div
        className={cn(
          "space-y-1 font-mono text-xs break-all whitespace-pre-wrap",
          clampable && !expanded && "max-h-24 overflow-hidden",
        )}
      >
        {shown.map((line) => (
          <div key={line.label}>
            <span className="text-muted-foreground">{line.label}: </span>
            <span className="text-foreground">{line.value}</span>
          </div>
        ))}
      </div>
      {clampable && (
        <button
          type="button"
          onClick={() => setExpanded((v) => !v)}
          className="mt-1.5 flex items-center gap-1 text-xs text-muted-foreground transition-colors hover:text-foreground"
        >
          {expanded ? <ChevronUp className="size-3" /> : <ChevronDown className="size-3" />}
          {expanded ? "Show less" : "Show everything"}
        </button>
      )}
    </div>
  );
}

/**
 * The scope control: what this approve buys (#374).
 *
 * Rendered **only** when the host marked the card `broadly_grantable`, so the
 * operator is never offered a choice the host would refuse. That is UX, not the
 * boundary — the host re-checks and answers 400 — but offering an option that
 * cannot work is its own kind of lie.
 *
 * Two options, and the default needs no interaction at all: doing nothing
 * approves once, exactly as before this existed. Picking the broader option
 * forces a duration, because there is no unbounded form to fall back to; the
 * radio and the duration are one control rather than two so an operator cannot
 * arrive at "for a period, unspecified".
 *
 * Lives here rather than in either view because both surfaces that decide an
 * approval must say the same thing about what a decision means. Two copies of
 * this wording would drift, and the half that drifted would be the one somebody
 * was reading when it mattered.
 */
export function ApprovalScopeControl({
  approval: a,
  askerNames,
  scope,
  onChange,
  disabled,
}: {
  approval: ApprovalSummary;
  /** Roster names, so the sentence says who — the same map the meta line uses. */
  askerNames: Map<string, string>;
  scope: GrantScope;
  onChange: (scope: GrantScope) => void;
  disabled?: boolean;
}) {
  const name = `scope-${a.id}`;
  if (!a.broadly_grantable) return null;

  return (
    <fieldset
      disabled={disabled}
      className="rounded-lg border bg-muted/30 px-3 py-2 text-sm disabled:opacity-60"
    >
      <legend className="px-1 text-xs text-muted-foreground">If you approve</legend>
      <div className="flex flex-col gap-1.5">
        <label className="flex items-center gap-2">
          <input
            type="radio"
            name={name}
            checked={scope.kind === "once"}
            onChange={() => onChange({ kind: "once" })}
            className="size-3.5 accent-primary"
          />
          <span>Just this once</span>
        </label>
        <label className="flex flex-wrap items-center gap-2">
          <input
            type="radio"
            name={name}
            checked={scope.kind === "tool"}
            // Picking the broader scope commits to a duration immediately —
            // the first option, not an empty one — so there is no state in
            // which "for a period" is selected with no period.
            onChange={() =>
              onChange({ kind: "tool", expiresInMillis: GRANT_DURATIONS[0].millis })
            }
            className="size-3.5 accent-primary"
          />
          <span>Let {askerLabel(a, askerNames)} use this tool for</span>
          <select
            value={scope.kind === "tool" ? scope.expiresInMillis : GRANT_DURATIONS[0].millis}
            disabled={scope.kind !== "tool"}
            onChange={(e) =>
              onChange({ kind: "tool", expiresInMillis: Number(e.target.value) })
            }
            aria-label="How long this permission lasts"
            className="rounded-md border bg-background px-1.5 py-0.5 text-xs disabled:opacity-50"
          >
            {GRANT_DURATIONS.map((d) => (
              <option key={d.millis} value={d.millis}>
                {d.label}
              </option>
            ))}
          </select>
        </label>
      </div>
      {scope.kind === "tool" && (
        <p className="mt-1.5 px-1 text-xs text-muted-foreground">
          It won't ask again for this tool until then — with any arguments. You can take it
          back from Standing permissions at any time.
        </p>
      )}
    </fieldset>
  );
}

/**
 * Who the broader scope would be granted to, by name.
 *
 * Falls back to the raw agent id, then to "this teammate". Naming the wrong
 * teammate would be worse than naming none, so this only ever narrows from what
 * the host actually said — it never guesses.
 */
function askerLabel(a: ApprovalSummary, askerNames: Map<string, string>): string {
  if (!a.agent) return "this teammate";
  return askerNames.get(a.agent) ?? a.agent;
}

/**
 * Agent id → display name, for the "Asked by" line.
 *
 * One roster read per company, not one per card: the ids on the queue are
 * roster ids, and the roster is small and stable. A host without the roster
 * route 404s, which is caught here — the card then shows the raw id rather than
 * dropping the attribution, because "which teammate asked" stays useful even
 * when we cannot pretty-print it.
 */
export function useAskerNames(
  client: OpenCompanyClient,
  company: string | null,
  approvals: ApprovalSummary[],
): Map<string, string> {
  const [names, setNames] = useState<Map<string, string>>(new Map());
  // Keyed on the set of asker ids rather than on `approvals` itself: the feed
  // hands us a fresh array on every poll, and depending on the array would
  // refetch the roster every few seconds for a roster that rarely changes.
  const askerKey = useMemo(
    () =>
      Array.from(new Set(approvals.map((a) => a.agent).filter((id): id is string => !!id)))
        .sort()
        .join(","),
    [approvals],
  );

  useEffect(() => {
    if (!askerKey) return;
    let live = true;
    void (async () => {
      const roster = await client.listTeam(company).catch(() => []);
      if (!live) return;
      setNames(new Map(roster.map((m) => [m.id, m.name?.trim() || m.role])));
    })();
    return () => {
      live = false;
    };
  }, [client, company, askerKey]);

  return names;
}
