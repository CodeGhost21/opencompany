import { useState } from "react";
import { ChevronRight, CircleDot, Hash, Lock } from "lucide-react";

import { TeammateAvatar } from "@/components/teammate-avatar";
import { cn } from "@/lib/utils";
import { channelSubtitle, dmFace, type Channel, type ChannelSection } from "./model";

/**
 * What an unread badge actually claims (issue #364).
 *
 * The one thing on this rail that is still console-local: unread is derived
 * here from when this tab last looked at a channel, because the host has no
 * read-receipt surface. Transcripts, threads and reactions are all the host's
 * now — this is not, and it says so rather than letting an operator read the
 * badge as "unread by my team".
 */
const UNREAD_IS_LOCAL = "Estimated in this browser — unread is not tracked on the company.";

interface Props {
  sections: ChannelSection[];
  activeId: string | null;
  /** Channel id → unread count. Absent or 0 reads as caught up. */
  unread: Record<string, number>;
  /**
   * Channel id → how many unread mentions name **this person** there.
   *
   * Deliberately separate from {@link unread}, and not a subset of it: the two
   * are computed from different places and answer different questions. Unread
   * is derived in this browser from what this tab has seen; a mention is a
   * durable, host-side fact about *you*, and survives a reload, a new device,
   * and a week away. Merging them would take the honest one and give it the
   * other's caveat.
   */
  mentions?: Record<string, number>;
  onSelect: (id: string) => void;
  className?: string;
}

/**
 * The workspace's channel list.
 *
 * Sections collapse, rows carry their own icon by kind (`#` for a channel, a
 * lock when private, the teammate's avatar for a DM), and an unread channel
 * goes bold with a count on the right. This is the second sidebar on the
 * screen — the app's own nav is to its left — so it stays visually quieter
 * than that one: no group headers in caps, no badges except unread.
 */
export function ChannelRail({
  sections,
  activeId,
  unread,
  mentions,
  onSelect,
  className,
}: Props) {
  return (
    <aside
      className={cn(
        "w-64 shrink-0 flex-col overflow-y-auto border-r bg-sidebar/40 pb-3",
        className,
      )}
    >
      <div className="px-3 py-3">
        <h2 className="truncate text-sm font-semibold tracking-tight">Chat</h2>
      </div>

      {sections.map((section) => (
        <Section
          key={section.id}
          section={section}
          activeId={activeId}
          unread={unread}
          mentions={mentions}
          onSelect={onSelect}
        />
      ))}
    </aside>
  );
}

function Section({
  section,
  activeId,
  unread,
  mentions,
  onSelect,
}: {
  section: ChannelSection;
  activeId: string | null;
  unread: Record<string, number>;
  mentions?: Record<string, number>;
  onSelect: (id: string) => void;
}) {
  const [open, setOpen] = useState(true);
  const hiddenUnread = !open
    ? section.channels.reduce((n, c) => n + (unread[c.id] ?? 0), 0)
    : 0;
  // A mention hidden by a collapsed section is exactly the case a mention
  // exists to cover: something arrived while you weren't looking. Aggregating
  // it onto the header — same as unread already does — is what keeps that
  // true when the section itself is closed.
  const hiddenMentions = !open
    ? section.channels.reduce((n, c) => n + (mentions?.[c.id] ?? 0), 0)
    : 0;

  return (
    <section className="group/section select-none px-2 pt-2">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
        className="flex w-full items-center gap-1 rounded-md px-1.5 py-1 text-left text-xs font-medium text-muted-foreground transition-colors hover:text-foreground"
      >
        <ChevronRight
          className={cn("size-3 shrink-0 transition-transform", open && "rotate-90")}
          aria-hidden
        />
        <span className="truncate">{section.label}</span>
        {(hiddenMentions > 0 || hiddenUnread > 0) && (
          <span className="ml-auto flex items-center gap-1">
            {hiddenMentions > 0 && (
              <span
                data-testid="section-mentions"
                title={
                  hiddenMentions === 1
                    ? "1 mention of you in this section"
                    : `${hiddenMentions} mentions of you in this section`
                }
                className="rounded-full bg-destructive px-1.5 text-3xs font-semibold leading-4 text-destructive-foreground"
              >
                @{hiddenMentions > 99 ? "99+" : hiddenMentions}
              </span>
            )}
            {hiddenUnread > 0 && (
              <span
                title={UNREAD_IS_LOCAL}
                className="rounded-full bg-primary px-1.5 text-3xs font-semibold leading-4 text-primary-foreground"
              >
                {hiddenUnread > 99 ? "99+" : hiddenUnread}
              </span>
            )}
          </span>
        )}
      </button>

      {open && (
        <ul className="mt-0.5 flex flex-col gap-px">
          {section.channels.map((channel) => (
            <li key={channel.id}>
              <ChannelRow
                channel={channel}
                active={channel.id === activeId}
                unread={unread[channel.id] ?? 0}
                mentions={mentions?.[channel.id] ?? 0}
                onSelect={onSelect}
              />
            </li>
          ))}
          {section.channels.length === 0 && (
            <li className="px-2 py-1 text-xs text-muted-foreground">Nothing here yet.</li>
          )}
        </ul>
      )}
    </section>
  );
}

