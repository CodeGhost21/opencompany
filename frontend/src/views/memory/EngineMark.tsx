import { cn } from "@/lib/utils";

/**
 * The mark on an engine tile.
 *
 * Drawn here as inline SVG rather than fetched, for two reasons that both
 * matter on this page. A self-hosted console is routinely offline or behind a
 * proxy that blocks third-party image hosts, and the Connections grid's remote
 * logos already fall back to a grey initial there — acceptable for a provider
 * you are about to authenticate with, wrong for the control that decides where
 * a company's memory lives. And these are neutral geometric marks, not
 * anyone's trademark: the console names the engine in text beside the mark,
 * which is what an operator picks by.
 *
 * ## Tinted from the identity palette, not from brand hexes
 *
 * Each vendor has a brand colour and none of them is used here. A hex cannot
 * theme (`scripts/ci/assert-design-tokens.sh`), and Supermemory's near-black
 * would be a mark you cannot see on this console's dark canvas. The `--tone-*`
 * ramp is what the rest of the product distinguishes *identities* with, so the
 * marks distinguish engines the same way: different, legible in both themes,
 * and re-tuned whenever the palette is.
 */

/** The identity tone per engine; the tile tints its mark with this. */
const ENGINE_TONES: Record<string, string> = {
  store: "text-tone-5-text",
  embedded: "text-tone-3-text",
  namespace: "text-tone-1-text",
  supermemory: "text-foreground",
  mem0: "text-tone-2-text",
  cognee: "text-tone-4-text",
  null: "text-muted-foreground",
};

/** The mark for `engine`, sized to the tile's 32px slot. */
export function EngineMark({ engine, className }: { engine: string; className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn(ENGINE_TONES[engine] ?? "text-muted-foreground", className)}
      data-testid={`engine-mark-${engine}`}
    >
      <svg viewBox="0 0 24 24" className="size-8" fill="none" stroke="currentColor" strokeWidth="1.75">
        <Glyph engine={engine} />
      </svg>
    </span>
  );
}

/**
 * One glyph per engine, each saying something about what the engine *is*:
 * a drum for the built-in store, a cell for the in-pod engine, nested frames
 * for the namespaced contract store, a cloud for each hosted service, and an
 * open circle with a slash for the engine that keeps nothing.
 */
function Glyph({ engine }: { engine: string }) {
  switch (engine) {
    case "store":
      return (
        <>
          <ellipse cx="12" cy="6" rx="7" ry="3" />
          <path d="M5 6v12c0 1.7 3.1 3 7 3s7-1.3 7-3V6" />
          <path d="M5 12c0 1.7 3.1 3 7 3s7-1.3 7-3" />
        </>
      );
    case "embedded":
      return (
        <>
          <circle cx="12" cy="12" r="3" />
          <path d="M12 3v6M12 15v6M3 12h6M15 12h6" />
          <path d="M6.2 6.2l3 3M14.8 14.8l3 3M17.8 6.2l-3 3M9.2 14.8l-3 3" />
        </>
      );
    case "namespace":
      return (
        <>
          <rect x="3" y="3" width="18" height="18" rx="3" />
          <rect x="7.5" y="7.5" width="9" height="9" rx="1.5" />
        </>
      );
    case "supermemory":
      return (
        <>
          <path d="M7 18a4 4 0 0 1 0-8 5.5 5.5 0 0 1 10.6-1.4A3.9 3.9 0 0 1 18 18Z" />
          <path d="M12 10.5v4M10 12.5h4" />
        </>
      );
    case "mem0":
      return (
        <>
          <path d="M7 18a4 4 0 0 1 0-8 5.5 5.5 0 0 1 10.6-1.4A3.9 3.9 0 0 1 18 18Z" />
          <circle cx="12" cy="13.5" r="1.6" />
        </>
      );
    case "cognee":
      return (
        <>
          <circle cx="6" cy="8" r="2.2" />
          <circle cx="18" cy="7" r="2.2" />
          <circle cx="12" cy="17" r="2.2" />
          <path d="M7.8 9.3 10.6 15M16.6 9 13.3 15.4M8.1 7.6h7.7" />
        </>
      );
    case "null":
      return (
        <>
          <circle cx="12" cy="12" r="8" strokeDasharray="3 3" />
          <path d="M7 17 17 7" />
        </>
      );
    default:
      return <circle cx="12" cy="12" r="8" strokeDasharray="3 3" />;
  }
}
