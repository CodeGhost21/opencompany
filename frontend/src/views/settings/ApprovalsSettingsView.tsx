// The autonomy tier and the always-ask list, as a page of their own.
//
// This was the second card down on General, directly under the harness list,
// on the reasoning that "an operator who comes to settings because they are
// drowning in approval cards is here for this". That reasoning is what retires
// it: if it is the thing an operator most often arrives at Settings *for*, then
// being high on a page they have to scroll is a worse answer than being a row
// they can hit from the rail.
//
// It is also the only thing General carried that is a *policy* — a standing
// rule about what teammates may do unattended — rather than a fact about this
// console's connection or this company's identity. General is a page about how
// the company is set up; this is a page about how much of itself it is allowed
// to run.
//
// Not to be confused with `#/approvals`, the sidebar row, which is the queue of
// requests waiting on a decision right now. That is the work; this is the rule
// that decides what reaches it.

import { PageHeader } from "@/components/page-header";
import { PolicySettings } from "@/components/policy-settings";
import type { OpenCompanyClient } from "@/api/client";
import { useCanManagePolicy } from "@/hooks/use-can-manage";

interface Props {
  client: OpenCompanyClient;
  company: string | null;
}

export function ApprovalsSettingsView({ client, company }: Props) {
  // The same hook General read, called here for the same reason: both write
  // routes behind these controls call `require_admin`, so a member gets the
  // read-only rendering rather than a control that 401s on save.
  const canManage = useCanManagePolicy(client, company);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <PageHeader title="Approvals" width="full" />
      <div className="min-h-0 w-full flex-1 space-y-6 overflow-y-auto px-4 py-6">
        <PolicySettings client={client} company={company} canManage={canManage} />
      </div>
    </div>
  );
}
