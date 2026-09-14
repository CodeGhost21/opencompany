import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";

import type { OpenCompanyClient } from "@/api/client";
import { finishCredentialLink } from "@/api/credential";
import { ApiError } from "@/api/types";
import { takeKeyLink, takeKeyLinkRefusal } from "@/lib/pending-key-link";

/**
 * Redeems a returning TinyHumans key grant (`POST …/credential/link/finish`).
 *
 * Non-visual on purpose. The grant comes back as a top-level navigation: `App`
 * takes the code off the URL before the first render, strips the address bar
 * because it is a live single-use credential, and parks it in a module-local
 * box that a reload empties. Whatever calls this hook is the only thing that
 * spends it — so a page must call it **unconditionally**, never behind state
 * that is null while the credential read is in flight or stays null when it
 * fails. The Account page calls it at the top of `ApiKeyView`; the Apps card
 * gets it through `ConnectTinyHumansButton`.
 *
 * Returns whether a redemption is in flight.
 */
export function useRedeemKeyGrant(
  client: OpenCompanyClient,
  company: string | null,
  onConnected?: () => void,
): boolean {
  const [busy, setBusy] = useState(false);
  // StrictMode double-invokes effects, and the code is single-use: a second
  // call would spend nothing and report the host's "expired" refusal over a
  // connection that in fact succeeded.
  const redeeming = useRef(false);

  const finish = useCallback(
    async (state: string, code: string) => {
      setBusy(true);
      try {
        const result = await finishCredentialLink(client, company, state, code);
        toast.success("Connected to TinyHumans.", { description: result.note });
        onConnected?.();
      } catch (err) {
        // The host's own words where it sent them: "that connection attempt has
        // expired" tells an operator to click again, which a generic failure
        // does not.
        toast.error(
          err instanceof ApiError ? err.message : "Couldn't finish connecting to TinyHumans.",
        );
      } finally {
        setBusy(false);
      }
    },
    [client, company, onConnected],
  );

  useEffect(() => {
    if (redeeming.current) return;
    if (takeKeyLinkRefusal()) {
      redeeming.current = true;
      // Cancelling on the hub's consent screen lands here too, which is why this
      // is not worded as an error. Nothing was created either way.
      toast.info("No key was created.", {
        description: "The TinyHumans connection was cancelled or refused.",
      });
      return;
    }
    const pending = takeKeyLink();
    if (!pending) return;
    redeeming.current = true;
    void finish(pending.state, pending.code);
  }, [finish]);

  return busy;
}
