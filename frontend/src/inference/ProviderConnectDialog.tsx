import { useState } from "react";

import type { OpenCompanyClient } from "@/api/client";
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
import {
  MANAGED_OPTION_SLUG,
  checkModelId,
  checkProviderName,
  checkSlug,
  clampToProviderNameLimit,
  credentialAsk,
  customProviderReady,
  endpointHasCredentials,
  modelIdErrorCopy,
  normalizeEndpoint,
  slugErrorCopy,
  slugify,
} from "./connect";
import type { ModelAsk } from "./connect";
import { ModelField } from "./ModelField";
import type { Provider } from "./types";

/** What connecting one provider sends. */
export interface ConnectDraft {
  kind: string;
  label?: string;
  baseUrl?: string;
  key?: string;
  /** The one model this row serves — required by the time the final submit fires (D-model). */
  model?: string;
  addAnyway?: boolean;
}

export type { ModelAsk };

/**
 * The fields a chosen provider needs, and nothing else.
 *
 * One dialog for all four shapes rather than four dialogs, because they differ
 * only in which fields are present and [`credentialAsk`](./connect.ts) already
 * answers that. Four components would be four places for the submit path — and
 * the submit path is where the credential is, which is the last thing worth
 * having four copies of.
 *
 * ## Three steps, one dialog (keys rework, issue #2306)
 *
 * `"details"` — the name/endpoint/key fields every kind but a CLI login asks
 * for. `"model"` — once step 1 has been submitted, this ALWAYS opens next (2c,
 * D-model: there is no longer a "this endpoint resolves tiers itself, skip the
 * model" branch — no tier name is ever sent as a model, 2d). `"edit"` — an
 * existing row's own fields, plus its own model, all in one form.
 *
 * The step-1 fields stay mounted (`hidden`, not unmounted) through the model
 * step, so their state survives the transition and `submit` still sends them —
 * only the model step's own fields are visible.
 *
 * ## The custom shape is this one plus a name
 *
 * A custom provider adds a **Name**, and the slug falls out of it rather than
 * being typed. The preview line under the field is not decoration: the slug is
 * what the operator should see before they commit to it.
 *
 * The slug is checked for empty, in-use and reserved **before the Add button is
 * enabled**, which is the console half of the same check the host performs
 * before it writes anything. Neither is redundant: this one is so the operator
 * is not told no after a round trip, and the host's is because a console is not
 * a security boundary.
 *
 * ## Add anyway
 *
 * Offered only once a probe has failed in a way that would otherwise reject the
 * add — never after a slug collision or a failed key write, because neither is
 * evidence that the endpoint is fine. It is cleared on every retry, so an
 * attempt that fails for an unrelated reason does not still offer to skip
 * verification.
 */
/**
 * What to put in the write-only key field of a submit.
 *
 * Three answers, and the middle one is the whole reason this is a function:
 * a provider that takes no key sends nothing, an **edit** with an untouched
 * field sends nothing (empty means unchanged, because a stored key cannot be
 * shown for the operator to leave alone), and everything else sends what was
 * typed. Only the explicit Remove key action sends an empty string, and it does
 * not come through this dialog.
 */
function keyToSend(needsKey: boolean, editing: boolean, typed: string): string | undefined {
  if (!needsKey) return undefined;
  const trimmed = typed.trim();
  if (editing && trimmed.length === 0) return undefined;
  return trimmed;
}

