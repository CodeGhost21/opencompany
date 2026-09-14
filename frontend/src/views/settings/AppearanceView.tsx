// Light, dark, or follow the system.
//
// A row on the rail rather than a card most of the way down General, and the
// reason is what it belongs to. Every other thing on General is a fact about
// the *company* — its connection, its domain, its mail, its lifecycle — and is
// the same for everyone who signs in. The theme is a fact about **this
// browser**: it is stored per client, changing it changes nothing for anybody
// else, and it is the one control on that page an operator goes looking for by
// name rather than meeting on the way past.
//
// One card on its own page reads thin, and that is acceptable here: the rail is
// a list of subjects, and "what this console looks like" is a subject even when
// the answer to it is a single three-way toggle. A page that grows a second
// control — density, motion, an accent — has somewhere to put it that is not
// the bottom of General.

import { PageHeader } from "@/components/page-header";
import { ThemeToggle } from "@/components/theme-toggle";
import {
  Card,
  CardAction,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";

export function AppearanceView() {
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <PageHeader title="Appearance" width="full" />
      <div className="min-h-0 w-full flex-1 space-y-6 overflow-y-auto px-4 py-6">
        {/* The trailing control goes in `CardAction`, not a bare child:
            `CardHeader` is a grid, so `flex-row justify-between` on it is inert
            and the control drops onto a row of its own below the description.
            `CardAction` is what switches the header to `grid-cols-[1fr_auto]`
            and parks the control at the top right. */}
        <Card>
          <CardHeader>
            <CardTitle className="text-base">Theme</CardTitle>
            <CardDescription>Switch between light, dark, and system themes.</CardDescription>
            <CardAction>
              <ThemeToggle />
            </CardAction>
          </CardHeader>
        </Card>
      </div>
    </div>
  );
}
