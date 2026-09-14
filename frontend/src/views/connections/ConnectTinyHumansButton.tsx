import { useCallback, useState } from "react";
import { Loader2, Sparkles } from "lucide-react";
import { toast } from "sonner";

import type { OpenCompanyClient } from "@/api/client";
import { startCredentialLink } from "@/api/credential";
import { ApiError } from "@/api/types";
import { Button } from "@/components/ui/button";
import { useRedeemKeyGrant } from "@/views/connections/use-redeem-key-grant";

interface Props {
  client: OpenCompanyClient;
  company: string | null;
  /** Whether this host can complete a grant at all (`hubLink` on the status). */
  available: boolean;
  /** Whether the signed-in operator may change the company's credential. */
  canManage: boolean;
  /** Whether a credential is already stored, which changes the verb. */
  configured: boolean;
  /** Called after a successful connection, so the page re-reads its status. */
  onConnected?: () => void;
}

/**
 * "Connect TinyHumans" — the whole key-grant flow, as one button.
 *
 * Clicking it starts a PKCE grant on the host and navigates to the hub; on the
 * way back the same component, freshly mounted, redeems the code through
 * {@link useRedeemKeyGrant}. The Account page does not render this button — it
 * calls that hook directly, unconditionally — so the return leg does not depend
 * on a button being shown.
 *
 * ## Why the return leg lives here rather than in `App`
 *
 * `App` captures the code off the URL, because that is where a landing URL is
 * read and stripped. It does not *redeem* it: a grant is not a session
 * credential and must not hold up the boot, and the operator who clicked this
 * button should see the result on the card they clicked — not behind a
 * full-screen "Signing in…" that has nothing to do with signing in.
 */
export function ConnectTinyHumansButton({
  client,
  company,
  available,
  canManage,
  configured,
  onConnected,
}: Props) {
  const redeeming = useRedeemKeyGrant(client, company, onConnected);
  const [starting, setStarting] = useState(false);

  const start = useCallback(async () => {
    setStarting(true);
    try {
      const { authorizeUrl } = await startCredentialLink(client, company);
      // A top-level navigation, not a fetch: the person signs in on the hub's
      // own origin and approves there, and both need its address bar visible.
      window.location.assign(authorizeUrl);
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Couldn't start the TinyHumans connection.",
      );
      setStarting(false);
    }
  }, [client, company]);

  // A host with no hub renders nothing at all, so the page it sits on looks
  // exactly as it did before this flow existed — the paste field, alone.
  if (!available || !canManage) return null;

  const busy = redeeming || starting;
  return (
    <div className="space-y-2">
      <Button
        type="button"
        disabled={busy}
        onClick={() => void start()}
        data-testid="connect-tinyhumans"
      >
        {busy ? <Loader2 className="size-4 animate-spin" /> : <Sparkles className="size-4" />}
        {configured ? "Reconnect TinyHumans" : "Connect TinyHumans"}
      </Button>
      <p className="text-xs text-muted-foreground">
        Sign in to TinyHumans and this company gets its key automatically — nothing to copy. It
        covers both the model your agents think with and the accounts they connect.
      </p>
    </div>
  );
}
