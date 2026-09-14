import { EyeOff } from "lucide-react";

import { cn } from "@/lib/utils";

/**
 * The band around the opening round.
 *
 * The single highest-value thing this surface says, because independence is the
 * property a room actually buys over one responder: a shared transcript destroys
 * it — the third speaker has read the first two — and one blind round is the
 * cheapest repair. An operator who cannot see which turns were written blind
 * cannot tell a deliberation from three agents agreeing in sequence.
 *
 * `derived` is load-bearing. While the console is inferring the round from the
 * speaking order rather than reading it off a frame, the band is dotted and says
 * so. A confident band around a guess would be worse than no band.
 */
export function BlindRoundBand({
  derived,
  children,
  className,
}: {
  derived: boolean;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "border-l-2 pl-3",
        derived ? "border-dashed border-border" : "border-solid border-primary/40",
        className,
      )}
    >
      <p className="flex items-center gap-1.5 pb-1 text-2xs text-muted-foreground">
        <EyeOff aria-hidden className="size-3" />
        {derived
          ? "Opening round — each seat appears to have written before reading the others"
          : "Opening round — each seat wrote before reading the others"}
      </p>
      {children}
    </div>
  );
}
