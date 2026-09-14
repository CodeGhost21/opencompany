import { useEffect, useRef, useState } from "react";
import { Plus } from "lucide-react";

import type { OpenCompanyClient } from "@/api/client";
import { ApiError } from "@/api/types";
import type { ProbeResult } from "@/api/inference";
import { TEST_RESULT_MS } from "./classify";
import type { TestState } from "./classify";
import type { ProbeClass } from "./types";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { SectionUnreachable } from "@/views/connections/SectionUnreachable";
import { AddProviderDialog } from "./AddProviderDialog";
import { DefaultModelDialog } from "./DefaultModelDialog";
import { ProviderConnectDialog } from "./ProviderConnectDialog";
import type { ConnectDraft, ModelAsk } from "./ProviderConnectDialog";
import { MANAGED_SLUG, NO_CREDENTIAL_RESOLVES, ProviderList } from "./ProviderList";
import { RemoveProviderDialog } from "./RemoveProviderDialog";
import type { RemovalIntent } from "./RemoveProviderDialog";
import { isAzureEndpoint } from "./catalogue";
import { MANAGED_OPTION_SLUG, defaultBrokenCopy, defaultNeedsModel, modelAskFromProbe, probeEndpoint } from "./connect";
import { MANAGED_TARGET_LABEL, managedFallbackNote, nothingCanAnswer } from "./managed-copy";
import { removalImpact } from "./removal";
import { RoutesNotCarriedBanner } from "./RoutesNotCarriedBanner";
import type { InferenceActions, InferenceState } from "./use-inference";
import type { Provider } from "./types";

/**
 * LLM Providers: what this company can reach a model through, and how to add one.
 *
 * One page (per-workload routing is gone, keys rework issue #2306 phase 5b) —
 * a company has one default `{provider, model}` and agents may pin their own
 * (Team → the agent → Harness & model). Every provider connects the same way:
 * add the key or endpoint, choose a model from what it actually publishes,
 * save. There is no state in which a row is "connected" but has no model.
 */
/**
 * Drops the error envelope's own prefix from a message meant for a person.
 *
 * The host answers a refusal as `invalid request: <sentence>`, and the prefix is
 * machine vocabulary: it says which *kind* of error this is to a caller that
 * might branch on it, and says nothing at all to the operator standing in front
 * of the field they have to correct. The sentence after it is already written
 * for them.
 */
export function stripEnvelopePrefix(message: string): string {
  return message.replace(/^(invalid request|conflict|not found):\s*/i, "");
}

/**
 * Fires a write whose only failure surface is the toast `write` already raised.
 *
 * `void promise` on a rejecting call is an unhandled rejection, and these are
 * the row controls — a switch, a default marker, a restart — with no form open
 * to show an error in. Swallowing here is deliberate and narrow: the toast has
 * already been raised by the time this runs, so the alternative is not "report
 * it better", it is "report it twice, once as a console error nobody reads".
 */
function fireAndForget(run: Promise<unknown>): void {
  void run.catch(() => {});
}

