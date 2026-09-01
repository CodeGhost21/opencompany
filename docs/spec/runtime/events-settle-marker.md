# The settle a channel can see (issue #377)

A card dispatched from a channel can park in `paused`, or bounce back to `todo`
on a failure or a cancellation. The channel showed none of that. All it got was
the orchestrator's relay prose (#151), which reads like an answer — so a reader,
live or arriving fresh after a reload, reasonably concluded the work had
finished. Two correct halves producing one wrong impression.

The fix is a **card-linked system marker**, not another bubble: `finished → In
review`, carrying the column and a link to the card, and deliberately **not** the
run's prose. The prose is already in that channel; repeating it would put one
run's words into one conversation twice. What was missing was never the words —
it was the structural fact that the card settled, and where.

**The origin is captured, never derived.** `DeskTaskCompleted` gains
`origin_chat_id`, stamped at the single emission point
(`HarnessBrain::journal_task_outcome`) off `TaskRecord.origin_chat_id`, which
every conversational creation path has recorded since #151. It cannot be
recovered from the fields that were already there: `desk` is the *responder*, an
agent id like `engineer`, and a channel is a desk id like `engineering`.
Re-deriving it at completion time would also put a second "which conversation is
this?" rule beside `chat_history`'s, which is precisely the drift #435 exists to
have removed.

**`None` means no conversation raised this card** — a board-native card, a
scheduler's — and is emphatically *not* folded into the General desk, even
though every other missing chat id in `chat_history` is. Folding it would post
markers about board-only work into the operator's main line: a new bug, not the
one being fixed. The frame omits `chatId` rather than sending null, so
"board-created" is a presence check on the console, the same shape
`approval_parked` uses for a page-only approval.

**`chat_history::owns` admits the terminal**, which is what makes the marker
survive a reload; without it the live line would appear and then vanish. Because
`MessageView` is shared with the GraphQL `Message` projection, `Chat.history`
starts returning system marker rows on existing fields — additive on both wire
surfaces, and named here rather than discovered in review.

**The stream frame drops `output`.** Nothing read it, and removing it at the
projection is what stops a later reader from reintroducing the duplicate. An
out-of-tree consumer of `/events` loses that field.

Dedupe is on **identity**, never content: the live line is born under the host's
own sequence (`h<seq>`, #483/#498's mechanism), which is exactly the id
`chat/history` mints for the same event, so hydration recognises its own twin.
The marker sentence exists twice — `dispatch_marker_text` on the host,
`dispatchMarkerText` in the console, because the live frame is thin and carries
the raw column id — and tests on both sides pin the identical literals. Drift can
only reword a marker across a reload; it can never double one.

Pre-#377 journal lines carry no origin, so existing channels grow no
retroactive markers. That is correct rather than a migration gap: the fact was
not recorded, and inventing one would be worse than its absence.

## The thread inside that channel (issue #1890 B)

A channel holds any number of live threads, so an origin naming only the channel
files every settle flat in it — and the thread that asked for the work never
shows it finishing. `TaskRecord` therefore gains `origin_parent:
Option<EventSeq>` beside `origin_chat_id`, and `DeskTaskCompleted` carries it on
exactly the terms above: **captured off the card** at the same single emission
point, never derived, since nothing else on the event knows which thread asked.

Read as a **pair**, never alone. A marker is filed by `origin_chat_id` first and
only then narrowed by `origin_parent`, because `None` means two different things
depending on the other field: the *channel-level conversation* when a channel is
named, and nothing at all when it is not — a board-native card has both absent.

Both wire surfaces carry it:

* `GET …/chat/history` and the GraphQL `Message` projection set the marker's
  existing `parentId`, rendered the same way an operator message's parent is —
  the host's sequence as a decimal string. A threaded marker is therefore folded
  into its root's replies by the console's timeline rather than rendered inline,
  which is the whole point.
* The `desk_task_completed` stream frame gains `parentId`, a **string**, and
  **omits it rather than sending null** — the same presence-check shape `chatId`
  takes, so absent means "raised at channel level" without a null check. The two
  must agree: a marker that rendered inline live and jumped into a thread on
  reload is the live-vs-history split the `h<seq>` identity dedupe exists to
  close.

Additive on the same contract as everything else here — `#[serde(default,
skip_serializing_if = …)]` on both fields — so no stored board migrates on any
backend, an unthreaded settle serializes byte-for-byte as it did before, and a
pre-#1890-B journal line replays as channel-level. Which is the truth about such
a line, not a default standing in for one.

One creation path deliberately still writes `None`: `POST …/tasks` ("Add to
board"), whose body carries a channel and no thread. That is unchanged from
before, and closing it needs a body field plus a console change — the transcript
holds a rendered message id, not an `EventSeq`.

---

Part of the [runtime event journal](events.md), split out under the 500-line
Markdown cap. The variant itself — `DeskTaskCompleted`, its fields, and how a
per-task timeline is assembled from it — is described there.
