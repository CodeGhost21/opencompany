import { X } from "lucide-react";

import { Markdown } from "@/components/markdown";
import { TeammateAvatar } from "@/components/teammate-avatar";
import { Button } from "@/components/ui/button";
import type { MessageIntent } from "@/api/tasks";
import type { ChatMessage } from "@/lib/chat";
import type { TeamMember } from "@/lib/team";
import { MessageComposer } from "./MessageComposer";
import { TypingLine } from "./TypingLine";
import { channelTitle, formatTime, senderOf, type Channel } from "./model";
import { type Mention, type Mentionable } from "./mentions";

interface Props {
  channel: Channel;
  /**
   * The roster, so a reply's own line can resolve its sender's mascot the
   * same way the main timeline does — `senderOf` needs it to look a named
   * agent up by id (issue #1185).
   */
  members: TeamMember[];
  /** The message the thread hangs off. */
  parent: ChatMessage;
  replies: ChatMessage[];
  sending: boolean;
  /**
   * Everything an `@` can name here (issue #1645). Drawn from the parent
   * ChatView's directory so the thread composer shares the same roster.
   * Absent when the host predates the route, or when the directory has not
   * loaded — the composer degrades to plain-text typing.
   */
  mentionables?: Mentionable[];
  /**
   * The ids of the teammates on the channel this thread belongs to, for the
   * composer's outside-channel warning. Absent when membership is unknown.
   */
  channelMemberIds?: string[];
  onSend: (text: string, intent?: MessageIntent, mentions?: Mention[]) => void;
  onClose: () => void;
  /**
   * Who is typing *in this thread* — scoped by the parent's own id, never the
   * channel's. Without this the thread panel had no typing signal at all: the
   * wire and `useTyping` already carry `parentId`, but nothing upstream of
   * this component filtered by it or rendered a line for it.
   */
  typingNames?: string[];
  /** This console is typing here. Distinct from the main composer's callback
   * so the ping this thread sends carries the thread's own `parentId`. */
  onTyping?: () => void;
}

/**
 * The thread panel.
 *
 * Replies live here rather than inline, so a busy exchange never pushes the
 * channel apart. The parent message sits at the top under a rule, and the
 * panel carries its own composer scoped to the thread.
 */
export function ThreadPanel({
  channel,
  members,
  parent,
  replies,
  sending,
  mentionables,
  channelMemberIds,
  onSend,
  onClose,
  typingNames = [],
  onTyping,
}: Props) {
  return (
    <aside className="flex w-96 shrink-0 flex-col border-l bg-background">
      <header className="flex h-13 shrink-0 items-center gap-2 border-b px-3">
        <div className="min-w-0 flex-1">
          <h2 className="text-sm font-semibold tracking-tight">Thread</h2>
          <p className="truncate text-xs text-muted-foreground">{channelTitle(channel)}</p>
        </div>
        <Button variant="ghost" size="icon" className="size-8" onClick={onClose} aria-label="Close thread">
          <X className="size-4" />
        </Button>
      </header>

      <div className="flex-1 overflow-y-auto">
        <Line channel={channel} members={members} message={parent} />
        <div className="flex items-center gap-2 px-4 py-2">
          <span className="text-xs font-medium text-muted-foreground">
            {replies.length} {replies.length === 1 ? "reply" : "replies"}
          </span>
          <span className="h-px flex-1 bg-border" aria-hidden />
        </div>
        {replies.map((r) => (
          <Line key={r.id} channel={channel} members={members} message={r} />
        ))}
      </div>

      <TypingLine names={typingNames} />
      <MessageComposer
        compact
        placeholder="Reply…"
        disabled={sending}
        mentionables={mentionables}
        channelMemberIds={channelMemberIds}
        onSend={onSend}
        onTyping={onTyping}
      />
    </aside>
  );
}

function Line({
  channel,
  members,
  message,
}: {
  channel: Channel;
  members: TeamMember[];
  message: ChatMessage;
}) {
  const sender = senderOf(message, channel, members);

  if (sender.kind === "system") {
    return (
      <p className="px-4 py-1 text-center text-xs text-muted-foreground">{message.text}</p>
    );
  }

  return (
    <div className="flex gap-2.5 px-4 py-2">
      <TeammateAvatar
        name={sender.name}
        tone={sender.tone}
        avatar={sender.avatar}
        company={sender.kind === "company"}
        className="size-8"
      />
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2">
          <span className="truncate text-sm font-semibold tracking-tight">{sender.name}</span>
          <span className="shrink-0 text-2xs text-muted-foreground tabular-nums">
            {formatTime(message.at)}
          </span>
        </div>
        <Markdown mentions={message.mentions} className="text-sm leading-6 break-words prose-p:my-0 prose-pre:my-1.5 prose-ul:my-1 prose-ol:my-1 prose-headings:my-1">{message.text}</Markdown>
      </div>
    </div>
  );
}