export function ProvidersTab({
  client,
  company,
  state,
  actions,
  canManage,
}: {
  client: OpenCompanyClient;
  company: string | null;
  state: InferenceState;
  actions: InferenceActions;
  canManage: boolean;
}) {
  /** Which option the connect dialog is open on, if any. */
  const [connecting, setConnecting] = useState<string | null>(null);
  /** The provider the connect dialog is editing, if it is editing one. */
  const [editing, setEditing] = useState<Provider | null>(null);
  const [adding, setAdding] = useState(false);
  /**
   * The action awaiting confirmation, if any (keys rework, issue #2306's
   * confirmation contract: every destructive action and every on/off toggle
   * confirms — `disable` and `enable` both included now).
   */
  const [confirming, setConfirming] = useState<{
    intent: RemovalIntent;
    provider: Provider;
  } | null>(null);
  /**
   * A fresher `usedBy` and message than `confirming.provider.usedBy`, from a
   * `409 in_use` this same confirm already hit once. The dialog stays open and
   * re-renders with these instead of closing on the refusal — a stale UI is
   * the one case `confirmInUse: true` is not sent blind.
   */
  const [confirmRefusal, setConfirmRefusal] = useState<{ message: string; usedBy?: Provider["usedBy"] } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /**
   * Whether the last failure was a **probe** failure.
   *
   * Gated on the class rather than on a boolean, and cleared on every attempt:
   * a slug collision or a failed key write must not offer to skip verification,
   * because neither is evidence that the endpoint is fine.
   */
  const [probeFailure, setProbeFailure] = useState<ProbeClass | null>(null);
  /**
   * The endpoint's catalogue, once the details step has been submitted. `null`
   * until then. Unlike the pre-rework flow this always opens once set — there
   * is no longer a "this endpoint resolves tiers itself, skip the model" case
   * (D-model, 2d).
   */
  const [modelAsk, setModelAsk] = useState<ModelAsk | null>(null);
  /**
   * What each row's Test is doing, keyed by slug.
   *
   * **Per row, not per page.** A single result under the card says nothing about
   * which of several providers was tested, which was the bug: two providers must
   * be able to show two different answers at once without ambiguity.
   */
  const [tests, setTests] = useState<Record<string, TestState>>({});
  /**
   * The row the "Set as default" dialog is open on, if any.
   */
  const [settingDefault, setSettingDefault] = useState<Provider | null>(null);
  const [defaultBusy, setDefaultBusy] = useState(false);
  const [defaultError, setDefaultError] = useState<string | null>(null);
  /**
   * Whether the legacy managed row's own "Remove key" is awaiting confirmation.
   *
   * @deprecated keys-rework #2306: its own flag rather than a `confirming`
   * entry, because `confirming` carries a `Provider` and the legacy managed
   * row has no record — it is a chain, which is the same reason its row is
   * rendered outside the list. Removable once item 10 drops the fallback
   * chains and the legacy row with them.
   */
  const [confirmingManaged, setConfirmingManaged] = useState(false);
  // Cleared on unmount, so a result that resolves after the page is gone does
  // not set state on a component nobody is looking at.
  const timers = useRef<Record<string, ReturnType<typeof setTimeout>>>({});
  useEffect(
    () => () => {
      for (const timer of Object.values(timers.current)) clearTimeout(timer);
    },
    [],
  );

  /** Runs a test for one row and lands its answer on that row. */
  const runTest = (slug: string, run: () => Promise<ProbeResult>) => {
    clearTimeout(timers.current[slug]);
    setTests((prev) => ({ ...prev, [slug]: { kind: "testing" } }));
    void run()
      .then((result) =>
        setTests((prev) => ({
          ...prev,
          [slug]: {
            kind: "done",
            ok: result.ok,
            // The host's own sentence, so a proxy failure reads differently
            // from a rejected key. "Failed" would throw that away at the last
            // step.
            message: result.ok
              ? "Reached the provider."
              : (result.message ?? "The check did not complete."),
          },
        })),
      )
      .catch((err) =>
        setTests((prev) => ({
          ...prev,
          [slug]: {
            kind: "done",
            ok: false,
            message: err instanceof ApiError ? err.message : "The check did not complete.",
          },
        })),
      )
      .finally(() => {
        timers.current[slug] = setTimeout(
          () => setTests((prev) => ({ ...prev, [slug]: { kind: "idle" } })),
          TEST_RESULT_MS,
        );
      });
  };

  if (state.load === "unavailable") return null;
  if (state.load === "loading") return <Skeleton className="h-64 rounded-xl" />;
  if (state.load === "error") {
    return <SectionUnreachable label="Couldn't read this company's model providers" />;
  }

  const closeConnect = () => {
    setConnecting(null);
    setEditing(null);
    setError(null);
    setProbeFailure(null);
    setModelAsk(null);
  };

  async function submitConnect(draft: ConnectDraft) {
    setBusy(true);
    // Cleared on every retry, so an attempt that fails for an unrelated reason
    // does not still offer to skip verification.
    setError(null);
    setProbeFailure(null);
    try {
      if (editing) {
        await actions.edit(editing.slug, {
          label: draft.label,
          baseUrl: draft.baseUrl,
          key: draft.key,
          model: draft.model,
        });
      } else if (!modelAsk) {
        // **Ask before writing, not after refusing.** A model is always
        // required (D-model), and the only honest moment to ask is with that
        // endpoint's own catalogue in hand. This runs for every kind now —
        // TinyHumans included (decision X6: the console no longer calls the
        // deprecated `PUT …/inference/managed/key`; it goes through this same
        // add flow like everything else) — never conditionally on whether the
        // endpoint "needs" one.
        const url = probeEndpoint(draft.kind, draft.baseUrl);
        const probe = url
          ? await actions.probeDraftEndpoint({ baseUrl: url, key: draft.key, kind: draft.kind })
          : null;
        setModelAsk(modelAskFromProbe(url, probe, isAzureEndpoint));
        return;
      } else {
        await actions.add({
          kind: draft.kind,
          label: draft.label,
          baseUrl: draft.baseUrl,
          key: draft.key,
          model: draft.model ?? "",
          addAnyway: draft.addAnyway,
        });
      }
      closeConnect();
    } catch (err) {
      // The envelope's own `invalid request: ` prefix is machine vocabulary and
      // this sentence is read by a person standing in front of the field they
      // have to correct.
      setError(err instanceof ApiError ? stripEnvelopePrefix(err.message) : "That did not work.");
      // The host refuses an add on exactly one probe class, and it is the only
      // refusal that unlocks "add anyway".
      if (err instanceof ApiError && err.message.includes("rejected the credential")) {
        setProbeFailure("auth");
      }
    } finally {
      setBusy(false);
    }
  }

  /**
   * Runs a confirmed provider action, always asserting confirmation (keys
   * rework, issue #2306's confirmation contract: the confirm dialog IS the
   * confirmation, so its button always sends `confirmInUse: true`).
   *
   * A `409 in_use` this still hits — the row's `usedBy` changed between the
   * dialog opening and the click — re-opens the dialog with the refusal's own
   * message and `usedBy` rather than closing on it; every other failure closes
   * and toasts, the same as any other write.
   */
  async function confirmedWrite(run: () => Promise<unknown>) {
    try {
      await run();
      setConfirming(null);
      setConfirmRefusal(null);
    } catch (err) {
      if (err instanceof ApiError && err.status === 409 && err.code === "in_use") {
        setConfirmRefusal({ message: stripEnvelopePrefix(err.message), usedBy: err.usedBy });
        return;
      }
      // Any other failure: the toast from `write()` already said so. Close,
      // the same as before this contract existed.
      setConfirming(null);
      setConfirmRefusal(null);
    }
  }

  const openDefaultDialog = (provider: Provider) => {
    setDefaultError(null);
    setSettingDefault(provider);
  };

  const brokenDefault = defaultBrokenCopy(state.status?.defaultChoice, state.providers);
  const defaultRow = state.providers.find((p) => p.slug === state.status?.defaultChoice?.provider);

  return (
    <div className="space-y-4">
      {/* Phase 5a: a routing table the boot-time carry could not fold into a
          single default. One release's bridge; nothing here reads or writes
          routing, which is gone (phase 5b). */}
      <RoutesNotCarriedBanner
        rows={state.status?.routesNotCarried}
        canManage={canManage}
        onChooseDefault={() => {
          // The first row naming a provider this company still has and has
          // enabled is the best guess at "choose a model for the row that
          // already resolves here"; otherwise fall back to Add.
          const named = (state.status?.routesNotCarried ?? [])
            .map((r) => r.route.split(":")[0]?.trim())
            .find((slug) => slug && state.providers.some((p) => p.slug === slug && p.enabled));
          const target = named ? state.providers.find((p) => p.slug === named) : undefined;
          if (target) openDefaultDialog(target);
          else setAdding(true);
        }}
      />

      {state.status?.restartRequired && (
        <RestartNotice
          canRestart={canManage && state.status.canRebuildInPlace}
          onRestart={() => fireAndForget(actions.restart())}
        />
      )}

      <Card>
        <CardContent className="flex flex-wrap items-center justify-between gap-3">
          <div className="grid gap-0.5">
            <h2 className="text-sm font-medium">LLM Providers</h2>
            <p className="text-xs text-muted-foreground">
              Add and configure language model providers.
            </p>
          </div>
          <Button
            type="button"
            disabled={!canManage}
            data-testid="inference-add-open"
            onClick={() => setAdding(true)}
          >
            <Plus className="size-4" />
            Add a provider
          </Button>
        </CardContent>
      </Card>

      {/* Decision Q1/2c: a bare-slug default (a provider chosen before this
          rework, no model) never resolves a turn. Said plainly, with the one
          action that fixes it. */}
      {defaultNeedsModel(state.status?.defaultChoice) && (
        <Card data-testid="inference-default-needs-model-banner">
          <CardContent className="flex flex-wrap items-center justify-between gap-3">
            <p className="text-sm">Your default provider has no model. Choose one.</p>
            {canManage && defaultRow && (
              <Button
                type="button"
                variant="outline"
                data-testid="inference-default-needs-model-choose"
                onClick={() => openDefaultDialog(defaultRow)}
              >
                Choose a model
              </Button>
            )}
          </CardContent>
        </Card>
      )}

      {/* Decision X14: disabling or deleting the default's provider never
          clears the stored default — it just stops resolving. This is the
          durable notice for that state, with the exact wording the agent
          editor's own fallback line repeats for every agent with no pin. */}
      {brokenDefault && (
        <Card data-testid="inference-default-broken-banner">
          <CardContent className="flex flex-wrap items-center justify-between gap-3">
            <p className="text-sm text-status-blocked-text">{brokenDefault}</p>
            {canManage && (
              <Button
                type="button"
                variant="outline"
                data-testid="inference-default-broken-choose"
                onClick={() => setAdding(true)}
              >
                Choose a default
              </Button>
            )}
          </CardContent>
        </Card>
      )}

      <Card>
        <CardContent className="px-0">
          <h3 className="px-4 pb-2 text-xs font-medium tracking-wide text-muted-foreground uppercase">
            Connected
          </h3>
          <ProviderList
            providers={state.providers}
            managed={state.status?.managed}
            defaultChoice={state.status?.defaultChoice}
            canManage={canManage}
            busySlug={state.busySlug}
            // Decision X3: every toggle confirms now, both directions.
            onToggle={(p, enabled) =>
              setConfirming({ intent: enabled ? "enable" : "disable", provider: p })
            }
            onEdit={(p) => {
              setEditing(p);
              setConnecting(p.kind);
            }}
            onTest={(p) => runTest(p.slug, () => actions.test(p.slug))}
            onRemove={(p) => setConfirming({ intent: "provider", provider: p })}
            onRemoveKey={(p) => setConfirming({ intent: "key", provider: p })}
            // The same dialog the add flow opens, in edit mode: adding a key and
            // replacing one are one code path.
            onReplaceKey={(p) => {
              setEditing(p);
              setConnecting(p.kind);
            }}
            onMakeDefault={(p) => openDefaultDialog(p)}
            // The same handler the header's button uses, passed down rather
            // than reimplemented: one way to add a provider, not two.
            onAdd={() => setAdding(true)}
            onManagedToggle={(enabled) =>
              setConfirming({
                intent: enabled ? "enable" : "disable",
                // @deprecated keys-rework #2306: the legacy managed row has no
                // provider record — synthesised just enough for the confirm
                // dialog's copy. `onConfirm` below still calls the ordinary
                // managed actions, never a real provider write against this.
                provider: { id: MANAGED_SLUG, slug: MANAGED_SLUG, label: MANAGED_TARGET_LABEL, kind: "tinyhumans", baseUrl: "", models: {}, enabled: true, keyConfigured: true },
              })
            }
            onManagedTest={() => runTest(MANAGED_SLUG, actions.testManagedChain)}
            // Decision X6: opens the ordinary TinyHumans catalogue add/edit
            // flow, never the deprecated `PUT …/inference/managed/key` route
            // directly.
            onManagedReplaceKey={() => {
              setEditing(null);
              setConnecting(MANAGED_OPTION_SLUG);
            }}
            // @deprecated keys-rework #2306: confirmed, like every other row's
            // Remove key. The one remaining caller of `saveManagedKey("")` —
            // see its own doc in `use-inference.ts`.
            onManagedRemoveKey={() => setConfirmingManaged(true)}
            testState={(slug) => tests[slug] ?? { kind: "idle" }}
          />
        </CardContent>
      </Card>

      {/* @deprecated keys-rework #2306: describes only the legacy managed
          fallback chain's transitional pre-row state — see `managed-copy.ts`. */}
      {managedFallbackNote(state.status?.managed) && (
        <p className="text-xs text-muted-foreground" data-testid="inference-managed-fallback">
          {managedFallbackNote(state.status?.managed)}
        </p>
      )}

      {state.providers.length > 0 &&
        nothingCanAnswer(state.providers, state.status?.managed?.configured) && (
          <p className="text-xs text-status-blocked-text" data-testid="inference-providers-dead-end">
            {NO_CREDENTIAL_RESOLVES}. Switch one of these back on, or connect a provider.
          </p>
        )}

      <AddProviderDialog
        open={adding}
        onOpenChange={setAdding}
        providers={state.providers}
        onChoose={(option) => {
          setAdding(false);
          setEditing(null);
          setConnecting(option);
        }}
      />
      {/* Keyed so the dialog is a fresh component per open. Its fields seed at
          mount from this row; without the key React would keep the previous
          open's state and the seeding would have to be an effect, which runs
          after paint and races with anything typed before it. */}
      <ProviderConnectDialog
        key={`${connecting ?? "closed"}:${editing?.slug ?? "new"}`}
        client={client}
        company={company}
        optionSlug={connecting}
        providers={state.providers}
        editing={editing}
        busy={busy}
        error={error}
        offerAddAnyway={probeFailure !== null}
        modelAsk={modelAsk}
        replacesKey={
          connecting === MANAGED_OPTION_SLUG && editing === null && state.status?.managed?.source === "provider_key"
        }
        onCancel={closeConnect}
        onBack={() => {
          setModelAsk(null);
          setError(null);
          setProbeFailure(null);
        }}
        onSubmit={(draft) => void submitConnect(draft)}
      />
      <DefaultModelDialog
        key={settingDefault?.slug ?? "none"}
        client={client}
        company={company}
        provider={settingDefault}
        defaultChoice={state.status?.defaultChoice}
        busy={defaultBusy}
        error={defaultError}
        onCancel={() => setSettingDefault(null)}
        onSubmit={(model) => {
          if (!settingDefault) return;
          setDefaultBusy(true);
          setDefaultError(null);
          void actions
            .makeDefault(settingDefault.slug, model)
            .then(() => setSettingDefault(null))
            .catch((err) =>
              setDefaultError(err instanceof ApiError ? stripEnvelopePrefix(err.message) : "That did not work."),
            )
            .finally(() => setDefaultBusy(false));
        }}
      />
      <RemoveProviderDialog
        intent={confirming?.intent ?? null}
        label={confirming?.provider.label ?? ""}
        impact={
          confirming
            ? {
                ...removalImpact(confirming.provider, state.providers),
                usedBy: confirmRefusal?.usedBy ?? confirming.provider.usedBy,
              }
            : { lastEnabled: false }
        }
        busy={busy}
        serverError={confirmRefusal?.message}
        // Offered only where it is genuinely the softer answer: turning a
        // provider off keeps its endpoint and its credential, which is what
        // somebody removing one usually wants. It is not an alternative to
        // clearing a credential.
        onDisable={
          confirming?.intent === "provider" && confirming.provider.enabled
            ? () => {
                const provider = confirming.provider;
                setBusy(true);
                void confirmedWrite(() => actions.setEnabled(provider.slug, false, true)).finally(() =>
                  setBusy(false),
                );
              }
            : undefined
        }
        onCancel={() => {
          setConfirming(null);
          setConfirmRefusal(null);
        }}
        onConfirm={() => {
          if (!confirming) return;
          const { intent, provider } = confirming;
          setBusy(true);
          void confirmedWrite(() => {
            switch (intent) {
              case "disable":
                return actions.setEnabled(provider.slug, false, true);
              case "enable":
                // @deprecated keys-rework #2306: the legacy managed row's
                // toggle has no real provider row — `provider.slug` here is
                // the synthesised `MANAGED_SLUG` stand-in from `onManagedToggle`.
                return provider.slug === MANAGED_SLUG && !state.providers.some((p) => p.slug === MANAGED_SLUG)
                  ? actions.setManagedOn(true)
                  : actions.setEnabled(provider.slug, true, true);
              case "key":
                return actions.edit(provider.slug, { key: "", confirmInUse: true });
              case "provider":
                return actions.remove(provider.slug, true);
            }
          }).finally(() => setBusy(false));
        }}
      />

      {/* @deprecated keys-rework #2306: the legacy managed row's own
          confirmation. The same dialog, because it is the same act — there is
          no provider record to compute `usedBy` from, so this asks plainly
          with no impact list. Removable with the row it belongs to. */}
      <RemoveProviderDialog
        intent={confirmingManaged ? "key" : null}
        label={MANAGED_TARGET_LABEL}
        impact={{ lastEnabled: false }}
        busy={busy}
        onCancel={() => setConfirmingManaged(false)}
        onConfirm={() => {
          setConfirmingManaged(false);
          // An empty key is how the store clears a value — it has no delete —
          // and it removes step 1 alone. The response re-reads the chain, so
          // the row immediately says whichever step answers next.
          fireAndForget(actions.saveManagedKey(""));
        }}
      />
    </div>
  );
}

