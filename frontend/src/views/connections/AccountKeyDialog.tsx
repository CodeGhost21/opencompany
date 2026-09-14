import { useEffect, useState } from "react";
import { ExternalLink, Loader2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { TINYHUMANS_API_KEYS_URL } from "@/lib/links";

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Whether a key of this company's own is already stored — changes the verb. */
  replacing: boolean;
  busy: boolean;
  /**
   * Why the last save failed, in the host's words where it sent some. Shown
   * inside the dialog, beside the field it is about, rather than as a toast
   * that disappears while the operator is still looking at the key they typed.
   */
  error: string | null;
  /** Saves the pasted value. The page closes the dialog once the write lands. */
  onSubmit: (key: string) => void;
}

/**
 * "Connect to TinyHumans" — the Account page's API-key option.
 *
 * The same ask the setup wizard makes for Managed: a key field and, for the
 * operator who has none, a link to where one is created
 * ({@link TINYHUMANS_API_KEYS_URL}, shared with the wizard so the two cannot
 * point at different pages).
 *
 * ## What it writes
 *
 * `PUT …/credential`, which stores `tinyhumans/key` and stops. That is the
 * company's TinyHumans identity, and since #2266 the slot a managed turn
 * resolves through. It does **not** declare the managed provider — only the
 * grant (`finish_link`) does — so the copy says it sets the account, not that
 * it chooses a model.
 *
 * Write-only, like every credential the console handles: the value is never
 * returned, so the field opens empty every time and "set" is reported by a
 * flag rather than by a masked value we would have had to receive.
 */
export function AccountKeyDialog({ open, onOpenChange, replacing, busy, error, onSubmit }: Props) {
  const [key, setKey] = useState("");

  // Cleared whenever the dialog opens or closes. A credential left in component
  // state after a save is a credential sitting in a heap snapshot for no
  // reason, and reopening on the previous paste would let a second Save write a
  // value the operator thinks they have already used.
  useEffect(() => {
    setKey("");
  }, [open]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{replacing ? "Replace your API key" : "Connect to TinyHumans"}</DialogTitle>
          <DialogDescription>
            Sets the TinyHumans account this company connects apps as, and that its TinyHumans
            models resolve through. It does not choose a model provider.
          </DialogDescription>
        </DialogHeader>

        <form
          className="space-y-4"
          onSubmit={(event) => {
            event.preventDefault();
            onSubmit(key.trim());
          }}
        >
          <div className="grid gap-1.5">
            <Label htmlFor="company-credential">Add your API key</Label>
            <Input
              id="company-credential"
              type="password"
              autoComplete="off"
              spellCheck={false}
              placeholder="Paste your TinyHumans API key"
              aria-describedby={error ? "account-key-error" : undefined}
              value={key}
              onChange={(event) => setKey(event.target.value)}
              data-testid="account-key-input"
            />
            <p className="text-xs text-muted-foreground">
              Don&apos;t have an API key?{" "}
              <a
                href={TINYHUMANS_API_KEYS_URL}
                target="_blank"
                rel="noreferrer"
                data-testid="account-key-get-link"
                className="inline-flex items-center gap-1 font-medium text-foreground underline underline-offset-4"
              >
                Get your API key
                <ExternalLink className="size-3" />
              </a>
            </p>
          </div>

          {/* Always present, filled only on failure: a live region mounted at the
              same moment as its text is frequently not announced. */}
          <p
            aria-live="polite"
            id="account-key-error"
            className="text-sm text-status-blocked-text empty:hidden"
            data-testid="account-key-error"
          >
            {error ?? ""}
          </p>

          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy || !key.trim()} data-testid="account-key-save">
              {busy ? <Loader2 className="size-4 animate-spin" /> : null}
              {replacing ? "Replace key" : "Save key"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
