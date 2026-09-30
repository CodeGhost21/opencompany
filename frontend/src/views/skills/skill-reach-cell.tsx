import { useEffect, useRef, useState } from "react";

import type { SkillAgentScope } from "@/api/skills";
import type { TeamMemberDto } from "@/api/types";
import { AgentAvatarButton } from "@/components/agent-profile-sheet";
import { TeammateAvatar } from "@/components/teammate-avatar";
import { avatarFor } from "@/lib/team";

/**
 * Who reads a skill, as faces rather than a count.
 *
 * Adapted from the MCP servers table's cell rather than shared with it, because
 * the two carry different answers. MCP's reach is binary: a teammate reaches a
 * server or it does not. A skill's scope has three stored states, and
 * `inherited` and `excluded` must never be collapsed — both hold nothing while
 * the skill is disabled, and only the first holds it again when the switch
 * returns. The faces here are the teammates that hold it *now*; the ratio
 * beside them is what a stack of faces cannot say, since five faces look the
 * same whether they are five of five or five of nine.
 *
 * The mark is the shipped mascot hashed from the teammate's id, never an
 * uploaded avatar — it must resolve synchronously from a static file, with no
 * fetch per face.
 */
/** One face's own width, and how far the next one is offset into it. */
const FACE = 24;
const PITCH = 18;
/** Room the ratio and the overflow control need beside the stack. */
const TRAILING_WIDTH = 96;

/** The display name for a teammate, falling back to its id. */
export function reachName(id: string, team: TeamMemberDto[] | null): string {
  const member = team?.find((candidate) => candidate.id === id);
  return member?.name?.trim() || member?.role?.trim() || id;
}

/** The teammates that hold this skill right now. */
export function holders(
  agents: SkillAgentScope[] | undefined,
): SkillAgentScope[] {
  return (agents ?? []).filter((agent) => agent.holds);
}

export function SkillReachCell({
  agents,
  team,
  enabled,
  onOverflow,
}: {
  agents: SkillAgentScope[] | undefined;
  team: TeamMemberDto[] | null;
  /** A switched-off skill reaches nobody, whatever the scope stores. */
  enabled: boolean;
  /** Opens this skill, where every teammate is named. */
  onOverflow: () => void;
}) {
  const wrap = useRef<HTMLDivElement | null>(null);
  /**
   * The measured width, or `null` before anything has measured it. `null` shows
   * every face.
   */
  const [width, setWidth] = useState<number | null>(null);

  useEffect(() => {
    const node = wrap.current;
    if (!node || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (entry) setWidth(entry.contentRect.width);
    });
    observer.observe(node);
    return () => observer.disconnect();
  }, []);

  // Not a ratio, so not faces: the two states below are sentences in the list
  // today and stay sentences, because "0 of 6" reads as a scope that excluded
  // everybody rather than as a skill nobody can reach whatever their scope says.
  if (!enabled) {
    return (
      <span
        className="text-xs text-status-blocked-text"
        data-testid="skill-reach"
      >
        Hidden from agents
      </span>
    );
  }
  const total = (agents ?? []).length;
  if (agents === undefined || total === 0) {
    return (
      <span className="text-xs text-muted-foreground" data-testid="skill-reach">
        {agents === undefined
          ? "Available for your agents to read"
          : "No agents to read it"}
      </span>
    );
  }

  const held = holders(agents);
  const fit = fitCount(held.length, width);
  const shown = held.slice(0, fit);
  const hidden = held.length - shown.length;

  return (
    <div
      ref={wrap}
      className="flex min-w-0 items-center gap-1.5"
      data-testid="skill-reach"
    >
      {held.length === 0 ? (
        <span className="text-xs text-muted-foreground">no teammate</span>
      ) : (
        <span className="flex shrink-0 items-center">
          {/* The overlap and the ring live on the tile, not on the button:
              `AgentAvatarButton` renders its children bare where no profile
              panel is mounted, and a stack that fell apart there would be the
              component's documented fallback taking the layout with it. */}
          {shown.map((agent) => (
            <AgentAvatarButton
              key={agent.id}
              agentId={agent.id}
              name={reachName(agent.id, team)}
              className="shrink-0"
            >
              <TeammateAvatar
                name={reachName(agent.id, team)}
                avatar={avatarFor(agent.id)}
                className="-ml-1.5 size-6 ring-2 ring-card"
              />
            </AgentAvatarButton>
          ))}
        </span>
      )}
      {hidden > 0 && (
        <button
          type="button"
          onClick={onOverflow}
          className="shrink-0 rounded-sm text-xs text-muted-foreground underline focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          data-testid="skill-reach-overflow"
        >
          +{hidden}
        </button>
      )}
      <span className="shrink-0 text-xs text-muted-foreground">
        {held.length} of {total}
      </span>
    </div>
  );
}

/**
 * How many faces the cell has room for. An unmeasured cell shows all of them.
 */
export function fitCount(total: number, width: number | null): number {
  if (width === null || width <= 0) return total;
  const all = FACE + PITCH * Math.max(0, total - 1);
  if (all <= width) return total;
  const room = width - TRAILING_WIDTH - FACE;
  return Math.max(1, 1 + Math.floor(room / PITCH));
}
