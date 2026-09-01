// The standing autonomy policy, stated in the window's title row — and, since
// the operator asked for it, changed from there too.
//
// This is a claim about what the agents are allowed to do without asking, so
// every word of it is read from the host rather than written here. Three
// findings shaped what it does and does not say, and they still govern what the
// menu is allowed to put in front of an operator:
//
// **The tier is one of four, not two.** `POLICY_MODES` in
// `src/company/types.rs:96` is `["readonly", "supervised", "auto", "full"]`.
// Flattening that to "Auto / not Auto" would report `readonly` — agents that
// change nothing and spend nothing — and `full` — the broadest autonomy this
// runtime has — as the same thing. The menu therefore lists whatever
// `status.tiers` holds and never a list of four written here.
//
// **The label and the sentence are the host's**, taken from the `tiers` list
// `GET {scope}/policy` returns. That list is filtered server-side to the modes
// *this* runtime accepts (`selectable_tiers`, `src/server/ops/policy.rs:153`),
// and its prose lives in `TIER_TEXT` (`policy.rs:121`) precisely so it cannot
// drift from the gate it describes. A console built against a newer host names
// a tier it has no text for by its mode word rather than dropping it — and,
// now that the menu is a control, *offers* it rather than dropping it, because
// a tier the host will accept that the console will not show is a capability
// silently withheld.
//
// **It deliberately says nothing about spending.** The obvious sentence here —
// "agents stop before spending" — is false in this build, and so is any
// rendering of `autoApproveUnderUsd`. The native gate is constructed
// `.with_policy_hitl_disabled()` (`src/runtime/builder.rs:2476`), and with that
// flag `evaluate` returns `Allow` for everything except `readonly` *before* it
// reaches the tier match or the spend threshold (`src/policy/gate.rs:749-757`);
// the tool path short-circuits the same way at
// `src/harness/built_in/policy.rs:1701`, above `always_approve`, the per-agent
// daily budget and `auto_approve_under_usd`. The console already says this out
// loud on the settings page, where the field is labelled "Spend approval
// threshold (inactive)" and disabled (`policy-settings.tsx:917`). A number that
// governs nothing is exactly the placeholder this row must not carry, so the
// pill carries the tier and the host's own words about it, and stops there.
//
// It renders **nothing** when the policy is unknown. See {@link useAutonomy}.

import { useState } from "react";
import {
  Check,
  ChevronDown,
  Eye,
  Infinity as InfinityIcon,
  ShieldCheck,
  UserCheck,
  Zap,
  type LucideIcon,
} from "lucide-react";
import { toast } from "sonner";

import { isPolicyStatus, NOT_A_POLICY, setPolicy, type PolicyStatus } from "@/api/policy";
import {
  AUTONOMY_CONFIRM_ACTION,
  AUTONOMY_CONFIRM_CANCEL,
  AUTONOMY_CONFIRM_TITLE,
  AUTONOMY_PROMPTS_NOTE,
  tierWideningExplanation,
  widensAutonomy,
} from "@/components/policy-settings";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { applyAutonomy } from "@/hooks/use-autonomy";
import { useConsole } from "@/lib/console-context";
import { cn } from "@/lib/utils";

/** The host's word for the tier in force, falling back to the mode itself. */
export function tierLabel(status: PolicyStatus): string {
  return status.tiers.find((tier) => tier.value === status.mode)?.label ?? status.mode;
}

/** The host's full description of the tier in force, when it ships one. */
export function tierDescription(status: PolicyStatus): string | null {
  return status.tiers.find((tier) => tier.value === status.mode)?.description ?? null;
}

