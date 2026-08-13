import { lazy, Suspense, useEffect, useMemo, useState } from "react";
import { TriangleAlert } from "lucide-react";

import { listPeople, type Person } from "@/api/auth";
import type { OpenCompanyClient } from "@/api/client";
import { discoverMcpTools, listMcpServers } from "@/api/mcp";
import { listMemory, type MemoryEntry } from "@/api/memory";
import { listSkills, type Skill } from "@/api/skills";
import { listTasks, type Task } from "@/api/tasks";
import { ApiError, type DeskDto, type McpServer, type McpToolInfo } from "@/api/types";
import { fromDto, type TeamMember } from "@/lib/team";
import { adapt, buildMemoryGraph } from "./overview/kg/adapter";
import { buildKnowledgeGraph } from "./overview/kg/model";
import { ownedBy } from "./overview/pulse";

// The graph carries the force simulation and every detail card with it. Its own
// chunk means a cold load paints the frame before the physics arrives.
const KnowledgeGraph = lazy(() =>
  import("./overview/kg/KnowledgeGraph").then((m) => ({ default: m.KnowledgeGraph })),
);

interface Props {
  client: OpenCompanyClient;
  company: string | null;
}

/** Everything the graph is drawn from, fetched once per company. */
interface Sources {
  tasks: Task[];
  team: TeamMember[];
  /**
   * The company's desks — ring 1 (issue #486).
   *
   * Best-effort like every other source here. A host that cannot serve them
   * draws a graph with no pillars, which is the same picture as a company that
   * declares no desks. The org chart treats a failed `/desks` as a hard error
   * because desks *are* that page; here they are one ring of five, and failing
   * the whole graph over them would take the real rings down with them.
   */
  desks: DeskDto[];
  people: Person[];
  skills: Skill[];
  memories: MemoryEntry[];
  servers: McpServer[];
  /** Server **name** → the tools it advertises. `name` is this API's server key. */
  toolsByServer: Record<string, McpToolInfo[]>;
  /**
   * The MCP read failed for a reason that is not "this host has no MCP".
   *
   * Kept apart from an empty `servers` on purpose. A company with no tool
   * servers and a company whose tool servers we could not reach draw the same
   * toolless graph, and only one of those is a true statement about the
   * company — collapsing them is how an outage comes to look like an empty
   * configuration. The notice below is the difference.
   */
  mcpUnreadable: boolean;
}

const EMPTY: Sources = {
  tasks: [],
  // Empty, not a fabricated roster. This is the pre-load state, and seeding it
  // with twelve invented agents drew a full org graph for a company that has
  // nobody in it (`docs/spec/runtime/company-setup.md`).
  team: [],
  desks: [],
  people: [],
  skills: [],
  memories: [],
  servers: [],
  toolsByServer: {},
  mcpUnreadable: false,
};

/**
 * Whether a server is worth asking for its tool list.
 *
 * Tool discovery opens a live connection to the remote server, so this skips
 * the ones already known not to answer: disabled, or last probed as broken /
 * missing its credential. A server never probed is asked once — we cannot know
 * it works without trying, and refusing to ask would leave a working server's
 * tools off the graph forever.
 */
function worthAsking(server: McpServer): boolean {
  if (!server.enabled) return false;
  const status = server.health?.status;
  return status !== "error" && status !== "needs_config";
}

/**
 * The command centre: the company's knowledge graph, and nothing else.
 *
 * The page is the graph — no header, no strip, no top bar (the shell hides its
 * own for this view). The company sits at the core, its desks are the pillars,
 * the jobs hang off each pillar, the teammate who does each job sits above it,
 * and their tools are the outer ring.
 *
 * The pillars are **declared**: they are the company's own desks (issue #486),
 * not the keyword-matched guess that used to stand there. What is still derived
 * is the tool assignments and the workflow templates — there is no per-agent
 * tool list and no flow API. See `DERIVED_NOTICE` in `kg/adapter.ts`; it is the
 * standing caveat on what remains.
 */
