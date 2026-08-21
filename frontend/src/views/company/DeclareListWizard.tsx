// The declare wizard (issue #1284): four plain-language questions and a
// review, replacing the JSON `fields[]`/`statuses[]` dialog that used to be
// "New ledger" on the Ledgers screen. Every step maps onto exactly the body
// `defineLedger()` already posts — `lib/ledger-wizard.ts` is the one place
// that assembly happens, so this component only ever collects a draft and
// hands it there.
//
// Reached from `ManageListsView`, itself reached from the Company page's
// "Manage lists" button — mirroring `DeskCreateDialog`'s place in the org
// chart, and built on `Stepper`, the one stepped-flow component the console
// already has (`views/setup/SetupWizard.tsx`).

import { useMemo, useState } from "react";
import { AlertTriangle, Plus, Trash2 } from "lucide-react";

import type { OpenCompanyClient } from "@/api/client";
import { defineLedger, type FieldRole, type LedgerSummary } from "@/api/ledgers";
import { Alert, AlertDescription } from "@/components/ui/alert";
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
import { Stepper, type Step } from "@/components/ui/stepper";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";
import {
  buildLedgerSpec,
  FIELD_PRESETS,
  slugify,
  STAGE_PRESETS,
  stagePreset,
  summarize,
  type FieldPresetId,
  type StagePresetId,
  type WizardField,
  type WizardStatus,
} from "@/lib/ledger-wizard";

const STEPS: readonly Step[] = [
  { id: "purpose", label: "What to track" },
  { id: "name", label: "Name it" },
  { id: "stages", label: "Stages" },
  { id: "fields", label: "Row details" },
  { id: "review", label: "Review" },
];

const CUSTOM_ROLE_OPTIONS: readonly { value: FieldRole; label: string }[] = [
  { value: "owner", label: "Owner" },
  { value: "prose", label: "Notes" },
  { value: "date", label: "Date" },
  { value: "number", label: "Number" },
  { value: "refs", label: "Links to other rows" },
];

interface Props {
  client: OpenCompanyClient;
  company: string;
  /** Every slug this company already holds, so the derived slug never collides. */
  existingSlugs: readonly string[];
  /** How many more this company may declare, from `useLedgerNav`. */
  remaining: number;
  onCancel: () => void;
  /** A list was declared. Carries the new summary so the caller can refresh. */
  onCreated: (created: LedgerSummary) => void;
}

