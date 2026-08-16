// The first-run setup wizard.
//
// One flow that configures an instance: pick a company template, choose how
// people sign in, point the brain at a credential, review the tool surfaces
// this build has, and commit. Before this existed the same decisions were
// spread across a hand-edited `config.toml`, a `serve --company` flag and six
// Settings sub-pages, and a freshly spun-up harness with no company dead-ended
// on "No companies are running on this host".
//
// ## What it writes, and what it only stages
//
// Everything here lands in `config.toml`, which is the *second* precedence
// layer (`env ⟵ config.toml ⟵ manifest ⟵ default`). Two consequences the UI
// has to be honest about rather than hide:
//
//   - A field the environment owns cannot be written at all. The host reports
//     `editable: false` for those and refuses the write; we render them
//     read-only with the owning layer shown, so nobody submits a change that
//     silently does nothing.
//   - Host-level fields are read once, at boot, so a change to some of them is
//     *staged* rather than applied. The host applies what it can in place (it
//     rebuilds companies for a new sign-in mode) and reports what is genuinely
//     left; the completion screen shows that answer, not a guess, and never
//     implies its own button performed the restart.

import { useCallback, useEffect, useMemo, useState } from "react";
import { AlertTriangle, Loader2, Lock, RotateCw } from "lucide-react";

import type { OpenCompanyClient } from "@/api/client";
import {
  changedFields,
  fieldsFor,
  getSetup,
  submitSetup,
  type SetupApplied,
  type SetupField,
  type SetupStatus,
} from "@/api/setup";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Stepper, type Step } from "@/components/ui/stepper";
import { cn } from "@/lib/utils";

/** The steps, in order. `fields` names the config keys each one owns. */
const STEPS: readonly (Step & { fields: readonly string[] })[] = [
  { id: "template", label: "Company", fields: [] },
  { id: "signin", label: "Sign-in", fields: ["auth_mode"] },
  {
    id: "brain",
    label: "Brain",
    fields: ["brain_mode", "tinyhumans_api_key", "api_url", "openhuman_url"],
  },
  { id: "tools", label: "Tools", fields: ["github_token"] },
  {
    id: "host",
    label: "Host",
    fields: ["bind", "public_url", "workspace.max_blob_mb", "workspace.storage_quota_gb"],
  },
  { id: "review", label: "Review", fields: [] },
];

/** How each sign-in mode is described, in consequences rather than mode names. */
const AUTH_MODE_COPY: Record<string, { label: string; hint: string }> = {
  email: {
    label: "Email",
    hint: "People sign in with a magic link sent to an invited address.",
  },
  wallet: {
    label: "Wallet",
    hint: "People sign in by signing a challenge with an invited wallet.",
  },
  none: {
    label: "No sign-in",
    hint: "Anyone who can reach this host is the owner. Only offered because this host is loopback-only.",
  },
};

/**
 * Headings for the steps that are otherwise a bare list of config keys.
 *
 * `tools` is absent on purpose: it renders its own heading above the build
 * status list, and a second one over the fields below would read as a new
 * section rather than the same one continuing.
 */
const STEP_INTRO: Record<string, { title: string; hint: string }> = {
  brain: {
    title: "What powers your teammates",
    hint: "Where the company's thinking runs, and the credential it uses. You can leave these on their defaults and set them later.",
  },
  host: {
    title: "How this host runs",
    hint: "The address it serves on and how much room its workspace gets. Defaults are fine for a laptop.",
  },
};

interface Props {
  client: OpenCompanyClient;
  /** Called once setup has been applied, so the caller can re-enter the console. */
  onDone: () => void;
  /**
   * Whether the operator can leave without finishing. False on a genuine first
   * run, where there is no console to go back to.
   */
  onCancel?: () => void;
}