export function Overview({ client, company }: Props) {
  const [sources, setSources] = useState<Sources>(EMPTY);

  useEffect(() => {
    let live = true;
    void (async () => {
      const [tasks, roster, desks, people, skills, memories, mcp] = await Promise.all([
        listTasks(client, company).catch(() => [] as Task[]),
        client.listTeam(company).catch(() => null),
        // Ring 1 (issue #486). Best-effort: see `Sources.desks`.
        client.listDesks(company).catch(() => [] as DeskDto[]),
        // Only an admin may list people; a member just gets no humans on the
        // graph, which is the right amount of information for them to have.
        listPeople(client, company).catch(() => [] as Person[]),
        listSkills(client, company).catch(() => [] as Skill[]),
        // The company's real durable memory (issue #36). A host without the
        // surface draws no constellation rather than a seeded one — the graph
        // must never claim the company remembers something it doesn't.
        listMemory(client, company).catch(() => [] as MemoryEntry[]),
        // `.../mcp/servers` answers with a bare array of servers. It used to be
        // read through a client helper that promised a `{ servers }` wrapper no
        // host has ever sent, so `mcp.servers` was `undefined` and filtering it
        // threw — on every host that mounts the route, which is all of them
        // (issue #407). The wrapper only looked survivable because the failure
        // path handed back a hand-built `{ servers: [] }`: the tab rendered
        // exactly when MCP was unreachable, and threw when it worked.
        listMcpServers(client, company).then(
          (servers) => ({ servers: servers ?? [], unreadable: false }),
          (error: unknown) => ({
            servers: [] as McpServer[],
            // A host with no MCP surface answers 404. That is a company with no
            // tool servers — a fact, not a failure, and not worth a warning.
            // Anything else (offline, 5xx, a body we couldn't parse) means we
            // genuinely do not know, which is what the notice says.
            unreadable: !(error instanceof ApiError && error.status === 404),
          }),
        ),
      ]);
      if (!live) return;

      const toolLists = await Promise.all(
        mcp.servers.filter(worthAsking).map((s) =>
          discoverMcpTools(client, company, s.name)
            .then((tools) => [s.name, tools ?? []] as const)
            // A host built without the MCP client answers `not_wired`, and a
            // server can simply be down. Neither is worth failing the graph
            // over: that server contributes no tools and the rest still draw.
            .catch(() => [s.name, [] as McpToolInfo[]] as const),
        ),
      );
      if (!live) return;

      setSources({
        tasks,
        // The host's roster, or nobody. Never a fabricated stand-in: an operator
        // reading this graph is reading who works here, and inventing twelve
        // agents made an unstaffed company look busy.
        team: roster?.length ? roster.map(fromDto) : [],
        desks,
        people,
        skills,
        memories,
        servers: mcp.servers,
        toolsByServer: Object.fromEntries(toolLists),
        mcpUnreadable: mcp.unreadable,
      });
    })();
    return () => {
      live = false;
    };
  }, [client, company]);

  const adapted = useMemo(
    () =>
      adapt({
        members: sources.team,
        desks: sources.desks,
        tasks: sources.tasks,
        people: sources.people,
        skills: sources.skills,
        servers: sources.servers,
        toolsByServer: sources.toolsByServer,
        ownedBy,
      }),
    [sources],
  );

  const graph = useMemo(
    () =>
      buildKnowledgeGraph(
        adapted.agents,
        adapted.departments,
        adapted.people,
        adapted.tasks,
        adapted.workflows,
      ),
    [adapted],
  );

  const memoryGraph = useMemo(() => buildMemoryGraph(sources.memories), [sources.memories]);

  return (
    // The whole viewport: the shell hides its top bar for this view, so there
    // is nothing above to subtract.
    <div
      className="oc-kg relative h-svh min-h-0 w-full min-w-0 overflow-hidden"
      // The guided tour's Overview stop anchors here. It used to spotlight the
      // quick-action row this page had before it became the graph; the graph is
      // the page now, so the graph is what gets spotlighted.
      data-tour="overview-graph"
    >
      {/* An unreadable MCP surface and a company with no tool servers draw the
          same toolless graph. Saying which one this is turns "this company has
          no tools" from an assertion the page cannot support into a question
          the operator knows to go and check. */}
      {sources.mcpUnreadable && (
        <p
          role="status"
          className="pointer-events-none absolute left-1/2 top-3 z-10 flex max-w-[calc(100%-1.5rem)] -translate-x-1/2 items-center gap-1.5 rounded-md border bg-background/90 px-2.5 py-1.5 text-xs text-muted-foreground shadow-sm backdrop-blur"
        >
          <TriangleAlert className="size-3.5 shrink-0" aria-hidden />
          <span className="min-w-0 truncate">
            Couldn&apos;t read this company&apos;s MCP tool servers — tools may be missing from
            the graph.
          </span>
        </p>
      )}
      <Suspense
        fallback={
          <div className="grid h-full place-items-center text-sm text-muted-foreground">
            Drawing the graph…
          </div>
        }
      >
        <KnowledgeGraph
          graph={graph}
          agents={adapted.agents}
          departments={adapted.departments}
          people={adapted.people}
          tasks={adapted.tasks}
          memory={memoryGraph}
          toolLabels={adapted.toolLabels}
        />
      </Suspense>
    </div>
  );
}
