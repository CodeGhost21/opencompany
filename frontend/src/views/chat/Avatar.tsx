import { Building2 } from "lucide-react";

import { TEAM_TONES } from "@/lib/team";
import { cn } from "@/lib/utils";
import { initials } from "./model";

interface Props {
  name: string;
  tone?: string;
  /** The company's own voice wears the brand mark rather than initials. */
  company?: boolean;
  /**
   * Draw the tone tile without initials.
   *
   * For decorative stacks small enough that no glyph can be read at a size
   * the tile can hold: a 16px facepile fits two letters only below 10px,
   * and below 10px is not a size, it is a bug. The tile's colour still
   * distinguishes one voice from the next, which is the whole of what a
   * facepile claims to say.
   */
  markOnly?: boolean;
  className?: string;
}

/**
 * A square-ish chat avatar: initials on a tone-tinted tile.
 *
 * Rounded rather than circular, which is what distinguishes a workspace
 * avatar from a contact-list one — DM rows, message gutters, and the member
 * pane all draw the same tile at different sizes.
 */
export function Avatar({ name, tone, company, markOnly, className }: Props) {
  if (company) {
    return (
      <span
        className={cn(
          "flex shrink-0 items-center justify-center rounded-md bg-primary text-primary-foreground",
          className,
        )}
        aria-hidden
      >
        <Building2 className="size-1/2" />
      </span>
    );
  }

  return (
    <span
      className={cn(
        "flex shrink-0 items-center justify-center rounded-md text-xs font-semibold",
        toneClass(tone),
        className,
      )}
      aria-hidden
    >
      {markOnly ? null : initials(name)}
    </span>
  );
}

/** Fall back to a hashed tone so an unnamed voice still gets a stable color. */
function toneClass(tone?: string): string {
  if (tone && TEAM_TONES[tone]) return TEAM_TONES[tone];
  const keys = Object.keys(TEAM_TONES);
  let hash = 0;
  const seed = tone ?? "";
  for (let i = 0; i < seed.length; i++) hash = (hash * 31 + seed.charCodeAt(i)) | 0;
  return TEAM_TONES[keys[Math.abs(hash) % keys.length]];
}