export function ProviderConnectDialog({
  client,
  company,
  optionSlug,
  providers,
  editing,
  busy,
  error,
  offerAddAnyway,
  modelAsk,
  replacesKey,
  onCancel,
  onBack,
  onSubmit,
}: {
  client: OpenCompanyClient;
  company: string | null;
  /** The chosen option, or `null` when the dialog is closed. */
  optionSlug: string | null;
  providers: readonly Provider[];
  /**
   * The row this dialog is editing, or `null` when it is adding one.
   *
   * Carries the things an edit must not invent: the stored label, endpoint and
   * model. It is also what excludes the row from its own slug collision check.
   */
  editing?: Provider | null;
  busy: boolean;
  /** What went wrong last time, if anything. */
  error: string | null;
  /** Whether the last failure was a probe failure, which is the only one that unlocks "add anyway". */
  offerAddAnyway: boolean;
  /**
   * The endpoint's catalogue, once step 1 has been submitted. `null` before
   * then — the model step does not appear at all until it does, and it always
   * does once it appears (D-model): there is no longer a "resolves tiers
   * itself, no model needed" case.
   */
  modelAsk: ModelAsk | null;
  /**
   * Whether connecting this option replaces a key already saved for the legacy
   * Managed row (keys rework, issue #2306, slice 2a) — TinyHumans and the
   * legacy chain share one slot, `provider/tinyhumans/key`.
   */
  replacesKey?: boolean;
  onCancel: () => void;
  /** Back from the model step to the details step, without closing the dialog. */
  onBack: () => void;
  onSubmit: (draft: ConnectDraft) => void;
}) {
  const open = optionSlug !== null;
  const ask = credentialAsk(optionSlug ?? "custom");
  const custom = optionSlug === "custom";
  const managed = optionSlug === MANAGED_OPTION_SLUG;

  // Seeded at mount, not in an effect.
  //
  // The caller gives this component a `key` of the option plus the row being
  // edited, so React unmounts and remounts it on every open and these
  // initialisers run once, before first paint. An effect that reset the same
  // fields was a race with its own dialog: `useEffect` is passive, so it runs
  // *after* the browser paints the visible dialog, and anything typed into a
  // field in between — a fast operator, or a browser test — was wiped by it
  // with nothing on screen to say so.
  //
  // **A conventional endpoint is a starting point for an ADD and a wrong answer
  // for an edit.** Seeding a local runtime's catalogue default over a stored one
  // turned "Edit endpoint" into one click that relocated an Ollama at
  // `http://10.0.0.5:11435` back to `localhost` without saying so, and the two
  // local runtimes that ship no default (LM Studio, OMLX) opened blank with the
  // button disabled until the operator retyped a URL from memory.
  const [label, setLabel] = useState(() => editing?.label ?? "");
  const [baseUrl, setBaseUrl] = useState(
    () => editing?.baseUrl ?? ask.defaultEndpoint ?? "",
  );
  // Never seeded. A stored credential is write-only — the host does not return
  // it and nothing here could display it — so an empty field in edit mode means
  // "leave it alone", which is what `submit` sends.
  const [key, setKey] = useState("");
  // Seeded from the row's own model in edit mode — leaving it unchanged sends
  // nothing (`submit` compares against this same seed).
  const [model, setModel] = useState(() => editing?.model ?? "");
  // Orchestrator decision X1 (2026-09-15): the first provider a company
  // connects always becomes its default — there is no checkbox to untick, and
  // no client-sent `makeDefault` flag. The host decides and sets it; this
  // dialog only says so. Every later add offers no default control at all —
  // "Set as default" (`DefaultModelDialog`) is the one way to change it after
  // connecting, and it confirms before replacing an existing full default.
  const firstProvider = providers.length === 0;

  // The row being edited is not its own collision. Its slug is already taken —
  // by it — and `edit` is keyed on the stored slug rather than on this one, so
  // including it made a custom provider's own name read as "taken" and left
  // both buttons disabled. Rotating its key meant inventing a name it would
  // never actually be given.
  const rivals = editing ? providers.filter((p) => p.slug !== editing.slug) : providers;

  const slug = slugify(label);
  // The name's own bound is reported before the slug's, because a name past the
  // limit is what the operator can actually see and fix — the slug is derived.
  const slugError = custom ? (checkProviderName(label) ?? checkSlug(rivals, slug)) : null;
  const endpointOk = !ask.needsEndpoint || normalizeEndpoint(baseUrl) !== null;
  const modelError = model.trim() ? checkModelId(model) : "empty";

  const step: "details" | "model" | "edit" = editing ? "edit" : modelAsk ? "model" : "details";

  const detailsOk =
    (custom
      ? customProviderReady(rivals, { label, baseUrl })
      : endpointOk && (!ask.needsKey || key.trim().length > 0));
  const ready =
    step === "model"
      ? modelError === null
      : step === "edit"
        ? detailsOk && modelError === null
        : detailsOk;

  const submit = (addAnyway: boolean) =>
    onSubmit({
      kind: optionSlug ?? "custom",
      label: custom ? label.trim() : undefined,
      // **An unchanged endpoint in edit mode is not sent.** The row's
      // `baseUrl` is the host's *redacted* form, which can mask a path segment
      // that only looks like a credential; posting it back would store that
      // mask over a working endpoint on a rename (Codex review on #2281). The
      // host keeps the stored endpoint when none is sent.
      baseUrl:
        !ask.needsEndpoint || (editing != null && baseUrl.trim() === editing.baseUrl.trim())
          ? undefined
          : (normalizeEndpoint(baseUrl) ?? baseUrl.trim()),
      // **An untouched field in edit mode is not an instruction.** The host
      // reads `Some("")` as "clear the credential", which is right for the
      // Remove key action and catastrophic here: renaming a provider would
      // silently disable every turn routed through it. The field starts empty
      // because a stored key cannot be shown, so empty has to mean "unchanged".
      key: keyToSend(ask.needsKey, editing != null, key),
      // Sent whenever there is a value to send. In edit mode an unchanged
      // value is simply the same string the host already has, so sending it
      // is a no-op rather than a risk — unlike the key, a model is never
      // write-only, so there is nothing to accidentally overwrite with a mask.
      model: model.trim() || undefined,
      addAnyway,
    });

  return (
    <Dialog open={open} onOpenChange={(next) => !next && onCancel()}>
      <DialogContent className="sm:max-w-md" data-testid="inference-connect-provider">
        <DialogHeader>
          <DialogTitle>{step === "model" ? `${ask.title}: choose a model` : ask.title}</DialogTitle>
          {/* Where the key goes, said plainly, or nothing. */}
          {ask.needsKey && step !== "model" ? (
            <DialogDescription>
              The key is stored on this company and never shown again.
            </DialogDescription>
          ) : null}
        </DialogHeader>

        <div className="grid gap-4">
          {/* The step-1 fields stay mounted (not unmounted) through the model
              step, so React keeps their state and `submit` still sends them —
              they are simply hidden while the model step has the floor. */}
          <div className={step === "model" ? "hidden" : "grid gap-4"}>
            {custom && (
              <div className="grid gap-1.5">
                <Label htmlFor="inference-connect-name">Name</Label>
                <Input
                  id="inference-connect-name"
                  value={label}
                  placeholder="My Provider"
                  autoComplete="off"
                  // The host holds this rule; clamping here only stops a paste
                  // becoming a 400 the operator has to read to understand. Counted
                  // in code points, as the host counts — never `maxLength`, which
                  // counts UTF-16 units and refuses names the host accepts.
                  onChange={(e) => setLabel(clampToProviderNameLimit(e.target.value))}
                />
                {/* The slug is what the operator will see elsewhere on this
                    page, so they should see it before they commit to it. */}
                <p
                  className="font-mono text-xs text-muted-foreground"
                  data-testid="inference-slug-preview"
                >
                  Slug: {slug || "None"}
                </p>
                {slugError && (
                  <p className="text-xs text-status-blocked-text" data-testid="inference-slug-error">
                    {slugErrorCopy(slugError)}
                  </p>
                )}
              </div>
            )}

            {ask.needsEndpoint && (
              <div className="grid gap-1.5">
                <Label htmlFor="inference-connect-url">
                  {custom ? "OpenAI URL" : "Endpoint"}
                </Label>
                <Input
                  id="inference-connect-url"
                  aria-describedby={error ? "inference-connect-error" : undefined}
                  value={baseUrl}
                  placeholder="https://api.openai.com/v1"
                  autoComplete="off"
                  spellCheck={false}
                  className="font-mono text-xs"
                  onChange={(e) => setBaseUrl(e.target.value)}
                />
                {baseUrl.trim() && !endpointOk && (
                  <p className="text-xs text-status-blocked-text">
                    {endpointHasCredentials(baseUrl)
                      ? "Remove the username and password from the URL and put the credential in the API key field — an endpoint is stored as written and is readable by everyone who can see this company's settings."
                      : "That must be an http or https address."}
                  </p>
                )}
              </div>
            )}

            {ask.needsKey && (
              <div className="grid gap-1.5">
                <Label htmlFor="inference-connect-key">API Key</Label>
                <Input
                  id="inference-connect-key"
                  aria-describedby={error ? "inference-connect-error" : undefined}
                  type="password"
                  value={key}
                  placeholder={ask.keyPlaceholder ?? "sk-..."}
                  autoComplete="off"
                  spellCheck={false}
                  className="font-mono text-xs"
                  onChange={(e) => setKey(e.target.value)}
                />
                {replacesKey && (
                  <p
                    className="text-xs text-muted-foreground"
                    data-testid="inference-connect-replaces-key"
                  >
                    A key is already saved for Managed. Connecting TinyHumans replaces it with
                    this one.
                  </p>
                )}
              </div>
            )}

            {/* Managed has two ways in, and only one of them is a key. The other
                writes the company's TinyHumans **account**, which is a different
                credential with a different lifecycle — it is rotated, and it moves
                every brokered surface at once, not just this one. It already has a
                home on Connections → Account, and a second form for one credential
                is how two surfaces come to disagree about whether a company has
                it. So this links there rather than duplicating it. */}
            {managed && (
              <div className="grid gap-1.5 rounded-md border border-border px-3 py-2">
                <p className="text-sm font-medium">Or connect your TinyHumans account</p>
                <p className="text-xs text-muted-foreground">
                  One account key pays for thinking and for app connections, and rotating it
                  reaches both. Set it up on Connections → Account.
                </p>
                <a
                  className="text-xs font-medium underline underline-offset-4"
                  href="#/connections/api-key"
                  data-testid="inference-managed-account-link"
                  onClick={onCancel}
                >
                  Go to Account
                </a>
              </div>
            )}

            {!ask.needsKey && !ask.needsEndpoint && (
              <p className="text-sm text-muted-foreground">
                Nothing to enter — another command line tool already holds this credential.
              </p>
            )}
          </div>

          {/* The model step. It always opens once step 1 succeeds (D-model) —
              every kind asks, always, because a provider is never shown as set
              without a model and no tier name is ever sent as one (2d). */}
          {(step === "model" || step === "edit") && (
            <div className="grid gap-1.5" data-testid="inference-connect-model-step">
              <ModelField
                client={client}
                company={company}
                slug={step === "edit" ? (editing?.slug ?? null) : null}
                id={step === "edit" ? "inference-edit-model" : "inference-connect-model"}
                value={model}
                disabled={busy}
                onChange={setModel}
                // In the add flow the list is already in hand from the draft
                // probe that opened this step — nothing is fetched again. In
                // edit mode the row is already saved, so its own catalogue is
                // read live instead (`slug` above, `models` omitted).
                {...(step === "model" && modelAsk
                  ? { models: modelAsk.models, freeTextOnly: modelAsk.freeTextOnly, catalogError: modelAsk.error }
                  : {})}
              />
              {model.trim() && modelError && (
                <p className="text-xs text-status-blocked-text" data-testid="inference-model-id-error">
                  {modelIdErrorCopy(modelError)}
                </p>
              )}
              {/* Decision X1: the first provider a company connects always
                  becomes its default. No checkbox — the host decides, this
                  just says so. Every later add offers no default control at
                  all; "Set as default" on the row is the way to change it. */}
              {step === "model" && firstProvider && (
                <p className="text-xs text-muted-foreground" data-testid="inference-first-default-note">
                  This becomes the company default.
                </p>
              )}
            </div>
          )}

          {/* Always present, never mounted with its text: a live region that
              appears at the same moment as its content is frequently missed by
              the announcement, and this one is the reason the operator is still
              looking at this dialog. */}
          <p
            aria-live="polite"
            id="inference-connect-error"
            className="text-sm text-status-blocked-text empty:hidden"
            data-testid="inference-connect-error"
          >
            {error ?? ""}
          </p>
        </div>

        <DialogFooter>
          {step === "model" ? (
            <Button type="button" variant="outline" onClick={onBack} disabled={busy} data-testid="inference-connect-back">
              Back
            </Button>
          ) : (
            <Button type="button" variant="outline" onClick={onCancel} disabled={busy}>
              Cancel
            </Button>
          )}
          {/* Gated on a typed probe failure, never on a boolean: a slug
              collision or a failed key write must not unlock it, because
              neither is evidence that the endpoint is fine. Only meaningful on
              the model step, since that is the step whose submit performs the
              write the probe failure would otherwise block. */}
          {offerAddAnyway && step === "model" && (
            <Button
              type="button"
              variant="outline"
              disabled={busy || !ready}
              data-testid="inference-add-anyway"
              onClick={() => submit(true)}
            >
              Add anyway
            </Button>
          )}
          {/* Says what it is doing. The probe is a network round trip and a
              button that only greys out reads as a click that did not land —
              which is how a Connect gets pressed twice. */}
          <Button
            type="button"
            disabled={busy || !ready}
            data-testid="inference-connect-submit"
            onClick={() => submit(false)}
          >
            {busy
              ? step === "details"
                ? "Reading models…"
                : "Saving…"
              : step === "details"
                ? "Continue"
                : step === "edit"
                  ? "Save"
                  : custom
                    ? "Add Provider"
                    : "Connect"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
