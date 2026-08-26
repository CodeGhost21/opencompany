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
CONSOLE_MANIFEST=frontend/package.json
DESKTOP_DOC=docs/spec/runtime/desktop.md

# Comma-separated cargo feature lists are order-insensitive to cargo but not to
# `=`, and a reordering is not a drift. Normalise before comparing so this
# script fails on the thing that matters — a feature present in one place and
# absent from another — rather than on a rewrite of the same set.
# Empty input must come back empty, not abort: a call site passing NO features is
# the failure this script is for, and `check` renders it `<none>`. `sed '/^$/d'`
# rather than `grep -v '^$'` because grep exits 1 when it emits nothing, which
# under `set -e -o pipefail` killed the script mid-run — it exited 1 without ever
# naming the offending call site, which is the one thing a reader needs.
normalise() {
  printf '%s' "$1" | tr ',' '\n' | sed 's/^[[:space:]]*//; s/[[:space:]]*$//' \
    | LC_ALL=C sort -u | sed '/^$/d' | paste -sd, -
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

# ...and the release build must actually PASS it. Everything below compares
# against this value on the strength of it being what reaches a user, so a
# `tauri build` that stopped consuming the variable would turn the source of
# truth into a lie: the DMG would revert to the default set while CI and the dev
# script kept `acp` and `composio`, and this script would report success. That is
# #1738 again with the roles reversed, and it is the one drift no comparison here
# could see. Symmetric with the `tauri dev` check further down.
release_invocations="$(
  grep -vE '^[[:space:]]*#' "$RELEASE_WORKFLOW" \
    | grep -nE 'tauri build|--features' | grep -E 'tauri build|--features' || true
)"
if ! printf '%s' "$release_invocations" | grep -q 'DESKTOP_RELEASE_FEATURES'; then
  echo "assert-desktop-features: $RELEASE_WORKFLOW declares DESKTOP_RELEASE_FEATURES" >&2
  echo "  but its 'tauri build' never passes it, so the shipped DMG would carry" >&2
  echo "  the default feature set while every other call site carries the release" >&2
  echo "  one. Restore '--features \"\$DESKTOP_RELEASE_FEATURES\"' on the build." >&2
  exit 1
fi

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

# `ci.yml`'s Desktop lane: every cargo command run against the desktop manifest.
# Both the clippy and the test step must carry the features — a lane that lints
# the shipped set but tests the default one is the same hole in half.
#
# Selected by the CARGO COMMAND, never by the presence of `--features`. Filtering
# on the flag was a hole big enough to drive the original bug through: a step
# that dropped `--features` would not match, would therefore not be checked, and
# the script would report the remaining step as "ok" and exit 0. The one thing
# this exists to catch — a call site compiling the default set — was the one
# thing it skipped.
ci_lines="$(grep -nE 'cargo (clippy|test|check|build)[^|]*--manifest-path src-tauri/Cargo.toml' "$CI_WORKFLOW" || true)"
if [ -z "$ci_lines" ]; then
  echo "assert-desktop-features: no cargo command against src-tauri/Cargo.toml in $CI_WORKFLOW." >&2
  echo "  Either the Desktop lane stopped compiling the shell, or this script's" >&2
  echo "  pattern is stale. Both need a human." >&2
  exit 1
fi
while IFS= read -r entry; do
  line="${entry%%:*}"
  # No match leaves this empty, which `check` reports as `<none>` rather than
  # skipping. That is the point of selecting by command above.
  value="$(printf '%s' "${entry#*:}" | sed -n 's/.*--features[= ]\{1,\}\([^ ]*\).*/\1/p')"
  check "$CI_WORKFLOW:$line" "$value"
done <<< "$ci_lines"

# The dev script's own copy of the string.
dev_value="$(
  sed -n 's/^DESKTOP_RELEASE_FEATURES=\(.*\)$/\1/p' "$DEV_SCRIPT" \
    | head -1 | sed 's/^"//; s/"$//'
)"
check "$DEV_SCRIPT" "$dev_value"

