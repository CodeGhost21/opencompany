#!/bin/sh
# Focused tests for scripts/aggregate-shadow-floor.sh.
set -eu

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
TARGET="${SCRIPT_DIR}/aggregate-shadow-floor.sh"

# A window with only deferred_group= lines (e.g. only publish_artifact calls)
# must not abort before the tool/desk/tier breakdowns run — a no-match grep
# inside field() used to exit 1 under set -euo pipefail and kill the script
# right after "by reason".
deferred_only_output=$(printf '%s\n%s\n' \
    "[policy:shadow-floor] deferred_group=publish_artifact tool='publish_artifact' agent='desk-1' mode='full' hitl=false" \
    "[policy:shadow-floor] deferred_group=publish_artifact tool='publish_artifact' agent='desk-2' mode='supervised' hitl=false" \
    | "$TARGET")

printf '%s\n' "$deferred_only_output" | grep -F "would-stop:       0" >/dev/null
printf '%s\n' "$deferred_only_output" | grep -F "deferred publish: 2" >/dev/null
printf '%s\n' "$deferred_only_output" | grep -F "by tool" >/dev/null
printf '%s\n' "$deferred_only_output" | grep -F "publish_artifact" >/dev/null
printf '%s\n' "$deferred_only_output" | grep -F "by desk" >/dev/null
printf '%s\n' "$deferred_only_output" | grep -F "by tier" >/dev/null

# A mixed window (would_stop present alongside deferred) must still count and
# break down normally — the fix must not change behavior when a match exists.
mixed_output=$(printf '%s\n%s\n' \
    "[policy:shadow-floor] would_stop=irreversible_send tool='composio_execute' agent='desk-1' mode='full' hitl=false" \
    "[policy:shadow-floor] deferred_group=publish_artifact tool='publish_artifact' agent='desk-2' mode='supervised' hitl=false" \
    | "$TARGET")

printf '%s\n' "$mixed_output" | grep -F "would-stop:       1" >/dev/null
printf '%s\n' "$mixed_output" | grep -F "irreversible_send" >/dev/null
printf '%s\n' "$mixed_output" | grep -F "composio_execute" >/dev/null

echo "aggregate-shadow-floor tests passed"
