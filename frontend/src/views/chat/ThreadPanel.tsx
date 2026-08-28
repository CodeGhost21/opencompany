import { X } from "lucide-react";

import { Markdown } from "@/components/markdown";
import { TeammateAvatar } from "@/components/teammate-avatar";
import { Button } from "@/components/ui/button";
import type { MessageIntent } from "@/api/tasks";
import type { AttachmentDto, CognitionState } from "@/api/types";
import type { ChatMessage } from "@/lib/chat";
import type { TeamMember } from "@/lib/team";
import { EchoPlaceholder, echoMarkerFor } from "./EchoPlaceholder";
import { MessageAttachments } from "./MessageAttachments";
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
  /**
   * Your own avatar reference, so your lines in this thread wear your face
   * (issue #1729).
   *
   * `senderOf` seeds a face off the sender's *name* when it is given none, and
   * a "you" line's name is the literal string "You" — so without this the panel
   * drew whatever mascot `avatarFor("You")` hashes to, which is the agent's
   * green one. Both participants then had the same face and a thread could not
   * be read at all. The main timeline has always passed it (`buildTimeline`);
   * only this panel resolved its senders without it.
   *
   * Absent until `loadViewer` has resolved who you are, exactly as in the main
   * timeline — the tile falls back to the name-seeded mascot for that first
   * render rather than showing nothing.
   */
  youAvatar?: string;
  /**
   * Resolves an attachment's bytes to an object URL for preview/download
   * (issue #1682). Threaded from the parent ChatView like the main timeline's
   * `MessageRow` gets it, so a thread line can render the same chips — a
   * reply with an attachment is legal on the wire and history preserves it,
   * and it was invisible without this path.
   */
  resolveAttachmentUrl?: (nodeId: string) => Promise<string>;
  onSend: (
    text: string,
    intent?: MessageIntent,
    // The thread composer never attaches (no `uploadAttachment`), so this is
    // always absent — kept in the signature so arg alignment with the merged
    // `MessageComposer.onSend` holds and mentions land in the right slot.
    _attachments?: AttachmentDto[],
    mentions?: Mention[],
  ) => void;
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
  /**
   * Whether this company's teammates can think (issue #1735), threaded here for
   * the same reason `MessageTimeline` gets it: on either echo state the company
   * side of this panel is the offline echo brain's output, not the teammate's.
   *
   * A thread is not a lesser transcript. The first cut of #1734 marked only the
   * channel timeline, and this panel's own `Line` kept rendering echoed replies
   * under the teammate's name and avatar with nothing to say otherwise — the
   * exact false attribution the fix exists to remove, one click away from where
   * it had been removed (CodeRabbit and codex both caught it on PR #1740).
   */
  cognition?: CognitionState | null;
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
  youAvatar,
  resolveAttachmentUrl,
  onSend,
  onClose,
  typingNames = [],
  onTyping,
  cognition,
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
        <Line
          channel={channel}
          members={members}
          message={parent}
          youAvatar={youAvatar}
          resolveAttachmentUrl={resolveAttachmentUrl}
          cognition={cognition}
        />
        <div className="flex items-center gap-2 px-4 py-2">
          <span className="text-xs font-medium text-muted-foreground">
            {replies.length} {replies.length === 1 ? "reply" : "replies"}
          </span>
          <span className="h-px flex-1 bg-border" aria-hidden />
        </div>
        {replies.map((r) => (
          <Line
            key={r.id}
            channel={channel}
            members={members}
            message={r}
            youAvatar={youAvatar}
            resolveAttachmentUrl={resolveAttachmentUrl}
            cognition={cognition}
          />
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
  youAvatar,
  resolveAttachmentUrl,
  cognition,
}: {
  channel: Channel;
  members: TeamMember[];
  message: ChatMessage;
  youAvatar?: string;
  resolveAttachmentUrl?: (nodeId: string) => Promise<string>;
  cognition?: CognitionState | null;
}) {
  // Four arguments, not three: `youAvatar` is the last parameter, and omitting
  // it left your own line with no avatar to seed from but the name "You" —
  // which collided with the agent's face (issue #1729).
  const sender = senderOf(message, channel, members, youAvatar);
  // Literally the same predicate `MessageRow` uses, not a second copy of it —
  // two spellings of "which rows are the echo brain's" is how this panel came
  // to be missing the marker in the first place.
  const placeholder = echoMarkerFor(message, sender, cognition);

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
        // Named by whose line it is, so a spec can assert that your face and
        // the agent's are two different faces (issue #1729) rather than
        // counting `img` elements in DOM order.
        data-testid={`thread-avatar-${sender.kind}`}
      />
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 flex-wrap items-baseline gap-x-2">
          <span className="truncate text-sm font-semibold tracking-tight">{sender.name}</span>
          {placeholder && <EchoPlaceholder author={sender.name} cause={placeholder} />}
          <span className="shrink-0 text-2xs text-muted-foreground tabular-nums">
            {formatTime(message.at)}
          </span>
        </div>
        <Markdown mentions={message.mentions} className="text-sm leading-6 break-words prose-p:my-0 prose-pre:my-1.5 prose-ul:my-1 prose-ol:my-1 prose-headings:my-1">{message.text}</Markdown>
        {message.attachments && message.attachments.length > 0 && (
          <MessageAttachments
            attachments={message.attachments}
            resolveUrl={resolveAttachmentUrl}
          />
        )}
      </div>
    </div>
  );
}
