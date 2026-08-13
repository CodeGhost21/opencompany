// First-run company setup: the one host call the flow needs
// (docs/spec/runtime/company-setup.md).
//
// `POST {scope}/setup/roster` returns four to six agents to create. It creates
// nothing itself — the build-out step turns each row into
// `client.addTeamMember`, the same `POST {scope}/team` an operator's own "Define
// an agent" uses. So a teammate setup made is byte-identical to one added by
// hand, and the build-out screen gets to reveal each name as its write lands
// rather than waiting on one opaque call.

import type { OpenCompanyClient } from "./client";
import type { SetupDraft } from "@/lib/company-setup";

/** One agent the host proposes. Shaped to pass straight to `addTeamMember`. */
export interface ProposedAgent {
  name: string;
  role: string;
  description: string;
}

export interface RosterProposal {
  agents: ProposedAgent[];
  /** Which reference team framed the proposal, e.g. `ecommerce`. */
  template: string;
  /**
   * Who wrote this team.
   *
   * `"model"` — designed from the operator's own answers.
   * `"fallback"` — the curated team for this kind of business, shipped whole
   * because no model was reachable, its answer could not be read, or what came
   * back was too thin to be a company.
   *
   * **The dialog says which, and an earlier version did not.** Rendering both
   * identically was defended as "to the operator they are the same thing — a
   * starting point they can edit". That is wrong in the direction that costs
   * trust: someone shown a canned team with no indication assumes a model read
   * their answers and wrote it, and judges the product on a roster it never
   * produced.
   */
  source: "model" | "fallback";
}

/**
 * Ask the host for a starting team.
 *
 * Never throws for a *business* reason — the host answers with the reference
 * team rather than an error when it cannot reach a model, because stranding
 * someone on the setup screen is worse than an imperfect roster. A rejection
 * here is a genuine transport or auth failure, which the caller surfaces as
 * "we couldn't reach your company".
 */
export function proposeRoster(
  client: OpenCompanyClient,
  company: string | null,
  draft: SetupDraft,
): Promise<RosterProposal> {
  return client.post<RosterProposal>(`${client.scopeFor(company)}/setup/roster`, {
    industry: draft.industry.trim(),
    teamHint: draft.teamHint.trim(),
    automate: draft.automate.trim(),
  });
}
