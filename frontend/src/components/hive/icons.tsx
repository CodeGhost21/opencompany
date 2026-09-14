/**
 * The icon each deliberation mark takes.
 *
 * A lookup rather than a `lucide` name interpolated at the call site, so the
 * projection in `lib/hive/tone.ts` can stay pure (it names an icon; it does not
 * import one) and every icon this surface can draw is visible in one list.
 */
import {
  CircleHelp,
  FileText,
  Gavel,
  Hand,
  Lightbulb,
  MinusCircle,
  Pin,
  Scale,
  ShieldX,
  ThumbsUp,
  type LucideIcon,
} from "lucide-react";

export const HIVE_ICONS: Record<string, LucideIcon> = {
  propose: Lightbulb,
  support: ThumbsUp,
  object: Hand,
  refute: ShieldX,
  evidence: FileText,
  question: CircleHelp,
  defer: MinusCircle,
  commit: Gavel,
  pin: Pin,
  scale: Scale,
};

/** The icon for a mark name, falling back to the neutral one. */
export function hiveIcon(name: string): LucideIcon {
  return HIVE_ICONS[name] ?? FileText;
}
