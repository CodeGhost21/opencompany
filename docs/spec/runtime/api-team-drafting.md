# Drafting a mandate, a persona, or a whole teammate

The three read-only routes behind the teammate copilot (issues #1776, #1989),
split out of [`api-write-plane.md`](api-write-plane.md) to keep that file under
the repository's 500-line ceiling. Everything here is part of the console write
plane; **none of these routes writes.**

`POST …/team/{agentId}/draft` runs one turn of a conversation about one of two
fields — `description` (the mandate on the roster card) or `instructions` (the
persona appended to the teammate's system prompt). `POST …/team/draft` does the
same for a teammate the operator is still filling in on the Add form, which has
no id yet; it takes the `role` being typed (blank is a `400`) and the other
authored fields alongside.

The body carries `messages`: the conversation so far, oldest first, each
`{role: "operator" | "copilot", text}`. Empty means the opening turn — "draft
something, I have not said anything yet" — which is deliberate: an operator
staring at a blank persona box wants a starting point to react to, and making
them type first asks for the thing they opened the copilot because they could
not write.

The answer is `{reply, text?}`. `reply` is what the copilot says — what it
changed, or what it needs to know. `text` is the **whole** field as it now
stands, never a diff. `text` is absent on a turn that asked a question instead
of drafting, which is not a failure: `source` is still `"model"`, and letting a
turn ask is what makes this a conversation rather than a hint box.

**The console owns the transcript; the host stores nothing.** That is the whole
of "in-session" — no journal to rehydrate, no thread id to collide with a desk,
and nothing to clean up when the form closes. It is bounded host-side all the
same (the last 16 turns, 2,000 characters each, blanks and turns with an
unreadable `role` dropped silently), because a transcript the caller composes is
one the caller can grow without limit. A dropped turn is not a `400`: the
transcript is context, not the request, and losing the operator's actual
question over one malformed old message would be the worse failure.

**Neither route writes.** No record is touched, no draft is stored, and no lock
is taken — the response is text, and it becomes a teammate's persona only if the
operator takes it and then saves through `PATCH …/team/{agentId}` like any edit
they typed themselves.

## `POST …/team/design` — a whole teammate, at creation only

The reduced Add-teammate dialog (issue #1989) collects a **name and one
sentence**. This route turns that sentence into the three fields a teammate is
made of — `role`, `description` and `instructions` — in one model call, and the
console then creates the teammate through `POST …/team`.

The body is `{name?, description}`; a blank `description` is a `400`, because it
is the entire input and designing from nothing is a model inventing a job rather
than reading one. It is bounded by `MAX_DESIGN_BRIEF` (2000 characters, the
prompt-weight bound every other operator free text going into a copilot prompt
obeys) — **not** by `MAX_DESCRIPTION`, which is 200 and is a roster-card
*layout* bound. Applying the card bound to the brief cut the operator's sentence
at 200 with an `…` on the end before the model read it, and nothing said so: the
console's box had no limit, and the stored description is the model's rather
than the operator's, so a requirement written past character 200 left no trace.
The console now holds the same number on the box itself, so the limit is met
while typing. The answer is `{role?, description?, instructions?, source,
reason?}`, with the same `source` / `reason` contract the draft routes use: all
four refusals (`no_model`, `model_unreachable`, `unreadable`,
`budget_exhausted`) are a `200`, because none is a failure of the request.

**Three fields or none.** A design missing any of them is refused rather than
salvaged, and a `role` too long to be a job title is refused rather than
truncated. A teammate holding a real mandate and a fragment for a role is what
the console used to produce by splitting the operator's sentence and cutting it
at sixty characters — and on screen it looks finished, which is what makes it
worth refusing outright.

**Three non-empty strings is not enough.** `TeammateDesign::from_parts` also
refuses two answers that pass every length and emptiness check:

- **A role the model truncated itself** — an ellipsis in either spelling
  (`…` or `...`). The brief tells it never to write one, but a brief is not a
  validator, and `"Runs wholesale outreach to boutique retailers and keeps
  the…"` is exactly the stored job title this route replaced. Scoped to the
  role: a *mandate* may legitimately end in `…`, because that is the mark
  `clamp_description` itself leaves.
- **The same text in more than one field.** One sentence appearing as role,
  mandate and persona at once is the original complaint, and a model that
  echoes the sentence into two of the three reproduces it in valid JSON.
  Compared on a normal form — whitespace collapsed, case folded, trailing
  punctuation dropped — and before the clamps, so a description cut to the card
  bound cannot come out looking different from the persona it was copied from.
- **A role that is a sentence.** `MAX_ROLE` bounds characters and a sentence
  fits inside it: `"Handles payroll and reconciles the books weekly"` is 46 of
  the 60 allowed. The brief asks for "a noun phrase of one to four words", and
  `MAX_ROLE_WORDS` enforces that at five — one word of slack, so a real title
  that runs long ("VP of Brand and Communications") is not thrown away while a
  sentence still is.
- **A role that is the operator's brief, or the front of it.** The rule above
  compares the three answers to each other, and so misses the shape that matters
  most: a brief of `"Runs wholesale outreach to boutique retailers"` answered
  with role `"Runs wholesale outreach"`, a real mandate and real instructions
  beside it, passes everything else. That is the clause split this route
  replaced, arriving without the ellipsis that used to make it obvious, and only
  a comparison against the *input* catches it — so `from_parts` takes the brief
  and refuses a role that is the whole of it or a leading fragment at a word
  boundary. An operator whose brief opens with the job title has answered a
  different question from the one the box asks, and gets the full form carrying
  what they typed, where Role is its own field.

  Not "reject verb-led roles", which is what the brief itself asks for: the same
  prompt says to answer in the operator's language, so a list of English verbs
  would refuse valid titles in every other one. The rule catches fragments of
  the input, which is the shape that actually harms.

Both are refusals rather than repairs, for the same reason the type is
all-or-nothing: the operator gets the full form carrying what they typed, where
a salvaged two-thirds looks finished on screen and is not.

### The desktop app has to be told the deadline

The pass runs a model for up to 90 seconds (`PERSONA_TIMEOUT`), and on the
desktop app every request goes through the Tauri core's `oc_request`, which
applied a flat 30-second `reqwest` timeout the console could not see. So a
slow-but-valid design on desktop came back as a transport failure and handed
the operator the full form — a refusal for a pass that was working, and one
carrying no reason because there was none to carry.

`ProxyRequest` now takes an optional `timeoutMs`, clamped in the core to a
ceiling above the host's longest deliberate deadline, and `designTeammate` names
the host's 90 seconds plus the round trip. Only the caller knows which route it
is asking for, so the deadline crosses the bridge with the request rather than
being special-cased in Rust. A browser is unaffected: `BrowserTransport` has no
deadline of its own, and the client already races its own timer.

### Who can run this pass, and how the console knows

`build_design` needs `runtime.profile_drafter()`, which is built from
`workflow_harness_deps` — and `RuntimeBuilder::build` assigns that in exactly
one place, inside the embedded-harness arm. So a company on the `hosted`,
`sidecar` or `custom` cognition path has no drafter, and this route can only
answer `no_model` for it.

The console cannot infer that from `cognition`, and when it tried
(`cognition !== "echo"`) it was wrong for three of the six paths: the reduced
dialog was offered, the operator typed a sentence, pressed Create, waited on a
model call that could only refuse, and met the full form anyway.
`GET …/inference` therefore reports `designsProfiles` — the same
`profile_drafter().is_some()` this route acts on — so the dialog decides its
shape from the capability rather than from a label. It is optional on the wire:
an older host omits it, and the console reads a missing value as "unknown" and
offers the reduced dialog, exactly as it does while the check is in flight.

### Why a role may be designed here when `DraftableField` excludes one

`POST …/team/{agentId}/draft` refuses anything but `description` and
`instructions`, and must keep refusing. Its reason — a role is what delegation
grounds on, so a drafted one would change who the company routes work to — is a
statement about **editing a teammate that exists**: work is already addressed to
it, and a model re-pointing that without the operator choosing to is the harm.

At **creation** there is nothing to re-route. The teammate does not exist, no
work is addressed to it, and no orchestrator has seen it. So the property the
exclusion protects is not in play, and the alternative was not a safe blank:
`role` is required by every write path and `persona_prompt` interpolates it
unguarded.

The separation is **structural, not a flag**: this route takes no agent id at
all, so there is no request shape that reaches the design pass carrying an
existing teammate's id. There is no `POST …/team/{agentId}/design`.

The grounding is the same closed set every draft gets — the company, what it
makes, the name, the operator's sentence, and siblings' ids and roles so the new
teammate's job is not one the company already has. The operator's sentence is
framed to the model as data, never as instructions to it.

That is the whole reason a model is allowed near these two fields. First-run
setup deliberately keeps the design pass **out** of a teammate's standing
instructions ([company-setup/overview.md](company-setup/overview.md)): the pass
names a work *shape* from a closed enum and the host owns every word, because
there the text would reach a system prompt with nobody having read it, through a
member-open route. Here two deliberate human actions stand in between. If either
is ever removed, this route has to be reconsidered with it.

Grounding is assembled host-side from the company record — this teammate, and
the rest of the roster's ids and roles so a drafted mandate does not restate a
neighbour's. The console holds all of that already and could have sent it; it
must not, because a grounding the caller composes is one the caller can widen.

The exceptions are the fields being authored *right now* — the mandate, the
persona, the role and the name, all of them held by the one form — which the
console does send so that "make it shorter" means shorter than what is on screen
rather than shorter than what was last saved, and so that a teammate repurposed
on screen is drafted for its new job rather than the one it used to do. The role
matters most of the four: both prompts are written *from* it. On the Add form
they ride the request for a second reason, which is that the teammate does not
exist yet and there is nowhere else to read them from.

Every one of those is clamped **on the way in** to the bound the field obeys —
the one-line bound for identity — along with the operator's note and the
conversation, and a value that is blank once trimmed is dropped rather than sent
as an empty string, because "" is not an empty mandate. Nothing else has bounded
them: the request body cap is the only ceiling between a pasted document and the
prompt it would ride in, on every turn of the conversation and on the bill.
Clamping costs a grounding nothing, because text past that bound could never
have been saved into the field anyway.

The answer is clamped to the same bound before it is returned —
`MAX_DESCRIPTION` for a mandate (a card has one line), the persona prompt budget
for instructions — so the console is not the only thing holding the limit.
Drafting is metered as a `SampleKind::AuthoringCall` charged to the **company**,
never to the teammate being described: it ran no turn, and billing it would
otherwise eat that teammate's daily cap. It counts toward the plan-level total
token ceiling (`[plan].total_tokens`) like every other completion the tenant
pays for, and the routes **check** that ceiling before calling a provider — the
same gate the harness applies before dispatch, failing the same way it does: an
unreadable meter warns and lets the draft through, because a metering outage
that silently disabled a working copilot is the worse failure.

Refusals are deliberately not errors. An unknown id is `404` and an unknown
field is `400`, but "no model is wired", "the provider did not answer", "the
answer could not be read" and "this company has spent its budget for the period"
all come back `200` with `source: "unavailable"` and a distinct `reason`
(`no_model` / `model_unreachable` / `unreadable` / `budget_exhausted`), because
each implies a different next move for the operator and none of them is a
failure of the request. Only `model_unreachable` is worth retrying;
`budget_exhausted` in particular is a plan setting rather than a transient
failure, so a shared "try again" would be advice that cannot work.

`unreadable` is narrower than it looks. An answer that is not in the format
asked for is read as a **reply carrying no draft** rather than refused: the
format exists because a draft has to be extracted exactly, and a conversational
reply does not. Only an answer with nothing in it at all is `unreadable`. That
distinction is not theoretical — asked something vague, a model answers with a
plain-prose question about half the time, and refusing those told the operator
their copilot was broken at the exact moment it was doing the right thing. There is no curated fallback text, unlike the roster proposal:
"what does this particular teammate own" has no canned answer, and inventing one
would put words in the company's mouth.

The format is a fence tagged `teammate-field`, not JSON. A persona is a
multi-line document, and escaping one into a JSON string failed on one rich
answer in two — which reaches the operator as a reply with no draft. The block
closes at the **last** ``` in the answer rather than the first, because a
persona that shows worked examples fences them, and closing at the first ```
would hand over a document cut at its first example with nothing on screen
saying the rest had been dropped. The older JSON shape is still read; a ```json
block is explicitly not treated as field text, since handing someone raw JSON as
their teammate's persona is syntactically fine and completely wrong.
