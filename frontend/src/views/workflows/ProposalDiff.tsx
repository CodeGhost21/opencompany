// The reviewable diff, shared by the copilot's proposal card (#415) and the
// task's In-Review workflow panel (#580).
//
// This is the list `ProposalCard` used to render inline, lifted out verbatim so
// both surfaces show a change the same way — step by step and connection by
// connection, computed from the candidate graph rather than a model's op list —
// without either forking the renderer. A change nobody proposed cannot appear
// in it, and an op that changes nothing shows as nothing. It renders nothing but
// the diff: the shell, the buttons and the settled/blocked states stay with
// whichever surface owns them.

import { ArrowRight, Minus, Plus } from "lucide-react";

import type { GraphDiff, NodeChange } from "@/api/workflow-proposal";

export function ProposalDiff({ diff }: { diff: GraphDiff }) {
  return (
    <ul className="mt-2 space-y-1">
      {diff.nodes.map((node) => (
        <li key={`n-${node.id}`}>
          <NodeLine node={node} />
        </li>
      ))}
      {diff.edges.map((edge) => (
        <li
          key={`e-${edge.change}-${edge.from}-${edge.to}`}
          className="flex items-center gap-1 text-muted-foreground"
        >
          {edge.change === "added" ? (
            <Plus className="size-3 shrink-0 text-emerald-600 dark:text-emerald-400" />
          ) : (
            <Minus className="size-3 shrink-0 text-destructive" />
          )}
          <span className="font-mono">{edge.from}</span>
          <ArrowRight className="size-3 shrink-0" aria-hidden />
          <span className="font-mono">{edge.to}</span>
        </li>
      ))}
      {diff.nodes.length === 0 && diff.edges.length === 0 && (
        <li className="text-muted-foreground">
          This would change nothing about the workflow as it stands.
        </li>
      )}
    </ul>
  );
}

/** One step's line: added, removed, or changed with the fields that moved. */
function NodeLine({ node }: { node: NodeChange }) {
  if (node.change === "added") {
    return (
      <span className="block text-muted-foreground">
        <span className="flex items-start gap-1">
          <Plus className="mt-px size-3 shrink-0 text-emerald-600 dark:text-emerald-400" />
          <span>
            new step <span className="font-mono">{node.id}</span>{" "}
            <span className="text-foreground">{node.name}</span>
          </span>
        </span>
        {/* A new step's whole payload, not just its name: its kind, and any
            schedule, config or approval flag it arrives with, each of which
            changes what the workflow does. Reviewing a name and applying a
            scheduled auto-approving node is the gap this closes. */}
        <span className="mt-0.5 ml-4 block space-y-0.5">
          {node.fields.map((field) => (
            <span key={field.field} className="block">
              <span className="font-medium text-foreground">{field.field}</span>:{" "}
              <span className="text-foreground">{field.after}</span>
            </span>
          ))}
        </span>
      </span>
    );
  }
  if (node.change === "removed") {
    return (
      <span className="flex items-start gap-1 text-muted-foreground">
        <Minus className="mt-px size-3 shrink-0 text-destructive" />
        <span>
          remove <span className="font-mono">{node.id}</span>{" "}
          <span className="text-foreground">{node.name}</span>
        </span>
      </span>
    );
  }
  return (
    <span className="block text-muted-foreground">
      <span className="flex items-start gap-1">
        <ArrowRight className="mt-px size-3 shrink-0" aria-hidden />
        <span>
          <span className="font-mono">{node.id}</span>{" "}
          <span className="text-foreground">{node.name}</span>
        </span>
      </span>
      <span className="mt-0.5 ml-4 block space-y-0.5">
        {node.fields.map((field) => (
          <span key={field.field} className="block">
            <span className="font-medium text-foreground">{field.field}</span>:{" "}
            <span className="line-through">{field.before}</span>{" "}
            <ArrowRight className="inline size-3 align-[-2px]" aria-hidden />{" "}
            <span className="text-foreground">{field.after}</span>
          </span>
        ))}
      </span>
    </span>
  );
}
