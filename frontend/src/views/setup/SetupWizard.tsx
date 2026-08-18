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
  proposeSetupRoster,
  submitSetup,
  type SetupApplied,
  type SetupField,
  type SetupRoster,
  type SetupStatus,
} from "@/api/setup";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Stepper, type Step } from "@/components/ui/stepper";
import {
  AUTOMATE_EXAMPLES,
  appendExample,
  emptySetupDraft,
  inferSignals,
  type SetupDraft,
} from "@/lib/company-setup";
import { cn } from "@/lib/utils";

/** The steps, in order. `fields` names the config keys each one owns. */
/**
 * The flow, question-first.
 *
 * The order is the whole design. Three cheap questions about *them* come before
 * anything about the machine, because nobody abandons "what do you sell" and
 * plenty abandon `bind`. The two asks that cost something — an address and a
 * credential — arrive fourth and fifth, after enough investment that their
 * purpose is self-evident rather than a wall on screen one.
 *
 * Everything else that used to be a step is now behind Advanced: sign-in mode,
 * brain, host and tools all have defaults that work, and a knob with a working
 * default is not a decision worth a screen.
 */
const STEPS: readonly (Step & { fields: readonly string[] })[] = [
  { id: "business", label: "Business", fields: [] },
  { id: "team", label: "Team", fields: [] },
  { id: "automate", label: "Automate", fields: [] },
  { id: "account", label: "You", fields: [] },
  { id: "power", label: "Power", fields: ["tinyhumans_api_key"] },
  { id: "review", label: "Review", fields: [] },
];

