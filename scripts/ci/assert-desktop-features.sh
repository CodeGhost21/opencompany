#!/usr/bin/env bash
#
# Fail if the desktop's shipped cargo feature set is not the one CI compiles and
# not the one `scripts/desktop-dev.sh` runs.
#
# Issue #1738. `src-tauri/Cargo.toml`'s feature list is NOT what the desktop
# ships. `acp` and `composio` are passed on the `tauri` command line — both are
# `= ["openhuman"]` in the root manifest, pure `cfg` switches adding no package,
# so a release can turn them on and still build `--locked`; the argument is on
# the `DESKTOP_RELEASE_FEATURES` env block in `release-desktop-macos.yml`.
#
# The cost of that is three copies of one string, and until #1738 there were
# only two: the dev script ran `tauri dev` bare. So a developer's desktop was
# the DEFAULT feature set and every DMG was the release one, which made them
# different products with nothing saying so. The visible half was Connections
# reporting `in_build: false`: eight tiles reading "not available here" over a
# card asking for a Composio token, on a build nobody ships.
# #1738 was filed reading that as the product's intent, which is what a surface
# only developers see and only users don't will keep producing.
#
# `ci.yml`'s copy already had a comment telling the next person to keep it in
# step with the release workflow. This is that instruction, enforced. The
# release workflow is the source of truth: it is the one whose value reaches a
# user.
#
# To change the shipped set: edit `DESKTOP_RELEASE_FEATURES` in
# `release-desktop-macos.yml`, run this script, and fix whatever it names.
set -euo pipefail

cd "$(dirname "$0")/../.."

RELEASE_WORKFLOW=.github/workflows/release-desktop-macos.yml
CI_WORKFLOW=.github/workflows/ci.yml
DEV_SCRIPT=scripts/desktop-dev.sh

# Comma-separated cargo feature lists are order-insensitive to cargo but not to
# `=`, and a reordering is not a drift. Normalise before comparing so this
# script fails on the thing that matters — a feature present in one place and
# absent from another — rather than on a rewrite of the same set.
normalise() {
  printf '%s' "$1" | tr ',' '\n' | sed 's/^[[:space:]]*//; s/[[:space:]]*$//' \
    | grep -v '^$' | LC_ALL=C sort -u | paste -sd, -
}

expected_raw="$(
  sed -n 's/^[[:space:]]*DESKTOP_RELEASE_FEATURES:[[:space:]]*\(.*\)$/\1/p' \
    "$RELEASE_WORKFLOW" | head -1 | sed 's/#.*//; s/[[:space:]]*$//; s/^"//; s/"$//'
)"

if [ -z "$expected_raw" ]; then
  echo "assert-desktop-features: could not read DESKTOP_RELEASE_FEATURES from $RELEASE_WORKFLOW" >&2
  echo "  Either the release build stopped naming its features there, or this" >&2
  echo "  script's pattern is stale. Both need a human." >&2
  exit 1
fi

expected="$(normalise "$expected_raw")"
echo "$RELEASE_WORKFLOW ships: $expected"

status=0
found=0

# One `check <where> <value>` per call site. `$value` may be empty — that is
# the #1738 failure itself (a call site passing no features at all), so it must
# report rather than be skipped.
check() {
  local where="$1" actual_raw="$2" actual
  found=$((found + 1))
  actual="$(normalise "$actual_raw")"
  if [ "$actual" = "$expected" ]; then
    echo "  ok  $where -> $actual"
  else
    echo "  BAD $where -> '${actual:-<none>}' (release ships '$expected')" >&2
    status=1
  fi
}

# `ci.yml`'s Desktop lane: every `--features` on a `--manifest-path
# src-tauri/Cargo.toml` command. Both the clippy and the test step must carry
# it — a lane that lints the shipped set but tests the default one is the same
# hole in half.
ci_lines="$(grep -n -- '--manifest-path src-tauri/Cargo.toml' "$CI_WORKFLOW" | grep -- '--features' || true)"
if [ -z "$ci_lines" ]; then
  echo "assert-desktop-features: no '--features' on any src-tauri cargo command in $CI_WORKFLOW." >&2
  echo "  The Desktop lane would be compiling a feature set the release does not" >&2
  echo "  ship, which is exactly what this script exists to catch." >&2
  exit 1
fi
while IFS= read -r entry; do
  line="${entry%%:*}"
  value="$(printf '%s' "${entry#*:}" | sed -n 's/.*--features[= ]\{1,\}\([^ ]*\).*/\1/p')"
  check "$CI_WORKFLOW:$line" "$value"
done <<< "$ci_lines"

# The dev script's own copy. Read from its assignment rather than from the
# `tauri dev` line, so the value is checked once and the command that consumes
# it is free to spell the flag however the CLI wants.
dev_value="$(
  sed -n 's/^DESKTOP_RELEASE_FEATURES=\(.*\)$/\1/p' "$DEV_SCRIPT" \
    | head -1 | sed 's/^"//; s/"$//'
)"
check "$DEV_SCRIPT" "$dev_value"

# A `tauri dev` that never reads the variable would pass every check above while
# still launching the default build — the original bug, with a decorative
# constant added. Assert the value is actually used.
if ! grep -q 'tauri dev --features "\${DESKTOP_RELEASE_FEATURES}"' "$DEV_SCRIPT"; then
  echo "  BAD $DEV_SCRIPT -> defines DESKTOP_RELEASE_FEATURES but no 'tauri dev' passes it" >&2
  status=1
fi

if [ "$status" -ne 0 ]; then
  echo >&2
  echo "assert-desktop-features: bring every call site to '$expected', or change" >&2
  echo "  DESKTOP_RELEASE_FEATURES in $RELEASE_WORKFLOW if the release is the one" >&2
  echo "  that is wrong. A desktop developers run and a desktop users install" >&2
  echo "  must be the same build (issue #1738)." >&2
  exit 1
fi

echo "All $found desktop feature call site(s) agree with $RELEASE_WORKFLOW."