function ChannelRow({
  channel,
  active,
  unread,
  mentions,
  onSelect,
}: {
  channel: Channel;
  active: boolean;
  unread: number;
  mentions: number;
  onSelect: (id: string) => void;
}) {
  const hasUnread = unread > 0 && !active;
  // A mention badge shows even on the open channel, unlike the unread count.
  // Unread means "you have not looked"; a mention means "somebody asked you
  // something", and having the channel open is not an answer to that. It
  // clears when the mention is marked read, not when the channel is viewed.
  const hasMentions = mentions > 0;

  return (
    <button
      type="button"
      onClick={() => onSelect(channel.id)}
      aria-current={active ? "page" : undefined}
      // The row's own label is `channel.name`, so a tooltip that resolves to
      // the same string is the header's issue-#1180 duplicate in a slower
      // form: you hover for a second fact and get the one already under the
      // cursor. No tooltip at all is the better answer, and `undefined` — not
      // `""` — is what suppresses the native bubble.
      title={channelSubtitle(channel) ?? undefined}
      className={cn(
        "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm transition-colors",
        active
          ? "bg-sidebar-accent font-medium text-sidebar-accent-foreground"
          : "text-muted-foreground hover:bg-sidebar-accent/50 hover:text-foreground",
        hasUnread && "font-semibold text-foreground",
      )}
    >
      <ChannelIcon channel={channel} />
      <span className="min-w-0 flex-1 truncate">{channel.name}</span>
      {hasMentions && (
        <span
          data-testid="channel-mentions"
          // Unlike unread, this is not a guess this browser made: the host
          // recorded who was named. So it gets no "only in this tab" caveat —
          // it means the same thing on every device.
          title={
            mentions === 1
              ? "1 mention of you here"
              : `${mentions} mentions of you here`
          }
          className="shrink-0 rounded-full bg-destructive px-1.5 text-3xs font-semibold leading-4 text-destructive-foreground"
        >
          @{mentions > 99 ? "99+" : mentions}
        </span>
      )}
      {hasUnread && (
        <span
          data-testid="channel-unread"
          // Issue #364: unread is derived in this browser from what this tab has
          // seen — the host keeps no read receipts. Two consoles will disagree,
          // and a badge that quietly means something narrower than it looks is
          // worse than one that says so.
          title={UNREAD_IS_LOCAL}
          className="shrink-0 rounded-full bg-primary px-1.5 text-3xs font-semibold leading-4 text-primary-foreground"
        >
          {unread > 99 ? "99+" : unread}
        </span>
      )}
    </button>
  );
}

function ChannelIcon({ channel }: { channel: Channel }) {
  if (channel.kind === "dm") {
    const face = dmFace(channel);
    return face ? (
      <TeammateAvatar {...face} className="size-5 text-3xs" />
    ) : (
      <CircleDot className="size-4 shrink-0" aria-hidden />
    );
  }
  const Icon = channel.private ? Lock : Hash;
  return <Icon className="size-4 shrink-0 opacity-70" aria-hidden />;
}
