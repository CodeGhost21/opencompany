#!/usr/bin/env bash
#
# Fail if any Markdown file is over 500 lines.
#
# `CLAUDE.md` has asked for this since the repository started — "Keep every
# Markdown file, including this one, at 500 lines or fewer. When a topic grows
# past that limit, split it into focused files and link them from the module's
# readme.md" — and until now nothing checked it. A convention nothing measures
# is a convention that drifts: issue #695 found three files over the cap, one
# of them at 699 lines, none of which arrived over it in a single commit. They
# grew a paragraph at a time, and every one of those paragraphs was reasonable.
#
# The cap is about reading, not tidiness. A spec page nobody can hold in their
# head stops being consulted and starts being duplicated, and a 700-line plan
# hides the ten lines of it that are still true. Splitting forces the seam to
# be named.
#
# Fixing a failure is a split, never a deletion: move a coherent section into a
# focused sibling, leave a pointer under the old heading so existing links and
# anchors still land, and index the new page from the folder's README. See
# `docs/spec/runtime/api.md` -> `api-write-plane.md` for the pattern.
set -uo pipefail

cd "$(dirname "$0")/../.." || exit 1

LIMIT=500

# Excluded trees, and why each one:
#   node_modules — dependency docs, not ours. Matched by `*/node_modules/*`
#     rather than `./node_modules/*`: the console's tree lives at
#     `frontend/node_modules`, which the anchored form walks straight past.
#   vendor       — the openhuman and tinyagents submodules; upstream's files.
#   target       — cargo build output.
#   .git         — packed refs and hook samples.
OVER=$(
  find . \
    -name '*.md' \
    -not -path '*/node_modules/*' \
    -not -path './vendor/*' \
    -not -path './target/*' \
    -not -path './.git/*' \
    -print0 \
  | xargs -0 wc -l \
  | awk -v limit="$LIMIT" '$1 > limit && $2 != "total" { print $1 "\t" $2 }' \
  | sort -rn
)

if [ -n "$OVER" ]; then
  echo
  echo "✗ Markdown files over the ${LIMIT}-line cap"
  echo "  Split each into focused files, leave a pointer under the old heading,"
  echo "  and link the new pages from the folder's README. See CLAUDE.md ->"
  echo "  'Documentation Expectations'."
  echo
  echo "$OVER" | sed 's/^/    /'
  echo
  exit 1
fi

echo "✓ markdown line cap: every file is ${LIMIT} lines or fewer"