/** The steps that used to be their own screens, now grouped under Advanced. */
const ADVANCED_GROUPS: readonly { id: string; label: string; fields: readonly string[] }[] = [
  { id: "signin", label: "Sign-in", fields: ["auth_mode"] },
  { id: "brain", label: "Brain", fields: ["brain_mode", "api_url", "openhuman_url"] },
  { id: "tools", label: "Tools", fields: ["github_token"] },
  {
    id: "host",
    label: "Host",
    fields: ["bind", "public_url", "workspace.max_blob_mb", "workspace.storage_quota_gb"],
  },
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
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [applied, setApplied] = useState<SetupApplied | null>(null);

  /** The three answers. */
  const [draft, setDraft] = useState<SetupDraft>(emptySetupDraft);
  /** The address that will be able to sign in. */
  const [email, setEmail] = useState("");
  /** Whether the operator has been shown a problem on the current step yet. */
  const [touched, setTouched] = useState(false);
  /** Whether the Advanced disclosure is open. */
  const [advanced, setAdvanced] = useState(false);
  /**
   * The team, once the host has designed one — and `null` until then.
   *
   * Held as state rather than refetched per render because the operator edits
   * it: what they approve on Review is exactly what gets built, and a second
   * pass could return a different team.
   */
  const [roster, setRoster] = useState<SetupRoster | null>(null);
  const [designing, setDesigning] = useState(false);
  const [designError, setDesignError] = useState<string | null>(null);
  /** How many teammates have landed, once the apply is building them. */
  const [built, setBuilt] = useState<number | null>(null);

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

  /**
   * Ask the host to design a team, on the way into Review.
   *
   * Never throws upward: the host answers with its curated team rather than an
   * error when it cannot reach a model, so a rejection here is a genuine
   * transport failure — and even then the operator gets a roster to review,
   * because being stranded five screens in is the one outcome worse than an
   * imperfect team.
   */
  const design = useCallback(async () => {
    setDesigning(true);
    setDesignError(null);
    try {
      const proposed = await proposeSetupRoster(client, {
        industry: draft.industry,
        teamHint: draft.teamHint,
        automate: draft.automate,
        inferenceKey: values.tinyhumans_api_key || null,
      });
      // The host is contracted never to answer with an empty roster, so a
      // missing or empty one is a failure rather than a team of nobody — and
      // trusting the shape here crashed Review on `.map` of undefined.
      if (!Array.isArray(proposed?.agents) || proposed.agents.length === 0) {
        throw new Error("The host answered without a team to review.");
      }
      setRoster(proposed);
    } catch (err: unknown) {
      setDesignError(err instanceof Error ? err.message : String(err));
      setRoster(null);
    } finally {
      setDesigning(false);
    }
  }, [client, draft, values.tinyhumans_api_key]);

  const submit = useCallback(async () => {
    if (!status) return;
    // A host with no company and no designed roster would finish setup into
    // exactly the dead end this flow exists to remove: a configured instance
    // with nothing to sign in to and no way back into setup.
    if (status.companies.length === 0 && !roster?.agents.length) return;
    setSaving(true);
    setSaveError(null);
    setBuilt(roster?.agents.length ?? null);
    try {
      const result = await submitSetup(client, {
        fields: changed,
        company:
          status.companies.length === 0 && roster
            ? {
                industry: draft.industry,
                teamHint: draft.teamHint,
                automate: draft.automate,
                // As reviewed, not as proposed.
                agents: roster.agents,
                adminEmail: email.trim() || null,
              }
            : null,
      });
      setApplied(result);
    } catch (err: unknown) {
      setSaveError(err instanceof Error ? err.message : String(err));
      setBuilt(null);
    } finally {
      setSaving(false);
    }
  }, [client, status, changed, roster, draft, email]);

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
              {/* No template was chosen — the team was designed from the
                  answers, and saying otherwise would credit a menu the
                  operator never saw. */}
              Built <strong>{applied.seeded_company}</strong> with{" "}
              {roster?.agents.length ?? 0}{" "}
              {roster?.agents.length === 1 ? "teammate" : "teammates"}.
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
  const needsCompany = status.companies.length === 0;
  // The one thing that must never be reachable: a configured instance with
  // nothing to sign in to and no way back into setup.
  const noRoster = needsCompany && !roster?.agents.length;

  /** Whether this step can be left, and why not when it cannot. */
  const problem = (): string | undefined => {
    if (current.id === "business" && !draft.industry.trim()) {
      return "Tell us a little about the company first.";
    }
    if (current.id === "account" && needsCompany && !email.trim() && requiresSignIn(status, values)) {
      return "We need an address, or nobody will be able to sign in to this company.";
    }
    return undefined;
  };

  const advance = () => {
    if (problem()) {
      setTouched(true);
      return;
    }
    setTouched(false);
    // Designing happens on the way into Review, so the wait sits between two
    // screens rather than in front of one.
    if (STEPS[step + 1]?.id === "review" && !roster && !designing) void design();
    setStep((n) => n + 1);
  };

  return (
    <Shell>
      <div className="space-y-6" data-testid="setup-wizard">
        <div className="space-y-1">
          <h1 className="text-xl font-semibold">
            {status.complete ? "Reconfigure this instance" : "Let's build your company"}
          </h1>
          <p className="text-sm text-muted-foreground">
            {status.complete
              ? "Saved to "
              : "A few questions, then we'll put a team together. Saved to "}
            <code className="font-mono text-xs">{status.config_path}</code>.
          </p>
        </div>

        <Stepper steps={STEPS} current={step} onSelect={setStep} />

        <div className="min-h-64 space-y-4">
          {current.id === "business" && (
            <QuestionStep
              title="What kind of company are you setting up?"
              hint="A sentence is plenty. What you sell, or what you do."
              placeholder="e.g. E-commerce — I sell homeware online"
              value={draft.industry}
              testId="industry"
              onChange={(v) => setDraft((d) => ({ ...d, industry: v }))}
              onEnter={advance}
              signals={inferSignals(draft.industry)}
            />
          )}

          {current.id === "team" && (
            <QuestionStep
              title="Anyone in particular you need on the team?"
              hint="Optional. We'll suggest a team either way — this just adds to it."
              placeholder="e.g. someone chasing the customers who go quiet"
              value={draft.teamHint}
              testId="teamHint"
              multiline
              onChange={(v) => setDraft((d) => ({ ...d, teamHint: v }))}
            />
          )}

          {current.id === "automate" && (
            <QuestionStep
              title="What are you trying to automate?"
              hint="List whatever comes to mind. This is what your team gets built around."
              placeholder="e.g. Meta ads, order dispatch, daily sales reports"
              value={draft.automate}
              testId="automate"
              multiline
              onChange={(v) => setDraft((d) => ({ ...d, automate: v }))}
              examples={AUTOMATE_EXAMPLES}
              onExample={(ex) =>
                setDraft((d) => ({ ...d, automate: appendExample(d.automate, ex) }))
              }
            />
          )}

          {current.id === "account" && (
            <AccountStep
              value={email}
              onChange={setEmail}
              onEnter={advance}
              required={needsCompany && requiresSignIn(status, values)}
            />
          )}

          {current.id === "power" && (
            <PowerStep
              status={status}
              value={values.tinyhumans_api_key ?? ""}
              onChange={(v) => set("tinyhumans_api_key", v)}
              onEnter={advance}
            />
          )}

          {current.id === "review" && (
            <ReviewStep
              designing={designing}
              designError={designError}
              roster={roster}
              onRoster={setRoster}
              onRetry={() => void design()}
              changed={changed}
              restartKeys={restartKeys}
              status={status}
              email={email}
              built={built}
            />
          )}

          {problem() && touched && (
            <p className="text-sm text-destructive" data-testid="setup-problem">
              {problem()}
            </p>
          )}

          <AdvancedPanel
            open={advanced}
            onToggle={() => setAdvanced((v) => !v)}
            status={status}
            values={values}
            set={set}
          />
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
            <Button variant="outline" disabled={step === 0} onClick={() => setStep((n) => n - 1)}>
              Back
            </Button>
            {last ? (
              <Button
                onClick={() => void submit()}
                disabled={saving || noRoster || designing}
                data-testid="setup-finish"
              >
                {saving && <Loader2 className="animate-spin" />}
                Build my company
              </Button>
            ) : (
              <Button onClick={advance} data-testid="setup-next">
                Next
              </Button>
            )}
          </div>
        </div>
      </div>
    </Shell>
  );
}