/**
 * One glyph per mode word, keyed by the word and never by position.
 *
 * Position would be the easy mapping and the wrong one: `status.tiers` is
 * whatever *this* host accepts, so a runtime that drops `full`, or inserts a
 * tier between two existing ones, would silently re-point every icon at a
 * different meaning. The key is the mode word the gate itself matches on.
 *
 * The four read as increasing autonomy, and each was picked for the thing the
 * host's own description says the tier does rather than for how alarming it
 * ought to look:
 *
 * - `readonly` — {@link Eye}: it may look. The whole tier is "observation, no
 *   effect", and an eye is the one glyph that says that without saying "safe".
 * - `supervised` — {@link UserCheck}: a person is in the loop. The distinction
 *   from `auto` is not how much the agents do, it is that somebody signs off,
 *   so the icon carries the person rather than a smaller amount of work.
 * - `auto` — {@link Zap}: it acts on its own. The step from `supervised` is the
 *   human leaving the loop, and a bolt is "this happens by itself".
 * - `full` — {@link InfinityIcon}: no ceiling. It is the *widest* tier, not the
 *   most dangerous one, and infinity says "nothing further to remove" while a
 *   warning triangle or a flame would say something the host does not.
 *
 * Colour deliberately stays neutral for the same reason. `--status-*` tokens
 * mean *state* — running, blocked, failed — and no tier here is any of those;
 * painting `full` as a warning would be the console inventing a risk judgement
 * the host does not make.
 */
const TIER_ICONS: Record<string, LucideIcon> = {
  readonly: Eye,
  supervised: UserCheck,
  auto: Zap,
  full: InfinityIcon,
};

/**
 * The glyph for a mode word, or the neutral shield for one this console has
 * never heard of.
 *
 * The fallback is load-bearing rather than defensive tidiness: the tier list is
 * the host's, so a console running against a newer runtime *will* meet a mode
 * with no entry here, and it must draw that row — with the host's own label and
 * sentence beside it — rather than a hole or a crash.
 */
export function tierIcon(mode: string): LucideIcon {
  return TIER_ICONS[mode] ?? ShieldCheck;
}

/**
 * The host's description cut to its first sentence, for the row itself.
 *
 * A **mechanical** cut, not a summary. The host's `auto` text is 114 characters
 * — "Balanced execution autonomy. Approval prompts are explicit through
 * `request_approval` while policy HITL is disabled." — which is a paragraph, not
 * a title-bar clause, and rewriting it shorter is the one thing this component
 * must not do: the prose is server-side so that it tracks the gate, and a
 * paraphrase here would be a second description free to drift.
 *
 * Taking the leading sentence keeps every word the host's own and keeps the
 * full text one hover away. If the description has no sentence break it is used
 * whole, and the row's own `hidden lg:inline` decides whether there is space
 * for it at all.
 *
 * The **menu** does not use this. A row in an opened dropdown has the space for
 * the whole sentence and an operator about to change what the agents may do
 * should read all of it.
 */
export function leadSentence(description: string): string {
  const end = description.indexOf(". ");
  return end === -1 ? description : description.slice(0, end + 1);
}

/**
 * The autonomy pill: what the agents may do, and the control that changes it.
 *
 * It used to be a `<span>` with no click target, on the argument that the row
 * it sits in is the window's drag surface and a button here would take presses
 * away from dragging. That was reversed deliberately: the tier is the fact this
 * row exists to carry, and the shortest path from reading it to changing it is
 * the thing an operator reaches for when a company is doing something they want
 * stopped.
 *
 * **The drag band survives it.** `data-tauri-drag-region` is opt-in per element
 * and does not inherit, so the attribute is simply removed from the pill rather
 * than fought with: `WindowTitleBar` already puts a dedicated `flex-1`
 * `self-stretch` spacer carrying its own `data-tauri-drag-region` between the
 * company switcher and this pill, and that spacer — not the pill — is the
 * elastic part of the row. Every other item in the row (the switcher, the
 * profile control) is likewise a control the drag band goes around. Giving the
 * pill its pointer events back costs the window a fixed ~200px of grab area at
 * the right-hand end and none of the elastic middle, which is where a window is
 * actually dragged from.
 *
 * **The displayed tier is only ever a value the host returned.** The write goes
 * through `setPolicy`, which answers with the resulting `PolicyStatus`; that
 * answer is handed to {@link applyAutonomy} so the row updates in one round
 * trip instead of waiting up to 30s for the next poll. Nothing is written
 * optimistically and nothing is written on failure, so a rejected change leaves
 * the previous tier on screen and says why in a toast. A pill that lied about
 * what the agents are allowed to do is the one failure this component cannot
 * have.
 *
 * **Widening the tier is confirmed here exactly as it is on the settings page.**
 * `policy-settings.tsx` has put an `AlertDialog` in front of any move *up* the
 * host's tier order since #1423, and this control reuses that comparison and
 * that wording rather than restating either. Without it the title bar would be
 * a strictly lower-friction route to the broadest autonomy this runtime has
 * than the page that was deliberately given the gate — which is not a
 * difference in style, it is the gate not being there. Narrowing is one click
 * in both places, for the same reason: pulling capability back is what an
 * operator does in a hurry.
 *
 * `client` and `company` come from {@link useConsole} rather than props: the
 * shell mounts this element inside its own `ConsoleProvider`, and the title row
 * between them (`WindowTitleBar`) is a pure layout component that takes the
 * pill as an opaque `ReactNode` and knows nothing about policy.
 */
