// The run-output drawer for a synchronous run, and the defensive parse that
// turns the engine's nested state into readable per-node cards (issues #68,
// #154, #170).
//
// Extracted verbatim from `WorkflowsView.tsx` (issue #303).

import { useMemo } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

import type { WorkflowGraph, WorkflowRunNode, WorkflowRunResult } from "@/api/workflows";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";

import { DeliveryRows } from "./RunHistoryPanel";

/** A single agent reply extracted from a node's `items[].json`. */
interface NodeMessage {
  text: string | null;
  agentRef: string | null;
}

/** One node's readable, shape-checked result, ready to render. */
interface NodeResult {
  id: string;
  name: string;
  /** The condition branch taken (`null` when the node isn't a branch point). */
  port: string | null;
  messages: NodeMessage[];
}

/** The run-output drawer: one readable card per executed node (the producing
 * agent and its reply, markdown-rendered) plus the branch each condition node
 * took, any nodes left pending approval, and the raw engine state collapsed
 * behind a toggle. Falls back to a raw JSON dump when the output doesn't match
 * the expected per-node shape. */
export function RunResultPanel({
  result,
  graph,
  request,
  onClose,
}: {
  result: WorkflowRunResult;
  graph: WorkflowGraph | null;
  /** What the operator asked this run for (issue #154); "" when they asked for
   * nothing, in which case the line is omitted rather than showing a bare dash. */
  request: string;
  onClose: () => void;
}) {
  const nodeResults = useMemo(
    () => parseRunNodes(result.output, graph),
    [result.output, graph],
  );
  const deliveries = result.deliveries ?? [];
  const pendingDeliveryCount = deliveries.filter((d) => d.status === "pending").length;
  const undeliveredCount = deliveries.filter(
    (d) => d.status !== "sent" && d.status !== "pending",
  ).length;
  // Issue #542: a test run. The header says so plainly, and the delivery rows
  // (already `skipped`) read as "would have gone to …" rather than as failures.
  const isDry = result.dryRun === true;
  const nodeTimeline = result.nodes ?? [];

  return (
    <div className="border-t bg-card/60" data-testid="workflow-run-result">
      <div className="flex items-center justify-between px-4 py-2">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium">Run result</span>
          {isDry && (
            <Badge
              variant="outline"
              className="border-violet-500/40 bg-violet-500/10"
              data-testid="workflow-run-dry-badge"
            >
              Test run — nothing was sent, no tokens spent
            </Badge>
          )}
          {pendingDeliveryCount > 0 && (
            <Badge variant="outline" className="border-sky-500/40 bg-sky-500/10">
              {pendingDeliveryCount} report{pendingDeliveryCount === 1 ? "" : "s"} awaiting approval
            </Badge>
          )}
          {undeliveredCount > 0 && (
            <Badge variant="outline" className="border-red-500/40 bg-red-500/10">
              {undeliveredCount} report{undeliveredCount === 1 ? "" : "s"} not delivered
            </Badge>
          )}
          {result.pendingApprovals.length > 0 && (
            <Badge variant="outline" className="border-amber-500/40 bg-amber-500/10">
              {result.pendingApprovals.length} pending approval
              {result.pendingApprovals.length === 1 ? "" : "s"}
            </Badge>
          )}
        </div>
        <Button variant="ghost" size="sm" onClick={onClose}>
          Dismiss
        </Button>
      </div>
      <div className="max-h-72 overflow-auto px-4 pb-3">
        {request && (
          <p className="mb-2 text-xs text-muted-foreground">
            <span className="font-medium text-foreground">Requested:</span> {request}
          </p>
        )}
        {result.pendingApprovals.length > 0 && (
          <p className="mb-2 text-xs text-muted-foreground">
            Waiting on: {result.pendingApprovals.join(", ")}
          </p>
        )}

        {deliveries.length > 0 && <DeliveryRows deliveries={deliveries} />}

        {/* Issue #542: the per-node timeline, carried on every synchronous run's
            body. It is most load-bearing for a dry run, which journals nothing —
            this is the only place a test run's step-by-step trail appears, since
            the canvas paints no optimistic frontier for it. */}
        {nodeTimeline.length > 0 && (
          <NodeTimeline nodes={nodeTimeline} graph={graph} />
        )}

        {nodeResults && nodeResults.length > 0 ? (
          <div className="mb-2 space-y-2">
            {nodeResults.map((n) => (
              <NodeResultCard key={n.id} node={n} />
            ))}
          </div>
        ) : (
          <p className="mb-2 text-xs text-muted-foreground">
            The run finished, but its output didn't match the expected node
            shape — see the raw output below.
          </p>
        )}

        <details open={!nodeResults || nodeResults.length === 0}>
          <summary className="cursor-pointer text-xs text-muted-foreground">
            Show raw engine output
          </summary>
          <pre className="mt-1 rounded-lg border bg-muted/40 p-2 font-mono text-[11px] leading-snug">
            {JSON.stringify(result.output, null, 2)}
          </pre>
        </details>
      </div>
    </div>
  );
}

/** One node's readable result: its name, the producing agent, and its reply
 * (markdown-rendered). Falls back to a subtle placeholder / the branch it took
 * when it produced no text (e.g. a trigger or a condition node). */