# ...and every place that actually launches the shell must pass it. A constant
# nothing reads would satisfy the check above while `tauri dev` still built the
# default set — the original bug with a decorative variable added.
#
# EVERY branch, not any. `desktop-dev.sh` picks between a path-qualified CLI and
# a `cargo tauri` fallback, so a `grep -q` for one spelling passes while the
# other launches bare. The first version of this check looked for the literal
# `tauri dev --features`, which matches ONLY the `cargo tauri dev` fallback —
# `"${TAURI_CLI}" dev` does not contain that string — so the primary branch,
# the one that actually runs on any checkout with the console installed, was
# never verified at all.
#
# Comment lines are stripped first: this file's own prose names these
# invocations, and so does the dev script's.
dev_invocations="$(grep -vE '^[[:space:]]*#' "$DEV_SCRIPT" | grep -nE '(\$\{TAURI_CLI\}"?|cargo tauri)[[:space:]]+dev([[:space:]]|$)' || true)"
if [ -z "$dev_invocations" ]; then
  echo "  BAD $DEV_SCRIPT -> no 'tauri dev' invocation found; this check asserts nothing" >&2
  status=1
else
  while IFS= read -r invocation; do
    text="${invocation#*:}"
    if printf '%s' "$text" | grep -q '\${DESKTOP_RELEASE_FEATURES}'; then
      echo "  ok  $DEV_SCRIPT launches with the features:$(printf '%s' "$text" | sed 's/^[[:space:]]*/ /')"
    else
      echo "  BAD $DEV_SCRIPT launches WITHOUT the features:$(printf '%s' "$text" | sed 's/^[[:space:]]*/ /')" >&2
      status=1
    fi
  done <<< "$dev_invocations"
fi

# The developer PACKAGING path. `npm run tauri:build` is how a developer builds
# the app locally, and the command `docs/spec/runtime/desktop.md` documents does
# the same thing by hand.
#
# This is the entry point #1738 did not reach and the dev-launcher fix did not
# cover. A `tauri build` with no `--features` packages the DEFAULT set, so the
# artifact has Composio and ACP compiled out while looking in every other respect
# like the shipped app — worse than the dev-window version of the bug, because
# there is no dev server or console banner to hint that this is not the real
# thing. Someone reproducing a user report against a locally-packaged build would
# be testing a different product and have no way to know.
#
# CI's own `Package` steps (`tauri build --debug --no-bundle`) are deliberately
# NOT checked here. They exist to prove `tauri.conf.json` executes from two
# working directories (issue #616), not to compile a feature set, and the
# `Clippy`/`Test` steps above them already build the shipped one.
manifest_build="$(
  grep -oE '"tauri:build"[[:space:]]*:[[:space:]]*"[^"]*"' "$CONSOLE_MANIFEST" || true
)"
if [ -z "$manifest_build" ]; then
  echo "assert-desktop-features: no 'tauri:build' script in $CONSOLE_MANIFEST." >&2
  echo "  Either the console stopped offering a packaging script, or this" >&2
  echo "  script's pattern is stale. Both need a human." >&2
  exit 1
fi
manifest_features="$(printf '%s' "$manifest_build" | sed -n 's/.*--features[= ]\{1,\}\([^ "]*\).*/\1/p')"
check "$CONSOLE_MANIFEST (tauri:build)" "$manifest_features"

# The documented by-hand equivalent, which a developer is at least as likely to
# copy as to run the npm script.
doc_build="$(grep -nE '^cargo tauri build' "$DESKTOP_DOC" || true)"
if [ -z "$doc_build" ]; then
  echo "  BAD $DESKTOP_DOC -> documents no 'cargo tauri build' command to check" >&2
  status=1
else
  while IFS= read -r line; do
    value="$(printf '%s' "${line#*:}" | sed -n 's/.*--features[= ]\{1,\}\([^ ]*\).*/\1/p')"
    check "$DESKTOP_DOC:${line%%:*}" "$value"
  done <<< "$doc_build"
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
