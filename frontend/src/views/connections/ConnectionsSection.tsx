import type { OpenCompanyClient } from "@/api/client";
import { resolveConnectionPage } from "@/views/connection-pages";
import { McpServersView } from "@/views/McpServersView";
import { OAuthView } from "@/views/OAuthView";

interface Props {
  client: OpenCompanyClient;
  company: string | null;
  /** The hash's second segment, e.g. `mcp` in `#/connections/mcp`. */
  sub: string | null;
}

/**
 * Connections, as a section rather than two settings tabs.
 *
 * # Why this replaced Settings → OAuth and Settings → MCP Servers
 *
 * The same argument `docs/spec/runtime/finance-console.md` makes about Billing.
 * Settings is where an operator changes how the company is configured — a place
 * they visit once, on the way to something else. Which apps the company can act
 * through, and which tool servers its teammates can call, is not that: it is
 * read repeatedly, it changes as the company's work changes, and an operator
 * arrives at it asking "can my teammates do X yet?" rather than "what is this
 * company's configuration?". Two clicks down a settings rail is the wrong depth
 * for a question asked that often.
 *
 * # This is not a revert of the Connections split
 *
 * A single "Connections" **page** once carried five subjects and was broken
 * apart on purpose (see the comment above the `inference` entry in
 * `settings-pages.ts`). Nothing here puts them back on one page: Apps and MCP
 * Servers are still two pages answering one question each. What they gain is a
 * parent, which is what the original split had no room to give them — and the
 * three credential forms that argument also covers (Inference, Hosting, Search)
 * deliberately stayed in Settings, beside the things they unlock.
 *
 * # Where the rail went
 *
 * This section shipped with a 240px sub-rail inside the content area, modelled
 * on `finance/FinanceSection.tsx`. That rail is gone: sub-navigation lives in
 * the **sidebar** now, under the section's own row
 * (`components/sidebar-navigation.tsx`). Two reasons, and neither is layout
 * fashion. A rail inside the page puts the same kind of list in two different
 * places depending on which section an operator is in — the sidebar for the
 * sections without sub-pages, a rail for the ones with — so there is no rule to
 * learn. And it charges the content pane 240px on every page under it, on a
 * screen that already has a sidebar.
 *
 * What is left is the dispatch, which is all this component ever did besides
 * draw the rail. `OAuthView` and `McpServersView` are re-parented, not
 * rewritten. The one content change is `OAuthView`'s title: the page is called
 * **Apps** now, because "OAuth" names the protocol a connection happens to use
 * rather than the thing an operator came to find, and under a section already
 * named Connections it said the same word twice.
 */
export function ConnectionsSection({ client, company, sub }: Props) {
  const page = resolveConnectionPage(sub);

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      {page === "apps" && <OAuthView client={client} company={company} />}
      {page === "mcp" && <McpServersView client={client} company={company} />}
    </div>
  );
}