/**
 * The one explanation that survives the deletion pass.
 *
 * A saved configuration that has landed and is not yet in effect looks exactly
 * like one that is — there is no control on the page whose appearance differs —
 * so this is the case where prose is carrying information rather than repeating
 * a button. Which brain a company runs is chosen when its runtime is built, so a
 * company that started with no model keeps echoing however the config changes
 * underneath it.
 *
 * The button appears only where the host said it can actually rebuild. Naming a
 * remedy and handing over a control that cannot perform it is worse than naming
 * the remedy alone.
 */
function RestartNotice({
  canRestart,
  onRestart,
}: {
  canRestart: boolean;
  onRestart: () => void;
}) {
  return (
    <Card data-testid="inference-restart-required">
      <CardContent className="flex flex-wrap items-center justify-between gap-3">
        <div className="grid min-w-0 flex-1 gap-1">
          <p className="text-sm">
            Restart required. This company booted without a model, so agents are still on the
            offline brain and the saved configuration is not yet in effect.
          </p>
          {/* The remedy, in both spellings the host could mean. The capability
              comes from the host, which does not know which shell it is
              packaged in — and this is the case where the operator cannot infer
              the next step from any control on the page, because there is no
              control for it. */}
          {!canRestart && (
            <p className="text-xs text-muted-foreground" data-testid="inference-restart-manual">
              This host cannot rebuild a company runtime in place: quit and reopen the app, or
              restart the server process.
            </p>
          )}
        </div>
        {canRestart && (
          <Button
            type="button"
            variant="outline"
            data-testid="inference-restart-now"
            onClick={onRestart}
          >
            Restart now
          </Button>
        )}
      </CardContent>
    </Card>
  );
}