export function SetupWizard({ client, onDone, onCancel }: Props) {
  const [status, setStatus] = useState<SetupStatus | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [step, setStep] = useState(0);
  const [values, setValues] = useState<Record<string, string>>({});
  const [template, setTemplate] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [applied, setApplied] = useState<SetupApplied | null>(null);

  useEffect(() => {
    let cancelled = false;
    getSetup(client)
      .then((s) => {
        if (cancelled) return;
        setStatus(s);
        // Seed the form from what the file already holds, so an operator
        // re-running setup edits their configuration rather than a blank one.
        const seeded: Record<string, string> = {};
        for (const f of s.fields) if (f.value !== null) seeded[f.key] = f.value;
        setValues(seeded);
      })
      .catch((err: unknown) => {
        if (!cancelled) setLoadError(err instanceof Error ? err.message : String(err));
      });
    return () => {
      cancelled = true;
    };
  }, [client]);

  const set = useCallback((key: string, value: string) => {
    setValues((prev) => ({ ...prev, [key]: value }));
  }, []);

  // See `changedFields`: unchanged fields are omitted, env-owned ones are never
  // sent (the host refuses them and an apply is all-or-nothing), and a secret
  // goes only when the operator typed one.
  const changed = useMemo(
    () => (status ? changedFields(status, values) : {}),
    [status, values],
  );

  const restartKeys = useMemo(() => {
    if (!status) return [];
    return Object.keys(changed).filter(
      (k) => status.fields.find((f) => f.key === k)?.requires_restart,
    );
  }, [status, changed]);

  const submit = useCallback(async () => {
    if (!status) return;
    // Mirrors the "Finish setup" button's `disabled`: a host with no
    // companies and no template chosen must not complete setup into a
    // configured dead end (see `noCompanyChosen` above the button).
    if (status.companies.length === 0 && template === null) return;
    setSaving(true);
    setSaveError(null);
    try {
      const result = await submitSetup(client, {
        fields: changed,
        template: status.companies.length === 0 ? template : null,
      });
      setApplied(result);
    } catch (err: unknown) {
      setSaveError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  }, [client, status, changed, template]);

  if (loadError) {
    return (
      <Shell>
        <Alert variant="destructive">
          <AlertTitle>Can&apos;t read this instance&apos;s setup</AlertTitle>
          <AlertDescription>{loadError}</AlertDescription>
        </Alert>
      </Shell>
    );
  }

  if (!status) {
    return (
      <Shell>
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          <Loader2 className="size-4 animate-spin" /> Reading this instance…
        </div>
      </Shell>
    );
  }

  if (applied) {
    return (
      <Shell>
        <div className="space-y-4" data-testid="setup-done">
          <h1 className="text-xl font-semibold">You&apos;re set up</h1>
          <p className="text-sm text-muted-foreground">
            Written to <code className="font-mono text-xs">{applied.config_path}</code>.
          </p>
          {applied.seeded_company && (
            <p className="text-sm text-muted-foreground">
              Started <strong>{applied.seeded_company}</strong> from your chosen template.
            </p>
          )}
          {/* The button below cannot restart the host — it only re-enters the
              console — so this must not read as something already handled.
              Naming the setting and the action keeps the two apart. */}
          {applied.restart_required.length > 0 && (
            <Alert>
              <RotateCw />
              <AlertTitle>
                You need to restart the host for {applied.restart_required.length} setting(s)
              </AlertTitle>
              <AlertDescription>
                <span className="block">
                  These are read once, when the host starts, so they are saved but{" "}
                  <strong>not yet in force</strong>:{" "}
                  <span className="font-mono text-xs">
                    {applied.restart_required.join(", ")}
                  </span>
                </span>
                <span className="mt-2 block">
                  Stop the <code className="font-mono text-xs">opencompany serve</code> process
                  and start it again. Opening the console now works, but with the previous
                  values for those settings.
                </span>
              </AlertDescription>
            </Alert>
          )}
          <Button onClick={onDone}>
            {applied.restart_required.length > 0 ? "Open the console anyway" : "Open the console"}
          </Button>
        </div>
      </Shell>
    );
  }

  const current = STEPS[step];
  const last = step === STEPS.length - 1;
  // A host with no companies and no template chosen would finish setup into
  // exactly the dead end this flow exists to remove: a configured instance
  // with nothing to sign in to and no way back into setup. Block completion
  // rather than let that state be reachable.
  const noCompanyChosen = status.companies.length === 0 && template === null;

  return (
    <Shell>
      <div className="space-y-6" data-testid="setup-wizard">
        <div className="space-y-1">
          <h1 className="text-xl font-semibold">
            {status.complete ? "Reconfigure this instance" : "Set up this instance"}
          </h1>
          <p className="text-sm text-muted-foreground">
            Saved to <code className="font-mono text-xs">{status.config_path}</code>.
          </p>
        </div>

        <Stepper steps={STEPS} current={step} onSelect={setStep} />

        <div className="min-h-64 space-y-4">
          {current.id === "template" && (
            <TemplateStep
              status={status}
              selected={template}
              onSelect={setTemplate}
            />
          )}

          {current.id === "signin" && (
            <SignInStep
              status={status}
              value={values.auth_mode ?? ""}
              onChange={(v) => set("auth_mode", v)}
            />
          )}

          {current.id === "tools" && <ToolsStep status={status} />}

          {current.id === "review" && (
            <ReviewStep
              changed={changed}
              restartKeys={restartKeys}
              template={template}
              status={status}
              noCompanyChosen={noCompanyChosen}
            />
          )}

          {/* The plain-field steps share one renderer. `tools` adds its fields
              below its own build-status list rather than instead of it. */}
          {(current.id === "brain" || current.id === "host" || current.id === "tools") && (
            <div className="space-y-4">
              {STEP_INTRO[current.id] && (
                <div className="space-y-1">
                  <h2 className="font-medium">{STEP_INTRO[current.id].title}</h2>
                  <p className="text-sm text-muted-foreground">
                    {STEP_INTRO[current.id].hint}
                  </p>
                </div>
              )}
              {fieldsFor(status, current.fields).map((f) => (
                <FieldRow
                  key={f.key}
                  field={f}
                  value={values[f.key] ?? ""}
                  onChange={(v) => set(f.key, v)}
                />
              ))}
            </div>
          )}
        </div>

        {saveError && (
          <Alert variant="destructive">
            <AlertTriangle />
            <AlertTitle>That didn&apos;t apply</AlertTitle>
            <AlertDescription>{saveError}</AlertDescription>
          </Alert>
        )}

        <div className="flex items-center justify-between gap-2 border-t pt-4">
          <div>
            {onCancel && (
              <Button variant="ghost" onClick={onCancel}>
                Cancel
              </Button>
            )}
          </div>
          <div className="flex gap-2">
            <Button variant="outline" disabled={step === 0} onClick={() => setStep((s) => s - 1)}>
              Back
            </Button>
            {last ? (
              <Button
                onClick={() => void submit()}
                disabled={saving || noCompanyChosen}
                data-testid="setup-finish"
              >
                {saving && <Loader2 className="animate-spin" />}
                Finish setup
              </Button>
            ) : (
              <Button onClick={() => setStep((s) => s + 1)}>Next</Button>
            )}
          </div>
        </div>
      </div>
    </Shell>
  );
}

// ---------------------------------------------------------------------------
// Steps
// ---------------------------------------------------------------------------

function TemplateStep({
  status,
  selected,
  onSelect,
}: {
  status: SetupStatus;
  selected: string | null;
  onSelect: (id: string) => void;
}) {
  // A host that already has a company must not be handed a second starter one —
  // the host refuses to seed in that case, so offering the choice would be a
  // control that does nothing.
  if (status.companies.length > 0) {
    return (
      <div className="space-y-2">
        <h2 className="font-medium">Company</h2>
        <p className="text-sm text-muted-foreground">
          This host is already running <strong>{status.companies.join(", ")}</strong>, so setup
          won&apos;t start another one. Everything else on this page still applies.
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <div className="space-y-1">
        <h2 className="font-medium">Start from a template</h2>
        <p className="text-sm text-muted-foreground">
          This becomes your first company. You can change everything about it afterwards.
        </p>
      </div>
      <div className="grid gap-2 sm:grid-cols-2">
        {status.templates.map((t) => (
          <button
            key={t.id}
            type="button"
            onClick={() => onSelect(t.id)}
            data-testid={`template-${t.id}`}
            aria-pressed={selected === t.id}
            className={cn(
              "rounded-lg border p-3 text-left transition-colors hover:bg-muted",
              selected === t.id && "border-primary bg-muted",
            )}
          >
            <div className="text-sm font-medium">{t.name}</div>
            {t.output && (
              <div className="mt-0.5 line-clamp-2 text-xs text-muted-foreground">{t.output}</div>
            )}
            <div className="mt-1.5 text-xs text-muted-foreground">
              {t.agent_count} teammate{t.agent_count === 1 ? "" : "s"}
            </div>
          </button>
        ))}
      </div>
    </div>
  );
}

function SignInStep({
  status,
  value,
  onChange,
}: {
  status: SetupStatus;
  value: string;
  onChange: (v: string) => void;
}) {
  const field = status.fields.find((f) => f.key === "auth_mode");
  const locked = field !== undefined && !field.editable;

  return (
    <div className="space-y-3">
      <div className="space-y-1">
        <h2 className="font-medium">How people sign in</h2>
        <p className="text-sm text-muted-foreground">
          This applies to every company this host serves.
        </p>
      </div>

      {locked && <LayerLock />}

      <div className="space-y-2">
        {status.auth_modes.map((mode) => {
          const copy = AUTH_MODE_COPY[mode] ?? { label: mode, hint: "" };
          const active = (value || field?.value) === mode;
          return (
            <button
              key={mode}
              type="button"
              disabled={locked}
              onClick={() => onChange(mode)}
              data-testid={`auth-mode-${mode}`}
              aria-pressed={active}
              className={cn(
                "w-full rounded-lg border p-3 text-left transition-colors",
                !locked && "hover:bg-muted",
                active && "border-primary bg-muted",
                locked && "opacity-60",
              )}
            >
              <div className="text-sm font-medium">{copy.label}</div>
              <div className="mt-0.5 text-xs text-muted-foreground">{copy.hint}</div>
            </button>
          );
        })}
      </div>

      {!status.auth_modes.includes("none") && (
        <p className="text-xs text-muted-foreground">
          &ldquo;No sign-in&rdquo; isn&apos;t offered because this host binds a routable address,
          where it would serve an unauthenticated admin console to anyone who can reach it.
        </p>
      )}
    </div>
  );
}

/**
 * Tool surfaces. Read-only by nature: these are cargo features of the host's
 * build, so the honest thing is to report them rather than to offer switches
 * that write nothing.
 */
function ToolsStep({ status }: { status: SetupStatus }) {
  const rows: { label: string; on: boolean; note?: string }[] = [
    {
      label: "MCP tool servers",
      on: status.build.mcp_in_build,
      note: status.build.mcp_in_build
        ? "Add servers in Settings → MCP Servers once you're in."
        : "Not compiled into this build.",
    },
    {
      label: "Agent harness",
      on: status.build.harness_in_build,
      note: status.build.harness_in_build ? undefined : "Not compiled into this build.",
    },
    {
      label: "Third-party connections",
      on: status.build.oauth_in_build,
      note: status.build.oauth_in_build ? "Connect accounts in Settings → Connections." : undefined,
    },
    {
      label: "Agent Client Protocol (ACP)",
      // In-build is necessary but not sufficient: this host compiles the ACP
      // session model without mounting a `/acp` route, so a client dialing it
      // would get a 404. Reporting the transport separately is the difference
      // between "not available" and "misconfigured".
      on: status.build.acp_in_build && status.build.acp_transport_mounted,
      note: !status.build.acp_in_build
        ? "Not compiled into this build."
        : status.build.acp_transport_mounted
          ? undefined
          : "Compiled in, but no endpoint is mounted yet — external ACP clients can't connect.",
    },
  ];

  return (
    <div className="space-y-4">
      <div className="space-y-1">
        <h2 className="font-medium">Tools this build has</h2>
        <p className="text-sm text-muted-foreground">
          These come from how the host was built, so they aren&apos;t settings you can change
          here.
        </p>
      </div>
      <div className="space-y-2">
        {rows.map((r) => (
          <div
            key={r.label}
            className="flex items-start justify-between gap-3 rounded-lg border p-3"
            data-testid={`build-${r.label}`}
          >
            <div className="min-w-0">
              <div className="text-sm font-medium">{r.label}</div>
              {r.note && <div className="mt-0.5 text-xs text-muted-foreground">{r.note}</div>}
            </div>
            <Badge variant={r.on ? "default" : "outline"}>{r.on ? "Available" : "Off"}</Badge>
          </div>
        ))}
      </div>
    </div>
  );
}

function ReviewStep({
  changed,
  restartKeys,
  template,
  status,
  noCompanyChosen,
}: {
  changed: Record<string, string | null>;
  restartKeys: string[];
  template: string | null;
  status: SetupStatus;
  noCompanyChosen: boolean;
}) {
  const keys = Object.keys(changed);
  const seeding = status.companies.length === 0 && template !== null;

  return (
    <div className="space-y-4">
      <div className="space-y-1">
        <h2 className="font-medium">Review</h2>
        <p className="text-sm text-muted-foreground">Nothing is written until you finish.</p>
      </div>

      {noCompanyChosen && (
        <Alert variant="destructive" data-testid="review-no-company-warning">
          <AlertTriangle />
          <AlertTitle>Pick a company template before finishing</AlertTitle>
          <AlertDescription>
            This host has no companies yet. Finishing without picking a template on the{" "}
            <strong>Company</strong> step would leave a configured instance with nothing to sign
            in to.
          </AlertDescription>
        </Alert>
      )}

      {seeding && (
        <div className="rounded-lg border p-3 text-sm">
          Start <strong>{status.templates.find((t) => t.id === template)?.name ?? template}</strong>{" "}
          as this instance&apos;s first company.
        </div>
      )}

      {keys.length === 0 ? (
        <p className="text-sm text-muted-foreground" data-testid="review-no-changes">
          No settings changed.
        </p>
      ) : (
        <ul className="space-y-1.5">
          {keys.map((k) => (
            <li key={k} className="flex items-baseline justify-between gap-3 text-sm">
              <span className="font-mono text-xs">{k}</span>
              <span className="truncate text-muted-foreground">
                {status.fields.find((f) => f.key === k)?.secret
                  ? "•••••"
                  : (changed[k] ?? "(cleared)")}
              </span>
            </li>
          ))}
        </ul>
      )}

      {/* An estimate, not the answer: the host decides what it can apply in
          place, and the completion screen reports what it actually did. Some of
          these (the sign-in mode) are applied live when the host can rebuild. */}
      {restartKeys.length > 0 && (
        <Alert>
          <RotateCw />
          <AlertTitle>Some of these may need a host restart</AlertTitle>
          <AlertDescription>
            The host reads settings like{" "}
            <span className="font-mono text-xs">{restartKeys.join(", ")}</span> when it starts.
            It applies what it can right away and will tell you exactly what is left to restart
            for.
          </AlertDescription>
        </Alert>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Shared bits
// ---------------------------------------------------------------------------

function FieldRow({
  field,
  value,
  onChange,
}: {
  field: SetupField;
  value: string;
  onChange: (v: string) => void;
}) {
  const locked = !field.editable;
  return (
    <div className="space-y-1.5">
      <div className="flex items-center gap-2">
        <Label htmlFor={field.key} className="font-mono text-xs">
          {field.key}
        </Label>
        {field.requires_restart && (
          <Badge variant="outline" className="text-3xs">
            restart
          </Badge>
        )}
      </div>
      <Input
        id={field.key}
        data-testid={`field-${field.key}`}
        value={locked ? (field.value ?? "") : value}
        disabled={locked}
        type={field.secret ? "password" : "text"}
        placeholder={field.secret ? "unchanged" : `set by ${field.layer}`}
        onChange={(e) => onChange(e.target.value)}
      />
      {locked && <LayerLock />}
    </div>
  );
}

/**
 * Why a field can't be edited.
 *
 * Worth its own component because the reason is not obvious and the failure it
 * prevents is silent: `config.toml` sits *below* the environment in precedence,
 * so writing an env-owned field would produce a saved value that the next boot
 * ignores. Saying so beats disabling an input with no explanation.
 */
function LayerLock() {
  return (
    <p className="flex items-start gap-1.5 text-xs text-muted-foreground">
      <Lock className="mt-0.5 size-3 shrink-0" />
      <span>
        Set by an environment variable on this host, which outranks{" "}
        <code className="font-mono">config.toml</code>. Change it where the host is deployed.
      </span>
    </p>
  );
}

function Shell({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex min-h-screen items-center justify-center p-6">
      <div className="w-full max-w-2xl">{children}</div>
    </div>
  );
}
