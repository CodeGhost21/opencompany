import { CreditCard, ExternalLink, KeyRound } from "lucide-react";

import type { HubAccountLinks as Links } from "@/api/credential";

interface Props {
  /** The two pages, as the host resolved them. Absent renders nothing. */
  account: Links | undefined;
  /** Whether a key is already stored, which changes what the line above says. */
  configured: boolean;
}

/**
 * The way out to the TinyHumans dashboard: the key list, and the top-up page.
 *
 * "Connect TinyHumans" mints the key without anyone leaving the console, and
 * two things it deliberately does not do are revoke one and pay for what it
 * spends — both end up somewhere this console has no business being, one
 * ending an instance's access and the other moving money. So they are links,
 * to that person's own dashboard, behind their own sign-in.
 *
 * Rendered by both pages that hold a TinyHumans key, from one component, for
 * the same reason `ConnectTinyHumansButton` is one: two copies of the same
 * errand drift into two vocabularies for it.
 *
 * The URLs come from the **host**, never assembled here. A console pointed at
 * the staging hub must link to the staging dashboard, and only the host knows
 * which hub it was pointed at — a link built in the browser would send an
 * operator to production's billing page to wonder where their top-up went. A
 * host on a backend the naming convention does not describe sends no links at
 * all, and this renders nothing rather than guessing an origin.
 */
export function HubAccountLinks({ account, configured }: Props) {
  if (!account) return null;

  return (
    <div className="space-y-1">
      <p className="text-xs text-muted-foreground">
        {configured
          ? "Your agents spend this account's balance. Top it up, or revoke the key, on TinyHumans:"
          : "Already have a key, or need to add funds first? Both live on TinyHumans:"}
      </p>
      <div className="flex flex-wrap items-center gap-3">
        <a
          className="inline-flex items-center gap-1 text-xs font-medium underline underline-offset-4"
          href={account.manageKeysUrl}
          target="_blank"
          rel="noreferrer"
          data-testid="hub-manage-keys"
        >
          <KeyRound className="size-3" /> Manage API keys <ExternalLink className="size-3" />
        </a>
        <a
          className="inline-flex items-center gap-1 text-xs font-medium underline underline-offset-4"
          href={account.topUpUrl}
          target="_blank"
          rel="noreferrer"
          data-testid="hub-top-up"
        >
          <CreditCard className="size-3" /> Top up balance <ExternalLink className="size-3" />
        </a>
      </div>
    </div>
  );
}
