#!/usr/bin/env bash
# Sync the branch rulesets in `.github/rulesets/*.json` to a GitHub repository.
#
# Usage: scripts/ci/apply-rulesets.sh [owner/repo]   (default: tinyhumansai/opencompany)
#
# Rulesets are repository settings, not files, so the JSON here is the
# reviewable source of truth and this script is how it reaches GitHub. Each
# file is matched to a live ruleset by its `name`: an existing one is updated
# in place (PUT), a missing one is created (POST). Live rulesets with no file
# here are listed but left alone — delete those by hand, on purpose.
#
# Needs `gh` authenticated as a repository admin (`gh auth status`).
# `.github/rulesets/README.md` explains what each ruleset does and why.
set -euo pipefail

REPO="${1:-tinyhumansai/opencompany}"
DIR="$(cd "$(dirname "$0")/../../.github/rulesets" && pwd)"

live="$(gh api "repos/${REPO}/rulesets" --paginate)"

seen=()
for file in "$DIR"/*.json; do
  name="$(jq -r .name "$file")"
  id="$(jq -r --arg n "$name" '.[] | select(.name == $n) | .id' <<<"$live" | head -n1)"
  if [ -n "$id" ]; then
    gh api -X PUT "repos/${REPO}/rulesets/${id}" --input "$file" >/dev/null
    echo "updated  ${name} (id ${id})"
  else
    id="$(gh api -X POST "repos/${REPO}/rulesets" --input "$file" --jq .id)"
    echo "created  ${name} (id ${id})"
  fi
  seen+=("$name")
done

jq -r '.[].name' <<<"$live" | while read -r name; do
  for s in "${seen[@]}"; do [ "$s" = "$name" ] && continue 2; done
  echo "untracked ${name} (live on ${REPO}, no file in .github/rulesets/)"
done
