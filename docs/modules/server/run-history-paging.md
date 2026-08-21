# Paging the workflow run history (issue #1012)

How `GET …/workflows/runs` cuts a page, orders it, and hands back the
cursor for the page behind it. Split out of
[workflow-routes.md](workflow-routes.md), which links here.

```text
GET …/workflows/runs?workflow=<wid>&limit=<n>&before_seq=<cursor>
→ { "runs": [ … ], "hasMore": true, "nextBeforeSeq": 41 }
```

The page is **cut by `seq` and displayed by `(atMillis, seq)`** — two different
keys, on purpose.

`seq` is the journal's append position: monotonic, and the only key the backward
read (`EventLog::read_before`) can be bounded by. `atMillis` is wall-clock and
is *not* monotonic in storage order — a clock step backwards (NTP correction, a
VM resume, an operator setting the date) writes a row whose time precedes the
row before it. Cutting the page on the display pair and then paging off the
boundary row's `seq` therefore used to lose runs outright: a run appended after
the boundary but timestamped before it sorts below the cut (dropped from this
page) and sits at or above the cursor (excluded from the next), and because the
cursor only descends, from every later page too. It became permanently
unreachable while `hasMore` still counted it.

Cutting on `seq` makes the two pages a **partition**: this response is exactly
`{ seq >= nextBeforeSeq }` of the candidates and the next request asks for
`{ seq < nextBeforeSeq }`. That is a structural guarantee, not a bound on how
far a clock may drift.

The accepted trade: under a clock regression a run is served on the page its
`seq` puts it on — correctly ordered *within* that page, possibly out of order
against the adjacent one. Under a monotonic clock the two keys agree and the
behaviour is identical to cutting on the pair. The degradation fires only on the
anomaly that previously caused permanent loss, and it degrades to "wrong order
at one seam", never to "the run is gone".

**`nextBeforeSeq` is server-issued, and clients must not derive it.** Under this
cut the boundary is the page's *lowest* `seq`, which no ordering of the returned
rows exposes as "the last one" — so `runs.at(-1).seq` is a different run
whenever the clock regressed. It is **omitted when `hasMore` is `false`**: there
is no page behind the last one.

A client talking to a host predating the field must fall back to the old
`runs.at(-1).seq` derivation — such a host still cuts its pages in display
order, so its last row genuinely is the boundary. Reading the absent field as
"there are no more pages" would re-ship this fix as a fresh silent truncation,
which is #1012's own symptom. `hasMore` remains the only thing that says whether
to keep going.

`limit` counts **runs**, not journal rows — a run is a group (`Started`, N node
rows, `Finished`) and a page of raw events is not a page of runs. `?workflow=`
is applied before the cut, so asking for one graph returns *its* most recent N
rather than whichever of the last N happen to match.