/**
 * The trigger's DOM id, so the confirmation dialog can hand focus back to it.
 *
 * A constant rather than `useId`: there is exactly one title row per window, so
 * a per-instance id buys nothing, and a stable string is what makes the
 * `finalFocus` lookup readable.
 */
const TRIGGER_ID = "autonomy-pill-trigger";

export function AutonomyPill({
  status,
  className,
}: {
  /** The effective policy, or `null` when it is not known. */
  status: PolicyStatus | null;
  className?: string;
}) {
  const { client, company } = useConsole();
  const [saving, setSaving] = useState(false);
  /**
   * The wider tier the operator picked and has not yet agreed to, or `null`.
   *
   * Holding the *tier* rather than a boolean is what lets the dialog quote the
   * host's own description of the thing being agreed to, and it is the only
   * record of the choice: nothing is written while it sits here, so dismissing
   * the dialog discards it and the row keeps stating the tier actually in force.
   */
  const [confirming, setConfirming] = useState<
    PolicyStatus["tiers"][number] | null
  >(null);
  const write = async (mode: string): Promise<boolean> => {
    if (!status || !client || saving || mode === status.mode) return false;
    setSaving(true);
    try {
      // Only `mode` is sent. An omitted field is left alone by the host, so
      // picking a tier here cannot silently discard the always-ask list, the
      // spend cap or the deadline — the same contract the settings page relies
      // on (`policy-settings.tsx` `saveTier`).
      const next = await setPolicy(client, company, { mode });
      // A PUT that answers 200 with something that is not a policy is a failed
      // write, not a successful one: the sentence below would read "Takes
      // effect undefined", and the value would go on to be rendered. Throwing
      // hands it to the `catch` below, which is where a failed write already
      // goes.
      if (!isPolicyStatus(next)) throw new Error(NOT_A_POLICY);
      applyAutonomy(client, company, next);
      // The host's own timing sentence, not a paraphrase: a tier change lands
      // on the NEXT turn, and an operator changing the tier because something
      // is running right now needs to know that turn finishes under the old
      // one. The settings page shows the same line; changing the tier from the
      // title bar must not be the quieter path to the same act.
      toast.success("Autonomy tier updated", {
        description: `Takes effect ${next.takesEffect}.`,
      });
      return true;
    } catch (error) {
      // Visible, and matching every other mutation in the console (`sonner`).
      // Silence here would read as success and leave the operator believing a
      // tier is in force that is not.
      toast.error(error instanceof Error ? error.message : "Could not change the tier.");
      return false;
    } finally {
      setSaving(false);
    }
  };

  /**
   * What a menu row does — which is not, for a widening, to write anything.
   *
   * **Widening is confirmed; narrowing is not.** The settings page has drawn
   * that line since #1423 and this is the same line, drawn with the same
   * comparison ([`widensAutonomy`]) against the same host-supplied tier order,
   * because a second route to the broadest autonomy this runtime has that
   * skipped the gate would make the title bar the cheap way around the page
   * that was deliberately given one. Narrowing stays a single click on purpose:
   * taking capability *away* is what an operator does mid-incident, and
   * friction there is friction in the wrong direction.
   *
   * A tier neither this console nor the host can order — an unknown current
   * mode — widens nothing by this comparison and so is written directly. That
   * is the settings page's behaviour too, and changing it here would mean the
   * two pages disagreed about what counts as an escalation.
   */
  const choose = (tier: PolicyStatus["tiers"][number]) => {
    // Selecting what is already in force is a no-op, not a redundant write: the
    // PUT is attributed and durable, so it would record an operator "changing"
    // the policy to the value it already had.
    if (!status || !client || saving || tier.value === status.mode) return;
    if (widensAutonomy(status.tiers, status.mode, tier.value)) {
      setConfirming(tier);
      return;
    }
    void write(tier.value);
  };

  // Unknown renders nothing at all. A placeholder tier would be a confident
  // sentence about what the agents may do, written at the moment we do not
  // know — see `useAutonomy`.
  if (!status) return null;

  const description = tierDescription(status);
  const CurrentIcon = tierIcon(status.mode);

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger
          id={TRIGGER_ID}
          data-testid="autonomy-pill"
          // No client means no authenticated write — the pill still states the
          // tier, it just cannot change it. Disabled rather than hidden, for the
          // reason the switcher's "New company" row is: a control that vanishes
          // teaches nothing.
          disabled={!client || saving}
          // The host's full sentence, unabridged, for the tier in force. A native
          // `title` rather than the tooltip component: the trigger already owns a
          // popup, and a tooltip on the same element fights it for the press.
          title={description ?? undefined}
          className={cn(
            // `flex-none`: it is a direct child of the title row's flex container,
            // and it must not be shrunk into an unreadable sliver by a long company
            // name. The sentence inside it is dropped at a breakpoint instead.
            //
            // `py-1.5` (was `py-0.5`): at `text-xs` that is a 30px pill, which
            // sits in the 52px row rather than looking pressed into it. It stays
            // well under the row height, which `WINDOW_TITLE_BAR_HEIGHT` fixes at
            // 52 for the traffic lights' centre line.
            "inline-flex flex-none items-center gap-1.5 rounded-full border bg-card px-2.5 py-1.5",
            "text-xs font-medium text-muted-foreground",
            "transition-colors hover:bg-accent hover:text-accent-foreground",
            "data-popup-open:bg-accent data-popup-open:text-accent-foreground",
            "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
            "disabled:cursor-default",
            className,
          )}
        >
          <CurrentIcon aria-hidden="true" className="size-3.5 flex-none" />
          {/* The tier's name. The last thing to go: a pill that has dropped its
              sentence still says which tier is in force, which is the fact this
              element exists to carry. */}
          <span className="flex-none text-foreground">{tierLabel(status)}</span>
          {/* The host's leading sentence. First to go as the window narrows — see
              the degradation note on `WindowTitleBar`. `hidden` rather than
              truncated, because half a sentence about what the agents may do is
              worse than none: it would still read as a complete claim. */}
          {description && (
            <span
              data-testid="autonomy-consequence"
              className="hidden whitespace-nowrap lg:inline"
            >
              {leadSentence(description)}
            </span>
          )}
          {/* The only thing on the pill that says it is a control. It never
              drops: the affordance has to survive the narrow window that already
              hid the sentence. */}
          <ChevronDown aria-hidden="true" className="size-3 flex-none opacity-60" />
        </DropdownMenuTrigger>
        {/* `align="end"`: the pill sits at the right-hand end of the row, beside
            the profile control, and a menu this wide anchored to its start would
            hang off the window on an 880px minimum. `w-80` overrides the
            primitive's default `w-(--anchor-width)` — the trigger shrinks to the
            tier name below `lg`, and a menu that shrank with it could not hold a
            sentence. */}
        <DropdownMenuContent align="end" side="bottom" className="w-80 rounded-lg">
          <DropdownMenuGroup>
            {/* `DropdownMenuLabel` is Base UI's `Menu.GroupLabel`, and it throws
                outside a `Menu.Group`. */}
            <DropdownMenuLabel>Autonomy</DropdownMenuLabel>
            {/* Every tier the host offers, in the order it returned them — which
                is increasing autonomy. Never a list written here: see the header. */}
            {status.tiers.map((tier) => {
              const TierIcon = tierIcon(tier.value);
              const current = tier.value === status.mode;
              return (
                <DropdownMenuItem
                  key={tier.value}
                  data-testid={`autonomy-tier-${tier.value}`}
                  // `aria-current`, and a `Check`, exactly as the host switcher
                  // marks the company you are already in.
                  aria-current={current}
                  onClick={() => choose(tier)}
                  className="items-start gap-2 py-1.5"
                >
                  <TierIcon aria-hidden="true" className="mt-0.5 size-4 flex-none" />
                  <span className="min-w-0 flex-1">
                    <span className={cn("block", current && "font-medium")}>
                      {tier.label}
                    </span>
                    {/* The host's description in full, not `leadSentence`: an
                        open menu has the room, and this is the moment the words
                        actually matter. */}
                    <span className="mt-0.5 block text-xs whitespace-normal text-muted-foreground">
                      {tier.description}
                    </span>
                  </span>
                  {current && (
                    <Check aria-hidden="true" className="mt-0.5 size-4 flex-none" />
                  )}
                </DropdownMenuItem>
              );
            })}
          </DropdownMenuGroup>
          <DropdownMenuSeparator />
          {/* The host's own timing sentence, on the menu rather than only in the
              toast that follows a change: an operator deciding whether to pull
              the tier down mid-incident needs to know a running turn finishes
              under the old tier BEFORE they choose, not after. */}
          <div
            data-testid="autonomy-takes-effect"
            className="px-1.5 py-1 text-xs text-muted-foreground"
          >
            Takes effect {status.takesEffect}.
          </div>
        </DropdownMenuContent>
      </DropdownMenu>
      {/* The same gate the settings page puts in front of the same decision,
          in the same words and the same primitive — see
          `AUTONOMY_CONFIRM_TITLE` and its neighbours in `policy-settings.tsx`.
          Deliberately NOT a title-bar-flavoured variant of it: an operator who
          has met this dialog on the settings page should recognise it here as
          the same commitment rather than read a second sentence and wonder
          what is different about it.

          Controlled, and rendered as a sibling of the menu rather than inside
          it: the menu unmounts its rows the moment one is pressed, so a dialog
          living under a row would be torn down with the row that opened it. */}
      <AlertDialog
        open={confirming !== null}
        onOpenChange={(open) => {
          // A PUT is in flight — keep the dialog up. Escape and outside-click
          // still reach here (the confirm action stops the primitive's own
          // Close), and dismissing now would let the request land under a
          // dialog the operator believes they cancelled.
          if (!open) {
            if (saving) return;
            // Cancelling writes NOTHING: the tier on the row is `status`,
            // which only ever came from the host, and it is untouched.
            setConfirming(null);
          }
        }}
      >
        {/* No `finalFocus`: the settings page has to name one because its
            dialog is opened from a radio that may not be the one left checked,
            but here the menu has already returned focus to the pill by the time
            the dialog closes, and the pill is where an operator who just backed
            out expects to be. Asserted, not assumed — see the focus check in
            `autonomy-pill.test.ts`. */}
        <AlertDialogContent
          // Where focus goes when the dialog closes, named explicitly — the
          // settings page names its own target for the same reason. Without one
          // Base UI leaves focus on `<body>`, which strands a keyboard operator
          // mid-row after cancelling; asserting focus without it failed 6 runs in
          // 12. Looked up by id rather than held in a ref because
          // `DropdownMenuTrigger`'s `render` prop does not forward one. The
          // element is the trigger they pressed to get here and is still mounted,
          // because closing the dialog changes nothing about the pill.
          finalFocus={() => document.getElementById(TRIGGER_ID)}
        >
          <AlertDialogHeader>
            <AlertDialogTitle>{AUTONOMY_CONFIRM_TITLE}</AlertDialogTitle>
            <AlertDialogDescription>
              {confirming
                ? tierWideningExplanation(description ?? undefined, confirming)
                : null}
            </AlertDialogDescription>
            <p className="text-sm text-muted-foreground">
              {AUTONOMY_PROMPTS_NOTE}
            </p>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel data-testid="autonomy-tier-cancel" disabled={saving}>
              {AUTONOMY_CONFIRM_CANCEL}
            </AlertDialogCancel>
            <AlertDialogAction
              data-testid="autonomy-tier-confirm"
              disabled={saving}
              onClick={(event) => {
                // The primitive's `Close` would dismiss the dialog before the
                // PUT resolves; a failed write has to keep it open for the
                // retry rather than closing on a change that did not happen.
                event.preventBaseUIHandler();
                if (!confirming) return;
                void write(confirming.value).then((saved) => {
                  if (saved) setConfirming(null);
                });
              }}
            >
              {AUTONOMY_CONFIRM_ACTION}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
