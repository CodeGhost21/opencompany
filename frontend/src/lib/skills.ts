// Skill presentation data for the console: per-category badge styling.
//
// Both the company's effective skills and the installable shared registry come
// from the host over the `…/skills` API (`@/api/skills`). Nothing about *which*
// skills exist lives on the client — a hardcoded registry array used to live
// here, and it had already drifted from what the backend could actually serve.

export type SkillCategory = "Marketing" | "Research" | "Ops" | "Content" | "Finance";

/**
 * One tint per category — identity, not state.
 *
 * The identity palette (`--tone-*`), not the status one: a skill filed under
 * Content is not "done", and one under Finance is not "failed", which is
 * exactly what the emerald and rose these replaced were saying.
 */
export const CATEGORY_STYLES: Record<SkillCategory, string> = {
  Marketing: "border-tone-1/30 bg-tone-1/10 text-tone-1-text",
  Research: "border-tone-2/30 bg-tone-2/10 text-tone-2-text",
  Ops: "border-tone-5/30 bg-tone-5/10 text-tone-5-text",
  Content: "border-tone-3/30 bg-tone-3/10 text-tone-3-text",
  Finance: "border-tone-4/30 bg-tone-4/10 text-tone-4-text",
};
