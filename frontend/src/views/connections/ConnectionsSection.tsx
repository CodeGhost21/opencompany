import type { OpenCompanyClient } from "@/api/client";
import { cn } from "@/lib/utils";
import {
  CONNECTION_PAGES,
  type ConnectionPage,
  resolveConnectionPage,
} from "@/views/connection-pages";
import { McpServersView } from "@/views/McpServersView";
import { OAuthView } from "@/views/OAuthView";

interface Props {
  client: OpenCompanyClient;
  company: string | null;
  /** The hash's second segment, e.g. `mcp` in `#/connections/mcp`. */
  sub: string | null;
  onNavigate: (page: ConnectionPage) => void;
}

/**
 * Connections, as a section rather than two settings tabs.
 *
 * Modelled on `finance/FinanceSection.tsx`, deliberately and down to the
 * breakpoint: a `w-60` sub-rail on `sm:` and up, a scrolling chip row below it,
 * and each sub-page its own route (`#/connections/mcp`) so it is linkable and
 * survives a refresh exactly as a top-level view does.
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
 * apart on purpose (see the comment above the `oauth` entry in
 * `settings-pages.ts`). Nothing here puts them back on one page: Apps and MCP
 * Servers are still two pages answering one question each. What they gain is a
 * parent, which is what the original split had no room to give them — and the
 * three credential forms that argument also covers (Inference, Hosting, Search)
 * deliberately stayed in Settings, beside the things they unlock.
 *
 * # Only the parent is new
 *
 * `OAuthView` and `McpServersView` are re-parented, not rewritten. The one
 * content change is `OAuthView`'s title: the page is called **Apps** now,
 * because "OAuth" names the protocol a connection happens to use rather than
 * the thing an operator came to find, and under a section already named
 * Connections it said the same word twice.
 */
export function ConnectionsSection({ client, company, sub, onNavigate }: Props) {
  const page = resolveConnectionPage(sub);

  return (
    <div className="flex min-h-0 flex-1">
      <nav
        aria-label="Connections"
        className="hidden w-60 shrink-0 flex-col gap-0.5 overflow-y-auto border-r p-3 sm:flex"
      >
        {/* A visual caption for the rail, not a heading. The `nav` is already
            named by its `aria-label`, so an `h2` here adds nothing for a screen
            reader and breaks the document outline: the rail renders before the
            sub-page, so heading navigation would meet a section-level heading
            ahead of the page's own `h1` (issue #1392). */}
        <div className="px-2 pb-2 pt-1 text-xs font-medium text-muted-foreground">
          Connections
        </div>
        {CONNECTION_PAGES.map((item) => (
          <button
            key={item.id}
            type="button"
            onClick={() => onNavigate(item.id)}
            aria-current={page === item.id ? "page" : undefined}
            className={cn(
              "flex items-start gap-2.5 rounded-lg px-2 py-2 text-left transition-colors",
              page === item.id ? "bg-accent text-accent-foreground" : "hover:bg-accent/50",
            )}
          >
            <item.icon className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
            <span className="min-w-0">
              <span className="block text-sm font-medium">{item.label}</span>
              <span className="block text-xs text-muted-foreground">{item.hint}</span>
            </span>
          </button>
        ))}
      </nav>

      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        {/* On the macOS desktop `ContentSurface` overlays every page's top 28px
            with a pointer-events-enabled drag band (`WindowDragBar`, z-20), and
            this row is the one page top that sits in it below `sm`. `relative
            z-30` gives the chips their own stacking context above that band —
            the same fix `SettingsSection`'s chip row carries, for the same
            reason (issue #1383's neighbour). */}
        <div className="relative z-30 flex gap-1 overflow-x-auto border-b p-2 sm:hidden">
          {CONNECTION_PAGES.map((item) => (
            <button
              key={item.id}
              type="button"
              onClick={() => onNavigate(item.id)}
              aria-current={page === item.id ? "page" : undefined}
              className={cn(
                "shrink-0 rounded-full px-3 py-1 text-xs font-medium transition-colors",
                page === item.id ? "bg-accent text-accent-foreground" : "text-muted-foreground",
              )}
            >
              {item.label}
            </button>
          ))}
        </div>

        {page === "apps" && <OAuthView client={client} company={company} />}
        {page === "mcp" && <McpServersView client={client} company={company} />}
      </div>
    </div>
  );
}
