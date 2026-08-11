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
   * Whether a model designed this team from the answers.
   *
   * `false` is the offline path (no inference credential) and every failure
   * path — a timeout, an unreadable answer — where the curated reference team
   * ships instead. The dialog renders both identically **on purpose**: to the
   * operator they are the same thing, a starting point they can edit, and
   * captioning one "we guessed" would undersell a perfectly good roster.
   */
  generated: boolean;
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
