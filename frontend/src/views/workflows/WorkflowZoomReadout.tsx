// The current canvas scale, kept with React Flow's built-in controls (issue
// #1370). A fit can open below 100%, so the icon controls alone do not tell an
// operator how far from actual size they are.

import { useViewport } from "@xyflow/react";

export function WorkflowZoomReadout() {
  const { zoom } = useViewport();
  const percent = Math.round(zoom * 100);

  return (
    <output
      className="flex h-[26px] items-center justify-center bg-card px-1.5 font-mono text-2xs text-muted-foreground"
      data-testid="workflow-zoom-readout"
      aria-label="Canvas zoom"
    >
      {percent}%
    </output>
  );
}
