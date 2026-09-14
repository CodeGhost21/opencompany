import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";

/**
 * A knob that is either the operator's number or the one the runtime derives.
 *
 * The distinction has to be visible, and it is not cosmetic: a **declared** `9`
 * that happens to equal the **derived** `9` behaves differently the moment
 * somebody joins the desk. Every default here is a function of the membership,
 * because the membership is the only thing the runtime reliably knows — so a
 * console that showed one number could not tell an operator whether adding a
 * seat would move it.
 *
 * Derived is the resting state and shows the formula. Overriding is deliberate,
 * and reversible in one click.
 */
export function DerivedNumber({
  label,
  hint,
  declared,
  derived,
  formula,
  min = 1,
  disabled,
  onChange,
}: {
  label: string;
  hint: string;
  /** The operator's number, or `undefined` when the runtime derives it. */
  declared: number | undefined;
  derived: number;
  /** How the derived value is computed, in the operator's terms. */
  formula: string;
  min?: number;
  disabled?: boolean;
  onChange: (next: number | undefined) => void;
}) {
  const overridden = declared !== undefined;
  // Zero is refused rather than clamped, in the host's own terms: an operator
  // who wrote a number meant it, and silently substituting a different one is
  // how a desk behaves in a way its manifest does not describe.
  const invalid = overridden && (!Number.isFinite(declared) || declared < min);

  return (
    <div className="space-y-1">
      <div className="flex items-baseline justify-between gap-2">
        <label className="text-sm font-medium">{label}</label>
        {overridden ? (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-auto px-1 py-0 text-2xs"
            disabled={disabled}
            onClick={() => onChange(undefined)}
          >
            back to derived
          </Button>
        ) : null}
      </div>

      {overridden ? (
        <Input
          type="number"
          min={min}
          value={String(declared)}
          disabled={disabled}
          aria-invalid={invalid}
          className={cn("h-8", invalid && "border-status-failed")}
          onChange={(e) => {
            const raw = e.target.value;
            const next = raw === "" ? undefined : Number(raw);
            onChange(next === undefined || Number.isFinite(next) ? next : undefined);
          }}
        />
      ) : (
        <button
          type="button"
          disabled={disabled}
          onClick={() => onChange(derived)}
          className="flex h-8 w-full items-center justify-between rounded-md border border-dashed border-border px-2 text-left text-sm text-muted-foreground hover:bg-accent"
        >
          <span>
            <span className="text-foreground">{derived}</span>{" "}
            <span className="text-2xs">({formula})</span>
          </span>
          <span className="text-2xs">override</span>
        </button>
      )}

      <p className="text-2xs text-muted-foreground">
        {invalid ? `Must be at least ${min}. ${hint}` : hint}
      </p>
    </div>
  );
}
