# `hive/conducted/`

Child modules of `hive/conducted.rs`, which stays the module root: it owns
`HiveDispatcher`, `Episode`, `run`/`resume` and the episode lifecycle, and each
file here is one coherent step of that lifecycle lifted out to keep the root
under the 750-line cap.

| File | What lives here |
| --- | --- |
| `closing.rs` | `HiveDispatcher::conclusion`, the step between the last seat settling and the closing row being journaled: route the closing turn (`hive::conclude::pick_concluder`), run it as a one-seat `Episode` with `concluding: true`, and read its message's sequence back out of the journal for `EpisodeCompleted.summary_seq`. Failure is warned and swallowed — the episode has settled, and losing a finished episode over an extra turn would be the worse trade. Why the step exists at all is documented on `hive::conclude`. Gated on `openhuman`. |