/**
 * Whether this host will ask anyone to sign in.
 *
 * On `none` there is nobody to invite and an address would be a field with no
 * consequence; on every other mode the address is the only thing standing
 * between the operator and a company they cannot get into.
 */
function requiresSignIn(status: SetupStatus, values: Record<string, string>): boolean {
  const chosen =
    values.auth_mode ?? status.fields.find((f) => f.key === "auth_mode")?.value ?? "email";
  return chosen !== "none";
}

// ---------------------------------------------------------------------------
// Steps
// ---------------------------------------------------------------------------
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

// ---------------------------------------------------------------------------
// The question screens
// ---------------------------------------------------------------------------

/**
 * One question, one field, nothing else on screen.
 *
 * Deliberately sparse. These three screens are where an operator decides
 * whether this product is worth their afternoon, and every additional control
 * is something to read before answering.
 */
function QuestionStep({
  title,
  hint,
  placeholder,
  value,
  testId,
  multiline,
  onChange,
  onEnter,
  examples,
  onExample,
  signals,
}: {
  title: string;
  hint: string;
  placeholder: string;
  value: string;
  testId: string;
  multiline?: boolean;
  onChange: (v: string) => void;
  onEnter?: () => void;
  examples?: readonly string[];
  onExample?: (example: string) => void;
  /** What we heard, echoed back under the field. */
  signals?: string[];
}) {
  return (
    <div className="space-y-3">
      <div className="space-y-1">
        <h2 className="font-medium" data-testid="setup-question">
          {title}
        </h2>
        <p className="text-sm text-muted-foreground">{hint}</p>
      </div>

      {multiline ? (
        <Textarea
          autoFocus
          rows={3}
          value={value}
          placeholder={placeholder}
          data-testid={`setup-field-${testId}`}
          onChange={(e) => onChange(e.target.value)}
        />
      ) : (
        <Input
          autoFocus
          value={value}
          placeholder={placeholder}
          data-testid={`setup-field-${testId}`}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") onEnter?.();
          }}
        />
      )}

      {/* Proof they were heard, straight after the one question they put
          effort into. Silent when nothing was recognised — a wrong chip is
          worse than no chip. */}
      {signals && signals.length > 0 && (
        <div className="flex flex-wrap items-center gap-1.5" data-testid="setup-signals">
          <span className="text-xs text-muted-foreground">Sounds like</span>
          {signals.map((signal) => (
            <Badge key={signal} variant="secondary">
              {signal}
            </Badge>
          ))}
        </div>
      )}

      {/* Beside the field, not instead of it: they append rather than replace,
          so the operator's own words always survive. */}
      {examples && onExample && (
        <div className="flex flex-wrap gap-1.5">
          {examples.map((example) => (
            <button
              key={example}
              type="button"
              onClick={() => onExample(example)}
              data-testid={`setup-example-${example.replace(/\s+/g, "-")}`}
              className="rounded-full border px-2.5 py-1 text-xs text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
            >
              + {example}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

/**
 * The address that will be able to sign in.
 *
 * Asked here rather than on screen one because by now it has an obvious
 * purpose — it is how they get back to the company they are about to build,
 * not a field on a form. It is also the fix for a real dead end: no shipped
 * template invites anybody, so without this an operator who keeps email
 * sign-in finishes setup and can then sign in as nobody.
 */
function AccountStep({
  value,
  onChange,
  onEnter,
  required,
}: {
  value: string;
  onChange: (v: string) => void;
  onEnter: () => void;
  required: boolean;
}) {
  return (
    <div className="space-y-3">
      <div className="space-y-1">
        <h2 className="font-medium" data-testid="setup-question">
          What&apos;s your email?
        </h2>
        <p className="text-sm text-muted-foreground">
          {required
            ? "This is how you sign back in, and the only address that can administer the company."
            : "Optional on this host — you chose no sign-in, so anyone who can reach it is the owner."}
        </p>
      </div>
      <Label htmlFor="setup-email" className="sr-only">
        Your email
      </Label>
      <Input
        id="setup-email"
        autoFocus
        type="email"
        value={value}
        placeholder="you@example.com"
        data-testid="setup-field-email"
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") onEnter();
        }}
      />
    </div>
  );
}

/**
 * The credential, framed as what it buys rather than as what it is.
 *
 * Fifth, not first. By now the operator has described their business and can
 * see what the key is *for*; an API-key field on screen one is a wall in front
 * of a product nobody has seen yet. Skipping is a first-class answer, and the
 * copy says exactly what it costs.
 */
function PowerStep({
  status,
  value,
  onChange,
  onEnter,
}: {
  status: SetupStatus;
  value: string;
  onChange: (v: string) => void;
  onEnter: () => void;
}) {
  const field = status.fields.find((f) => f.key === "tinyhumans_api_key");
  const locked = field !== undefined && !field.editable;

  return (
    <div className="space-y-3">
      <div className="space-y-1">
        <h2 className="font-medium" data-testid="setup-question">
          What powers your team
        </h2>
        <p className="text-sm text-muted-foreground">
          Your teammates think with a hosted model. With a key we design the team
          around what you just told us; without one you get a solid standard team
          for your industry, and you can add a key later.
        </p>
      </div>

      {locked && <LayerLock />}

      <Label htmlFor="setup-key" className="sr-only">
        TinyHumans API key
      </Label>
      <Input
        id="setup-key"
        autoFocus
        type="password"
        value={value}
        disabled={locked}
        placeholder="th-…"
        data-testid="setup-field-key"
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") onEnter();
        }}
      />
      <p className="text-xs text-muted-foreground">
        Get one at tinyhumans.ai. Leave it blank to carry on without.
      </p>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Review, and the team as reviewed
// ---------------------------------------------------------------------------

/**
 * The team, before it exists.
 *
 * The screen that earns the ownership. People value what they had a hand in
 * shaping, and this is the honest place to catch a wrong guess — while it is
 * still four rows in a browser rather than six records on a host.
 *
 * It also says where the team came from, in a sentence. An operator shown a
 * curated roster with no indication assumes a model read their answers and
 * produced it, and judges the product on a team it never designed.
 */
function ReviewStep({
  designing,
  designError,
  roster,
  onRoster,
  onRetry,
  changed,
  restartKeys,
  status,
  email,
  built,
}: {
  designing: boolean;
  designError: string | null;
  roster: SetupRoster | null;
  onRoster: (roster: SetupRoster) => void;
  onRetry: () => void;
  changed: Record<string, string | null>;
  restartKeys: string[];
  status: SetupStatus;
  email: string;
  /** Non-null once the apply is building, so the button reads as progress. */
  built: number | null;
}) {
  if (designing) {
    return (
      <div className="flex flex-col items-center gap-3 py-12" data-testid="setup-designing">
        <Loader2 className="size-6 animate-spin text-primary" />
        <p className="text-sm text-muted-foreground">Designing your team…</p>
      </div>
    );
  }

  if (designError || !roster) {
    return (
      <Alert variant="destructive" data-testid="setup-design-error">
        <AlertTriangle />
        <AlertTitle>We couldn&apos;t design a team</AlertTitle>
        <AlertDescription className="space-y-2">
          <p>{designError ?? "The host returned nothing to review."}</p>
          <Button size="sm" variant="outline" onClick={onRetry}>
            Try again
          </Button>
        </AlertDescription>
      </Alert>
    );
  }

  const drop = (index: number) =>
    onRoster({ ...roster, agents: roster.agents.filter((_, i) => i !== index) });

  const rename = (index: number, role: string) =>
    onRoster({
      ...roster,
      agents: roster.agents.map((a, i) => (i === index ? { ...a, role } : a)),
    });

  return (
    <div className="space-y-4" data-testid="setup-review">
      <div className="space-y-1">
        <h2 className="font-medium">Your team</h2>
        <p className="text-sm text-muted-foreground">
          {roster.source === "model"
            ? "Built from what you told us. Rename or drop anyone — you can add more later."
            : "A solid standard team for your industry — we couldn't reach a model to tailor it. Rename or drop anyone, and add a key later to redesign."}
        </p>
      </div>

      <ul className="space-y-2">
        {roster.agents.map((agent, i) => (
          <li
            key={`${agent.role}-${i}`}
            className="flex items-start gap-3 rounded-lg border p-3"
            data-testid="setup-review-agent"
          >
            <div className="min-w-0 flex-1 space-y-1">
              <Input
                value={agent.role}
                aria-label={`Role for ${agent.role}`}
                data-testid="setup-review-role"
                onChange={(e) => rename(i, e.target.value)}
                className="h-8"
              />
              <p className="truncate text-xs text-muted-foreground">{agent.description}</p>
            </div>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => drop(i)}
              aria-label={`Remove ${agent.role}`}
              data-testid="setup-review-remove"
            >
              Remove
            </Button>
          </li>
        ))}
      </ul>

      {roster.agents.length === 0 && (
        <Alert>
          <AlertTriangle />
          <AlertTitle>That&apos;s everyone gone</AlertTitle>
          <AlertDescription>
            A company needs at least one teammate. Add one back, or start again.
          </AlertDescription>
        </Alert>
      )}

      {/* Stated, not asked. Nobody five screens in can answer a governance
          question; they can recognise a sentence and change it in Advanced. */}
      <div className="rounded-lg border border-dashed p-3 text-sm text-muted-foreground">
        <p>
          Anything that leaves the company — sending, publishing, spending — waits
          for you until you say otherwise.
        </p>
        {email.trim() && (
          <p className="mt-1">
            You&apos;ll sign in as <span className="font-medium text-foreground">{email.trim()}</span>.
          </p>
        )}
      </div>

      {built !== null && (
        <p className="text-sm text-muted-foreground" data-testid="setup-building">
          Building {built} {built === 1 ? "teammate" : "teammates"}…
        </p>
      )}

      {(Object.keys(changed).length > 0 || restartKeys.length > 0) && (
        <details className="rounded-lg border p-3">
          <summary className="cursor-pointer text-sm font-medium">
            Settings this will write ({Object.keys(changed).length})
          </summary>
          <ul className="mt-2 space-y-1 text-xs text-muted-foreground">
            {Object.keys(changed).map((key) => (
              <li key={key}>
                <code className="font-mono">{key}</code>
                {restartKeys.includes(key) && " — takes effect after a restart"}
              </li>
            ))}
          </ul>
        </details>
      )}

      {status.companies.length > 0 && (
        <p className="text-xs text-muted-foreground">
          This host already serves a company, so no new one will be created.
        </p>
      )}
    </div>
  );
}

/**
 * Everything that used to be a step.
 *
 * A disclosure rather than four screens: sign-in mode, brain, host and tools all
 * have defaults that work on a laptop, and a knob with a working default is not
 * a decision worth putting in front of someone who has not seen the product yet.
 * Nothing is hidden — it is one click away from every screen, and the fields
 * keep their layer locks, so an env-owned value still refuses to pretend.
 */
function AdvancedPanel({
  open,
  onToggle,
  status,
  values,
  set,
}: {
  open: boolean;
  onToggle: () => void;
  status: SetupStatus;
  values: Record<string, string>;
  set: (key: string, value: string) => void;
}) {
  return (
    <div className="rounded-lg border">
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={open}
        data-testid="setup-advanced-toggle"
        className="flex w-full items-center justify-between px-3 py-2 text-left text-sm font-medium"
      >
        Advanced settings
        <span className="text-xs font-normal text-muted-foreground">
          {open ? "Hide" : "Sign-in, brain, host, tools"}
        </span>
      </button>

      {open && (
        <div className="space-y-6 border-t p-3" data-testid="setup-advanced">
          <SignInStep
            status={status}
            value={values.auth_mode ?? ""}
            onChange={(v) => set("auth_mode", v)}
          />
          {ADVANCED_GROUPS.filter((group) => group.id !== "signin").map((group) => (
            <div key={group.id} className="space-y-3">
              <h3 className="text-sm font-medium">{group.label}</h3>
              {group.id === "tools" && <ToolsStep status={status} />}
              {fieldsFor(status, group.fields).map((f) => (
                <FieldRow
                  key={f.key}
                  field={f}
                  value={values[f.key] ?? ""}
                  onChange={(v) => set(f.key, v)}
                />
              ))}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