export function DeclareListWizard({
  client,
  company,
  existingSlugs,
  remaining,
  onCancel,
  onCreated,
}: Props) {
  const [step, setStep] = useState(0);
  const [purpose, setPurpose] = useState("");
  const [title, setTitle] = useState("");
  const [slug, setSlug] = useState("");
  const [slugTouched, setSlugTouched] = useState(false);
  const [stageChoice, setStageChoice] = useState<StagePresetId>("todo-in-progress-done");
  const [customStatuses, setCustomStatuses] = useState<WizardStatus[]>([
    { name: "", closed: false, needs_reason: false },
  ]);
  const [fieldChoices, setFieldChoices] = useState<Set<FieldPresetId>>(new Set());
  const [customFields, setCustomFields] = useState<WizardField[]>([]);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const derivedSlug = useMemo(() => slugify(title, existingSlugs), [title, existingSlugs]);
  const effectiveSlug = slugTouched ? slug : derivedSlug;

  const statuses = useMemo<WizardStatus[]>(() => {
    if (stageChoice === "custom") {
      return customStatuses.filter((s) => s.name.trim() !== "");
    }
    return stagePreset(stageChoice)?.statuses ?? [];
  }, [stageChoice, customStatuses]);

  const fields = useMemo<WizardField[]>(() => {
    const preset = FIELD_PRESETS.filter((p) => fieldChoices.has(p.id)).map((p) => p.field);
    return [...preset, ...customFields.filter((f) => f.name.trim() !== "")];
  }, [fieldChoices, customFields]);

  const draft = { purpose: purpose.trim(), title: title.trim(), slug: effectiveSlug, statuses, fields };

  const stepValid: Record<number, boolean> = {
    0: purpose.trim().length > 0,
    1: title.trim().length > 0 && effectiveSlug.length > 0,
    2: statuses.length > 0,
    3: true,
    4: true,
  };

  const canAdvance = stepValid[step] ?? true;

  const toggleFieldPreset = (id: FieldPresetId) => {
    setFieldChoices((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const submit = async () => {
    setError(null);
    setSubmitting(true);
    try {
      const spec = buildLedgerSpec(draft);
      const created = await defineLedger(client, company, spec);
      onCreated(created);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Dialog open onOpenChange={onCancel}>
      <DialogContent className="max-h-[85vh] sm:max-w-2xl overflow-y-auto">
        <DialogHeader>
          <DialogTitle>New list</DialogTitle>
          <DialogDescription>
            {remaining} more can be declared. Answer four questions and this
            company gets a new list, right beside the ones it already has.
          </DialogDescription>
        </DialogHeader>

        <Stepper steps={STEPS} current={step} className="mb-2" />

        {step === 0 && (
          <div className="space-y-2">
            <Label htmlFor="wizard-purpose">What do you want to track?</Label>
            <Textarea
              id="wizard-purpose"
              rows={3}
              placeholder="What we told a customer we would do, and whether we did it."
              value={purpose}
              onChange={(e) => setPurpose(e.target.value)}
            />
            <p className="text-xs text-muted-foreground">
              A sentence or two. This shows up wherever the list explains
              itself — including to any teammate who reads it.
            </p>
          </div>
        )}

        {step === 1 && (
          <div className="space-y-3">
            <div className="space-y-2">
              <Label htmlFor="wizard-title">Name it</Label>
              <Input
                id="wizard-title"
                placeholder="Customer promises"
                value={title}
                onChange={(e) => setTitle(e.target.value)}
                autoFocus
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="wizard-slug">Short id</Label>
              <Input
                id="wizard-slug"
                value={effectiveSlug}
                onChange={(e) => {
                  setSlugTouched(true);
                  setSlug(e.target.value.toLowerCase());
                }}
              />
              <p className="text-xs text-muted-foreground">
                How this list is addressed everywhere else — lowercase letters,
                digits and hyphens. Derived from the name; edit it if it reads
                oddly.
              </p>
            </div>
          </div>
        )}

        {step === 2 && (
          <div className="space-y-3">
            <p className="text-sm text-muted-foreground">
              What stages does one row go through?
            </p>
            <div className="grid gap-2 sm:grid-cols-2">
              {STAGE_PRESETS.map((preset) => (
                <button
                  key={preset.id}
                  type="button"
                  onClick={() => setStageChoice(preset.id)}
                  className={cn(
                    "rounded-md border p-3 text-left text-sm",
                    stageChoice === preset.id
                      ? "border-primary bg-accent"
                      : "border-border hover:bg-accent/50",
                  )}
                >
                  <div className="font-medium">{preset.label}</div>
                  <div className="text-xs text-muted-foreground">{preset.hint}</div>
                </button>
              ))}
              <button
                type="button"
                onClick={() => setStageChoice("custom")}
                className={cn(
                  "rounded-md border p-3 text-left text-sm",
                  stageChoice === "custom"
                    ? "border-primary bg-accent"
                    : "border-border hover:bg-accent/50",
                )}
              >
                <div className="font-medium">Custom</div>
                <div className="text-xs text-muted-foreground">
                  Name your own stages.
                </div>
              </button>
            </div>

            {stageChoice === "custom" && (
              <div className="space-y-2 rounded-md border p-3">
                {customStatuses.map((status, i) => (
                  <div key={i} className="flex flex-wrap items-center gap-2">
                    <Input
                      className="w-40"
                      placeholder="stage name"
                      value={status.name}
                      onChange={(e) =>
                        setCustomStatuses((current) =>
                          current.map((s, idx) =>
                            idx === i ? { ...s, name: e.target.value } : s,
                          ),
                        )
                      }
                    />
                    <label className="flex items-center gap-1.5 text-xs">
                      <Switch
                        checked={status.closed}
                        onCheckedChange={(checked) =>
                          setCustomStatuses((current) =>
                            current.map((s, idx) =>
                              idx === i
                                ? { ...s, closed: checked, needs_reason: checked && s.needs_reason }
                                : s,
                            ),
                          )
                        }
                      />
                      Ends the row
                    </label>
                    <label className="flex items-center gap-1.5 text-xs">
                      <Switch
                        checked={status.needs_reason}
                        disabled={!status.closed}
                        onCheckedChange={(checked) =>
                          setCustomStatuses((current) =>
                            current.map((s, idx) =>
                              idx === i ? { ...s, needs_reason: checked } : s,
                            ),
                          )
                        }
                      />
                      Needs a reason
                    </label>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      onClick={() =>
                        setCustomStatuses((current) => current.filter((_, idx) => idx !== i))
                      }
                      disabled={customStatuses.length <= 1}
                    >
                      <Trash2 className="size-4" />
                    </Button>
                  </div>
                ))}
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={() =>
                    setCustomStatuses((current) => [
                      ...current,
                      { name: "", closed: false, needs_reason: false },
                    ])
                  }
                >
                  <Plus className="mr-2 size-4" />
                  Add a stage
                </Button>
              </div>
            )}
          </div>
        )}

        {step === 3 && (
          <div className="space-y-3">
            <p className="text-sm text-muted-foreground">
              Every row already gets a title. What else does each one need?
            </p>
            <div className="space-y-2">
              {FIELD_PRESETS.map((preset) => (
                <label key={preset.id} className="flex items-center gap-2 text-sm">
                  <Switch
                    checked={fieldChoices.has(preset.id)}
                    onCheckedChange={() => toggleFieldPreset(preset.id)}
                  />
                  {preset.label}
                </label>
              ))}
            </div>
            <div className="space-y-2 rounded-md border p-3">
              {customFields.map((field, i) => (
                <div key={i} className="flex flex-wrap items-center gap-2">
                  <Input
                    className="w-40"
                    placeholder="field name"
                    value={field.name}
                    onChange={(e) =>
                      setCustomFields((current) =>
                        current.map((f, idx) =>
                          idx === i ? { ...f, name: e.target.value } : f,
                        ),
                      )
                    }
                  />
                  <select
                    className="h-9 rounded-md border bg-transparent px-2 text-sm"
                    value={field.role}
                    onChange={(e) =>
                      setCustomFields((current) =>
                        current.map((f, idx) =>
                          idx === i ? { ...f, role: e.target.value as FieldRole } : f,
                        ),
                      )
                    }
                  >
                    {CUSTOM_ROLE_OPTIONS.map((opt) => (
                      <option key={opt.value} value={opt.value}>
                        {opt.label}
                      </option>
                    ))}
                  </select>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    onClick={() =>
                      setCustomFields((current) => current.filter((_, idx) => idx !== i))
                    }
                  >
                    <Trash2 className="size-4" />
                  </Button>
                </div>
              ))}
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() =>
                  setCustomFields((current) => [...current, { name: "", role: "prose" }])
                }
              >
                <Plus className="mr-2 size-4" />
                Add a custom field
              </Button>
            </div>
          </div>
        )}

        {step === 4 && (
          <div className="space-y-3">
            <p className="text-sm">{summarize(draft)}</p>
            <p className="text-xs text-muted-foreground">
              Addressed as <code>{effectiveSlug}</code>.
              {statuses.some((s) => s.needs_reason) &&
                " Closing into a status that needs a reason will ask for one before it saves."}
            </p>
          </div>
        )}

        {error && (
          <Alert variant="destructive">
            <AlertTriangle className="size-4" />
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}

        <DialogFooter>
          <Button variant="ghost" onClick={onCancel}>
            Cancel
          </Button>
          {step > 0 && (
            <Button variant="outline" onClick={() => setStep((n) => n - 1)}>
              Back
            </Button>
          )}
          {step < STEPS.length - 1 ? (
            <Button disabled={!canAdvance} onClick={() => setStep((n) => n + 1)}>
              Next
            </Button>
          ) : (
            <Button disabled={submitting} onClick={() => void submit()}>
              {submitting ? "Creating…" : "Create list"}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
