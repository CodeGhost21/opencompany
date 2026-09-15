import { useEffect, useState } from "react";
import { ExternalLink, Loader2 } from "lucide-react";

import type { OpenCompanyClient } from "@/api/client";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { TINYHUMANS_API_KEYS_URL } from "@/lib/links";
import { ModelField } from "@/inference/ModelField";
import {
  COMPOSIO_PAGE_HREF,
  LLM_PAGE_HREF,
  accountFillLine,
  modelStepTitle,
  type AccountFills,
} from "@/views/connections/account-fill";

/** The second step's shape, once the host has answered `needsModel` (keys rework #2306, 4a/4b). */
export interface AccountKeyModelStep {
  /** Catalog ids to offer — may be empty, in which case {@link ModelField} falls back to free text. */
  models: string[];
  /** Whether the model chosen here would also become the company default. */
  setsDefault: boolean;
  /** The host's own note from the first save, shown verbatim above the field. */
  note: string;
}

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
  /** Which of the LLM/Composio slots this save would fill — `null` renders no line. */
  fills: AccountFills | null;
  /** Set once the host answers `needsModel` for the key just saved — the dialog then shows step two. */
  modelStep: AccountKeyModelStep | null;
  /** Saves the model chosen in step two, against the same key already saved in step one. */
  onSubmitModel: (model: string) => void;
  /** Needed only by step two's {@link ModelField}, which takes list mode and fetches nothing. */
  client: OpenCompanyClient;
  company: string | null;
}

/**
 * "Connect to TinyHumans" — the Account page's API-key option.
 *
 * The same ask the setup wizard makes for Managed: a key field and, for the
 * operator who has none, a link to where one is created
 * ({@link TINYHUMANS_API_KEYS_URL}, shared with the wizard so the two cannot
 * point at different pages).
 *
 * Deliberately minimal (operator request, 2026-09-14): a heading, the field,
 * the "Get an API key" link, Save and Cancel, and an error only when a save
 * fails — plus, since the keys rework (issue #2306), one conditional line
 * naming the LLM/Composio slots this save would fill (Q9) and, when the host
 * answers `needsModel`, a second step asking for the model to finish setting
 * up TinyHumans for LLM. Still no other explanatory paragraph.
 *
 * ## What it writes
 *
 * `PUT …/credential`, under a per-company lock
 * (`company_key::fan_out`, slice 4a): the account key itself, and — never
 * overwriting a key set on that page's own (Q7) — its copies at
 * `composio/tinyhumans/key` and `provider/tinyhumans/key`. A `tinyhumans` row
 * is only ever created with a model (a key with no row is not "set" — see
 * `account-fill.ts`), which is what step two is for.
 *
 * Write-only, like every credential the console handles: the value is never
 * returned, so the field opens empty every time and "set" is reported by a
 * flag rather than by a masked value we would have had to receive.
 */
export function AccountKeyDialog({
  open,
  onOpenChange,
  replacing,
  busy,
  error,
  onSubmit,
  fills,
  modelStep,
  onSubmitModel,
  client,
  company,
}: Props) {
  const [key, setKey] = useState("");
  const [model, setModel] = useState("");

  // Cleared whenever the dialog opens or closes. A credential left in component
  // state after a save is a credential sitting in a heap snapshot for no
  // reason, and reopening on the previous paste would let a second Save write a
  // value the operator thinks they have already used. `model` gets the same
  // treatment — it is never sensitive, but a stale value from a previous save
  // reopening on step two would be its own kind of surprise.
  useEffect(() => {
    setKey("");
    setModel("");
  }, [open]);

  const fillLine = accountFillLine(fills);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        {modelStep ? (
          <>
            <DialogHeader>
              <DialogTitle>{modelStepTitle(modelStep.setsDefault)}</DialogTitle>
            </DialogHeader>

            <form
              className="space-y-4"
              onSubmit={(event) => {
                event.preventDefault();
                onSubmitModel(model.trim());
              }}
            >
              <p className="text-xs text-muted-foreground" data-testid="account-key-note">
                {modelStep.note}
              </p>

              <div data-testid="account-key-model-step">
                <ModelField
                  client={client}
                  company={company}
                  slug={null}
                  id="account-key-model"
                  label="Model"
                  value={model}
                  onChange={setModel}
                  models={modelStep.models}
                  catalogError={
                    modelStep.models.length === 0
                      ? "TinyHumans did not list its models, so type an id."
                      : undefined
                  }
                />
              </div>

              <p
                aria-live="polite"
                id="account-key-error"
                className="text-sm text-status-blocked-text empty:hidden"
                data-testid="account-key-error"
              >
                {error ?? ""}
              </p>

              <DialogFooter>
                {/* The key and its Composio copy are already saved (step one
                    landed before step two ever opens), so Cancel here closes
                    the dialog rather than rolling anything back. Disabled
                    while busy (round-3b review, P2-4) — the parent's own
                    `onOpenChange` guard already refuses the close, but a
                    button that visibly does nothing on click is its own kind
                    of confusing. */}
                <Button type="button" variant="outline" onClick={() => onOpenChange(false)} disabled={busy}>
                  Cancel
                </Button>
                <Button
                  type="submit"
                  disabled={busy || !model.trim()}
                  data-testid="account-key-model-save"
                >
                  {busy ? <Loader2 className="size-4 animate-spin" /> : null}
                  Save model
                </Button>
              </DialogFooter>
            </form>
          </>
        ) : (
          <>
            <DialogHeader>
              <DialogTitle>{replacing ? "Replace your API key" : "Connect to TinyHumans"}</DialogTitle>
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
                    Get an API key
                    <ExternalLink className="size-3" />
                  </a>
                </p>
                {fillLine && (
                  <p className="text-xs text-muted-foreground" data-testid="account-key-fill-line">
                    {/* `llmShown` matches `accountFillLine`'s own `llm` gate
                        exactly (round-3b review, P3-4) — a row that already
                        has a model gets no LLM clause in the sentence, so it
                        must get no dangling "LLM page" link either. */}
                    {fillLine}{" "}
                    {fills?.llm && !fills?.llmHasModel && (
                      <a
                        href={LLM_PAGE_HREF}
                        data-testid="account-key-llm-link"
                        className="font-medium text-foreground underline underline-offset-4"
                      >
                        LLM page
                      </a>
                    )}
                    {fills?.llm && !fills?.llmHasModel && fills?.composio && " · "}
                    {fills?.composio && (
                      <a
                        href={COMPOSIO_PAGE_HREF}
                        data-testid="account-key-composio-link"
                        className="font-medium text-foreground underline underline-offset-4"
                      >
                        Composio page
                      </a>
                    )}
                  </p>
                )}
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
                {/* Disabled while busy (round-3b review, P2-4) — same reason
                    as step two's Cancel button above. */}
                <Button type="button" variant="outline" onClick={() => onOpenChange(false)} disabled={busy}>
                  Cancel
                </Button>
                <Button type="submit" disabled={busy || !key.trim()} data-testid="account-key-save">
                  {busy ? <Loader2 className="size-4 animate-spin" /> : null}
                  {replacing ? "Replace key" : "Save key"}
                </Button>
              </DialogFooter>
            </form>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}
