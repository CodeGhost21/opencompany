#!/usr/bin/env bash
#
# Fail if the tree holds more than one Tauri app, or if a script would launch
# the wrong one.
#
# The Tauri CLI locates a project by scanning SUBFOLDERS of its working
# directory, not ancestors. That makes "which app am I running" a property of
# where the command was typed, and it is invisible in the command itself.
#
# The tree had two: the shell in `src-tauri/`, and a leftover console wrapper in
# `frontend/src-tauri/` that shared its `productName`. Because `tauri:dev` and
# `tauri:build` live in `frontend/package.json`, npm ran them from `frontend/` —
# so `npm run tauri:dev`, the obvious way to start the desktop app, started the
# wrapper. The wrapper registered one command, `desktop_config`, which the
# console had stopped invoking; every `oc_*` command the console uses to find a
# host was missing from its `generate_handler!`. The window opened, the console
# rendered, and no server was ever reachable. Nothing in CI could see it: both
# crates compiled, both were tested, and the lane packaged only the right one.
#
# Two rules, because either alone lets the failure back:
#
#   1. Exactly one `tauri.conf.json`, and it is `src-tauri/tauri.conf.json`. A
#      second app is the ambiguity itself; there is no version of it that is
#      safe just because it is currently correct.
#   2. No `package.json` script invokes a bare `tauri` binary. A bare `tauri`
#      resolves its project from npm's working directory, which is wherever the
#      manifest happens to live — the mechanism above. Scripts must name the
#      directory (`cd ../src-tauri && …`) or delegate to a script that does.
set -uo pipefail

cd "$(dirname "$0")/../.." || exit 1

status=0

# Vendored checkouts own their apps; this rule is about this repository's.
configs=$(
    find . -name tauri.conf.json \
        -not -path './node_modules/*' \
        -not -path '*/node_modules/*' \
        -not -path './vendor/*' \
        -not -path './target/*' \
        -not -path '*/target/*' \
        -not -path './worktrees/*' \
        -not -path './.git/*' |
        sed 's|^\./||' | sort
)

if [ "${configs}" != "src-tauri/tauri.conf.json" ]; then
    echo "assert-single-tauri-app: expected exactly one Tauri app, at src-tauri/tauri.conf.json." >&2
    echo "Found:" >&2
    echo "${configs}" | sed 's/^/    /' >&2
    echo >&2
    echo "The CLI picks a project by scanning subfolders of the working directory," >&2
    echo "so a second app makes 'which app ran' depend on where the command was typed." >&2
    status=1
fi

# `"tauri ` or `"tauri"` at the start of a script value, i.e. the binary invoked
# with no directory established first. `cd ../src-tauri && …/tauri build` and
# `../scripts/desktop-dev.sh` both pass, because both name the directory.
while IFS= read -r manifest; do
    offenders=$(grep -nE '"[^"]*"[[:space:]]*:[[:space:]]*"tauri([[:space:]]|")' "${manifest}")
    if [ -n "${offenders}" ]; then
        echo "assert-single-tauri-app: ${manifest} runs a bare 'tauri' binary:" >&2
        echo "${offenders}" | sed 's/^/    /' >&2
        echo >&2
        echo "npm runs a script from the manifest's own directory, so a bare 'tauri'" >&2
        echo "picks up whatever project sits beneath it. Name the directory instead:" >&2
        echo '    "tauri:build": "npm run build && cd ../src-tauri && ../frontend/node_modules/.bin/tauri build"' >&2
        status=1
    fi
done < <(
    find . -name package.json \
        -not -path './node_modules/*' \
        -not -path '*/node_modules/*' \
        -not -path './vendor/*' \
        -not -path './worktrees/*' \
        -not -path './.git/*'
)

if [ "${status}" -eq 0 ]; then
    echo "assert-single-tauri-app: one Tauri app (src-tauri/), no bare 'tauri' invocations."
fi

exit "${status}"
