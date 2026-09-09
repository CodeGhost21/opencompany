#!/usr/bin/env bash
# Aggregate the consequence-floor shadow measurement (issue #2147).
#
# Reads tenant logs on stdin or from files and counts what an enforced
# consequence floor would have stopped, per reason, tool and desk.
#
#   kubectl logs -n <ns> <tenant-pod> --since=168h | scripts/aggregate-shadow-floor.sh
#   scripts/aggregate-shadow-floor.sh tenant-week.log
#
# The reader emits one line per call it would have stopped and nothing for the
# rest, so the counts below are the numerator. The denominator — total tool
# calls over the same window — is not in this stream; take it from the run step
# trace, and do not present a percentage without it.

set -euo pipefail

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
    sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'
    exit 0
fi

input=$(cat "$@")
lines=$(grep -F '[policy:shadow-floor]' <<<"$input" || true)

if [[ -z "$lines" ]]; then
    echo "No [policy:shadow-floor] lines found." >&2
    echo >&2
    echo "Before concluding the floor would fire rarely, check the build actually carries" >&2
    echo "the reader and that the log filter passes it: DEFAULT_LOG_FILTER must name" >&2
    echo "policy::shadow_floor=info, and RUST_LOG (if set) replaces that string wholesale." >&2
    echo "An empty result reads as a finding, which is the one way this measurement lies." >&2
    exit 1
fi

total=$(wc -l <<<"$lines" | tr -d ' ')
stops=$(grep -c 'would_stop=' <<<"$lines" || true)
deferred=$(grep -c 'deferred_group=' <<<"$lines" || true)

field() { { grep -oE "$1=[^ ]+" <<<"$lines" || true; } | cut -d= -f2- | sort | uniq -c | sort -rn; }

echo "shadow-floor measurement"
echo "  lines:            $total"
echo "  would-stop:       $stops"
echo "  deferred publish: $deferred   (#658 carve-out, counted separately and never folded in)"
echo

echo "by reason"
field 'would_stop' | sed 's/^/  /'
echo

echo "by tool"
{ grep -oE "tool='[^']+'" <<<"$lines" || true; } | cut -d"'" -f2 | sort | uniq -c | sort -rn | sed 's/^/  /'
echo

echo "by desk"
field 'agent' | sed 's/^/  /'
echo

echo "by tier"
field 'mode' | sed 's/^/  /'
echo

hitl_on=$(grep -c 'hitl=true' <<<"$lines" || true)
if [[ "$hitl_on" != "0" ]]; then
    echo "note: $hitl_on line(s) came from a build with policy HITL ENABLED, where the"
    echo "      tier already parks. Those are not the population a floor would newly stop —"
    echo "      separate them before quoting a number."
    echo
fi

echo "reading this: irreversible_send is the number that decides the design. If Composio"
echo "sends dominate, a taxonomically narrow floor is still practically wide, and Send"
echo "belongs behind the per-company-type axis rather than in the floor itself."