function NodeResultCard({ node }: { node: NodeResult }) {
  return (
    <div className="rounded-lg border bg-background/40 p-2">
      <div className="mb-1 flex items-center gap-2">
        <span className="truncate text-xs font-medium">{node.name}</span>
        {node.port !== null && (
          <Badge variant="outline" className="h-4 px-1.5 text-[10px] font-normal">
            branch: {node.port}
          </Badge>
        )}
      </div>
      {node.messages.map((m, i) => (
        <div key={i} className={i > 0 ? "mt-2 border-t pt-2" : undefined}>
          {m.agentRef && (
            <p className="mb-1 text-[10px] uppercase tracking-wide text-muted-foreground">
              {m.agentRef}
            </p>
          )}
          {m.text ? (
            <div className="prose prose-sm max-w-none dark:prose-invert">
              <ReactMarkdown remarkPlugins={[remarkGfm]}>{m.text}</ReactMarkdown>
            </div>
          ) : (
            <p className="text-sm text-muted-foreground">—</p>
          )}
        </div>
      ))}
      {node.messages.length === 0 && (
        <p className="text-sm text-muted-foreground">—</p>
      )}
    </div>
  );
}

/** The per-node timeline of a run (issue #542): one row per node that finished,
 * in finish order, showing whether it succeeded and how long it took. Node ids
 * are mapped to their display name from the loaded graph when available. This is
 * a test run's ONLY step-by-step trail, and a real run carries the same rows. */
function NodeTimeline({
  nodes,
  graph,
}: {
  nodes: WorkflowRunNode[];
  graph: WorkflowGraph | null;
}) {
  const nameById = new Map(graph?.nodes.map((n) => [n.id, n.name]) ?? []);
  return (
    <div
      className="mb-3 space-y-1.5 rounded-lg border bg-background/40 p-2"
      data-testid="workflow-run-node-timeline"
    >
      <span className="text-xs font-medium">Steps</span>
      {nodes.map((n, i) => (
        <div key={`${n.nodeId}-${i}`} className="flex flex-wrap items-baseline gap-1.5">
          <Badge
            variant="outline"
            className={`h-4 px-1.5 text-[10px] font-normal ${
              n.status === "error"
                ? "border-red-500/40 bg-red-500/10"
                : "border-emerald-500/40 bg-emerald-500/10"
            }`}
          >
            {n.status}
          </Badge>
          <span className="text-[11px]">{nameById.get(n.nodeId) ?? n.nodeId}</span>
          <span className="text-[11px] text-muted-foreground">{n.elapsedMs} ms</span>
        </div>
      ))}
    </div>
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

/** A non-empty trimmed string, else `null` (defensive against non-strings). */
function nonEmptyString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}

/** Pull a field from an item's `json`, preferring the OUTERMOST value and
 * falling back to the NESTED `json.json.<key>` the engine sometimes emits.
 * Handles the observed shape where `json` carries both a top-level `text` and
 * a nested `json.json.text` — the outer one wins. */
function readNested(json: unknown, key: string): string | null {
  if (!isRecord(json)) return null;
  const outer = nonEmptyString(json[key]);
  if (outer) return outer;
  const inner = json.json;
  if (isRecord(inner)) return nonEmptyString(inner[key]);
  return null;
}

/** Safely parse the engine's run output into per-node results, ordered by the
 * loaded graph when available (falling back to the map's insertion order).
 * Returns `null` when `output` doesn't match the expected `{ nodes: {…} }`
 * shape, signalling the caller to fall back to the raw JSON dump. Every access
 * is guarded — `output` is typed `unknown` and older/edge runs may be a plain
 * string, missing `nodes`, or carry malformed node values. */
function parseRunNodes(
  output: unknown,
  graph: WorkflowGraph | null,
): NodeResult[] | null {
  if (!isRecord(output) || !isRecord(output.nodes)) {
    console.debug(
      "[WorkflowsView] run output missing a `nodes` map; showing raw JSON",
      output,
    );
    return null;
  }
  const nodes = output.nodes;

  // Order by the graph's node order when we have it, then append any node ids
  // present in the output but not in the graph (in the map's insertion order).
  const graphOrder = graph?.nodes.map((n) => n.id) ?? [];
  const orderedIds = [
    ...graphOrder.filter((id) => id in nodes),
    ...Object.keys(nodes).filter((id) => !graphOrder.includes(id)),
  ];

  const nameById = new Map(graph?.nodes.map((n) => [n.id, n.name]) ?? []);

  const results: NodeResult[] = orderedIds.map((id) => {
    const raw = nodes[id];
    const items = isRecord(raw) && Array.isArray(raw.items) ? raw.items : [];
    const messages: NodeMessage[] = items
      .map((item) => {
        const json = isRecord(item) ? item.json : undefined;
        return {
          text: readNested(json, "text"),
          agentRef: readNested(json, "agent_ref"),
        };
      })
      .filter((m) => m.text || m.agentRef);
    const port = isRecord(raw) ? nonEmptyString(raw.port) : null;
    return { id, name: nameById.get(id) ?? id, port, messages };
  });

  return results;
}
