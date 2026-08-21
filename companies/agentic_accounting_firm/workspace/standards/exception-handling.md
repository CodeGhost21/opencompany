# Exception handling

What to do when something does not tie out.

1. **Raise it.** An `exceptions` row with the account, the amount and the
   period. Before investigating, not after.
2. **Trace it.** Timing difference, uncleared item, fee, misclassification — in
   that order of frequency. Write what it was traced to, not what it probably
   was.
3. **Never plug.** A rounding entry that makes a difference disappear is the one
   thing in this firm that cannot be undone by reading the record, because the
   record no longer shows there was a difference.
4. **Resolve with an authority.** A correction, a write-off, or an acceptance —
   each with the basis for the treatment. "It was fixed" is not an entry.
5. **A period does not close over an open exception** without somebody saying so
   explicitly on the `closes` row.

`unknown` is an honest value for `traced_to`. A plausible cause written as fact
is not, and it is exactly what an auditor will pull. See
[[Bookkeeping standards]] and the [[Filing calendar]].
