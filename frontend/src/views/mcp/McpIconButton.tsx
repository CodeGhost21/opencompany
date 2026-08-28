import type { LucideIcon } from "lucide-react";
import { Loader2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

interface Props {
  /** What the control does, in the imperative. The tooltip *and* the accessible name. */
  label: string;
  icon: LucideIcon;
  onClick: () => void;
  disabled?: boolean;
  /** Show a spinner in place of the icon while this control's work is in flight. */
  busy?: boolean;
  /** `primary` for the one control a row is waiting on (a missing credential). */
  tone?: "default" | "primary" | "destructive";
  testId?: string;
}

/**
 * One control on an MCP connection row: an icon, and a tooltip saying what it
 * does.
 *
 * A row carries up to six controls — sign in, rotate a credential, connect,
 * re-probe, list tools, remove — and at labelled-button width they wrapped onto
 * a second line and pushed the row's own information off the screen. Icons fit;
 * icons alone do not *say* anything, which is why the label is mandatory here
 * and is spent twice: as the tooltip for a pointer, and as `aria-label` for a
 * screen reader and for anything driving this page by accessible name. There is
 * no arrangement of this component that produces a control with no name.
 */
export function McpIconButton({
  label,
  icon: Icon,
  onClick,
  disabled,
  busy,
  tone = "default",
  testId,
}: Props) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            variant={tone === "primary" ? "default" : "ghost"}
            size="icon-sm"
            aria-label={label}
            data-testid={testId}
            disabled={disabled}
            onClick={onClick}
            className={tone === "destructive" ? "text-muted-foreground hover:text-destructive" : ""}
          />
        }
      >
        {busy ? <Loader2 className="size-4 animate-spin" /> : <Icon className="size-4" />}
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}
